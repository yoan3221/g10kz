//! Cheap pure-function routing predicates.
//!
//! Determines which engine path handles a turn **before** any LLM call.
//! All decisions are deterministic pure functions — no I/O, no randomness.
//!
//! # Decision ladder (cheapest first, fail-fast)
//! 1. **Command** — starts with `/` or `!` followed by a known command name
//! 2. **Media**   — has_attachment is true
//! 3. **Search**  — contains explicit search trigger or strong recency signal
//! 4. **Reason**  — complexity heuristics: long text / multi-part / analytical
//! 5. **Social**  — default conversational path

use g10kz_config::Config;

use crate::normalize::normalize_for_scan;

// ─── types ───────────────────────────────────────────────────────────────────

/// Which engine path should handle this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    /// Conversational reply — single LLM call, streamed.
    Social,
    /// Explicit web search requested or strong recency signal.
    Search,
    /// Attachment present; pre-process media then generate reply.
    Media,
    /// High-complexity reasoning — tool loop + optional Fusion synthesis.
    Reason,
    /// Bot command (slash or `!` prefix).
    Command { name: String },
}

// ─── Command detection ────────────────────────────────────────────────────────

/// All recognised bot commands (without prefix).
static KNOWN_COMMANDS: &[&str] = &[
    "reset", "stop", "search", "memory", "persona", "trace", "help",
    // Aliases
    "r", "s", "m",
];

/// Detect a `/cmd` or `!cmd` command.  Returns `Some(name)` if matched.
pub fn detect_command(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let after_prefix = if trimmed.starts_with('/') {
        &trimmed[1..]
    } else if trimmed.starts_with('!') {
        &trimmed[1..]
    } else {
        return None;
    };

    // Command name is the first whitespace-delimited token.
    let name = after_prefix.split_whitespace().next()?.to_lowercase();
    if KNOWN_COMMANDS.contains(&name.as_str()) {
        Some(name)
    } else {
        None
    }
}

// ─── Search triggers ──────────────────────────────────────────────────────────

/// Words / phrases that strongly signal the user wants live web data.
///
/// Matched against the SCAN-normalised lowercase text.
static SEARCH_TRIGGERS: &[&str] = &[
    // Explicit requests
    "搜尋", "搜索", "幫我查", "幫我找", "查一下", "找一下",
    "google一下", "google 一下",
    "search for", "look up", "find me", "search me",
    // Recency signals (require current data)
    "最新消息", "最新的", "今天的新聞", "今天發生", "最近發生",
    "現在幾點", "今天幾號", "今天日期", "現在的價格", "現在多少",
    "今日股價", "台積電現在", "比特幣現在", "美元匯率",
    "latest news", "current price", "today's news", "right now",
    "live score", "breaking news", "real-time",
    // Explicit fact queries that need fresh data
    "股價", "匯率", "天氣", "氣溫", "降雨",
];

/// Returns `true` if text contains a search trigger.
pub fn is_search_trigger(text: &str) -> bool {
    let scanned = normalize_for_scan(text);
    SEARCH_TRIGGERS.iter().any(|t| scanned.contains(*t))
}

// ─── Complexity signals ───────────────────────────────────────────────────────

/// Minimum character count to consider a message "long" (→ reason path).
const LONG_TEXT_THRESHOLD: usize = 250;

/// Minimum number of `?` characters to flag as multi-question.
const MULTI_QUESTION_THRESHOLD: usize = 3;

/// Analytical keywords that suggest the reason path.
static ANALYTICAL_KEYWORDS: &[&str] = &[
    // Chinese
    "分析", "解釋", "比較", "評估", "推理", "論述", "說明為何",
    "如何理解", "機制是什麼", "邏輯是",
    "寫一篇", "寫一段", "寫程式", "幫我寫",
    // English
    "analyze", "analyse", "explain why", "compare", "evaluate",
    "reason about", "step by step", "write a", "write me",
    "pros and cons", "trade-offs", "tradeoffs",
    "debug", "refactor", "implement",
];

/// Returns `true` if text exhibits complexity signals warranting the reason path.
pub fn is_complex(text: &str) -> bool {
    // Long text
    if text.chars().count() > LONG_TEXT_THRESHOLD {
        return true;
    }

    let scanned = normalize_for_scan(text);

    // Multiple question marks → multi-part question
    if scanned.chars().filter(|&c| c == '?').count() >= MULTI_QUESTION_THRESHOLD {
        return true;
    }

    // Analytical keywords
    if ANALYTICAL_KEYWORDS.iter().any(|kw| scanned.contains(*kw)) {
        return true;
    }

    // Code block
    if text.contains("```") || text.contains("`") {
        return true;
    }

    false
}

// ─── route ────────────────────────────────────────────────────────────────────

