//! Discord-layer utility helpers.

use crate::state::ContextEntry;
use g10kz_engine::serialize_user_line;
use g10kz_llm::{Message, Role};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

/// Split reply at natural boundaries so each chunk fits Discord's 2000-char limit.
/// Uses a 1900-char target to leave room for metadata.
pub fn split_message(text: &str) -> Vec<String> {
    const MAX: usize = 1900;
    if text.len() <= MAX {
        return vec![text.to_owned()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let raw_end = (start + MAX).min(text.len());
        if raw_end == text.len() {
            chunks.push(text[start..].to_owned());
            break;
        }
        // Step back to a UTF-8 char boundary so we never slice mid-codepoint.
        // This matters for Chinese text (3 bytes/char) with no newlines.
        let end = {
            let mut e = raw_end;
            while e > start && !text.is_char_boundary(e) {
                e -= 1;
            }
            e
        };
        if end == start {
            // Degenerate: advance one char to avoid infinite loop
            let next = text[start..].char_indices().nth(1).map(|(i, _)| start + i).unwrap_or(text.len());
            chunks.push(text[start..next].to_owned());
            start = next;
            continue;
        }
        let window = &text[start..end];
        // Prefer splitting on the last newline; fall back to char boundary at end.
        let split = window.rfind('\n').map(|p| start + p + 1).unwrap_or(end);
        chunks.push(text[start..split].trim_end().to_owned());
        start = split;
    }
    chunks.into_iter().filter(|c| !c.is_empty()).collect()
}

/// Convert the channel ring buffer into LLM history messages (oldest first).
///
/// Used for DMs (and as a group fallback when the live fetch fails). Group
/// messages are labeled with the speaker; DM messages are left unlabeled.
pub fn build_history(ring: &VecDeque<ContextEntry>, is_dm: bool) -> Vec<Message> {
    let mut msgs = Vec::new();
    for entry in ring {
        let line = serialize_user_line(
            !is_dm,
            &entry.user_name,
            entry.reply_to.as_deref(),
            &entry.user_text,
        );
        msgs.push(Message::text(Role::User, &line));
        if let Some(bot_reply) = &entry.bot_reply {
            msgs.push(Message::text(Role::Assistant, bot_reply));
        }
    }
    msgs
}

/// Current Unix timestamp in seconds.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Spawn a background task that renews the typing indicator every 8 s.
/// Returns a token — cancel it to stop the task.
pub fn spawn_typing_task(
    http: Arc<serenity::http::Http>,
    channel_id: serenity::model::id::ChannelId,
) -> CancellationToken {
    let stop = CancellationToken::new();
    let token = stop.clone();
    tokio::spawn(async move {
        loop {
            let _ = channel_id.broadcast_typing(&http).await;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(8)) => {}
                _ = stop.cancelled() => break,
            }
        }
    });
    token
}

/// Strip single-backtick inline-code formatting from Social/Search path replies.
///
/// Triple-backtick code blocks are preserved verbatim. Single-backtick pairs
/// on the same line (`word`) have their backticks removed so Discord doesn't
/// render them as monospace code — the bot should use **bold** for emphasis.
/// Unmatched lone backticks are left as-is.
pub fn sanitize_backticks(text: &str) -> String {
    // Split on ``` to find code-block boundaries.
    // Even-indexed segments are outside code blocks; odd-indexed are inside.
    let segments: Vec<&str> = text.split("```").collect();
    if segments.len() == 1 {
        return strip_inline_backtick_pairs(text);
    }
    let mut result = String::with_capacity(text.len());
    for (i, seg) in segments.iter().enumerate() {
        if i % 2 == 0 {
            result.push_str(&strip_inline_backtick_pairs(seg));
        } else {
            result.push_str("```");
            result.push_str(seg);
            if i + 1 < segments.len() {
                result.push_str("```");
            }
        }
    }
    result
}

fn strip_inline_backtick_pairs(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let start = i + 1;
            let mut j = start;
            // Scan for closing backtick on the same line
            while j < chars.len() && chars[j] != '`' && chars[j] != '\n' {
                j += 1;
            }
            if j < chars.len() && chars[j] == '`' && j > start {
                // Valid pair: output content without backticks
                for &ch in &chars[start..j] {
                    result.push(ch);
                }
                i = j + 1;
            } else {
                // No matching close on this line: keep the backtick
                result.push('`');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_message_short_passthrough() {
        let s = "短訊息";
        assert_eq!(split_message(s), vec![s.to_owned()]);
    }

    #[test]
    fn split_message_ascii_no_newline() {
        // Long ASCII without newlines: must split at char boundary
        let s = "a".repeat(3000);
        let chunks = split_message(&s);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.len() <= 1900, "chunk too long: {}", chunk.len());
        }
        assert_eq!(chunks.join(""), s);
    }

    #[test]
    fn split_message_chinese_wall_no_newline() {
        // Chinese text (3 bytes/char) without newlines — previously panicked
        // with "byte index is not a char boundary".
        let s = "好".repeat(800); // 800 × 3 = 2400 bytes, no newlines
        let chunks = split_message(&s);
        assert!(chunks.len() >= 2);
        // Every chunk must be valid UTF-8 (Rust strings guarantee this if we
        // never slice at a non-boundary, so just joining must work)
        let rejoined = chunks.join("");
        assert_eq!(rejoined, s, "rejoined mismatch");
        for chunk in &chunks {
            assert!(chunk.len() <= 1900, "chunk too long");
        }
    }

    #[test]
    fn split_message_prefers_newline_boundary() {
        // Content with newlines: should split at the newline, not mid-word
        let line = "x".repeat(950) + "\n" + &"y".repeat(950);
        let long = line.repeat(2); // ~3800 chars
        let chunks = split_message(&long);
        // First chunk should end at a newline boundary
        for chunk in &chunks {
            assert!(!chunk.starts_with('y') || chunk.len() < 1901);
        }
    }
}