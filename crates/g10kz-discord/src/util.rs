//! Discord-layer utility helpers.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;
use g10kz_engine::serialize_user_line;
use g10kz_llm::{Message, Role};
use crate::state::ContextEntry;

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
        let end = (start + MAX).min(text.len());
        if end == text.len() {
            chunks.push(text[start..].to_owned());
            break;
        }
        let window = &text[start..end];
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
