//! Tool trait, `ToolBox` registry, tool loop, and media pre-processing.
//!
//! L2 — depends on [`g10kz_config`] and [`g10kz_llm`].

pub mod builtins;
pub mod media;
pub mod r#loop;
pub mod tool;

pub use tool::{Tool, ToolBox, ToolCall, ToolResult};
pub use r#loop::{run_tool_loop, tool_schema_snippet};
pub use builtins::{EscalateTool, FetchPageTool, TimeTool, TwStockTool, WebSearchTool};

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
