//! Tool trait, `ToolBox` registry, tool loop, and media pre-processing.
//!
//! L2 — depends on [`g10kz_config`] and [`g10kz_llm`].

pub mod builtins;
pub mod r#loop;
pub mod media;
pub mod tool;

pub use builtins::{EscalateTool, TimeTool, TwStockTool, WebSearchTool};
pub use r#loop::{run_tool_loop, tool_schema_snippet};
pub use tool::{Tool, ToolBox, ToolCall, ToolResult};

/// Errors from tool execution.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("tool execution failed: {0}")]
    Execution(String),

    #[error("max iterations reached")]
    MaxIterations,

    #[error("media processing error: {0}")]
    Media(String),
}
