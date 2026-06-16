//! Pure-function validation kernel — no I/O (except persona card loading at init),
//! deterministic, exhaustively tested.
//!
//! L1 — depends only on [`g10kz_config`].
//!
//! # Modules
//! | Module | Purpose |
//! |---|---|
//! | [`normalize`] | Text normalisation pipeline (NFKC, homoglyphs, zero-width, full-width) |
//! | [`guard`]     | Pre-turn gate: injection defense, owner bypass, blacklist |
//! | [`reject`]    | Canned in-character response pool keyed by reject reason |
//! | [`persona`]   | SillyTavern V2 character card loader |
//! | [`route`]     | Cheap pure-function routing predicates |
//! | [`sanitize`]  | Post-LLM leak detection, anti-repetition, format normalisation |
//! | [`proactive`] | Proactive messaging decision (clock/RNG injected) |

pub mod guard;
pub mod normalize;
pub mod persona;
pub mod proactive;
pub mod reject;
pub mod route;
pub mod sanitize;

// ─── re-exports ──────────────────────────────────────────────────────────────

pub use guard::{pre_guard, GuardVerdict, RejectReason};
pub use normalize::{normalize_for_scan, normalize_input};
pub use persona::PersonaCard;
pub use proactive::should_send_proactive;
pub use reject::canned_response;
pub use route::{route, RouteDecision};
pub use sanitize::{format_output, sanitize_output, SanitizeResult};

// ─── s
// ─── error type ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    #[error("persona parse failed: {0}")]
    PersonaParse(String),
}
