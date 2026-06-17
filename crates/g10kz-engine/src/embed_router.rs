//! Semantic route refinement via llama.cpp embedding server.
//!
//! Uses the OpenAI-compatible `/v1/embeddings` endpoint
//! (llama-server started with `--embedding --pooling last`).
//!
//! Key differences vs. the Ollama backend:
//! - **Batch warmup**: 24 examples → 2 HTTP requests (one per class).
//! - **No idle unload**: llama-server keeps the model hot in VRAM permanently.
//! - **Faster startup**: no 27 s cold-start after idle.
//!
//! # Two HTTP clients
//! - `warmup_client` (60 s): initial GPU load at startup
//! - `client` (8 s): per-message refine calls; model already warm

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use serde::{Deserialize, Serialize};

use g10kz_kernel::RouteDecision;

// ─── Tunables ─────────────────────────────────────────────────────────────────

/// Minimum cosine similarity needed to upgrade away from Social.
const THRESHOLD: f32 = 0.72;

// ─── Training examples ───────────────────────────────────────────────────────

const SEARCH_EXAMPLES: &[&str] = &[
    // 即時資料
    "幫我搜尋最新的消息",
    "查一下今天的股價",
    "最新的天氣預報是什麼",
    "現在美元匯率是多少",
    "最近有什麼新聞",
    "今天比特幣多少錢",
    // 明確查詢指令
    "查詢fable5什麼時候可以用",
    "查一下這個遊戲什麼時候出",
    "幫我找找這個軟體的發布日期",
    "搜一下這個模型什麼時候上線",
    "去查查看XXX的最新消息",
    "查查有沒有相關的新聞",
    "幫我查這個東西的資料",
    "找一下這個產品什麼時候發售",
    "查詢最新版本是什麼",
    "幫我搜一下有沒有更新",
    // 發布/上線時間類
    "這個遊戲什麼時候出？",
    "新版本什麼時候發布",
    "什麼時候可以用這個功能",
    "這個什麼時候正式上線",
    // 英文
    "look up the latest news about OpenAI",
    "search for current bitcoin price",
    "what's the weather like today",
    "find me the latest tech news",
    "what time is it in Tokyo right now",
    "current stock price of NVIDIA",
    "when does this game release",
    "search when will X be available",
    "look up the release date for this",
    "find information about this product",
];

const REASON_EXAMPLES: &[&str] = &[
    "分析這個演算法的時間複雜度",
    "解釋量子纏繞的機制是什麼",
    "幫我寫一個Rust的async函式",
    "比較GraphQL和REST API的優缺點",
    "請逐步說明HTTPS的工作原理",
    "幫我debug這段程式碼",
    "explain the pros and cons of microservices",
    "step by step how does HTTPS work",
    "analyze the trade-offs between SQL and NoSQL",
    "write a Rust async function that retries on error",
    "debug this code and tell me what's wrong",
    "compare React and Vue for a large application",
];

// ─── HTTP types (OpenAI-compatible /v1/embeddings) ────────────────────────────

/// Batch request — input is a list of strings.
#[derive(Serialize)]
struct BatchRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

/// Single-string request used during per-message refine().
#[derive(Serialize)]
struct SingleRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

// ─── Centroids ────────────────────────────────────────────────────────────────

struct Centroids {
    search: Vec<f32>,
    reason: Vec<f32>,
}

// ─── EmbeddingRouter ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct EmbeddingRouter {
    client: reqwest::Client,
    warmup_client: reqwest::Client,
    embed_url: String,
    centroids: Arc<RwLock<Option<Centroids>>>,
}

