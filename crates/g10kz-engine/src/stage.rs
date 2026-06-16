//! `Stage` enum — the nodes of the per-turn state machine.

/// State machine stages for a single turn.
///
/// Transitions are driven by pure predicates in [`g10kz_kernel::route`].
/// Full state machine wired in **P6**; P0/P1 only uses `Social`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Context gathering (history + memory + group ring buffer).
    Gather,
    /// Pre-turn guard (injection defense, blacklist, owner check).
    Guard,
    /// Text normalisation.
    Normalize,
    /// Routing decision.
    Route,
    /// Conversational reply — single LLM call, streamed.
    Social,
    /// Web search tool invocation.
    Search,
    /// Media pre-processing (image / video / audio).
    Media,
    /// Reasoning path — tool loop + Fusion synthesis.
    Reason,
    /// Post-LLM sanitisation and leak detection.
    Sanitize,
    /// Format normalisation + render to string.
    Render,
    /// Background persist (EverOS write, conversation history update).
    Persist,
    /// Terminal: reply delivered.
    Done,
    /// Terminal: rejected (canned response).
    Rejected,
}
