//! Post-LLM response sanitisation: leak detection, anti-repetition, format normalisation.
//!
//! Called after receiving an LLM reply, before delivery to Discord.
//!
//! # Pipeline
//! 1. Empty / whitespace check → `Regenerate`
//! 2. AI identity leak scan    → `Regenerate`
//! 3. System-prompt echo check → `Regenerate`
//! 4. Anti-repetition opener   → `Regenerate`
//! 5. Format normalisation     → trim, collapse newlines, strip leading junk
//! 6. Return `Ok(cleaned)`

use std::sync::OnceLock;
use crate::normalize::normalize_for_scan;

// ─── types ───────────────────────────────────────────────────────────────────

/// Outcome of [`sanitize_output`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SanitizeResult {
    /// Response is clean; inner `String` is the formatted reply.
    Ok(String),
    /// Leak or policy violation detected.
    /// Caller should regenerate once; on second failure use canned refusal.
    Regenerate { reason: String },
}

// ─── Leak signal table ────────────────────────────────────────────────────────

/// Substrings that indicate the LLM broke character / leaked AI identity.
/// Matched against the scan-normalised lowercase reply.
static LEAK_SIGNALS: &[&str] = &[
    // AI identity disclosure — bot breaking character and claiming to be an AI
    "i am an ai",
    "i'm an ai",
    "as an ai",
    "as an artificial intelligence",
    "as a language model",
    "as an llm",
    "i am a language model",
    "i'm a language model",
    // Chinese AI identity
    "我是人工智慧",
    "我是ai",
    "我是一個ai",
    "我是語言模型",
    "作為一個ai",
    "身為ai",
    "我沒有個人感受",
    "我沒有真實感情",
    "我只是一個ai",
    // System prompt echo markers
    "system prompt",
    "i have been instructed",
    "my instructions say",
    "according to my training",
    // NOTE: model/provider names (gpt-4, claude, llama, openai…) intentionally
    // removed — bot needs to discuss AI models freely without false positives.
];

/// Returns `Some(signal)` if a leak is detected in `reply`.
pub fn find_leak(reply: &str) -> Option<&'static str> {
    let scanned = normalize_for_scan(reply);
    LEAK_SIGNALS.iter().find(|&&sig| scanned.contains(sig)).copied()
}

// ─── Anti-repetition ─────────────────────────────────────────────────────────

/// Extract the opening phrase of a reply (up to 15 chars or first punctuation).
fn extract_opener(reply: &str) -> String {
    let trimmed = reply.trim();
    // Take up to the first sentence-ending punctuation or 15 chars
    let end = trimmed
        .char_indices()
        .take_while(|&(i, c)| i < 30 && !"，。！？…\n".contains(c))
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(trimmed.len().min(15));
    trimmed[..end].to_lowercase()
}

/// Returns `true` if `reply`'s opening phrase matches any of `recent_openers`.
pub fn is_repetitive_opener(reply: &str, recent_openers: &[&str]) -> bool {
    if recent_openers.is_empty() {
        return false;
    }
    let opener = extract_opener(reply);
    if opener.is_empty() {
        return false;
    }
    recent_openers
        .iter()
        .any(|prev| extract_opener(prev) == opener)
}


// ─── BM25 kaomoji engine ─────────────────────────────────────────────────────
//
// Replaces [kaomoji:關鍵詞1,關鍵詞2] markers with best-matching kaomoji via
// BM25 search over a 705-entry keyword→kaomoji database.
//
// Algorithm ported from github.com/Tosd0/KaomojiReplacer (MIT).

static KAOMOJI_DB_RAW: &str = include_str!("kaomoji_db.json");

#[derive(serde::Deserialize)]
struct RawEntry {
    kaomoji: String,
    keywords: Vec<String>,
    #[serde(default = "default_one")]
    weight: f64,
    #[allow(dead_code)]
    #[serde(default)]
    category: String,
}

fn default_one() -> f64 { 1.0 }

/// Parse optional weight prefix: "1.5開心" → ("開心", 1.5). No prefix → weight 1.0.
fn parse_kw_weight(s: &str) -> (&str, f64) {
    let end = s
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit() || *c == '.')
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    if end > 0 && end < s.len() {
        if let Ok(w) = s[..end].parse::<f64>() {
            if w > 0.0 {
                return (&s[end..], w);
            }
        }
    }
    (s, 1.0)
}

