//! serenity [`EventHandler`] implementation.

use crate::{
    commands::{global_commands, handle_command},
    state::{BotState, ContextEntry, RING_SIZE},
    transcript::{fetch_channel_history, reply_snippet, resolve_mentions},
    util::{build_history, now_unix, spawn_typing_task, split_message},
};
use g10kz_engine::turn::{run_turn, TurnInput};
use g10kz_kernel::{classify_activation, PersonalityState, RouteDecision};
use serenity::builder::EditMessage;
use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{application::Interaction, channel::Message as DiscordMessage, gateway::Ready},
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// How many recent channel messages to pull for group-channel context.
const HISTORY_FETCH_LIMIT: u8 = 15;

/// Minimum gap between progressive message edits during streaming.
/// Discord allows ~5 edits / 5s per message; 1s keeps us well under the limit.
const STREAM_EDIT_INTERVAL: Duration = Duration::from_millis(1000);

/// Clip a string to Discord's 2000-character message limit (char-safe).
fn clip2000(s: &str) -> String {
    if s.chars().count() <= 2000 {
        s.to_string()
    } else {
        s.chars().take(2000).collect()
    }
}

/// Extract the first usable image URL from a message.
///
/// Priority: uploaded file attachments first, then image/GIF embeds — this is
/// how Tenor / Giphy GIFs (sent via the picker as `gifv` embeds) and pasted
/// image links arrive, since they are NOT file attachments. For animated GIFs
/// we prefer the `.gif` video URL so Gemini can sample multiple frames; we fall
/// back to the embed's still image / thumbnail otherwise.
fn first_image_url(msg: &DiscordMessage) -> Option<String> {
    // 1. Direct file attachment (uploaded image or GIF).
    if let Some(att) = msg.attachments.first() {
        return Some(att.url.clone());
    }
    // 2. Image / GIF embeds (Tenor, Giphy, pasted image links).
    for embed in &msg.embeds {
        match embed.kind.as_deref() {
            Some("gifv") => {
                // Prefer an actual animated GIF so the vision model sees motion.
                if let Some(v) = &embed.video {
                    if v.url.contains(".gif") {
                        return Some(v.url.clone());
                    }
                }
                if let Some(img) = &embed.image {
                    return Some(img.url.clone());
                }
                if let Some(t) = &embed.thumbnail {
                    return Some(t.url.clone());
                }
            }
            Some("image") => {
                if let Some(img) = &embed.image {
                    return Some(img.url.clone());
                }
                if let Some(t) = &embed.thumbnail {
                    return Some(t.url.clone());
                }
            }
            _ => {}
        }
    }
    None
}

