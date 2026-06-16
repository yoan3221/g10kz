//! Slash command definitions and dispatch.

use std::sync::Arc;
use serenity::{
    builder::{
        CreateCommand, CreateCommandOption, CreateInteractionResponse,
        CreateInteractionResponseMessage,
    },
    client::Context,
    model::application::{CommandInteraction, CommandOptionType},
};
use tracing::{info, warn};
use g10kz_kernel::persona::PersonaCard;
use crate::state::BotState;

/// Build the list of global slash commands to register on `ready`.
pub fn global_commands() -> Vec<CreateCommand> {
    vec![
        CreateCommand::new("reset")
            .description("清除此頻道的對話記錄"),
        CreateCommand::new("stop")
            .description("取消目前正在生成的回覆"),
        CreateCommand::new("memory")
            .description("搜尋小十的記憶")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "query", "搜尋關鍵字")
                    .required(true),
            ),
        CreateCommand::new("persona")
            .description("重新載入人設檔案"),
        CreateCommand::new("trace")
            .description("切換除錯追蹤輸出"),
    ]
}

/// Dispatch a slash command interaction to the appropriate handler.
pub async fn handle_command(ctx: &Context, cmd: &CommandInteraction, state: &Arc<BotState>) {
    let channel_id = cmd.channel_id;
    let user_id = cmd.user.id.get();

    let reply_text = match cmd.data.name.as_str() {
        "reset" => {
            state.channel_ctx.lock().await.remove(&channel_id);
            info!(%channel_id, "history reset");
            "已清除此頻道的對話記錄。".to_owned()
        }

        "stop" => {
            if let Some(token) = state.cancel_map.lock().await.remove(&channel_id) {
                token.cancel();
                info!(%channel_id, "turn cancelled via /stop");
                "已取消當前回覆。".to_owned()
            } else {
                "目前沒有正在生成的回覆。".to_owned()
            }
        }

        "memory" => {
            let query = cmd
                .data
                .options
                .iter()
                .find(|o| o.name == "query")
                .and_then(|o| o.value.as_str())
                .unwrap_or("");
            if query.is_empty() {
                "請提供搜尋關鍵字。".to_owned()
            } else {
                let entries = state.memory.search(user_id, query, 5).await;
                if entries.is_empty() {
                    format!("找不到與「{}」相關的記憶。", query)
                } else {
                    entries
                        .iter()
                        .map(|e| format!("• {}", e.text))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
        }

        "persona" => {
            let path_str = state.config.persona_card_path.clone();
            let path = std::path::Path::new(&path_str);
            match PersonaCard::load(path) {
                Ok(card) => {
                    *state.persona.write().await = card;
                    info!(path = %path_str, "persona reloaded");
                    "人設已重新載入。".to_owned()
                }
                Err(e) => {
                    warn!(error = ?e, "persona reload failed");
                    format!("人設載入失敗：{e:?}")
                }
            }
        }

        "trace" => {
            let mut traces = state.trace_channels.lock().await;
            if traces.contains(&channel_id) {
                traces.remove(&channel_id);
                "除錯追蹤已關閉。".to_owned()
            } else {
                traces.insert(channel_id);
                "除錯追蹤已開啟（此頻道）。".to_owned()
            }
        }

        other => {
            warn!(cmd = other, "unknown slash command");
            return;
        }
    };

    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(&reply_text)
            .ephemeral(true),
    );
    if let Err(e) = cmd.create_response(ctx, response).await {
        warn!(error = %e, "failed to respond to slash command");
    }
}
