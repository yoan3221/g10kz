//! `Memory` trait implementations: `NullMemory` and `EverosMemory`.
//!
//! # EverosMemory features
//! - reqwest client with **800ms timeout** for search calls
//! - **Circuit breaker**: opens after 3 consecutive failures; half-open probe after 30s
//! - **Search TTL cache**: 30-second in-process cache keyed by (user_id, query, limit)
//! - **Batched writes**: accumulates up to 8 entries, then spawns a background flush task

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{BoxFuture, Memory};

// ─── Constants ───────────────────────────────────────────────────────────────

const CIRCUIT_OPEN_THRESHOLD: u32 = 3;
const CIRCUIT_RESET_SECS: u64 = 30;
const SEARCH_TIMEOUT_MS: u64 = 800;
const CACHE_TTL_SECS: u64 = 30;
const WRITE_BATCH_MAX: usize = 8;

// ─── MemoryEntry ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub text: String,
    #[serde(default)]
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl MemoryEntry {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            score: 0.0,
            tag: None,
        }
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

pub type MemoryResult = Vec<MemoryEntry>;

// ─── NullMemory ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct NullMemory;

impl Memory for NullMemory {
    fn search<'a>(
        &'a self,
        _uid: u64,
        _q: &'a str,
        _lim: usize,
    ) -> BoxFuture<'a, Vec<MemoryEntry>> {
        Box::pin(async { vec![] })
    }
    fn add<'a>(&'a self, _uid: u64, _e: MemoryEntry) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

// ─── Internal cache / write-buffer types ─────────────────────────────────────

type CacheKey = (u64, String, usize);

struct CacheEntry {
    results: Vec<MemoryEntry>,
    expires: Instant,
}

// ─── EverosMemory ─────────────────────────────────────────────────────────────

/// HTTP client for the EverOS memory sidecar.
#[derive(Clone)]
pub struct EverosMemory {
    client: Arc<reqwest::Client>,
    base_url: Arc<String>,
    failures: Arc<AtomicU32>,
    last_failure_ts: Arc<AtomicU32>,
    /// Short-lived search result cache. Eviction is lazy (checked on access).
    cache: Arc<Mutex<HashMap<CacheKey, CacheEntry>>>,
    /// Accumulates pending writes until WRITE_BATCH_MAX is reached.
    write_buf: Arc<Mutex<Vec<(u64, MemoryEntry)>>>,
}

impl std::fmt::Debug for EverosMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EverosMemory")
            .field("base_url", &self.base_url)
            .field("failures", &self.failures.load(Ordering::Relaxed))
            .finish()
    }
}

impl EverosMemory {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(SEARCH_TIMEOUT_MS))
            .build()
            .expect("reqwest client build failed");
        Self {
            client: Arc::new(client),
            base_url: Arc::new(base_url.into()),
            failures: Arc::new(AtomicU32::new(0)),
            last_failure_ts: Arc::new(AtomicU32::new(0)),
            cache: Arc::new(Mutex::new(HashMap::new())),
            write_buf: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn from_config(config: &g10kz_config::Config) -> Self {
        Self::new(&config.everos_url)
    }

    pub fn circuit_open(&self) -> bool {
        let f = self.failures.load(Ordering::Relaxed);
        if f < CIRCUIT_OPEN_THRESHOLD {
            return false;
        }
        let last = self.last_failure_ts.load(Ordering::Relaxed) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(last) < CIRCUIT_RESET_SECS
    }

    fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }

    fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.last_failure_ts.store(now, Ordering::Relaxed);
    }

    /// Drain write buffer and POST to EverOS. Returns count flushed.
    pub async fn flush_pending_writes(&self) -> usize {
        let items = {
            let mut buf = self.write_buf.lock().unwrap();
            std::mem::take(&mut *buf)
        };
        if items.is_empty() {
            return 0;
        }
        let n = items.len();
        flush_batch(self.client.clone(), self.base_url.clone(), items).await;
        n
    }

    /// Number of writes currently buffered (for testing).
    pub fn pending_write_count(&self) -> usize {
        self.write_buf.lock().unwrap().len()
    }
}

// ─── EverOS HTTP helpers ──────────────────────────────────────────────────────

/// POST the batch to {base_url}/memory/batch (fire-and-forget).
async fn flush_batch(
    client: Arc<reqwest::Client>,
    base_url: Arc<String>,
    items: Vec<(u64, MemoryEntry)>,
) {
    let url = format!("{}/memory/batch", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "items": items.iter().map(|(uid, e)| serde_json::json!({
            "user_id": uid,
            "text":    e.text,
            "tag":     e.tag,
            "score":   e.score,
        })).collect::<Vec<_>>()
    });
    match client.post(&url).json(&body).send().await {
        Ok(r) if r.status().is_success() => {
            debug!(count = items.len(), "EverOS batch flushed");
        }
        Ok(r) => warn!("EverOS batch HTTP {}", r.status()),
        Err(e) => warn!("EverOS batch flush error: {e}"),
    }
}

/// EverOS search response shape.
#[derive(Deserialize)]
struct SearchResponse {
    #[serde(alias = "results", alias = "data")]
    memories: Vec<MemoryEntry>,
}

// ─── impl Memory ─────────────────────────────────────────────────────────────