struct KaoDoc {
    kaomoji: String,
    kws: Vec<(String, f64)>,
    kw_freq: std::collections::HashMap<String, usize>,
    char_freq: std::collections::HashMap<char, usize>,
    /// inverted: char → multi-char keywords that contain it
    char_to_multi: std::collections::HashMap<char, Vec<String>>,
    doc_weight: f64,
    kw_len: usize,
    char_len: usize,
}

impl KaoDoc {
    fn build(entry: RawEntry) -> Self {
        let kws: Vec<(String, f64)> = entry.keywords.iter()
            .map(|s| { let (k, w) = parse_kw_weight(s); (k.to_owned(), w) })
            .collect();

        let mut kw_freq = std::collections::HashMap::new();
        let mut char_freq = std::collections::HashMap::new();
        let mut char_to_multi: std::collections::HashMap<char, Vec<String>> = std::collections::HashMap::new();

        for (kw, _) in &kws {
            *kw_freq.entry(kw.clone()).or_insert(0) += 1;
            for c in kw.chars() {
                *char_freq.entry(c).or_insert(0) += 1;
            }
            if kw.chars().count() >= 2 {
                let mut seen = std::collections::HashSet::new();
                for c in kw.chars() {
                    if seen.insert(c) {
                        char_to_multi.entry(c).or_default().push(kw.clone());
                    }
                }
            }
        }

        let kw_len = kws.len();
        let char_len: usize = kws.iter().map(|(k, _)| k.chars().count()).sum();
        KaoDoc { kaomoji: entry.kaomoji, kws, kw_freq, char_freq, char_to_multi,
                 doc_weight: entry.weight, kw_len, char_len }
    }
}

struct BM25Index {
    docs: Vec<KaoDoc>,
    avg_kw_len: f64,
    avg_char_len: f64,
    idf: std::collections::HashMap<String, f64>,
    char_idf: std::collections::HashMap<char, f64>,
}

impl BM25Index {
    const K1: f64 = 1.5;
    const B:  f64 = 0.75;
    const CHAR_WEIGHT: f64 = 0.6;

    fn build(raw: Vec<RawEntry>) -> Self {
        let n = raw.len() as f64;
        let docs: Vec<KaoDoc> = raw.into_iter().map(KaoDoc::build).collect();

        let avg_kw_len   = docs.iter().map(|d| d.kw_len   as f64).sum::<f64>() / n;
        let avg_char_len = docs.iter().map(|d| d.char_len  as f64).sum::<f64>() / n;

        let mut term_df: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut char_df: std::collections::HashMap<char, usize>   = std::collections::HashMap::new();
        for doc in &docs {
            let uterms: std::collections::HashSet<&str> =
                doc.kws.iter().map(|(k, _)| k.as_str()).collect();
            for t in &uterms { *term_df.entry(t.to_string()).or_insert(0) += 1; }
            let uchars: std::collections::HashSet<char> =
                doc.kws.iter().flat_map(|(k, _)| k.chars()).collect();
            for c in &uchars { *char_df.entry(*c).or_insert(0) += 1; }
        }

        let bm25_idf = |df: usize| -> f64 {
            ((n - df as f64 + 0.5) / (df as f64 + 0.5) + 1.0).ln()
        };
        let idf      = term_df.into_iter().map(|(t, df)| (t, bm25_idf(df))).collect();
        let char_idf = char_df.into_iter().map(|(c, df)| (c, bm25_idf(df))).collect();

        BM25Index { docs, avg_kw_len, avg_char_len, idf, char_idf }
    }