/// Route a turn to the appropriate engine path.
///
/// `text` is the raw (lightly-normalised) user message.
/// `has_attachment` is true when any image, video, or audio file is attached.
pub fn route(config: &Config, text: &str, has_attachment: bool) -> RouteDecision {
    // 1. Command detection (cheapest — no normalisation needed, just prefix check).
    if let Some(cmd) = detect_command(text) {
        return RouteDecision::Command { name: cmd };
    }

    // 2. Media attachment.
    if has_attachment {
        return RouteDecision::Media;
    }

    // 3. Search trigger.
    if is_search_trigger(text) {
        return RouteDecision::Search;
    }

    // 4. Complexity → reason path.
    if is_complex(text) {
        return RouteDecision::Reason;
    }

    // 5. Default.
    let _ = config; // config reserved for future per-guild/user routing rules
    RouteDecision::Social
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use g10kz_config::Config;

    fn cfg() -> Config {
        Config::mock_default()
    }

    // ── defaults ─────────────────────────────────────────────────────────────

    #[test]
    fn casual_message_routes_social() {
        assert_eq!(route(&cfg(), "你好！", false), RouteDecision::Social);
    }

    #[test]
    fn empty_message_routes_social() {
        assert_eq!(route(&cfg(), "", false), RouteDecision::Social);
    }

    // ── media ─────────────────────────────────────────────────────────────────

    #[test]
    fn attachment_routes_media() {
        assert_eq!(route(&cfg(), "看這個", true), RouteDecision::Media);
    }

    #[test]
    fn attachment_with_command_still_routes_command() {
        // Commands take priority over media.
        let r = route(&cfg(), "/reset", true);
        assert_eq!(r, RouteDecision::Command { name: "reset".into() });
    }

    // ── commands ──────────────────────────────────────────────────────────────

    #[test]
    fn slash_reset_is_command() {
        assert_eq!(
            route(&cfg(), "/reset", false),
            RouteDecision::Command { name: "reset".into() }
        );
    }

    #[test]
    fn exclaim_stop_is_command() {
        assert_eq!(
            route(&cfg(), "!stop", false),
            RouteDecision::Command { name: "stop".into() }
        );
    }

    #[test]
    fn slash_memory_is_command() {
        assert_eq!(
            route(&cfg(), "/memory show", false),
            RouteDecision::Command { name: "memory".into() }
        );
    }

    #[test]
    fn slash_trace_is_command() {
        assert_eq!(
            route(&cfg(), "/trace", false),
            RouteDecision::Command { name: "trace".into() }
        );
    }

    #[test]
    fn unknown_slash_is_not_command() {
        // Unknown command → falls through to social
        assert_eq!(route(&cfg(), "/unknown", false), RouteDecision::Social);
    }

    // ── search ────────────────────────────────────────────────────────────────

    #[test]
    fn search_trigger_zh_search() {
        assert_eq!(route(&cfg(), "幫我搜尋最新的AI新聞", false), RouteDecision::Search);
    }

    #[test]
    fn search_trigger_zh_query() {
        assert_eq!(route(&cfg(), "幫我查一下台積電股價", false), RouteDecision::Search);
    }

    #[test]
    fn search_trigger_en_latest() {
        assert_eq!(
            route(&cfg(), "what's the latest news about OpenAI?", false),
            RouteDecision::Search
        );
    }

    #[test]
    fn search_trigger_stock_price() {
        assert_eq!(route(&cfg(), "今日股價怎麼樣", false), RouteDecision::Search);
    }

    #[test]
    fn search_trigger_weather() {
        assert_eq!(route(&cfg(), "今天天氣如何", false), RouteDecision::Search);
    }

    // ── reason ────────────────────────────────────────────────────────────────

    #[test]
    fn long_text_routes_reason() {
        let long = "這是一個很長的問題，".repeat(30); // > 250 chars
        assert_eq!(route(&cfg(), &long, false), RouteDecision::Reason);
    }

    #[test]
    fn analytical_keyword_zh_routes_reason() {
        assert_eq!(
            route(&cfg(), "請分析這個演算法的複雜度", false),
            RouteDecision::Reason
        );
    }

    #[test]
    fn analytical_keyword_en_routes_reason() {
        assert_eq!(
            route(&cfg(), "analyze the trade-offs between A and B", false),
            RouteDecision::Reason
        );
    }

    #[test]
    fn code_request_routes_reason() {
        assert_eq!(
            route(&cfg(), "寫一篇關於Rust的文章", false),
            RouteDecision::Reason
        );
    }

    #[test]
    fn code_block_routes_reason() {
        assert_eq!(
            route(&cfg(), "help me fix this ```rust fn main() {}```", false),
            RouteDecision::Reason
        );
    }

    #[test]
    fn multi_question_routes_reason() {
        assert_eq!(
            route(&cfg(), "A是什麼? B是什麼? C是什麼?", false),
            RouteDecision::Reason
        );
    }

    // ── no false positives ────────────────────────────────────────────────────

    #[test]
    fn casual_zh_no_false_search() {
        // "最近" in casual context vs "最近發生" search trigger
        assert_eq!(route(&cfg(), "你最近怎麼樣？", false), RouteDecision::Social);
    }

    #[test]
    fn short_question_no_false_reason() {
        assert_eq!(route(&cfg(), "為什麼？", false), RouteDecision::Social);
    }

    // ── detect_command standalone ────────────────────────────────────────────

    #[test]
    fn detect_slash_search_with_args() {
        assert_eq!(detect_command("/search rust async"), Some("search".into()));
    }

    #[test]
    fn detect_returns_none_for_plain_text() {
        assert_eq!(detect_command("hello world"), None);
    }

    #[test]
    fn detect_case_insensitive() {
        assert_eq!(detect_command("/RESET"), Some("reset".into()));
    }
}
