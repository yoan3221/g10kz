//! EverOS 1.0 HTTP client — `NullMemory` + `EverosMemory`.
//!
//! # EverOS 1.0 API used
//! | Method | Path                          | Purpose                        |
//! |--------|-------------------------------|--------------------------------|
//! | POST   | /api/v1/memory/search         | Hybrid BM25 + vector retrieval |
//! | POST   | /api/v1/memory/add            | Ingest conversation turn       |
//! | POST   | /api/v1/memory/flush          | Force boundary extraction      |
//!
//! # Search response mapping
//! `data.episodes[].atomic_facts[].content` → `MemoryEntry` (tag = "fact")
//! `data.episodes[].summary`               → `MemoryEntry` (tag = "episode")
//! Results are sorted descending by score and truncated to `limit`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{BoxFuture, Memory};

// ─── Constants ─────────────────────────────────────────────────────────────

const APP_ID:     &str = "default";
const PROJECT_ID: &str = "default";
const AGENT_ID:   &str = "g10kz";

const CIRCUIT_THRESHOLD: u32 = 5;
const CIRCUIT_RESET_SECS: u64 = 30;
const SEARCH_TIMEOUT_MS: u64 = 3000;
const WRITE_TIMEOUT_MS:  u64 = 20000;  // add() only; flush is fire-and-forget
const CACHE_TTL_SECS:    u64 = 30;

// ─── MemoryEntry ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub text:  String,
    #[serde(default)]
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag:   Option<String>,
}

impl MemoryEntry {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), score: 0.0, tag: None }
    }
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

pub type MemoryResult = Vec<MemoryEntry>;

// ─── NullMemory ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct NullMemory;

impl Memory for NullMemory {
    fn search<'a>(&'a self, _uid: u64, _q: &'a str, _lim: usize) -> BoxFuture<'a, Vec<MemoryEntry>> {
        Box::pin(async { vec![] })
    }
    fn add<'a>(&'a self, _uid: u64, _e: MemoryEntry) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn add_turn<'a>(
        &'a self,
        _uid: u64,
        _session_id: &'a str,
        _user_text: &'a str,
        _bot_reply: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

// ─── EverOS 1.0 request / response shapes ───────────────────────────────────

#[derive(Serialize)]
struct SearchReq<'a> {
    user_id:    &'a str,
    app_id:     &'static str,
    project_id: &'static str,
    query:      &'a str,
    top_k:      usize,
}

#[derive(Serialize)]
struct AddReq<'a> {
    session_id:  &'a str,
    app_id:      &'static str,
    project_id:  &'static str,
    messages:    Vec<AddMsg<'a>>,
}

#[derive(Serialize)]
struct AddMsg<'a> {
    sender_id:  &'a str,
    role:       &'static str,
    timestamp:  i64,   // Unix ms
    content:    &'a str,
}

#[derive(Serialize)]
struct FlushReq {
    session_id:  String,
    app_id:      &'static str,
    project_id:  &'static str,
}

// Search response
#[derive(Deserialize, Debug)]
struct SearchResp {
    data: SearchData,
}

#[derive(Deserialize, Debug)]
struct SearchData {
    #[serde(default)]
    episodes: Vec<Episode>,
    #[serde(default)]
    profiles: Vec<ProfileFact>,
}

#[derive(Deserialize, Debug)]
struct Episode {
    summary: String,
    #[serde(default)]
    score: f32,
    #[serde(default)]
    atomic_facts: Vec<AtomicFact>,
}

#[derive(Deserialize, Debug)]
struct AtomicFact {
    content: String,
    #[serde(default)]
    score: f32,
}

#[derive(Deserialize, Debug)]
struct ProfileFact {
    #[serde(default)]
    content: String,
    #[serde(default)]
    score: f32,
}

// ─── Cache ─────────────────────────────────────────────────────────────────

type CacheKey = (u64, String, usize);
struct CacheEntry { results: Vec<MemoryEntry>, expires: Instant }

// ─── EverosMemory ──────────────────────────────────────────────────────────

/// HTTP client for the EverOS 1.0 memory sidecar.
#[derive(Clone)]
pub struct EverosMemory {
    search_client: Arc<reqwest::Client>,
    write_client:  Arc<reqwest::Client>,
    base_url:      Arc<String>,
    failures:      Arc<AtomicU32>,
    last_fail_ts:  Arc<AtomicU32>,
    cache:         Arc<Mutex<HashMap<CacheKey, CacheEntry>>>,
}

