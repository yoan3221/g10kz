//! `Tool` trait and `ToolBox` registry.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Object-safe async future alias used by [`Tool::call`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ─── types ───────────────────────────────────────────────────────────────────

/// A parsed function call emitted by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool identifier — must match a registered tool name.
    pub name: String,
    /// JSON arguments from the LLM.
    pub arguments: Value,
}

/// The result of executing a [`ToolCall`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Mirrors the originating [`ToolCall::name`].
    pub name: String,
    /// Serialised result (or error description) to feed back to the LLM.
    pub content: String,
    /// Whether execution succeeded.
    pub success: bool,
}

// ─── Tool trait ──────────────────────────────────────────────────────────────

/// A single callable tool available to the LLM.
pub trait Tool: Send + Sync {
    /// Unique name used in function-calling schema.
    fn name(&self) -> &str;

    /// Natural-language description sent to the LLM.
    fn description(&self) -> &str;

    /// JSON Schema describing the `arguments` object.
    fn schema(&self) -> Value;

    /// Execute with `arguments` and return a [`ToolResult`].
    ///
    /// Must be implemented as `Box::pin(async move { ... })` so the trait
    /// remains object-safe (used via `&dyn Tool` in [`ToolBox`]).
    fn call<'a>(&'a self, call: ToolCall) -> BoxFuture<'a, ToolResult>;
}

// ─── ToolBox ─────────────────────────────────────────────────────────────────

/// Registry of available tools.
///
/// In P5 this includes: `time`, `tw_stock`, `web_search`, `escalate`.
/// The `escalate` tool is the self-escalation escape hatch for the social path.
#[derive(Default)]
pub struct ToolBox {
    tools: Vec<Box<dyn Tool>>,
    /// Maximum iterations in the tool loop before giving up.
    pub max_iterations: usize,
}

impl ToolBox {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            max_iterations: 8,
        }
    }

    /// Register a tool.
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.push(Box::new(tool));
    }


    /// True if no tools have been registered.
    pub fn is_empty(&self) -> bool { self.tools.is_empty() }

    /// Iterator over all registered tools.
    pub fn tools(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.iter().map(|t| t.as_ref())
    }
    /// Find a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    /// JSON schema array for the `tools` field in an OpenAI-compatible request.
    pub fn schema_array(&self) -> Value {
        let schemas: Vec<Value> = self
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.schema(),
                    }
                })
            })
            .collect();
        Value::Array(schemas)
    }

    /// Execute a tool call and return its result.
    ///
    /// Returns an error-flagged `ToolResult` if the tool is not found.
    pub async fn dispatch(&self, call: ToolCall) -> ToolResult {
        match self.get(&call.name) {
            Some(tool) => tool.call(call).await,
            None => {
                let content = format!("tool '{}' not found", call.name);
                ToolResult {
                    name: call.name,
                    content,
                    success: false,
                }
            }
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toolbox_schema_is_empty_array() {
        let tb = ToolBox::new();
        assert_eq!(tb.schema_array(), Value::Array(vec![]));
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error_result() {
        let tb = ToolBox::new();
        let call = ToolCall {
            name: "nonexistent".into(),
            arguments: Value::Null,
        };
        let result = tb.dispatch(call).await;
        assert!(!result.success);
    }
}
