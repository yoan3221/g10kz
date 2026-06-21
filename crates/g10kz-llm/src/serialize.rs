//! OpenAI-compatible JSON request and response types.
//!
//! Used by [`crate::openrouter::OpenRouterProvider`] to build request bodies
//! and parse completion responses.
//!
//! # Content format
//! We always send content as an **array** (the multimodal form) rather than a
//! plain string — this allows text and image parts to coexist and is accepted
//! by all OpenAI-compatible endpoints.
//!
//! # Prefix-cache marking
//! When [`crate::types::CompletionParams::cache_system_prompt`] is `true`, the
//! first content part of the system message receives a `cache_control` field:
//! `{"type": "ephemeral"}`.  OpenRouter forwards this to Anthropic for KV-cache
//! prefix caching; other providers silently ignore the extra field.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::types::{CompletionParams, Message, Part, Role, Usage};

// ─── Request ─────────────────────────────────────────────────────────────────

/// Serialise a list of [`Message`]s + params into an OpenAI completion request body.
pub fn build_request(
    messages: &[Message],
    params: &CompletionParams,
    extra_fields: Option<Value>,
) -> Value {
    let serialised: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let is_system = matches!(msg.role, Role::System);
            let mark_cache = is_system && params.cache_system_prompt;
            serialise_message(msg, mark_cache, i)
        })
        .collect();

    let mut body = json!({
        "model":       params.model,
        "messages":    serialised,
        "max_tokens":  params.max_tokens,
    });
    // claude-opus-4+ (extended thinking) does not accept temperature
    if !params.model.contains("opus-4") {
        body["temperature"] = serde_json::json!(params.temperature);
    }

    // Merge any extra fields (e.g. Fusion plugin, tools array).
    if let Some(extra) = extra_fields {
        if let (Some(map), Some(extra_map)) = (body.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_map {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    body
}

/// Serialise a single [`Message`] to OpenAI JSON.
fn serialise_message(msg: &Message, mark_cache_on_first: bool, _idx: usize) -> Value {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    let parts: Vec<Value> = msg
        .parts
        .iter()
        .enumerate()
        .map(|(part_i, part)| serialise_part(part, mark_cache_on_first && part_i == 0))
        .collect();

    json!({ "role": role, "content": parts })
}

/// Serialise a single [`Part`] to OpenAI content-part JSON.
fn serialise_part(part: &Part, add_cache_control: bool) -> Value {
    let mut obj = match part {
        Part::Text { text } => json!({
            "type": "text",
            "text": text,
        }),
        Part::ImageUrl { url } => json!({
            "type": "image_url",
            "image_url": { "url": url },
        }),
        Part::AudioTranscript { text } => json!({
            "type": "text",
            "text": text,
        }),
    };

    if add_cache_control {
        if let Some(map) = obj.as_object_mut() {
            map.insert("cache_control".into(), json!({ "type": "ephemeral" }));
        }
    }

    obj
}

// ─── Response ────────────────────────────────────────────────────────────────

/// OpenAI-style completion response.
#[derive(Debug, Deserialize)]
pub struct CompletionResponse {
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<UsageRaw>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ChoiceMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChoiceMessage {
    pub content: Option<String>,
}

/// Cached-token breakdown (OpenAI-style `prompt_tokens_details`). Lets us
/// observe prefix-cache hits when the upstream gateway reports them.
#[derive(Debug, Default, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

/// Raw token usage from the API response.
#[derive(Debug, Deserialize)]
pub struct UsageRaw {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(default)]
    pub cost: Option<f64>, // OpenRouter-specific field
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>, // cache-hit observability
}

/// Extract reply text and [`Usage`] from a parsed [`CompletionResponse`].
pub fn extract_reply(resp: CompletionResponse) -> anyhow::Result<(String, Usage)> {
    let choice = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no choices in completion response"))?;

    let text = choice.message.content.unwrap_or_default();

    let usage = resp
        .usage
        .map(|u| {
            let cached = u
                .prompt_tokens_details
                .as_ref()
                .is_some_and(|d| d.cached_tokens > 0);
            Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                cost_usd: u.cost.unwrap_or(0.0),
                cached,
            }
        })
        .unwrap_or_default();

    Ok((text, usage))
}

// ─── Streaming (SSE) chunk ───────────────────────────────────────────────────

/// One `data: {...}` chunk from an OpenAI-compatible SSE stream.
#[derive(Debug, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<UsageRaw>,
}

