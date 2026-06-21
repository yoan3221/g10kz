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
    pub(crate) example_index: ExampleIndex,

    /// First message the bot sends when entering a new conversation.
    pub first_message: String,
    /// Lorebook entries loaded from OKF `lore/` directory.
    pub lore_entries: Vec<LoreEntry>,
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

        let raw = std::fs::read_to_string(path)
            .map_err(|e| KernelError::PersonaParse(format!("read error: {e}")))?;

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
        let index_raw = std::fs::read_to_string(&index_path)
            .map_err(|e| KernelError::PersonaParse(format!("okf index.md: {e}")))?;

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

        // ── lore/ directory (optional) ───────────────────────────────────────────
        let lore_dir = dir.join("lore");
        let lore_entries = if lore_dir.is_dir() {
            let mut entries = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&lore_dir) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    if let Ok(raw) = std::fs::read_to_string(&path) {
                        let (front, body) = parse_frontmatter(&raw);
                        let trigger_words: Vec<String> = front
                            .get("trigger_words")
                            .map(|v| {
                                v.split(',')
                                    .map(|s| s.trim().to_lowercase())
                                    .filter(|s| !s.is_empty())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let content = body.trim().to_string();
                        if !trigger_words.is_empty() && !content.is_empty() {
                            entries.push(LoreEntry {
                                trigger_words,
                                content,
                            });
                        }
                    }
                }
            }
            entries
        } else {
            vec![]
        };

        Ok(Self {
            name,
            system_prompt: system_body.trim().to_string(),
            example_index: ExampleIndex::build(&example_dialogue),
            first_message: first_message.trim().to_string(),
            lore_entries,
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
            example_index: ExampleIndex::build(&example_dialogue),
            first_message: d.first_mes.clone(),
            lore_entries: vec![],
        })
    }

    /// Minimal built-in persona used when no card file is configured.
    /// Select up to `n` dialogue examples most relevant to `query` (BM25).
    /// Returns `(user_line, char_line)` pairs ready for few-shot injection.
    pub fn query_examples(&self, query: &str, n: usize) -> Vec<(String, String)> {
        self.example_index.query(query, n)
    }

    /// Number of loaded example pairs.
    pub fn example_count(&self) -> usize {
        self.example_index.len()
    }

    /// Return the content of all lore entries whose trigger words appear in `text`.
    pub fn matched_lore<'a>(&'a self, text: &str) -> Vec<&'a str> {
        self.lore_entries
            .iter()
            .filter(|e| e.matches(text))
            .map(|e| e.content.as_str())
            .collect()
    }

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
            example_index: ExampleIndex::build(&[
                ("你好".into(), "哼，這種問題你也要問我？".into()),
                ("你在嗎".into(), "⋯又不是說我在乎你啊。".into()),
            ]),
            first_message: "你來了啊⋯又不是說我在等你。".into(),
            lore_entries: vec![],
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
        let Some(colon) = line.find(':') else {
            continue;
        };
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
    for marker in &[
        "## First Message\n",
        "## First Message\r\n",
        "## First Message",
    ] {
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
fn parse_example_dialogue(raw: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut pending: Option<String> = None;
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t == "<START>" || t == "<END>" {
            continue;
        }
        if let Some(u) = t.strip_prefix("{{user}}:") {
            pending = Some(u.trim().to_owned());
        } else if let Some(ch) = t.strip_prefix("{{char}}:") {
            if let Some(u) = pending.take() {
                pairs.push((u, ch.trim().to_owned()));
            }
        }
    }
    pairs
}

// ─── Lorebook ────────────────────────────────────────────────────────────────

/// A single lorebook / World-Info entry.
/// Loaded from OKF `lore/<name>.md` files. The YAML frontmatter field
/// `trigger_words: word1, word2, …` lists comma-separated keywords; when any
/// keyword appears (case-insensitive) in the user message the entry's body is
/// injected into the system context.
#[derive(Debug, Clone)]
pub struct LoreEntry {
    pub trigger_words: Vec<String>,
    pub content: String,
}

impl LoreEntry {
    /// True if any trigger word appears (case-insensitive) in `text`.
    pub fn matches(&self, text: &str) -> bool {
        let lower = text.to_lowercase();
        self.trigger_words
            .iter()
            .any(|w| lower.contains(w.as_str()))
    }
}

// ─── BM25 example index ───────────────────────────────────────────────────────

