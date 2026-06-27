//! Agentic tool loop — XML-framed tool calls.
//!
//! The LLM is instructed to use `<tool_call>JSON</tool_call>` markup.
//! The engine parses the call, dispatches via [`ToolBox`], and injects
//! `<tool_result>` back into the conversation.
//!
//! Terminates on:
//! - A reply with no `<tool_call>` (done)
//! - `max_iterations` rounds exhausted (returns last text)
//! - An `ESCALATE` sentinel from [`crate::builtins::EscalateTool`]

use tracing::{debug, warn};

use g10kz_llm::{
    provider::Provider,
    types::{CompletionParams, Message, Part, Role, Usage},
};

use crate::tool::{ToolBox, ToolCall};

const TOOL_CALL_OPEN: &str = "<tool_call>";
const TOOL_CALL_CLOSE: &str = "</tool_call>";

// ─── Public API ──────────────────────────────────────────────────────────────

/// Run the agentic tool loop.
///
/// Prepend `tool_schema_snippet(toolbox)` to the system prompt **before**
/// calling this function so the LLM knows which tools are available.
pub async fn run_tool_loop(
    provider: &dyn Provider,
    toolbox: &ToolBox,
    mut messages: Vec<Message>,
    params: &CompletionParams,
) -> anyhow::Result<(String, Usage)> {
    let mut total = Usage::default();
    let mut last_text = String::new();

    for iteration in 0..toolbox.max_iterations {
        let (reply, usage) = provider.complete(&messages, params).await?;
        total.prompt_tokens += usage.prompt_tokens;
        total.completion_tokens += usage.completion_tokens;
        total.cost_usd += usage.cost_usd;
        last_text = reply.clone();

        // Done — no tool call tag
        let Some(call) = parse_tool_call(&reply) else {
            debug!(iteration, "tool loop: no tool call, done");
            return Ok((reply, total));
        };

        debug!(iteration, tool = %call.name, "tool loop: dispatching");

        // ESCALATE sentinel — signal caller to switch path
        if call.name == "escalate" {
            debug!("tool loop: escalate signal");
            return Ok(("ESCALATE".into(), total));
        }

        // Execute
        let result = toolbox.dispatch(call.clone()).await;

        // Append assistant turn (text up to the tag) + tool result injected as user turn.
        let text_before = text_before_tool_call(&reply);
        messages.push(Message::text(Role::Assistant, text_before));
        let result_text = format!(
            "<tool_result name=\"{}\">\n{}\n</tool_result>\n\nContinue your answer based on the result above.",
            result.name, result.content
        );
        if result.images.is_empty() {
            messages.push(Message::text(Role::User, result_text));
        } else {
            // Vision tool (e.g. view_page): feed the screenshot(s) back as image
            // parts so a multimodal model can actually see the page.
            let mut parts: Vec<Part> = result
                .images
                .iter()
                .cloned()
                .map(|url| Part::ImageUrl { url })
                .collect();
            parts.push(Part::Text { text: result_text });
            messages.push(Message { role: Role::User, parts });
        }

        if !result.success {
            warn!(tool = %result.name, "tool returned failure");
        }
    }

    warn!("tool loop: max_iterations reached, returning last reply");
    Ok((last_text, total))
}

/// Build the tool-schema block to append to the system prompt.
pub fn tool_schema_snippet(toolbox: &ToolBox) -> String {
    if toolbox.is_empty() {
        return String::new();
    }

    let entries: Vec<String> = toolbox
        .tools()
        .map(|t| format!("- `{}`: {}", t.name(), t.description()))
        .collect();

    format!(
        "\n\n## 工具（一次一個）\n{}\n\
         用 <tool_call>{{\"name\":\"...\",\"arguments\":{{...}}}}</tool_call>，結果見 <tool_result> 後繼續。",
        entries.join("\n")
    )
}

// ─── Parsing ─────────────────────────────────────────────────────────────────

fn parse_tool_call(text: &str) -> Option<ToolCall> {
    let start = text.find(TOOL_CALL_OPEN)? + TOOL_CALL_OPEN.len();
    let end = text[start..].find(TOOL_CALL_CLOSE)? + start;
    let json_str = text[start..end].trim();
    match serde_json::from_str(json_str) {
        Ok(c) => Some(c),
        Err(e) => {
            warn!("tool_call parse error: {e}  raw={json_str}");
            None
        }
    }
}

fn text_before_tool_call(text: &str) -> &str {
    match text.find(TOOL_CALL_OPEN) {
        Some(pos) => text[..pos].trim_end(),
        None => text.trim(),
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_tool_call() {
        let text = r#"Let me search. <tool_call>{"name":"web_search","arguments":{"query":"rust"}}</tool_call>"#;
        let call = parse_tool_call(text).unwrap();
        assert_eq!(call.name, "web_search");
    }

    #[test]
    fn parse_no_tag_returns_none() {
        assert!(parse_tool_call("No tool call here.").is_none());
    }

    #[test]
    fn parse_malformed_json_returns_none() {
        let text = "<tool_call>{bad json}</tool_call>";
        assert!(parse_tool_call(text).is_none());
    }

    #[test]
    fn text_before_tag_is_trimmed() {
        let text = "Here is my answer. <tool_call>{}</tool_call>";
        assert_eq!(text_before_tool_call(text), "Here is my answer.");
    }

    #[test]
    fn text_before_tag_no_call() {
        assert_eq!(text_before_tool_call("plain text"), "plain text");
    }

    #[test]
    fn schema_snippet_empty_when_no_tools() {
        assert!(tool_schema_snippet(&ToolBox::new()).is_empty());
    }
}
