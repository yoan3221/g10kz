//! Shared message and parameter types.

use serde::{Deserialize, Serialize};

// ─── Role ────────────────────────────────────────────────────────────────────

/// Message role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

// ─── Part ────────────────────────────────────────────────────────────────────

/// A single content part within a message (text or media).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    /// Plain text.
    Text { text: String },

    /// Inline image (base64-encoded data URL or remote URL).
    ImageUrl { url: String },

    /// Raw audio bytes placeholder — transcribed in `g10kz-tools` before
    /// reaching the LLM.
    AudioTranscript { text: String },
}

// ─── Message ─────────────────────────────────────────────────────────────────

/// A single entry in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// Multi-part content (at minimum one [`Part::Text`]).
    pub parts: Vec<Part>,
}

impl Message {
    /// Convenience constructor for a plain-text message.
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![Part::Text { text: text.into() }],
        }
    }

    /// Return the concatenated text content of all [`Part::Text`] parts.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                Part::AudioTranscript { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ─── CompletionParams ────────────────────────────────────────────────────────

/// Per-call tuning parameters forwarded to the provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionParams {
    /// Model identifier (provider-specific string).
    pub model: String,
    /// Maximum tokens in the completion.
    pub max_tokens: u32,
    /// Sampling temperature (0.0 – 2.0).
    pub temperature: f32,
    /// Whether to mark the system prompt as a prefix-cache candidate.
    /// Translated to provider-specific cache-control headers in P3.
    pub cache_system_prompt: bool,
}

impl CompletionParams {
    /// Defaults for the social (conversational) path.
    pub fn social(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_tokens: 280,
            temperature: 0.9,
            cache_system_prompt: true,
        }
    }

    /// Defaults for the reason (analysis/tool-loop) path.
    pub fn reason(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_tokens: 1500,
            temperature: 0.4,
            cache_system_prompt: true,
        }
    }

    /// Defaults for the Fusion judge.
    pub fn judge(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_tokens: 800,
            temperature: 0.3,
            cache_system_prompt: true,
        }
    }
}

// ─── Usage ───────────────────────────────────────────────────────────────────

/// Token usage returned by a provider (for cost metering).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Provider-reported cost in USD (0.0 if not available).
    pub cost_usd: f64,
    /// Whether the prompt was served from a prefix cache.
    pub cached: bool,
}

// ─── StreamItem ──────────────────────────────────────────────────────────────

/// One event from a streaming completion.
#[derive(Debug, Clone)]
pub enum StreamItem {
    /// An incremental text delta (one or more tokens).
    Token(String),
    /// Terminal event carrying final token usage.
    Done(Usage),
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_content_joins_text_parts() {
        let msg = Message {
            role: Role::User,
            parts: vec![
                Part::Text { text: "hello".into() },
                Part::Text { text: "world".into() },
            ],
        };
        assert_eq!(msg.text_content(), "hello world");
    }

    #[test]
    fn message_text_helper() {
        let msg = Message::text(Role::User, "你好");
        assert_eq!(msg.text_content(), "你好");
    }
}