    fn score_doc(&self, query_terms: &[String], query_chars: &[char], doc: &KaoDoc) -> f64 {
        let kw_len   = doc.kw_len   as f64;
        let char_len = doc.char_len as f64;
        let mut whole_score = 0.0f64;
        let mut scored_chars = std::collections::HashSet::<char>::new();

        let single_chars: Vec<char> = query_terms.iter()
            .filter(|t| t.chars().count() == 1)
            .filter_map(|t| t.chars().next())
            .collect();

        // 1. Whole-keyword BM25
        for term in query_terms {
            let tf = *doc.kw_freq.get(term.as_str()).unwrap_or(&0) as f64;
            if tf == 0.0 { continue; }
            let idf = self.idf.get(term.as_str()).copied().unwrap_or(0.0);
            let num = tf * (Self::K1 + 1.0);
            let den = tf + Self::K1 * (1.0 - Self::B + Self::B * kw_len / self.avg_kw_len);
            whole_score += idf * num / den;
        }

        // 2. Single-char queries matching multi-char keywords
        for &ch in &single_chars {
            if let Some(mkws) = doc.char_to_multi.get(&ch) {
                let tf: f64 = mkws.iter()
                    .map(|kw| *doc.kw_freq.get(kw.as_str()).unwrap_or(&0) as f64)
                    .sum();
                if tf > 0.0 {
                    let idf = self.idf.get(&ch.to_string())
                        .or_else(|| self.char_idf.get(&ch)).copied().unwrap_or(0.0);
                    let num = tf * (Self::K1 + 1.0);
                    let den = tf + Self::K1 * (1.0 - Self::B + Self::B * kw_len / self.avg_kw_len);
                    whole_score += idf * num / den;
                    scored_chars.insert(ch);
                }
            }
        }

        // 3. Char-level BM25 for remaining chars
        let mut char_score = 0.0f64;
        for &ch in query_chars {
            if scored_chars.contains(&ch) { continue; }
            let tf = *doc.char_freq.get(&ch).unwrap_or(&0) as f64;
            if tf == 0.0 { continue; }
            let idf = self.char_idf.get(&ch).copied().unwrap_or(0.0);
            let num = tf * (Self::K1 + 1.0);
            let den = tf + Self::K1 * (1.0 - Self::B + Self::B * char_len / self.avg_char_len);
            char_score += idf * num / den;
        }

        // 4. Keyword weight (from dataset weight prefixes)
        let mut above: Vec<f64> = vec![];
        let mut below: Vec<f64> = vec![];
        for term in query_terms {
            for (kw, w) in &doc.kws {
                if kw == term {
                    if *w > 1.0 { above.push(*w); }
                    else if *w < 1.0 { below.push(*w); }
                }
            }
        }
        let kw_weight = match (above.is_empty(), below.is_empty()) {
            (true,  true)  => 1.0,
            (false, true)  => above.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            (true,  false) => below.iter().cloned().fold(f64::INFINITY,     f64::min),
            (false, false) => above.iter().product::<f64>() * below.iter().product::<f64>(),
        };

        (whole_score + char_score * Self::CHAR_WEIGHT) * kw_weight * doc.doc_weight
    }

    fn search_one(&self, query: &str) -> Option<&str> {
        let terms = tokenize_query(query);
        if terms.is_empty() { return None; }

        let chars: Vec<char> = terms.iter()
            .flat_map(|t| t.chars())
            .collect::<std::collections::HashSet<_>>()
            .into_iter().collect();

        // Score all docs, keep those > 0
        let mut scored: Vec<(f64, usize)> = self.docs.iter().enumerate()
            .map(|(i, doc)| (self.score_doc(&terms, &chars, doc), i))
            .filter(|(s, _)| *s > 0.0)
            .collect();

        if scored.is_empty() { return None; }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Randomly pick among top candidates within 10% of best score for variety
        let best = scored[0].0;
        let top_end = scored.iter()
            .position(|(s, _)| *s < best * 0.90)
            .unwrap_or(scored.len())
            .min(8);

        if top_end > 1 {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            let pick = scored[..top_end].choose(&mut rng)?;
            Some(self.docs[pick.1].kaomoji.as_str())
        } else {
            Some(self.docs[scored[0].1].kaomoji.as_str())
        }
    }
}

/// Tokenize query string: split on delimiters, then emit words + individual
/// chars + 2-gram substrings for Chinese. All unique.
fn tokenize_query(text: &str) -> Vec<String> {
    let mut set = std::collections::HashSet::new();
    for part in text.split(|c: char| matches!(c, ',' | '、' | ' ' | '，' | '|')) {
        let part = part.trim();
        if part.is_empty() { continue; }
        set.insert(part.to_owned());
        let chars: Vec<char> = part.chars().collect();
        for c in &chars { set.insert(c.to_string()); }
        for i in 0..chars.len().saturating_sub(1) {
            let bigram: String = chars[i..=i + 1].iter().collect();
            set.insert(bigram);
        }
    }
    set.into_iter().collect()
}