pub struct Handler {
    pub state: Arc<BotState>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(name = %ready.user.name, "Discord gateway ready");
        match serenity::model::application::Command::set_global_commands(
            &ctx.http,
            global_commands(),
        )
        .await
        {
            Ok(cmds) => info!(count = cmds.len(), "slash commands registered"),
            Err(e) => warn!(error = %e, "slash command registration failed"),
        }
    }

    async fn message(&self, ctx: Context, msg: DiscordMessage) {
        if msg.author.bot {
            return;
        }

        let channel_id = msg.channel_id;
        let is_dm = msg.guild_id.is_none();
        let bot_id = ctx.cache.current_user().id;
        let is_mention = msg.mentions.iter().any(|u| u.id == bot_id);
        let is_reply_to_bot = msg
            .referenced_message
            .as_ref()
            .map(|rm| rm.author.id == bot_id)
            .unwrap_or(false);

        if !is_dm && !is_mention && !is_reply_to_bot {
            // Lurk mode: optionally reply in designated channels with configured probability.
            let lurk_prob = self.state.config.lurk_reply_probability;
            let in_lurk_channel = self.state.config.lurk_channels.contains(&channel_id.get());
            let mut will_respond = false;
            if in_lurk_channel && lurk_prob > 0.0 {
                // Lightweight pseudo-random from nanosecond clock (sufficient for lurk decisions).
                let nano = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as f64;
                let roll = (nano % 1_000_000.0) / 1_000_000.0;
                will_respond = roll < lurk_prob;
            }
            if !will_respond {
                // 主動觀察：bot 旁觀（不回應）的群組訊息也寫入長期記憶，
                // 讓她「記得」群裡發生的事。只 add 不 flush，跳過過短噪音。
                if let Some(everos) = self.state.everos.clone() {
                    let observed = resolve_mentions(&msg, bot_id, &ctx.cache);
                    if observed.chars().count() >= 4 {
                        let uid = msg.author.id.get();
                        let session = format!("g10kz-{uid}");
                        tokio::spawn(async move {
                            everos.observe(uid, &session, &observed).await;
                        });
                    }
                }
                return;
            }
        }

        let msg_id = msg.id;
        {
            let mut in_flight = self.state.in_flight.lock().await;
            if in_flight.contains(&msg_id) {
                return;
            }
            in_flight.insert(msg_id);
        }

        // Resolve all mention tokens to readable names; strip the bot's own.
        let clean_text = resolve_mentions(&msg, bot_id, &ctx.cache);

        if clean_text.is_empty() && first_image_url(&msg).is_none() {
            self.state.in_flight.lock().await.remove(&msg_id);
            return;
        }

        // Display name: guild nick > global display name > username.
        let display_name = msg
            .member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .or_else(|| msg.author.global_name.clone())
            .unwrap_or_else(|| msg.author.name.clone());

        // Reply context (group channels only).
        let reply_context = if is_dm {
            None
        } else {
            msg.referenced_message
                .as_ref()
                .map(|rm| reply_snippet(rm, bot_id))
        };

        self.state
            .last_seen
            .lock()
            .await
            .insert(channel_id, now_unix());

        // History source:
        //  - group: live channel transcript (includes other users' messages)
        //  - DM:    per-channel ring buffer
        let history = if is_dm {
            let ctx_map = self.state.channel_ctx.lock().await;
            ctx_map
                .get(&channel_id)
                .map(|ring| build_history(&ring.entries, true))
                .unwrap_or_default()
        } else {
            let fetched = fetch_channel_history(
                &ctx.http,
                &ctx.cache,
                channel_id,
                msg_id,
                bot_id,
                HISTORY_FETCH_LIMIT,
            )
            .await;
            if fetched.is_empty() {
                // Fetch failed — fall back to the ring buffer.
                let ctx_map = self.state.channel_ctx.lock().await;
                ctx_map
                    .get(&channel_id)
                    .map(|ring| build_history(&ring.entries, false))
                    .unwrap_or_default()
            } else {
                fetched
            }
        };

        let attachment_url = first_image_url(&msg);
        let has_attachment = attachment_url.is_some();

        // ── Guild / channel name for environment-aware system prompt ──────────
        let guild_name: Option<String> = if !is_dm {
            msg.guild_id
                .and_then(|gid| ctx.cache.guild(gid).map(|g| g.name.clone()))
        } else {
            None
        };
        let channel_name: Option<String> = if !is_dm {
            ctx.cache.channel(channel_id).map(|ch| ch.name.clone())
        } else {
            None
        };

        // ── JPAF: read personality modifier for this user ─────────────────────
        let personality_modifier: Option<String> = {
            let states = self.state.personality_states.lock().await;
            states
                .get(&msg.author.id.get())
                .and_then(|s| s.render_modifier())
        };

        let cancel = CancellationToken::new();
        self.state
            .cancel_map
            .lock()
            .await
            .insert(channel_id, cancel.clone());

        let typing_stop = spawn_typing_task(ctx.http.clone(), channel_id);

        let persona = self.state.persona.read().await.clone();
        let mut turn_input = TurnInput::new(
            &self.state.config,
            &persona,
            self.state.provider.as_ref(),
            self.state.memory.as_ref(),
            &self.state.toolbox,
            msg.author.id.get(),
            clean_text.clone(),
        );
        turn_input.user_name = display_name.clone();
        turn_input.is_dm = is_dm;
        turn_input.reply_context = reply_context.clone();
        turn_input.history = history;
        turn_input.has_attachment = has_attachment;
        turn_input.attachment_url = attachment_url;
        turn_input.cancel = cancel.clone();
        turn_input.embed_router = Some(self.state.embed_router.clone());
        turn_input.prompt_guard = Some(self.state.prompt_guard.clone());
        turn_input.guild_name = guild_name;
        turn_input.channel_name = channel_name;
        turn_input.personality_modifier = personality_modifier;

        // ── Streaming: lazily create a placeholder message and progressively
        // edit it as the engine streams cumulative-text snapshots. The consumer
        // ends when the sender (held inside turn_input) is dropped after the
        // turn completes. Non-streaming paths never send a snapshot, so no
        // placeholder is created and the reply is sent normally below.
        let (stream_tx, mut stream_rx) = mpsc::channel::<String>(32);
        let stream_http = ctx.http.clone();
        let stream_consumer = tokio::spawn(async move {
            let mut placeholder: Option<DiscordMessage> = None;
            let mut last_edit = Instant::now();
            while let Some(snap) = stream_rx.recv().await {
                let content = clip2000(&snap);
                if content.is_empty() {
                    continue;
                }
                match placeholder.as_mut() {
                    None => {
                        if let Ok(m) = channel_id.say(&stream_http, content).await {
                            placeholder = Some(m);
                            last_edit = Instant::now();
                        }
                    }
                    Some(m) => {
                        if last_edit.elapsed() >= STREAM_EDIT_INTERVAL {
                            let _ = m
                                .edit(&stream_http, EditMessage::new().content(content))
                                .await;
                            last_edit = Instant::now();
                        }
                    }
                }
            }
            placeholder
        });
        turn_input.stream_sink = Some(stream_tx);

        let result = run_turn(turn_input).await;

        // The turn is done → its stream sender is dropped → consumer finishes.
        let placeholder = stream_consumer.await.unwrap_or(None);

        typing_stop.cancel();
        self.state.cancel_map.lock().await.remove(&channel_id);
        self.state.in_flight.lock().await.remove(&msg_id);

        let reply_text = match result {
            Ok(output) => {
                debug!(path = ?output.path, ptok = output.usage.prompt_tokens, "turn ok");

                // JPAF: classify the exchange and update per-user personality state
                {
                    let activated = classify_activation(&clean_text, &output.reply);
                    let mut states = self.state.personality_states.lock().await;
                    states
                        .entry(msg.author.id.get())
                        .or_insert_with(PersonalityState::default)
                        .update(activated);
                }

                // EverOS: persist the conversation turn in background
                if let Some(everos) = self.state.everos.clone() {
                    let uid = msg.author.id.get();
                    let session = format!("g10kz-{uid}");
                    let user_text = clean_text.clone();
                    let bot_reply = output.reply.clone();
                    tokio::spawn(async move {
                        everos.add_turn(uid, &session, &user_text, &bot_reply).await;
                    });
                }

                // Sanitize backtick misuse on conversational paths:
                // strip single-backtick inline-code pairs so they don't
                // render as monospace in Discord. Triple-backtick blocks intact.
                let reply = output.reply;
                match output.path {
                    RouteDecision::Social | RouteDecision::Search => {
                        crate::util::sanitize_backticks(&reply)
                    }
                    _ => reply,
                }
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    return;
                }
                warn!(error = %e, "turn error");
                "（發生錯誤，請稍後再試。）".to_owned()
            }
        };

        if reply_text.is_empty() {
            return;
        }

        {
            let mut ctx_map = self.state.channel_ctx.lock().await;
            let now = now_unix();
            // Evict channels idle for more than 7 days (lazy TTL on write path).
            const TTL_SECS: u64 = 7 * 24 * 3600;
            ctx_map.retain(|_, ring| now.saturating_sub(ring.last_touch_secs) < TTL_SECS);
            let ring = ctx_map.entry(channel_id).or_default();
            ring.last_touch_secs = now;
            ring.entries.push_back(ContextEntry {
                user_id: msg.author.id.get(),
                user_name: display_name.clone(),
                reply_to: reply_context.clone(),
                user_text: clean_text,
                bot_reply: Some(reply_text.clone()),
            });
            while ring.entries.len() > RING_SIZE {
                ring.entries.pop_front();
            }
        }

        let chunks = split_message(&reply_text);
        if let Some(mut ph) = placeholder {
            // Streamed: finalize the placeholder with the authoritative
            // (sanitized) text, then send any overflow chunks as new messages.
            if let Some(first) = chunks.first() {
                if let Err(e) = ph
                    .edit(&ctx.http, EditMessage::new().content(clip2000(first)))
                    .await
                {
                    warn!(error = %e, "final edit failed");
                }
            }
            for chunk in chunks.iter().skip(1) {
                if let Err(e) = channel_id.say(&ctx.http, chunk).await {
                    warn!(error = %e, "reply send failed");
                    break;
                }
            }
        } else {
            // Non-streaming path: send the reply as new message(s).
            for chunk in chunks {
                if let Err(e) = channel_id.say(&ctx.http, &chunk).await {
                    warn!(error = %e, "reply send failed");
                    break;
                }
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(cmd) = interaction {
            handle_command(&ctx, &cmd, &self.state).await;
        }
    }
}
