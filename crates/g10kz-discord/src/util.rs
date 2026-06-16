//! Discord-layer utility helpers.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;
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
/// Each user message is prefixed with `[name]` so the LLM can tell
/// different participants apart in group channels.
pub fn build_history(ring: &VecDeque<ContextEntry>) -> Vec<Message> {
    let mut msgs = Vec::new();
    for entry in ring {
        let labeled = format!("[{}] {}", entry.user_name, entry.user_text);
        msgs.push(Message::text(Role::User, &labeled));
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