impl std::fmt::Debug for EverosMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EverosMemory")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl EverosMemory {
    pub fn new(base_url: impl Into<String>) -> Self {
        let mk = |ms| reqwest::Client::builder()
            .timeout(Duration::from_millis(ms))
            .build()
            .expect("reqwest client build failed");
        Self {
            search_client: Arc::new(mk(SEARCH_TIMEOUT_MS)),
            write_client:  Arc::new(mk(WRITE_TIMEOUT_MS)),
            base_url:      Arc::new(base_url.into()),
            failures:      Arc::new(AtomicU32::new(0)),
            last_fail_ts:  Arc::new(AtomicU32::new(0)),
            cache:         Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn from_config(config: &g10kz_config::Config) -> Self {
        Self::new(&config.everos_url)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.trim_end_matches('/'))
    }

    fn circuit_open(&self) -> bool {
        let f = self.failures.load(Ordering::Relaxed);
        if f < CIRCUIT_THRESHOLD { return false; }
        let last = self.last_fail_ts.load(Ordering::Relaxed) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(last) < CIRCUIT_RESET_SECS
    }
    fn record_ok(&self)   { self.failures.store(0, Ordering::Relaxed); }
    fn record_fail(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.last_fail_ts.store(now, Ordering::Relaxed);
    }

    // ── Public: add a full conversation turn ─────────────────────────────────
    //
    // Sends the user message + bot reply to EverOS, then forces extraction
    // via /flush.  Designed to be called from a tokio::spawn background task.
    pub async fn add_turn(
        &self,
        user_id: u64,
        session_id: &str,
        user_text: &str,
        bot_reply: &str,
    ) {
        if self.circuit_open() {
            debug!("EverOS circuit open — skipping add_turn");
            return;
        }

        let uid_str = user_id.to_string();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let add_body = AddReq {
            session_id,
            app_id:     APP_ID,
            project_id: PROJECT_ID,
            messages: vec![
                AddMsg { sender_id: &uid_str, role: "user",      timestamp: now_ms,     content: user_text },
                AddMsg { sender_id: AGENT_ID, role: "assistant", timestamp: now_ms + 1, content: bot_reply },
            ],
        };

        let add_resp = self.write_client
            .post(self.url("/api/v1/memory/add"))
            .json(&add_body)
            .send()
            .await;

        match add_resp {
            Err(e) => { warn!("EverOS add_turn add error: {e}"); self.record_fail(); return; }
            Ok(r) if !r.status().is_success() => {
                warn!("EverOS add_turn add HTTP {}", r.status());
                self.record_fail(); return;
            }
            Ok(_) => { debug!("EverOS add_turn: accumulated"); }
        }

        // Fire-and-forget flush — EverOS processes extraction asynchronously anyway;
        // we don't need to wait (and waiting risks timeout when EverOS is busy).
        self.record_ok();
        let flush_client = Arc::clone(&self.write_client);
        let flush_url   = self.url("/api/v1/memory/flush");
        let flush_body  = FlushReq {
            session_id:  session_id.to_string(),
            app_id:      APP_ID,
            project_id:  PROJECT_ID,
        };
        tokio::spawn(async move {
            match flush_client.post(&flush_url).json(&flush_body).send().await {
                Err(e) => debug!("EverOS flush fire-and-forget error: {e}"),
                Ok(r) if !r.status().is_success() => {
                    debug!("EverOS flush HTTP {} (non-fatal)", r.status());
                }
                Ok(_) => debug!("EverOS flush ok"),
            }
        });
    }
}

// ─── impl Memory ───────────────────────────────────────────────────────────

