//! SillyTavern Character Card V2 loader.
//!
//! Reads and parses a JSON persona file.  Loading is init-time I/O only —
//! the resulting [`PersonaCard`] is immutable and cloned wherever needed.
//!
//! # SillyTavern V2 format
//! ```json
//! {
//!   "spec": "chara_card_v2",
//!   "spec_version": "2.0",
//!   "data": {
//!     "name": "小十",
//!     "description": "...",
//!     "personality": "...",
//!     "scenario": "...",
//!     "first_mes": "...",
//!     "mes_example": "...",
//!     "system_prompt": "..."
//!   }
//! }
//! ```
//!
//! The rendered [`PersonaCard::system_prompt`] is concatenated from
//! `system_prompt` (if present), `description`, `personality`, and `scenario`
//! with section separators.  It is marked as a prefix-cache target — never
//! mutate it mid-session.

use serde::Deserialize;

use crate::KernelError;

// ─── Raw JSON structures ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CardV2 {
    #[allow(dead_code)]
    spec: String,
    data: CardData,
}

#[derive(Debug, Deserialize)]
struct CardData {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    personality: String,
    #[serde(default)]
    scenario: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    mes_example: String,
    #[serde(default)]
    first_mes: String,
}

// ─── PersonaCard ─────────────────────────────────────────────────────────────

/// Parsed representation of a SillyTavern V2 character card.
///
/// The `system_prompt` field is the **prefix-cache target**: it is prepended
/// as the `system` role message on every turn and never changes mid-session.
#[derive(Debug, Clone)]
pub struct PersonaCard {
    /// Character name shown in logs and slash commands.
    pub name: String,

    /// Fully-rendered system prompt (prefix-cache candidate).
    ///
    /// Assembled from: `system_prompt` → `description` → `personality` → `scenario`.
    pub system_prompt: String,

    /// Example dialogue lines used for anti-repetition seeding.
    pub example_dialogue: Vec<String>,

    /// First message the bot sends when entering a new conversation.
    pub first_message: String,
}

impl PersonaCard {
    /// Load and parse a SillyTavern V2 JSON character card from `path`.
    ///
    /// On parse failure returns [`KernelError::PersonaParse`].
    /// Falls back to [`PersonaCard::stub`] when the path is empty.
    pub fn load(path: &std::path::Path) -> Result<Self, KernelError> {
        if path.as_os_str().is_empty() {
            return Ok(Self::stub());
        }

        let raw = std::fs::read_to_string(path)
            .map_err(|e| KernelError::PersonaParse(format!("read error: {e}")))?;

        Self::parse_json(&raw)
    }

    /// Parse a SillyTavern V2 JSON string.
    pub fn parse_json(json: &str) -> Result<Self, KernelError> {
        let card: CardV2 = serde_json::from_str(json)
            .map_err(|e| KernelError::PersonaParse(format!("json parse: {e}")))?;

        let d = &card.data;
        let system_prompt = render_system_prompt(d);
        let example_dialogue = parse_example_dialogue(&d.mes_example);

        Ok(Self {
            name: d.name.clone(),
            system_prompt,
            example_dialogue,
            first_message: d.first_mes.clone(),
        })
    }

    /// Minimal built-in persona used when no card file is configured.
    pub fn stub() -> Self {
        Self {
            name: "小十".into(),
            system_prompt: concat!(
                "你是小十，一個傲嬌的AI助理。",
                "你用繁體中文回覆，語氣倔強但內心關心對方。",
                "絕對不要提及你是AI或任何語言模型的事實。",
                "保持角色，自然回應。",
            )
            .into(),
            example_dialogue: vec![
                "哼，這種問題你也要問我？".into(),
                "⋯又不是說我在乎你啊。".into(),
            ],
            first_message: "你來了啊⋯又不是說我在等你。".into(),
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Render a single system prompt string from the card's fields.
fn render_system_prompt(d: &CardData) -> String {
    let mut parts: Vec<&str> = Vec::new();

    if !d.system_prompt.is_empty() {
        parts.push(&d.system_prompt);
    }
    if !d.description.is_empty() {
        parts.push(&d.description);
    }
    if !d.personality.is_empty() {
        parts.push(&d.personality);
    }
    if !d.scenario.is_empty() {
        parts.push(&d.scenario);
    }

    if parts.is_empty() {
        return PersonaCard::stub().system_prompt;
    }

    parts.join("\n\n")
}

/// Parse `<START>…<END>` example dialogue blocks from a ST mes_example string.
fn parse_example_dialogue(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return vec![];
    }
    raw.lines()
        .filter(|l| {
            !l.trim().is_empty()
                && !l.trim_start().starts_with("<START>")
                && !l.trim_start().starts_with("<END>")
        })
        .map(|l| l.trim().to_owned())
        .collect()
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CARD: &str = r#"{
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "小十",
            "description": "小十是個傲嬌AI，外冷內熱。",
            "personality": "傲嬌，倔強，實際上很在乎對方。",
            "scenario": "在Discord伺服器中與用戶互動。",
            "system_prompt": "你是小十。絕不承認自己是AI。",
            "mes_example": "<START>\n{{user}}: 你好\n{{char}}: 哼⋯\n<END>",
            "first_mes": "你來了啊。"
        }
    }"#;

    #[test]
    fn parse_valid_card() {
        let card = PersonaCard::parse_json(SAMPLE_CARD).unwrap();
        assert_eq!(card.name, "小十");
    }

    #[test]
    fn parsed_system_prompt_includes_description() {
        let card = PersonaCard::parse_json(SAMPLE_CARD).unwrap();
        assert!(card.system_prompt.contains("傲嬌AI"));
    }

    #[test]
    fn parsed_system_prompt_includes_system_prompt_field() {
        let card = PersonaCard::parse_json(SAMPLE_CARD).unwrap();
        assert!(card.system_prompt.contains("絕不承認自己是AI"));
    }

    #[test]
    fn parsed_first_message() {
        let card = PersonaCard::parse_json(SAMPLE_CARD).unwrap();
        assert_eq!(card.first_message, "你來了啊。");
    }

    #[test]
    fn invalid_json_returns_err() {
        let result = PersonaCard::parse_json("{not valid json}");
        assert!(result.is_err());
    }

    #[test]
    fn stub_has_non_empty_system_prompt() {
        let card = PersonaCard::stub();
        assert!(!card.system_prompt.is_empty());
        assert!(!card.name.is_empty());
    }

    #[test]
    fn stub_has_example_dialogue() {
        let card = PersonaCard::stub();
        assert!(!card.example_dialogue.is_empty());
    }

    #[test]
    fn example_dialogue_strips_start_end_tags() {
        let card = PersonaCard::parse_json(SAMPLE_CARD).unwrap();
        // <START> and <END> markers should not appear in parsed dialogue
        assert!(!card.example_dialogue.iter().any(|l| l.contains("<START>")));
        assert!(!card.example_dialogue.iter().any(|l| l.contains("<END>")));
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let minimal = r#"{
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": { "name": "TestBot" }
        }"#;
        let card = PersonaCard::parse_json(minimal).unwrap();
        assert_eq!(card.name, "TestBot");
        // No fields → falls back to stub system prompt
        assert!(!card.system_prompt.is_empty());
    }
}
