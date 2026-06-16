//! LLM provider abstraction, OpenRouter client, and Fusion synthesis.
//!
//! L1 — depends only on [`g10kz_config`].
//!
//! # Modules
//! | Module | Purpose |
//! |---|---|
//! | [`types`]       | `Message`, `Part`, `CompletionParams`, `Usage` |
//! | [`provider`]    | `Provider` trait (object-safe via `BoxFuture`) |
//! | [`mock`]        | `MockProvider` — scripted, round-robin, deterministic |
//! | [`serialize`]   | OpenAI-compatible JSON request/response types |
//! | [`openrouter`]  | HTTP client with retry, circuit breaker, cancellation |
//! | [`fusion`]      | Multi-model fan-out + quorum + consensus + judge |

pub mod fusion;
pub mod mock;
pub mod openrouter;
pub mod provider;
pub mod serialize;
pub mod types;

// ─── re-exports ──────────────────────────────────────────────────────────────

pub use fusion::{all_drafts_agree, fusion_complete, jaccard_similarity, FusionConfig};
pub use mock::MockProvider;
pub use openrouter::OpenRouterProvider;
pub use provider::{BoxFuture, Provider};
pub use types::{CompletionParams, Message, Part, Role, Usage};

// ─── error type ──────────────────────────────────────────────────────────────

/// Errors from LLM operations.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("provider request failed: {0}")]
    Request(String),

    #[error("context window exceeded")]
    ContextOverflow,

    #[error("all providers failed or timed out (exhausted)")]
    Exhausted,

    #[error("cancelled")]
    Cancelled,
}