impl Memory for EverosMemory {
    fn search<'a>(
        &'a self,
        user_id: u64,
        query:   &'a str,
        limit:   usize,
    ) -> BoxFuture<'a, Vec<MemoryEntry>> {
        Box::pin(async move {
            if self.circuit_open() {
                debug!("EverOS circuit open — skipping search");
                return vec![];
            }

            let key: CacheKey = (user_id, query.to_string(), limit);
            {
                let mut c = self.cache.lock().unwrap();
                if let Some(e) = c.get(&key) {
                    if e.expires > Instant::now() {
                        debug!("EverOS search cache hit");
                        return e.results.clone();
                    }
                    c.remove(&key);
                }
            }

            let uid_str = user_id.to_string();
            let body = SearchReq {
                user_id:    &uid_str,
                app_id:     APP_ID,
                project_id: PROJECT_ID,
                query,
                top_k: limit * 2,   // over-fetch; we trim after merging
            };

            let resp = match self.search_client
                .post(self.url("/api/v1/memory/search"))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("EverOS search error: {e}");
                    self.record_fail();
                    return vec![];
                }
            };

            if !resp.status().is_success() {
                warn!("EverOS search HTTP {}", resp.status());
                self.record_fail();
                return vec![];
            }

            let parsed: SearchResp = match resp.json().await {
                Ok(p) => p,
                Err(e) => {
                    warn!("EverOS search parse error: {e}");
                    self.record_fail();
                    return vec![];
                }
            };

            self.record_ok();

            // Flatten: atomic facts (most specific) + episode summaries + profiles
            let mut entries: Vec<MemoryEntry> = Vec::new();

            for ep in &parsed.data.episodes {
                for af in &ep.atomic_facts {
                    if !af.content.is_empty() {
                        entries.push(MemoryEntry {
                            text:  af.content.clone(),
                            score: af.score,
                            tag:   Some("fact".into()),
                        });
                    }
                }
                // Episode summary as broader context
                if !ep.summary.is_empty() {
                    entries.push(MemoryEntry {
                        text:  ep.summary.clone(),
                        score: ep.score * 0.8,   // de-prioritise vs facts
                        tag:   Some("episode".into()),
                    });
                }
            }

            for p in &parsed.data.profiles {
                if !p.content.is_empty() {
                    entries.push(MemoryEntry {
                        text:  p.content.clone(),
                        score: p.score,
                        tag:   Some("profile".into()),
                    });
                }
            }

            // Sort descending by score, take top `limit`
            entries.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            entries.truncate(limit);

            let expires = Instant::now() + Duration::from_secs(CACHE_TTL_SECS);
            self.cache.lock().unwrap().insert(key, CacheEntry { results: entries.clone(), expires });

            debug!(count = entries.len(), "EverOS search ok");
            entries
        })
    }

    /// Legacy single-entry add — wraps as a single-message session.
    fn add<'a>(&'a self, user_id: u64, entry: MemoryEntry) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let session = format!("g10kz-legacy-{user_id}");
            self.add_turn(user_id, &session, &entry.text, "").await;
        })
    }

    fn add_turn<'a>(
        &'a self,
        user_id:   u64,
        session_id: &'a str,
        user_text: &'a str,
        bot_reply: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(self.add_turn(user_id, session_id, user_text, bot_reply))
    }
}

// ─── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_memory_search_empty() {
        assert!(NullMemory.search(1, "q", 5).await.is_empty());
    }
    #[tokio::test]
    async fn null_memory_add_noop() {
        NullMemory.add(1, MemoryEntry::new("x")).await;
    }
    #[tokio::test]
    async fn null_memory_add_turn_noop() {
        NullMemory.add_turn(1, "s", "u", "b").await;
    }

    #[test]
    fn circuit_starts_closed() {
        let m = EverosMemory::new("http://127.0.0.1:19900");
        assert!(!m.circuit_open());
    }
    #[test]
    fn circuit_opens_after_failures() {
        let m = EverosMemory::new("http://127.0.0.1:19900");
        for _ in 0..CIRCUIT_THRESHOLD { m.record_fail(); }
        assert!(m.circuit_open());
    }
    #[test]
    fn circuit_resets_after_success() {
        let m = EverosMemory::new("http://127.0.0.1:19900");
        for _ in 0..CIRCUIT_THRESHOLD { m.record_fail(); }
        m.record_ok();
        assert!(!m.circuit_open());
    }
    #[tokio::test]
    async fn search_returns_empty_when_unreachable() {
        let m = EverosMemory::new("http://127.0.0.1:19900");
        assert!(m.search(1, "test", 5).await.is_empty());
    }
    #[test]
    fn memory_entry_new() {
        let e = MemoryEntry::new("hello").with_tag("fact");
        assert_eq!(e.tag.as_deref(), Some("fact"));
    }
}
