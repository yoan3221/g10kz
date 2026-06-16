//! EverOS long-term memory sidecar client.
//!
//! L2 — depends on [`g10kz_config`].
//!
//! # Design
//! - `Memory` trait — the only interface the engine sees
//! - `NullMemory` — no-op, always available (used in P0/P1 and as degraded fallback)
//! - `EverosMemory` — real HTTP client: search + add, circuit breaker,
//!   800ms timeout, batched-write flush, short TTL search cache

use std::future::Future;
use std::pin::Pin;

pub mod memory;

pub use memory::{EverosMemory, MemoryEntry, MemoryResult, NullMemory};

// ─── BoxFuture ───────────────────────────────────────────────────────────────

/// `Pin<Box<dyn Future<Output = T> + Send + 'a>>` — makes `Memory` object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ─── Memory trait ────────────────────────────────────────────────────────────

/// Long-term memory interface.
///
/// **Object-safe**: use `&dyn Memory` or `Box<dyn Memory>`.
/// All failures degrade gracefully to empty results.
pub trait Memory: Send + Sync {
    fn search<'a>(
        &'a self,
        user_id: u64,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Vec<MemoryEntry>>;

    fn add<'a>(
        &'a self,
        user_id: u64,
        entry: MemoryEntry,
    ) -> BoxFuture<'a, ()>;
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
