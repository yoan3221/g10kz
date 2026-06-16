//! Per-turn structured tracing span.
//!
//! Emits a JSON span on turn completion containing:
//!   path, latency_ms, prompt_tokens, completion_tokens, cache_hit,
//!   memory_hit, stages_visited, degraded.
//!
//! Replaces LangGraph Studio — replay via `/trace` slash command in P7.

use std::time::Instant;

use serde::Serialize;
use tracing::info;

use crate::stage::Stage;

/// Accumulated metrics for a single turn.
#[derive(Debug, Default, Serialize)]
pub struct TurnTrace {
    pub path: String,
    pub latency_ms: u64,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub cache_hit: bool,
    pub memory_hit: bool,
    pub stages: Vec<String>,
    /// True if any subsystem (EverOS, search, Fusion) fell back to degraded mode.
    pub degraded: bool,
}

/// RAII guard that records latency and emits the span on drop.
pub struct TurnTracer {
    start: Instant,
    pub trace: TurnTrace,
}

impl TurnTracer {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            start: Instant::now(),
            trace: TurnTrace {
                path: path.into(),
                ..Default::default()
            },
        }
    }

    pub fn enter_stage(&mut self, stage: &Stage) {
        self.trace.stages.push(format!("{stage:?}"));
    }

    /// Emit the structured span.  Called automatically on drop.
    pub fn finish(&mut self) {
        self.trace.latency_ms = self.start.elapsed().as_millis() as u64;
        // Emit as a structured tracing event so it's picked up by JSON subscriber.
        info!(
            path = %self.trace.path,
            ms = self.trace.latency_ms,
            ptok = self.trace.prompt_tokens,
            ctok = self.trace.completion_tokens,
            cost = self.trace.cost_usd,
            cache = self.trace.cache_hit,
            memory = self.trace.memory_hit,
            degraded = self.trace.degraded,
            "[turn]"
        );
    }
}

impl Drop for TurnTracer {
    fn drop(&mut self) {
        self.finish();
    }
}
