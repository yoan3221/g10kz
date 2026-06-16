//! Per-turn state machine.
//!
//! L3 — integrates g10kz-kernel, g10kz-llm, g10kz-everos, g10kz-tools.
//!
//! # Modules
//! - [`turn`]   — `TurnInput`, `TurnOutput`, `run_turn` entry point
//! - [`stage`]  — `Stage` enum (state machine nodes)
//! - [`tracer`] — per-turn structured tracing span
//!
//! # Turn flow (see DECISIONS.md §Data flow per turn)
//! ```text
//! gather (3-way parallel: history | memory | group_ctx)
//!   → guard  (pure, fast)
//!   → normalize
//!   → route  (pure predicates)
//!   ↓
//!   social path  →  [1 LLM call, streamed]
//!   search path  →  search tool → social reply
//!   media path   →  media pre-proc → social reply
//!   reason path  →  tool loop → Fusion synthesis
//!   ↓
//! sanitize → render → persist (background)
//! ```

pub mod stage;
pub mod tracer;
pub mod turn;

pub use stage::Stage;
pub use turn::{run_turn, TurnInput, TurnOutput};

// ─── error type ──────────────────────────────────────────────────────────────

/// Errors produced by the engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("turn cancelled")]
    Cancelled,

    #[error("LLM call failed: {0}")]
    Llm(#[from] anyhow::Error),

    #[error("guard rejected: {reason}")]
    Rejected { reason: String },
}
