//! Discord → LLM transcript serialization.
//!
//! Turns raw Discord messages into clean, attributable text for the LLM:
//! resolves `<@id>` / `<@&id>` / `<#id>` tokens into readable names, renders
//! reply context, and (for group channels) fetches the live channel history so
//! the model can see messages between *other* users — not just bot-directed ones.

use serenity::builder::GetMessages;
use serenity::cache::Cache;
use serenity::http::Http;
use serenity::model::channel::Message as DiscordMessage;
use serenity::model::id::{ChannelId, MessageId, UserId};
use tracing::warn;

use g10kz_engine::serialize_user_line;
use g10kz_llm::types::Part;
use g10kz_llm::{Message, Role};

/// Replace every mention token in `msg.content` with a readable form and strip
/// the bot's own mention. Returns the trimmed result.
pub fn resolve_mentions(msg: &DiscordMessage, bot_id: UserId, cache: &Cache) -> String {
    let mut text = msg.content.clone();

    // ── user mentions ──
    for u in &msg.mentions {
        let n1 = format!("<@{}>", u.id);
        let n2 = format!("<@!{}>", u.id);
        if u.id == bot_id {
            text = text.replace(&n1, "").replace(&n2, "");
        } else {
            let name = u.global_name.clone().unwrap_or_else(|| u.name.clone());
            let rep = format!("@{name}");
            text = text.replace(&n1, &rep).replace(&n2, &rep);
        }
    }

    // ── role mentions (best-effort via cache) ──
    if let Some(gid) = msg.guild_id {
        for role_id in &msg.mention_roles {
            let needle = format!("<@&{}>", role_id);
            let name = cache
                .guild(gid)
                .and_then(|g| g.roles.get(role_id).map(|r| r.name.clone()));
            let rep = name
                .map(|n| format!("@{n}"))
                .unwrap_or_else(|| "@角色".to_string());
            text = text.replace(&needle, &rep);
        }
    }

    // ── leftover channel / unknown mention tokens ──
    cleanup_tokens(&text).trim().to_string()
}

/// Neutralize leftover `<#id>` → `#頻道` and strip leftover `<@id>` / `<@&id>`
/// tokens. Custom emoji (`<:name:id>`) are left untouched.
fn cleanup_tokens(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && i + 1 < bytes.len()
            && (bytes[i + 1] == b'#' || bytes[i + 1] == b'@')
        {
            if let Some(rel) = input[i..].find('>') {
                let inner = &input[i + 1..i + rel];
                let body = inner.trim_start_matches(['#', '@', '!', '&']);
                if !body.is_empty() && body.chars().all(|ch| ch.is_ascii_digit()) {
                    if inner.starts_with('#') {
                        out.push_str("#頻道");
                    }
                    i += rel + 1;
                    continue;
                }
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Render a one-line reply context, e.g. `Alice「我昨天說的方法…」`.
/// When the replied-to author is the bot, the speaker is shown as `你`.
pub fn reply_snippet(referenced: &DiscordMessage, bot_id: UserId) -> String {
    let who = if referenced.author.id == bot_id {
        "你".to_string()
    } else {
        referenced
            .author
            .global_name
            .clone()
            .unwrap_or_else(|| referenced.author.name.clone())
    };
    let raw = referenced.content.replace('\n', " ");
    let truncated: String = raw.chars().take(24).collect();
    let snippet = if raw.chars().count() > 24 {
        format!("{truncated}…")
    } else {
        truncated
    };
    format!("{who}「{snippet}」")
}

/// Fetch the most recent `limit` messages before `before` and serialize them
/// into LLM history (oldest first). Bot messages become `Assistant` turns;
/// other humans become labeled `User` turns. Consecutive same-role messages are
/// merged. Returns empty on fetch failure (caller may fall back to the ring).
pub async fn fetch_channel_history(
    http: &Http,
    cache: &Cache,
    channel_id: ChannelId,
    before: MessageId,
    bot_id: UserId,
    limit: u8,
) -> Vec<Message> {
    let builder = GetMessages::new().before(before).limit(limit);
    let mut fetched = match channel_id.messages(http, builder).await {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "fetch channel history failed");
            return vec![];
        }
    };
    fetched.reverse(); // newest-first → oldest-first

    let mut msgs: Vec<Message> = Vec::new();
    for m in &fetched {
        if m.author.id == bot_id {
            let content = m.content.trim();
            if content.is_empty() {
                continue;
            }
            push_or_merge(&mut msgs, Role::Assistant, content);
        } else {
            if m.author.bot {
                continue;
            }
            let resolved = resolve_mentions(m, bot_id, cache);
            if resolved.is_empty() {
                continue;
            }
            let name = m.author.global_name.clone().unwrap_or_else(|| m.author.name.clone());
            let reply = m.referenced_message.as_ref().map(|r| reply_snippet(r, bot_id));
            let line = serialize_user_line(true, &name, reply.as_deref(), &resolved);
            push_or_merge(&mut msgs, Role::User, &line);
        }
    }
    msgs
}

/// Append `text` as a new turn, merging into the previous turn when it shares
/// the same role (avoids consecutive same-role messages some providers reject).
fn push_or_merge(msgs: &mut Vec<Message>, role: Role, text: &str) {
    if let Some(last) = msgs.last_mut() {
        if last.role == role {
            if let Some(Part::Text { text: t }) = last.parts.last_mut() {
                t.push('\n');
                t.push_str(text);
                return;
            }
        }
    }
    msgs.push(Message::text(role, text));
}
