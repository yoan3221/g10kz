//! Character card loader — SillyTavern V2 JSON **and** OKF markdown bundle.
//!
//! # Supported formats
//!
//! ## OKF bundle (directory)
//!
//! A directory containing at minimum `index.md`:
//! ```
//! persona/g10kz/
//!   index.md      ← YAML frontmatter (title) + body + "## First Message" section
//!   examples.md   ← dialogue examples (optional)
//!   log.md        ← change log (ignored by loader)
//! ```
//!
//! `PersonaCard::load()` auto-detects directory vs. file path.
//!
//! ## SillyTavern V2 JSON (file, legacy)
//!
//! ```json
//! {
//!   "spec": "chara_card_v2",
//!   "data": {
//!     "name": "...", "system_prompt": "...", "mes_example": "...", "first_mes": "..."
//!   }
//! }
//! ```

use std::collections::HashMap;

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

/// Parsed representation of a character card (OKF or SillyTavern V2).
#[derive(Debug, Clone)]
pub struct PersonaCard {
    /// Character name shown in logs and slash commands.
    pub name: String,

    /// Fully-rendered system prompt.
    pub system_prompt: String,

    /// Example dialogue lines used for anti-repetition seeding.
    pub example_dialogue: Vec<String>,

    /// First message the bot sends when entering a new conversation.
    pub first_message: String,
}

impl PersonaCard {
    /// Load a character card from `path`.
    ///
    /// - **Directory** → OKF bundle (`index.md` + optional `examples.md`)
    /// - **File**      → SillyTavern V2 JSON
    /// - **Empty**     → returns [`PersonaCard::stub`]
    pub fn load(path: &std::path::Path) -> Result<Self, KernelError> {
        if path.as_os_str().is_empty() {
            return Ok(Self::stub());
        }

        if path.is_dir() {
            return Self::load_okf(path);
        }

        let raw = std::fs::read_to_string(path).map_err(|e| {
            KernelError::PersonaParse(format!("read error: {e}"))
        })?;

        Self::parse_json(&raw)
    }

    // ── OKF loader ────────────────────────────────────────────────────────────

    /// Load an OKF bundle from a directory.
    ///
    /// Required: `index.md`
    /// Optional: `examples.md`
    fn load_okf(dir: &std::path::Path) -> Result<Self, KernelError> {
        // ── index.md ──────────────────────────────────────────────────────────
        let index_path = dir.join("index.md");
        let index_raw = std::fs::read_to_string(&index_path).map_err(|e| {
            KernelError::PersonaParse(format!("okf index.md: {e}"))
        })?;

        let (front, body) = parse_frontmatter(&index_raw);

        let name = front
            .get("title")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "g10kz".into());

        // Split body at "## First Message" marker
        let (system_body, first_message) = split_first_message(&body);

        // ── examples.md (optional) ────────────────────────────────────────────
        let examples_path = dir.join("examples.md");
        let example_dialogue = if examples_path.exists() {
            let raw = std::fs::read_to_string(&examples_path).unwrap_or_default();
            let (_, ex_body) = parse_frontmatter(&raw);
            parse_example_dialogue(&ex_body)
        } else {
            vec![]
        };

        Ok(Self {
            name,
            system_prompt: system_body.trim().to_string(),
            example_dialogue,
            first_message: first_message.trim().to_string(),
        })
    }

    // ── JSON loader ───────────────────────────────────────────────────────────

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

// ─── OKF helpers ─────────────────────────────────────────────────────────────