#[derive(Debug, Deserialize)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: Delta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<String>,
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompletionParams, Message, Part, Role};

    fn text_msg(role: Role, text: &str) -> Message {
        Message::text(role, text)
    }

    // ── serialisation ─────────────────────────────────────────────────────────

    #[test]
    fn system_message_serialises_correctly() {
        let msgs = vec![text_msg(Role::System, "你是小十")];
        let params = CompletionParams::social("model-x");
        let body = build_request(&msgs, &params, None);

        let messages = &body["messages"];
        assert_eq!(messages[0]["role"], "system");
        // Content is an array
        assert!(messages[0]["content"].is_array());
        assert_eq!(messages[0]["content"][0]["text"], "你是小十");
    }

    #[test]
    fn cache_control_added_to_system_first_part() {
        let msgs = vec![text_msg(Role::System, "sys")];
        let mut params = CompletionParams::social("m");
        params.cache_system_prompt = true;
        let body = build_request(&msgs, &params, None);

        let first_part = &body["messages"][0]["content"][0];
        assert!(
            first_part.get("cache_control").is_some(),
            "cache_control missing"
        );
        assert_eq!(first_part["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn cache_control_absent_when_disabled() {
        let msgs = vec![text_msg(Role::System, "sys")];
        let mut params = CompletionParams::social("m");
        params.cache_system_prompt = false;
        let body = build_request(&msgs, &params, None);

        let first_part = &body["messages"][0]["content"][0];
        assert!(first_part.get("cache_control").is_none());
    }

    #[test]
    fn image_part_serialises_to_image_url_type() {
        let msg = Message {
            role: Role::User,
            parts: vec![
                Part::Text {
                    text: "describe this".into(),
                },
                Part::ImageUrl {
                    url: "https://example.com/img.jpg".into(),
                },
            ],
        };
        let body = build_request(&[msg], &CompletionParams::social("m"), None);
        let parts = &body["messages"][0]["content"];
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "https://example.com/img.jpg");
    }

    #[test]
    fn audio_transcript_serialises_as_text() {
        let msg = Message {
            role: Role::User,
            parts: vec![Part::AudioTranscript {
                text: "hello".into(),
            }],
        };
        let body = build_request(&[msg], &CompletionParams::social("m"), None);
        let part = &body["messages"][0]["content"][0];
        assert_eq!(part["type"], "text");
        assert_eq!(part["text"], "hello");
    }

    #[test]
    fn model_and_params_propagate() {
        let msgs = vec![text_msg(Role::User, "hi")];
        let params = CompletionParams {
            model: "openai/gpt-4o".into(),
            max_tokens: 1234,
            temperature: 0.75,
            cache_system_prompt: false,
        };
        let body = build_request(&msgs, &params, None);
        assert_eq!(body["model"], "openai/gpt-4o");
        assert_eq!(body["max_tokens"], 1234);
        assert!((body["temperature"].as_f64().unwrap() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn extra_fields_merged_into_body() {
        let msgs = vec![text_msg(Role::User, "hi")];
        let params = CompletionParams::social("m");
        let extra = json!({ "stream": false, "plugins": [{"id": "fusion"}] });
        let body = build_request(&msgs, &params, Some(extra));
        assert_eq!(body["stream"], false);
        assert!(body["plugins"].is_array());
    }

    // ── response parsing ──────────────────────────────────────────────────────

    #[test]
    fn extract_reply_from_valid_response() {
        let resp: CompletionResponse = serde_json::from_value(json!({
            "choices": [{"message": {"content": "哼，你好。"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        }))
        .unwrap();

        let (text, usage) = extract_reply(resp).unwrap();
        assert_eq!(text, "哼，你好。");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
    }

    #[test]
    fn extract_reply_no_choices_is_err() {
        let resp: CompletionResponse = serde_json::from_value(json!({
            "choices": []
        }))
        .unwrap();
        assert!(extract_reply(resp).is_err());
    }

    #[test]
    fn extract_reply_null_content_becomes_empty() {
        let resp: CompletionResponse = serde_json::from_value(json!({
            "choices": [{"message": {"content": null}, "finish_reason": "stop"}]
        }))
        .unwrap();
        let (text, _) = extract_reply(resp).unwrap();
        assert_eq!(text, "");
    }

    #[test]
    fn multiple_messages_correct_roles() {
        let msgs = vec![
            text_msg(Role::System, "sys"),
            text_msg(Role::User, "user"),
            text_msg(Role::Assistant, "asst"),
        ];
        let body = build_request(&msgs, &CompletionParams::social("m"), None);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][2]["role"], "assistant");
    }
}
