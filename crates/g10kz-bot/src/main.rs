//! g10kz v5 — 傲嬌 AI Discord bot
//!
//! # Subcommands
//! - `once <text>`  — offline single-turn (development / CI smoke test)
//! - `daemon`       — connect to Discord and run until signal
//! - `proactive`    — note: proactive runs automatically inside daemon

use anyhow::Context;
use tracing::info;

use g10kz_config::Config;
use g10kz_engine::turn::{run_turn, TurnInput};
use g10kz_everos::NullMemory;
use g10kz_kernel::persona::PersonaCard;
use g10kz_llm::{MockProvider, OpenRouterProvider};

// ─── main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env().context("failed to load config")?;
    init_tracing(&config.log_level);

    let args: Vec<String> = std::env::args().collect();
    let subcommand = args.get(1).map(String::as_str).unwrap_or("help");

    match subcommand {
        "once" => {
            let text = args[2..].join(" ");
            cmd_once(&config, text).await
        }
        "daemon" => cmd_daemon(&config).await,
        "proactive" => {
            info!("proactive messaging runs automatically within daemon mode");
            info!("hint: use `g10kz-bot daemon` — proactive fires every 60 s for idle channels");
            Ok(())
        }
        _ => {
            eprintln!("g10kz-bot v{}", env!("CARGO_PKG_VERSION"));
            eprintln!();
            eprintln!("USAGE:");
            eprintln!("  g10kz-bot once <text>   — offline single-turn smoke test");
            eprintln!("  g10kz-bot daemon         — connect to Discord (needs DISCORD_TOKEN)");
            eprintln!("  g10kz-bot proactive      — alias: proactive runs inside daemon");
            Ok(())
        }
    }
}

// ─── subcommands ─────────────────────────────────────────────────────────────

/// Offline single-turn mode — no Discord, no network required when LLM_PROVIDER=mock.
async fn cmd_once(config: &Config, text: String) -> anyhow::Result<()> {
    info!(text = %text, "once mode");

    let persona = PersonaCard::stub();

    let reply = if config.llm_provider == "mock" {
        let provider = MockProvider::social_default();
        let memory = NullMemory;
        let toolbox = g10kz_tools::ToolBox::new();
        let input = TurnInput::new(config, &persona, &provider, &memory, &toolbox, 0, text);
        run_turn(input).await?.reply
    } else {
        let provider = OpenRouterProvider::from_config(config);
        let memory = NullMemory;
        let toolbox = g10kz_tools::ToolBox::new();
        let input = TurnInput::new(config, &persona, &provider, &memory, &toolbox, 0, text);
        run_turn(input).await?.reply
    };

    println!("{reply}");
    Ok(())
}

/// Full Discord daemon — connects gateway, handles messages, runs proactive loop.
async fn cmd_daemon(config: &Config) -> anyhow::Result<()> {
    info!("daemon mode");
    g10kz_discord::run_gateway(config).await
}

// ─── tracing init ────────────────────────────────────────────────────────────

fn init_tracing(log_level: &str) {
    use tracing_subscriber::{fmt, EnvFilter};

    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .with_target(true)
        .compact()
        .init();
}
