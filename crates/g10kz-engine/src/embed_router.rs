//! Semantic route refinement using Ollama local embedding model.
//!
//! Runs **after** the cheap pure-function router in `turn.rs`.
//! Only consulted when the pure router falls through to `Social`.
//! Upgrades to `Search` or `Reason` when cosine similarity to the
//! class centroid exceeds [`THRESHOLD`].
//!
//! # Graceful degradation
//! If Ollama is unavailable (not started, busy, wrong URL) `refine()`
//! returns `None` and the caller keeps the `Social` decision unchanged.
//! There is no panic, no error log spam — just a single `warn!` at warmup.
//!
//! # Warmup
//! Call [`EmbeddingRouter::spawn_warmup`] once at startup.  It spawns a
//! background task that embeds a small set of labelled examples and averages
//! them into per-class centroids.  Until warmup completes, `refine()` is
//! a no-op (returns `None`).  Typical warmup time: ~100 ms.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use serde::{Deserialize, Serialize};

use g10kz_kernel::RouteDecision;

// ─── Tunables ─────────────────────────────────────────────────────────────────

const MODEL: &str = "qwen3-embedding:0.6b";

/// Minimum cosine similarity needed to upgrade away from Social.
/// Tune down toward 0.65 for more aggressive routing; up toward 0.80
/// for fewer false upgrades.
const THRESHOLD: f32 = 0.72;

// ─── Training examples (used to compute centroids at startup) ─────────────────

const SEARCH_EXAMPLES: &[&str] = &[
    // zh
    "幫我搜尋最新的消息",
    "查一下今天的股價",
    "最新的天氣預報是什麼",
    "現在美元匯率是多少",
    "最近有什麼新聞",
    "今天比特幣多少錢",
    // en
    "look up the latest news about OpenAI",
    "search for current bitcoin price",
    "what's the weather like today",
    "find me the latest tech news",
    "what time is it in Tokyo right now",
    "current stock price of NVIDIA",
];

const REASON_EXAMPLES: &[&str] = &[
    // zh
    "分析這個演算法的時間複雜度",
    "解釋量子纏繞的機制是什麼",
    "幫我寫一個Rust的async函式",
    "比較GraphQL和REST API的優缺點",
    "請逐步說明HTTPS的工作原理",
    "幫我debug這段程式碼",
    // en
    "explain the pros and cons of microservices",
    "step by step how does HTTPS work",
    "analyze the trade-offs between SQL and NoSQL",
    "write a Rust async function that retries on error",
    "debug this code and tell me what's wrong",
    "compare React and Vue for a large application",
];

// ─── HTTP types ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

// ─── Centroids ────────────────────────────────────────────────────────────────

struct Centroids {
    search: Vec<f32>,
    reason: Vec<f32>,
}

// ─── EmbeddingRouter ──────────────────────────────────────────────────────────

/// Semantic upgrade layer over the pure-function keyword router.
///
/// Clone is cheap (`Arc` inside).
#[derive(Clone)]
pub struct EmbeddingRouter {
    client: reqwest::Client,
    embed_url: String,
    centroids: Arc<RwLock<Option<Centroids>>>,
}

impl EmbeddingRouter {
    /// Create a new router pointed at `ollama_base` (e.g. `"http://localhost:11434"`).
    /// Call [`spawn_warmup`] afterwards.
    pub fn new(ollama_base: &str) -> Self {
        let embed_url = format!("{ollama_base}/api/embeddings");
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap(),
            embed_url,
            centroids: Arc::new(RwLock::new(None)),
        }
    }

    /// Spawn a background task to compute class centroids.
    /// Returns immediately; `refine()` is a no-op until the task completes.
    pub fn spawn_warmup(&self) {
        let router = self.clone();
        tokio::spawn(async move {
            router.run_warmup().await;
        });
    }

    async fn run_warmup(&self) {
        match self.compute_centroids().await {
            Ok(c) => {
                *self.centroids.write().await = Some(c);
                debug!("embedding router: centroids ready (search + reason)");
            }
            Err(e) => {
                warn!("embedding router warmup failed — {e:#}; falling back to keyword routing");
            }
        }
    }

    async fn compute_centroids(&self) -> anyhow::Result<Centroids> {
        let search = self.centroid(SEARCH_EXAMPLES).await?;
        let reason = self.centroid(REASON_EXAMPLES).await?;
        Ok(Centroids { search, reason })
    }

    /// Average the embeddings of all `examples` into one centroid vector.
    async fn centroid(&self, examples: &[&str]) -> anyhow::Result<Vec<f32>> {
        let mut sum: Vec<f32> = Vec::new();
        for ex in examples {
            let v = self.embed(ex).await?;
            if sum.is_empty() {
                sum = v;
            } else {
                for (a, b) in sum.iter_mut().zip(v.iter()) {
                    *a += b;
                }
            }
        }
        let n = examples.len() as f32;
        Ok(sum.into_iter().map(|x| x / n).collect())
    }

    /// Call Ollama embeddings endpoint for a single `text`.
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let resp = self
            .client
            .post(&self.embed_url)
            .json(&EmbedRequest {
                model: MODEL,
                prompt: text,
            })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;
        Ok(resp.embedding)
    }

    /// Try to upgrade a `Social` route decision.
    ///
    /// Returns:
    /// - `Some(RouteDecision::Search)` — message is semantically closer to search intent
    /// - `Some(RouteDecision::Reason)` — message is semantically closer to reasoning intent
    /// - `None` — centroids not ready, Ollama unreachable, or similarity below threshold
    pub async fn refine(&self, text: &str) -> Option<RouteDecision> {
        let guard = self.centroids.read().await;
        let c = guard.as_ref()?;

        let embedding = match self.embed(text).await {
            Ok(e) => e,
            Err(e) => {
                debug!("embed_router: embed failed — {e:#}");
                return None;
            }
        };

        let search_sim = cosine_sim(&embedding, &c.search);
        let reason_sim = cosine_sim(&embedding, &c.reason);

        debug!(search_sim, reason_sim, "embed_router similarities");

        // Only upgrade when at least one class clears the threshold.
        if search_sim < THRESHOLD && reason_sim < THRESHOLD {
            return None;
        }

        if search_sim >= reason_sim {
            Some(RouteDecision::Search)
        } else {
            Some(RouteDecision::Reason)
        }
    }
}

// ─── Math ─────────────────────────────────────────────────────────────────────

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0_f32, 0.0, 0.0];
        assert!((cosine_sim(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        assert!(cosine_sim(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_is_zero() {
        let a = vec![0.0_f32, 0.0];
        let b = vec![1.0_f32, 2.0];
        assert_eq!(cosine_sim(&a, &b), 0.0);
    }

    #[test]
    fn embed_router_new_does_not_panic() {
        let _ = EmbeddingRouter::new("http://localhost:11434");
    }

    #[tokio::test]
    async fn refine_returns_none_before_warmup() {
        // Centroids not set — should return None gracefully.
        let router = EmbeddingRouter::new("http://localhost:11434");
        assert!(router.refine("hello").await.is_none());
    }
}