/// Parse YAML frontmatter delimited by `---` lines.
///
/// Returns `(fields, body)`.  Only simple `key: value` pairs are parsed;
/// list/nested YAML is silently skipped.  Quoted string values are unquoted.
fn parse_frontmatter(content: &str) -> (HashMap<String, String>, String) {
    // Strip UTF-8 BOM if present
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);

    let Some(after_open) = content.strip_prefix("---") else {
        return (HashMap::new(), content.to_string());
    };

    // The closing --- must be on its own line
    let close_marker = "\n---";
    let Some(close_pos) = after_open.find(close_marker) else {
        return (HashMap::new(), content.to_string());
    };

    let front_text = &after_open[..close_pos];
    let rest = &after_open[close_pos + close_marker.len()..];
    let body = rest.strip_prefix('\n').unwrap_or(rest).to_string();

    let mut fields = HashMap::new();
    for line in front_text.lines() {
        // Skip list items and blank lines
        let line = line.trim();
        if line.is_empty() || line.starts_with('-') || line.starts_with('[') {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let key = line[..colon].trim().to_string();
        let val = line[colon + 1..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if !key.is_empty() {
            fields.insert(key, val);
        }
    }

    (fields, body)
}

/// Split a markdown body at a `## First Message` heading.
///
/// Returns `(system_prompt_section, first_message_section)`.
fn split_first_message(body: &str) -> (String, String) {
    // Look for the heading in any of its common forms
    for marker in &["## First Message\n", "## First Message\r\n", "## First Message"] {
        if let Some(pos) = body.find(marker) {
            let sys = body[..pos].to_string();
            let rest = &body[pos + marker.len()..];
            // Skip the first blank line after heading if present
            let msg = rest.trim_start_matches('\n').trim_start_matches('\r');
            return (sys, msg.to_string());
        }
    }
    // No separator → entire body is system prompt
    (body.to_string(), String::new())
}

// ─── JSON helpers ─────────────────────────────────────────────────────────────

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

    // ── JSON tests (unchanged) ────────────────────────────────────────────────

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
        assert!(!card.system_prompt.is_empty());
    }

    // ── OKF frontmatter tests ─────────────────────────────────────────────────

    #[test]
    fn frontmatter_parses_title() {
        let md = "---\ntype: Character\ntitle: g10kz\ntags: [a, b]\n---\nbody text";
        let (fields, body) = parse_frontmatter(md);
        assert_eq!(fields.get("title").map(String::as_str), Some("g10kz"));
        assert_eq!(fields.get("type").map(String::as_str), Some("Character"));
        assert_eq!(body.trim(), "body text");
    }

    #[test]
    fn frontmatter_handles_no_frontmatter() {
        let md = "just plain body";
        let (fields, body) = parse_frontmatter(md);
        assert!(fields.is_empty());
        assert_eq!(body, "just plain body");
    }

    #[test]
    fn split_first_message_splits_correctly() {
        let body = "system stuff here\n\n## First Message\nhello there";
        let (sys, first) = split_first_message(body);
        assert!(sys.contains("system stuff"));
        assert_eq!(first.trim(), "hello there");
    }

    #[test]
    fn split_first_message_no_marker_returns_full_body() {
        let body = "system stuff only";
        let (sys, first) = split_first_message(body);
        assert_eq!(sys, "system stuff only");
        assert!(first.is_empty());
    }

    #[test]
    fn okf_load_from_dir() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.md");
        let mut f = std::fs::File::create(&index).unwrap();
        writeln!(f, "---").unwrap();
        writeln!(f, "type: Character").unwrap();
        writeln!(f, "title: TestChar").unwrap();
        writeln!(f, "---").unwrap();
        writeln!(f, "System prompt text here.").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "## First Message").unwrap();
        writeln!(f, "Hello from OKF!").unwrap();

        let ex = dir.path().join("examples.md");
        let mut fe = std::fs::File::create(&ex).unwrap();
        writeln!(fe, "---\ntype: Dialogue Examples\ntitle: ex\n---").unwrap();
        writeln!(fe, "{{{{user}}}}: hi").unwrap();
        writeln!(fe, "{{{{char}}}}: hey").unwrap();

        let card = PersonaCard::load(dir.path()).unwrap();
        assert_eq!(card.name, "TestChar");
        assert!(card.system_prompt.contains("System prompt text here"));
        assert_eq!(card.first_message, "Hello from OKF!");
        assert_eq!(card.example_dialogue.len(), 2);
    }
}