impl Memory for EverosMemory {
    fn search<'a>(
        &'a self,
        user_id: u64,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Vec<MemoryEntry>> {
        Box::pin(async move {
            if self.circuit_open() {
                debug!("EverOS circuit open, skipping search");
                return vec![];
            }

            let key: CacheKey = (user_id, query.to_string(), limit);

            // Cache lookup (drop lock before await)
            {
                let mut cache = self.cache.lock().unwrap();
                if let Some(e) = cache.get(&key) {
                    if e.expires > Instant::now() {
                        debug!("EverOS cache hit");
                        return e.results.clone();
                    }
                    cache.remove(&key);
                }
            }

            let url = format!("{}/memory/search", self.base_url.trim_end_matches('/'));
            let body = serde_json::json!({ "user_id": user_id, "query": query, "limit": limit });

            let resp = match self.client.post(&url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("EverOS search error: {e}");
                    self.record_failure();
                    return vec![];
                }
            };

            if !resp.status().is_success() {
                warn!("EverOS search HTTP {}", resp.status());
                self.record_failure();
                return vec![];
            }

            match resp.json::<SearchResponse>().await {
                Ok(sr) => {
                    self.record_success();
                    let results = sr.memories;
                    let expires = Instant::now() + Duration::from_secs(CACHE_TTL_SECS);
                    self.cache.lock().unwrap().insert(
                        key,
                        CacheEntry {
                            results: results.clone(),
                            expires,
                        },
                    );
                    results
                }
                Err(e) => {
                    warn!("EverOS search parse error: {e}");
                    self.record_failure();
                    vec![]
                }
            }
        })
    }

    fn add<'a>(&'a self, user_id: u64, entry: MemoryEntry) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // Push to buffer; if full, drain and spawn background flush.
            let maybe_flush = {
                let mut buf = self.write_buf.lock().unwrap();
                buf.push((user_id, entry));
                if buf.len() >= WRITE_BATCH_MAX {
                    Some(std::mem::take(&mut *buf))
                } else {
                    None
                }
            };

            if let Some(items) = maybe_flush {
                let client = self.client.clone();
                let base_url = self.base_url.clone();
                tokio::spawn(async move {
                    flush_batch(client, base_url, items).await;
                });
            }
        })
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_everos() -> EverosMemory {
        EverosMemory::new("http://127.0.0.1:19900") // no server — tests degradation
    }

    // ── NullMemory ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn null_memory_returns_empty() {
        assert!(NullMemory.search(1, "query", 5).await.is_empty());
    }

    #[tokio::test]
    async fn null_memory_add_is_noop() {
        NullMemory.add(1, MemoryEntry::new("test")).await;
    }

    // ── Circuit breaker ───────────────────────────────────────────────────────

    #[test]
    fn circuit_starts_closed() {
        assert!(!make_everos().circuit_open());
    }

    #[test]
    fn circuit_opens_after_threshold() {
        let m = make_everos();
        for _ in 0..CIRCUIT_OPEN_THRESHOLD {
            m.record_failure();
        }
        assert!(m.circuit_open());
    }

    #[test]
    fn circuit_closes_after_success() {
        let m = make_everos();
        for _ in 0..CIRCUIT_OPEN_THRESHOLD {
            m.record_failure();
        }
        m.record_success();
        assert!(!m.circuit_open());
    }

    // ── Degradation ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_returns_empty_when_unreachable() {
        let m = make_everos();
        let r = m.search(1, "test query", 5).await;
        assert!(r.is_empty());
        assert!(
            m.failures.load(Ordering::Relaxed) > 0,
            "should have recorded failure"
        );
    }

    #[tokio::test]
    async fn search_returns_empty_when_circuit_open() {
        let m = make_everos();
        for _ in 0..CIRCUIT_OPEN_THRESHOLD {
            m.record_failure();
        }
        assert!(m.circuit_open());
        let r = m.search(1, "test", 5).await;
        assert!(r.is_empty());
        // Failures should NOT have increased (circuit short-circuited)
        assert_eq!(m.failures.load(Ordering::Relaxed), CIRCUIT_OPEN_THRESHOLD);
    }

    // ── Write batching ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_buffers_writes_below_batch_max() {
        let m = make_everos();
        for i in 0..(WRITE_BATCH_MAX - 1) {
            m.add(1, MemoryEntry::new(format!("entry {i}"))).await;
        }
        assert_eq!(m.pending_write_count(), WRITE_BATCH_MAX - 1);
    }

    #[tokio::test]
    async fn add_flushes_at_batch_max() {
        let m = make_everos();
        for i in 0..WRITE_BATCH_MAX {
            m.add(1, MemoryEntry::new(format!("entry {i}"))).await;
        }
        // Buffer should be empty (spawned flush task drained it)
        assert_eq!(m.pending_write_count(), 0);
    }

    // ── MemoryEntry helpers ───────────────────────────────────────────────────

    #[test]
    fn memory_entry_with_tag() {
        let e = MemoryEntry::new("hello").with_tag("fact");
        assert_eq!(e.tag.as_deref(), Some("fact"));
    }

    #[test]
    fn memory_entry_serde_roundtrip() {
        let e = MemoryEntry {
            text: "abc".into(),
            score: 0.9,
            tag: Some("event".into()),
        };
        let json = serde_json::to_string(&e).unwrap();
        let e2: MemoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(e2.text, "abc");
        assert!((e2.score - 0.9).abs() < 0.001);
    }
}
