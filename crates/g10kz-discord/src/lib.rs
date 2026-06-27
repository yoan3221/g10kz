//! Discord gateway integration using serenity.

mod commands;
mod handler;
pub mod state;
mod transcript;
mod util;

use std::sync::Arc;
use std::time::Duration;

use serenity::prelude::GatewayIntents;
use tracing::{info, warn};

use g10kz_config::Config;
use g10kz_engine::{
    turn::{run_turn, TurnInput},
    EmbeddingRouter,
};
use g10kz_everos::{EverosMemory, NullMemory};
use g10kz_kernel::persona::PersonaCard;
use g10kz_llm::OpenRouterProvider;
use g10kz_tools::{FetchPageTool, TimeTool, ToolBox, TwStockTool, ViewPageTool, WebSearchTool};

use crate::handler::Handler;
use crate::state::BotState;
use crate::util::now_unix;

pub async fn run_gateway(config: &Config) -> anyhow::Result<()> {
    let state = build_state(config);

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = serenity::Client::builder(&config.discord_token, intents)
        .event_handler(Handler {
            state: state.clone(),
        })
        .await?;

    let proactive_http = client.http.clone();
    let proactive_state = state.clone();
    let inactive_secs = config.proactive_inactive_secs;
    tokio::spawn(async move {
        proactive_loop(proactive_state, proactive_http, inactive_secs).await;
    });

    info!("starting Discord gateway");
    client.start().await?;
    Ok(())
}

pub fn build_state(config: &Config) -> Arc<BotState> {
    let provider = OpenRouterProvider::from_config(config);

    let mut toolbox = ToolBox::new();
    toolbox.register(TimeTool);
    toolbox.register(TwStockTool::new());
    toolbox.register(WebSearchTool::new());
    toolbox.register(FetchPageTool::new());
    toolbox.register(ViewPageTool::new());

    let persona = PersonaCard::load(std::path::Path::new(&config.persona_card_path))
        .unwrap_or_else(|e| {
            warn!(error = ?e, "persona load failed, using stub");
            PersonaCard::stub()
        });

    let embed_router = EmbeddingRouter::new(&config.embed_server_url);
    if !config.embed_server_url.is_empty() {
        embed_router.spawn_warmup();
        info!(url = %config.embed_server_url, "embedding router warmup spawned");
    }

    if config.everos_url.is_empty() {
        BotState::new(
            config.clone(),
            provider,
            NullMemory,
            toolbox,
            persona,
            embed_router,
            None,
        )
    } else {
        let memory = EverosMemory::from_config(config);
        let everos_write = EverosMemory::from_config(config);
        BotState::new(
            config.clone(),
            provider,
            memory,
            toolbox,
            persona,
            embed_router,
            Some(everos_write),
        )
    }
}

async fn proactive_loop(state: Arc<BotState>, http: Arc<serenity::http::Http>, inactive_secs: u64) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let now = now_unix();

        let idle: Vec<serenity::model::id::ChannelId> = {
            state
                .last_seen
                .lock()
                .await
                .iter()
                .filter(|(_, &ts)| now.saturating_sub(ts) >= inactive_secs)
                .map(|(ch, _)| *ch)
                .collect()
        };

        for ch in idle {
            let persona = state.persona.read().await.clone();
            let input = TurnInput::new(
                &state.config,
                &persona,
                state.provider.as_ref(),
                state.memory.as_ref(),
                &state.toolbox,
                0,
                "[系統] 你有一段時間沒有聯繫用戶了，請主動傳一則符合角色的訊息。",
            );
            match run_turn(input).await {
                Ok(out) if !out.reply.is_empty() => {
                    info!(%ch, "sending proactive message");
                    if let Err(e) = ch.say(&http, &out.reply).await {
                        warn!(%ch, error = %e, "proactive send failed");
                    } else {
                        state.last_seen.lock().await.insert(ch, now_unix());
                    }
                }
                _ => {}
            }
        }
    }
}