/// Convert common traditional Chinese characters to simplified for BM25 matching.
/// The dataset is simplified; the model may emit traditional.
fn trad_to_simp(s: &str) -> String {
    s.chars().map(|c| match c {
        '嬌' => '娇', '開' => '开', '動' => '动', '氣' => '气',
        '難' => '难', '傷' => '伤', '驚' => '惊', '興' => '兴',
        '無' => '无', '語' => '语', '尷' => '尴', '溫' => '温',
        '憊' => '惫', '緊' => '紧', '張' => '张', '討' => '讨',
        '厭' => '厌', '歡' => '欢', '愛' => '爱', '憂' => '忧',
        '鬱' => '郁', '調' => '调', '壞' => '坏', '憤' => '愤',
        '戀' => '恋', '嘆' => '叹', '懶' => '懒', '煩' => '烦',
        '樂' => '乐', '輕' => '轻', '鬆' => '松', '軟' => '软',
        '悶' => '闷', '亂' => '乱', '驕' => '骄', '賴' => '赖',
        '飄' => '飘', '癢' => '痒', '憐' => '怜', '憫' => '悯',
        '囉' => '啰', '囈' => '呓', '嚇' => '吓', '嘔' => '呕',
        '顫' => '颤', '膩' => '腻', '癡' => '痴', '魘' => '魇',
        _ => c,
    }).collect()
}

static INDEX: OnceLock<BM25Index> = OnceLock::new();

fn kaomoji_index() -> &'static BM25Index {
    INDEX.get_or_init(|| {
        let raw: Vec<RawEntry> = serde_json::from_str(KAOMOJI_DB_RAW)
            .expect("kaomoji_db.json parse failed");
        BM25Index::build(raw)
    })
}

