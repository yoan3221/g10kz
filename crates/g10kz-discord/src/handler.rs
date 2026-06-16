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
use crate::{
    commands::{global_commands, handle_command},
    state::{BotState, ContextEntry, RING_SIZE},
    util::{build_history, now_unix, split_message, spawn_typing_task},
};

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

        // Strip @mention from content
        let mention = format!("<@{}>", bot_id);
        let mention_nick = format!("<@!{}>", bot_id);
        let clean_text = msg
            .content
            .replace(&mention, "")
            .replace(&mention_nick, "")
            .trim()
            .to_owned();

        if clean_text.is_empty() && msg.attachments.is_empty() {
            self.state.in_flight.lock().await.remove(&msg_id);
            return;
        }

        // Resolve the display name: guild nick > global display name > username.
        let display_name = msg
            .member
            .as_ref()
            .and_then(|m| m.nick.clone())
            .or_else(|| msg.author.global_name.clone())
            .unwrap_or_else(|| msg.author.name.clone());

        self.state.last_seen.lock().await.insert(channel_id, now_unix());

        let history = {
            let ctx_map = self.state.channel_ctx.lock().await;
            ctx_map
                .get(&channel_id)
                .map(|ring| build_history(ring))
                .unwrap_or_default()
        };

        let (has_attachment, attachment_url) = msg
            .attachments
            .first()
            .map(|a| (true, Some(a.url.clone())))
            .unwrap_or((false, None));

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
        turn_input.history = history;
        turn_input.has_attachment = has_attachment;
        turn_input.attachment_url = attachment_url;
        turn_input.cancel = cancel.clone();
        turn_input.embed_router = Some(self.state.embed_router.clone());

        let result = run_turn(turn_input).await;

        typing_stop.cancel();
        self.state.cancel_map.lock().await.remove(&channel_id);
        self.state.in_flight.lock().await.remove(&msg_id);

        let reply_text = match result {
            Ok(output) => {
                debug!(path = ?output.path, ptok = output.usage.prompt_tokens, "turn ok");
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
