//! EverOS long-term memory sidecar client.
//!
//! L2 — depends on [`g10kz_config`].
//!
//! # Design
//! - `Memory` trait — the only interface the engine sees
//! - `NullMemory` — no-op, always available (used in offline/test runs)
//! - `EverosMemory` — real HTTP client targeting EverOS 1.0 API

use std::future::Future;
use std::pin::Pin;

pub mod memory;

pub use memory::{EverosMemory, MemoryEntry, MemoryResult, NullMemory};

// ─── BoxFuture ───────────────────────────────────────────────────────────────

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ─── Memory trait ────────────────────────────────────────────────────────────

/// Long-term memory interface.  All failures degrade gracefully to empty results.
pub trait Memory: Send + Sync {
    fn search<'a>(
        &'a self,
        user_id: u64,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Vec<MemoryEntry>>;

    fn add<'a>(&'a self, user_id: u64, entry: MemoryEntry) -> BoxFuture<'a, ()>;

    /// Add a full conversation turn (user text + bot reply) as a session.
    /// Default: no-op — override in `EverosMemory`.
    fn add_turn<'a>(
        &'a self,
        _user_id: u64,
        _session_id: &'a str,
        _user_text: &'a str,
        _bot_reply: &'a str,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

// ─── EverosError ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum EverosError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("circuit breaker open")]
    CircuitOpen,
    #[error("EverOS response parse failed: {0}")]
    Parse(String),
}