impl EmbeddingRouter {
    /// Create a new router pointed at `embed_base`
    /// (e.g. `"http://localhost:8082"` for llama-server).
    pub fn new(embed_base: &str) -> Self {
        let embed_url = format!("{embed_base}/v1/embeddings");
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .unwrap(),
            warmup_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap(),
            embed_url,
            centroids: Arc::new(RwLock::new(None)),
        }
    }

    /// Spawn a background task that builds per-class centroids.
    /// Returns immediately; `refine()` is a no-op until warmup finishes.
    pub fn spawn_warmup(&self) {
        let router = self.clone();
        tokio::spawn(async move { router.run_warmup().await });
    }

    async fn run_warmup(&self) {
        match self.compute_centroids().await {
            Ok(c) => {
                *self.centroids.write().await = Some(c);
                debug!("embedding router: centroids ready");
            }
            Err(e) => {
                warn!("embedding router warmup failed — {e:#}; keyword routing only");
            }
        }
    }

    async fn compute_centroids(&self) -> anyhow::Result<Centroids> {
        // Two batch requests total: one per class (24 examples → 2 round-trips).
        let search = self.batch_centroid(SEARCH_EXAMPLES).await?;
        let reason = self.batch_centroid(REASON_EXAMPLES).await?;
        Ok(Centroids { search, reason })
    }

    /// Embed all `examples` in one batch request, average into a centroid.
    async fn batch_centroid(&self, examples: &[&str]) -> anyhow::Result<Vec<f32>> {
        let resp = self
            .warmup_client
            .post(&self.embed_url)
            .json(&BatchRequest { model: "embed", input: examples })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;

        let n = resp.data.len();
        anyhow::ensure!(n > 0, "empty batch embedding response");

        let dim = resp.data[0].embedding.len();
        let mut sum = vec![0.0_f32; dim];
        for item in &resp.data {
            for (a, b) in sum.iter_mut().zip(item.embedding.iter()) {
                *a += b;
            }
        }
        Ok(sum.into_iter().map(|x| x / n as f32).collect())
    }

    /// Embed a single string using the short-timeout client (model warm).
    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let resp = self
            .client
            .post(&self.embed_url)
            .json(&SingleRequest { model: "embed", input: text })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;

        resp.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    /// Try to upgrade a `Social` route decision.
    ///
    /// Returns:
    /// - `Some(Search)` — closer to search intent
    /// - `Some(Reason)` — closer to reasoning intent
    /// - `None`         — centroids not ready, server down, or below threshold
    pub async fn refine(&self, text: &str) -> Option<RouteDecision> {
        // ── Keyword fast-path: explicit search/query commands bypass embedding ──
        let lower = text.to_lowercase();
        let search_keywords: &[&str] = &[
            // 明確查詢指令
            "查詢", "搜一下", "搜搜看", "幫我找", "幫我查", "幫我搜",
            "查一查", "找一下", "查看看", "去查", "查查",
            // 操作/教學類（複合詞才觸發，避免誤判）
            "怎麼用", "怎麼開通", "怎麼設定", "怎麼安裝", "怎麼啟用",
            "如何使用", "如何開通", "如何設定", "如何安裝",
            "怎麼申請", "怎麼訂閱", "如何申請",
            "教學", "使用方法", "操作步驟",
            // 英文
            "search for", "look up", "find me", "google",
            "how to use", "how do i", "how to set up", "how to install",
            "tutorial for", "guide for",
        ];
        if search_keywords.iter().any(|kw| lower.contains(kw)) {
            debug!("embed_router: keyword fast-path → Search");
            return Some(RouteDecision::Search);
        }

        let guard = self.centroids.read().await;
        let c = guard.as_ref()?;

        let emb = match self.embed_one(text).await {
            Ok(e) => e,
            Err(e) => {
                debug!("embed_router: embed failed — {e:#}");
                return None;
            }
        };

        let search_sim = cosine_sim(&emb, &c.search);
        let reason_sim = cosine_sim(&emb, &c.reason);
        debug!(search_sim, reason_sim, "embed_router similarities");

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
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { return 0.0; }
    dot / (na * nb)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical() {
        let v = vec![1.0_f32, 0.0, 0.0];
        assert!((cosine_sim(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        assert!(cosine_sim(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![0.0_f32, 0.0];
        let b = vec![1.0_f32, 2.0];
        assert_eq!(cosine_sim(&a, &b), 0.0);
    }

    #[test]
    fn new_does_not_panic() {
        let _ = EmbeddingRouter::new("http://localhost:8082");
    }

    #[tokio::test]
    async fn refine_none_before_warmup() {
        let r = EmbeddingRouter::new("http://localhost:8082");
        assert!(r.refine("hello").await.is_none());
    }
}
