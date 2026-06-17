//! serenity [`EventHandler`] implementation.

use std::sync::Arc;
use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{
        application::Interaction,
        channel::Message as DiscordMessage,
        gateway::Ready,
    },
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use g10kz_engine::turn::{run_turn, TurnInput};
use g10kz_kernel::{classify_activation, PersonalityState};
use crate::{
    commands::{global_commands, handle_command},
    state::{BotState, ContextEntry, RING_SIZE},
    transcript::{fetch_channel_history, reply_snippet, resolve_mentions},
    util::{build_history, now_unix, split_message, spawn_typing_task},
};

/// How many recent channel messages to pull for group-channel context.
const HISTORY_FETCH_LIMIT: u8 = 15;

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
            return;
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

        if clean_text.is_empty() && msg.attachments.is_empty() {
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

        self.state.last_seen.lock().await.insert(channel_id, now_unix());

        // History source:
        //  - group: live channel transcript (includes other users' messages)
        //  - DM:    per-channel ring buffer
        let history = if is_dm {
            let ctx_map = self.state.channel_ctx.lock().await;
            ctx_map
                .get(&channel_id)
                .map(|ring| build_history(ring, true))
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
                    .map(|ring| build_history(ring, false))
                    .unwrap_or_default()
            } else {
                fetched
            }
        };

        let (has_attachment, attachment_url) = msg
            .attachments
            .first()
            .map(|a| (true, Some(a.url.clone())))
            .unwrap_or((false, None));

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
            states.get(&msg.author.id.get())
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
        turn_input.guild_name = guild_name;
        turn_input.channel_name = channel_name;
        turn_input.personality_modifier = personality_modifier;

        let result = run_turn(turn_input).await;

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
                    states.entry(msg.author.id.get())
                          .or_insert_with(PersonalityState::default)
                          .update(activated);
                }
                output.reply
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
            let ring = ctx_map.entry(channel_id).or_default();
            ring.push_back(ContextEntry {
                user_id: msg.author.id.get(),
                user_name: display_name.clone(),
                reply_to: reply_context.clone(),
                user_text: clean_text,
                bot_reply: Some(reply_text.clone()),
            });
            while ring.len() > RING_SIZE {
                ring.pop_front();
            }
        }

        let chunks = split_message(&reply_text);
        for chunk in chunks {
            if let Err(e) = channel_id.say(&ctx.http, &chunk).await {
                warn!(error = %e, "reply send failed");
                break;
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(cmd) = interaction {
            handle_command(&ctx, &cmd, &self.state).await;
        }
    }
}