/// Minimal BM25 index over OKF dialogue example pairs.
/// Documents are the {{user}} lines; used for per-turn few-shot selection.
/// k1 = 1.5, b = 0.75 (Robertson defaults).
#[derive(Debug, Clone)]
pub(crate) struct ExampleIndex {
    pairs: Vec<(String, String)>,
    idf: std::collections::HashMap<String, f32>,
    tfs: Vec<std::collections::HashMap<String, f32>>,
    avg_dl: f32,
}

impl ExampleIndex {
    pub(crate) fn build(pairs: &[(String, String)]) -> Self {
        use std::collections::{HashMap, HashSet};
        if pairs.is_empty() {
            return Self {
                pairs: vec![],
                idf: HashMap::new(),
                tfs: vec![],
                avg_dl: 1.0,
            };
        }
        let tokenized: Vec<Vec<String>> = pairs.iter().map(|(u, _)| tokenize_cjk(u)).collect();
        let n = pairs.len() as f32;
        let mut df: HashMap<String, usize> = HashMap::new();
        for toks in &tokenized {
            let uniq: HashSet<_> = toks.iter().cloned().collect();
            for t in uniq {
                *df.entry(t).or_insert(0) += 1;
            }
        }
        let idf: HashMap<String, f32> = df
            .iter()
            .map(|(t, &d)| {
                let v = ((n - d as f32 + 0.5) / (d as f32 + 0.5) + 1.0).ln();
                (t.clone(), v)
            })
            .collect();
        let total: usize = tokenized.iter().map(|t| t.len()).sum();
        let avg_dl = total as f32 / n;
        let tfs: Vec<HashMap<String, f32>> = tokenized
            .iter()
            .map(|toks| {
                let mut m: HashMap<String, f32> = HashMap::new();
                for t in toks {
                    *m.entry(t.clone()).or_insert(0.0) += 1.0;
                }
                m
            })
            .collect();
        Self {
            pairs: pairs.to_vec(),
            idf,
            tfs,
            avg_dl,
        }
    }

    /// Return up to `n` pairs ordered by BM25 relevance to `query`.
    pub(crate) fn query(&self, query: &str, n: usize) -> Vec<(String, String)> {
        if self.pairs.is_empty() || n == 0 {
            return vec![];
        }
        const K1: f32 = 1.5;
        const B: f32 = 0.75;
        let qtoks = tokenize_cjk(query);
        if qtoks.is_empty() {
            // No query tokens — return first n pairs as fallback
            return self.pairs.iter().take(n).cloned().collect();
        }
        let mut scores: Vec<(usize, f32)> = self
            .tfs
            .iter()
            .enumerate()
            .map(|(i, tf)| {
                let dl: f32 = tf.values().sum();
                let s: f32 = qtoks
                    .iter()
                    .map(|t| {
                        let idf = self.idf.get(t).copied().unwrap_or(0.0);
                        let f = tf.get(t).copied().unwrap_or(0.0);
                        idf * (f * (K1 + 1.0))
                            / (f + K1 * (1.0 - B + B * dl / self.avg_dl.max(1.0)))
                    })
                    .sum();
                (i, s)
            })
            .collect();
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
            .iter()
            .take(n)
            .map(|(i, _)| self.pairs[*i].clone())
            .collect()
    }

    pub(crate) fn len(&self) -> usize {
        self.pairs.len()
    }
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

fn tokenize_cjk(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            word.push(ch.to_ascii_lowercase());
        } else {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            // Skip whitespace and CJK punctuation; keep CJK characters as individual tokens
            let is_cjk_punct = matches!(ch,
                '\u{3001}'..='\u{303F}'  // CJK punctuation block
                | '\u{FF01}'..='\u{FF0F}' // fullwidth !-/
                | '\u{FF1A}'..='\u{FF20}' // fullwidth :-@
                | '\u{2026}'              // ellipsis …
                | '\u{2018}'..='\u{201F}' // typographic quotes
            );
            if !ch.is_whitespace() && !ch.is_ascii_punctuation() && !is_cjk_punct {
                tokens.push(ch.to_string());
            }
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens
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
        assert!(card.example_count() > 0);
    }

    #[test]
    fn example_dialogue_strips_start_end_tags() {
        let raw = "<START>\n{{user}}: 你好\n{{char}}: 哼⋯\n<END>";
        let parsed = parse_example_dialogue(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "你好");
        assert!(!parsed[0].1.contains("<END>"));
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
        assert_eq!(card.example_count(), 2);
    }
}