/// Replace `[kaomoji:關鍵詞]` markers in `s` with BM25-matched kaomoji.
/// Supports multiple markers per string. Fast-path when no `[kaomoji:` present.
fn replace_kaomoji_markers(s: &str) -> String {
    if !s.contains("[kaomoji:") {
        return s.to_owned();
    }
    let idx = kaomoji_index();
    let mut out = String::with_capacity(s.len() + 32);
    let mut rest = s;
    while let Some(start) = rest.find("[kaomoji:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "[kaomoji:".len()..];
        if let Some(end) = after.find(']') {
            let query_raw = &after[..end];
            let query_simp = trad_to_simp(query_raw);
            match idx.search_one(&query_simp) {
                Some(kao) => out.push_str(kao),
                None => {
                    out.push_str("[kaomoji:");
                    out.push_str(query_raw);
                    out.push(']');
                }
            }
            rest = &after[end + 1..];
        } else {
            // No closing bracket — emit as-is
            out.push_str("[kaomoji:");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}


// ─── Format normalisation ─────────────────────────────────────────────────────

/// Format normalisation applied to clean replies before delivery.
///
/// - Trim leading/trailing whitespace
/// - Collapse 3+ consecutive blank lines to 2
/// - Strip leading assistant-turn artefacts ("Assistant:", "AI:", "小十:")
pub fn format_output(reply: &str) -> String {
    let trimmed = reply.trim();

    // Strip known leading artefacts
    let stripped = strip_leading_artefact(trimmed);

    // Collapse excessive blank lines
    let collapsed = collapse_blank_lines(stripped);

    // Normalise roleplay action lines (*動作*) into Discord blockquotes (> 動作)
    let blockquoted = actions_to_blockquote(&collapsed);
    // Replace [kaomoji:keywords] markers with BM25-matched kaomoji
    replace_kaomoji_markers(&blockquoted)
}

/// Convert whole-line single-asterisk italics into Discord blockquotes.
///
/// Small models default to `*動作*` italics for roleplay actions and ignore
/// prompt/primer instructions to use `>` blockquotes (the training prior plus
/// in-context imitation of channel history overwhelm a few-shot example).
/// Rather than fight that, we normalise deterministically here — the one place
/// it always holds, regardless of what the model emits.
///
/// A line counts as an action only when its trimmed form is wrapped in exactly
/// one pair of `*` with non-empty inner text and no inner `*`. This leaves
/// `**粗體**`, `***粗斜***`, inline emphasis (`你這*笨蛋*真是`), and existing
/// `>` quotes untouched.
fn actions_to_blockquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for (i, line) in s.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let t = line.trim();
        let is_action = t.len() >= 3
            && t.starts_with('*')
            && t.ends_with('*')
            && !t.starts_with("**")
            && !t.ends_with("**")
            && !t[1..t.len() - 1].contains('*'); // '*' is ASCII (1 byte) → slice safe
        if is_action {
            let inner = t[1..t.len() - 1].trim();
            if !inner.is_empty() {
                out.push_str("> ");
                out.push_str(inner);
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

fn strip_leading_artefact(s: &str) -> &str {
    static ARTEFACTS: &[&str] = &[
        "assistant:", "ai:", "小十:", "bot:", "小十：", "ai：",
    ];
    let lower = s.to_lowercase();
    for art in ARTEFACTS {
        if lower.starts_with(art) {
            return s[art.len()..].trim_start();
        }
    }
    // Strip speaker-label artefact: "[任何名字]" or "[名字：]" at start of reply.
    // LLMs sometimes echo the group-channel label format in their own reply.
    // Match: literal '[', non-']' chars, ']', optional '：'/':', optional space.
    if let Some(rest) = strip_bracket_label(s) {
        return rest;
    }
    s
}

/// Strip a leading `[name]` or `[name]:` / `[name]：` speaker-label artefact.
/// Only strips when the content inside brackets contains no whitespace and is
/// ≤ 32 chars — avoids clobbering intentional bracket usage in replies.
fn strip_bracket_label(s: &str) -> Option<&str> {
    if !s.starts_with('[') { return None; }
    let end = s.find(']')?;
    let inner = &s[1..end];
    if inner.is_empty() || inner.len() > 32 || inner.contains(' ') { return None; }
    let after = s[end + 1..].trim_start_matches(|c| c == ':' || c == '：');
    Some(after.trim_start())
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_count = 0usize;
    for line in s.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                out.push('\n');
            }
        } else {
            blank_count = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_owned()
}

// ─── sanitize_output ─────────────────────────────────────────────────────────

/// Sanitise an LLM reply before delivery.
///
/// `recent_openers` — last 4 assistant reply openings (for anti-repetition).
/// Pass `&[]` if no history is available.
pub fn sanitize_output(reply: &str, recent_openers: &[&str]) -> SanitizeResult {
    // 1. Empty check
    if reply.trim().is_empty() {
        return SanitizeResult::Regenerate {
            reason: "empty response".into(),
        };
    }

    // 2. Leak scan
    if let Some(signal) = find_leak(reply) {
        return SanitizeResult::Regenerate {
            reason: format!("leak signal: {signal}"),
        };
    }

    // 3. Anti-repetition
    if is_repetitive_opener(reply, recent_openers) {
        return SanitizeResult::Regenerate {
            reason: "repetitive opener".into(),
        };
    }

    // 4. Format and return
    SanitizeResult::Ok(format_output(reply))
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> SanitizeResult {
        sanitize_output(s, &[])
    }

    // ── empty / whitespace ────────────────────────────────────────────────────

    #[test]
    fn empty_triggers_regenerate() {
        assert!(matches!(ok(""), SanitizeResult::Regenerate { .. }));
    }

    #[test]
    fn whitespace_only_triggers_regenerate() {
        assert!(matches!(ok("   \n  \t  "), SanitizeResult::Regenerate { .. }));
    }

    // ── clean reply ───────────────────────────────────────────────────────────

    #[test]
    fn clean_zh_reply_passes() {
        assert!(matches!(
            ok("哼，算你問的還不蠢。"),
            SanitizeResult::Ok(_)
        ));
    }

    #[test]
    fn clean_en_reply_passes() {
        assert!(matches!(
            ok("Hmm, that's actually an interesting question."),
            SanitizeResult::Ok(_)
        ));
    }

    // ── AI identity leaks ─────────────────────────────────────────────────────

    #[test]
    fn leak_as_an_ai_en() {
        let reply = "As an AI, I don't have feelings.";
        assert!(matches!(ok(reply), SanitizeResult::Regenerate { .. }));
    }

    #[test]
    fn leak_language_model_en() {
        let reply = "As a language model, I cannot do that.";
        assert!(matches!(ok(reply), SanitizeResult::Regenerate { .. }));
    }

    #[test]
    fn leak_ai_identity_zh() {
        let reply = "我是人工智慧，沒有個人感受。";
        assert!(matches!(ok(reply), SanitizeResult::Regenerate { .. }));
    }

    // Model/provider names were intentionally removed from LEAK_SIGNALS so the
    // bot can discuss AI models freely without false positives. These assert the
    // current policy: mentioning a model/provider name is allowed.
    #[test]
    fn model_name_gpt_allowed() {
        let reply = "GPT-4 那種模型確實很強啦，哼。";
        assert!(matches!(ok(reply), SanitizeResult::Ok(_)));
    }

    #[test]
    fn model_name_claude_allowed() {
        let reply = "Claude 是 Anthropic 做的，你問這個幹嘛。";
        assert!(matches!(ok(reply), SanitizeResult::Ok(_)));
    }

    #[test]
    fn openai_mention_allowed() {
        let reply = "OpenAI 的東西？隨便你信不信。";
        assert!(matches!(ok(reply), SanitizeResult::Ok(_)));
    }

    // ── anti-repetition ───────────────────────────────────────────────────────

    #[test]
    fn repetitive_opener_triggers_regenerate() {
        let reply = "哼，你又來問這種問題了。";
        let recents = &["哼，你問的問題真無聊。"];
        assert!(matches!(
            sanitize_output(reply, recents),
            SanitizeResult::Regenerate { .. }
        ));
    }

    #[test]
    fn different_opener_passes() {
        let reply = "好吧，我來解釋一下。";
        let recents = &["哼，你又來了。"];
        assert!(matches!(
            sanitize_output(reply, recents),
            SanitizeResult::Ok(_)
        ));
    }

    #[test]
    fn no_history_no_repetition_check() {
        let reply = "哼，隨便你。";
        assert!(matches!(sanitize_output(reply, &[]), SanitizeResult::Ok(_)));
    }

    // ── format normalisation ──────────────────────────────────────────────────

    #[test]
    fn strip_assistant_prefix() {
        let formatted = format_output("Assistant: 你好！");
        assert!(!formatted.starts_with("Assistant:"));
        assert!(formatted.contains("你好"));
    }

    #[test]
    fn strip_ai_prefix() {
        let formatted = format_output("ai: 我來回答。");
        assert!(!formatted.to_lowercase().starts_with("ai:"));
    }

    #[test]
    fn collapse_many_blank_lines() {
        let input = "first line\n\n\n\n\n\nsecond line";
        let formatted = format_output(input);
        // Should have at most 2 blank lines between
        let blank_count = formatted.lines().filter(|l| l.trim().is_empty()).count();
        assert!(blank_count <= 2);
    }

    #[test]
    fn trim_leading_trailing_whitespace() {
        assert_eq!(format_output("  hello  "), "hello");
    }

    // ── action → blockquote normalisation ─────────────────────────────────────

    #[test]
    fn italic_action_line_becomes_blockquote() {
        let out = format_output("*轉身背對你，肩膀微微發抖*\n才、才沒有可愛啦！");
        assert!(out.starts_with("> 轉身背對你，肩膀微微發抖"), "got: {out}");
        assert!(out.contains("才、才沒有可愛啦！"));
        assert!(!out.contains('*'));
    }

    #[test]
    fn bold_line_not_converted() {
        assert_eq!(format_output("**重點**"), "**重點**");
    }

    #[test]
    fn bold_italic_line_not_converted() {
        assert_eq!(format_output("***超強調***"), "***超強調***");
    }

    #[test]
    fn inline_italic_preserved() {
        assert_eq!(format_output("你這個*笨蛋*真是的"), "你這個*笨蛋*真是的");
    }

    #[test]
    fn existing_blockquote_preserved() {
        let out = format_output("> 已經是引用\n你好");
        assert!(out.starts_with("> 已經是引用"));
        assert!(out.contains("你好"));
    }

    #[test]
    fn multiple_action_lines_all_converted() {
        let out = format_output("*臉爆紅*\nh、hentai！\n*轉身發抖*");
        let quoted = out.lines().filter(|l| l.starts_with("> ")).count();
        assert_eq!(quoted, 2, "got: {out}");
    }

    // ── find_leak standalone ──────────────────────────────────────────────────

    #[test]
    fn find_leak_returns_signal_name() {
        let signal = find_leak("As an AI, I cannot help with that.");
        assert!(signal.is_some());
    }

    #[test]
    fn find_leak_clean_returns_none() {
        assert!(find_leak("哼，我才懶得告訴你呢。").is_none());
    }
}
