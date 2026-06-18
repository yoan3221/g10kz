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
    actions_to_blockquote(&collapsed)
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
