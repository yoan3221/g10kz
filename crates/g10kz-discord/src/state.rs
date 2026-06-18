//! Shared bot state — Arc-wrapped, passed into every event handler.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use serenity::model::id::{ChannelId, MessageId};
use g10kz_config::Config;
use g10kz_everos::{EverosMemory, Memory};
use g10kz_kernel::persona::PersonaCard;
use g10kz_kernel::PersonalityState;
use g10kz_llm::Provider;
use g10kz_tools::ToolBox;
use g10kz_engine::{EmbeddingRouter, PromptGuardClient};

/// Maximum conversation exchanges per channel kept in the ring buffer.
pub const RING_SIZE: usize = 30;

/// One exchange in the per-channel conversation ring buffer.
pub struct ContextEntry {
    pub user_id: u64,
    /// Discord display name at message time (guild nick > global name > username).
    pub user_name: String,
    /// Reply context (`Alice「…」`) if this message replied to another. Group only.
    pub reply_to: Option<String>,
    pub user_text: String,
    pub bot_reply: Option<String>,
}

/// All shared mutable state for the bot lifetime.
pub struct BotState {
    pub config: Arc<Config>,
    pub provider: Arc<dyn Provider>,
    pub memory: Arc<dyn Memory>,
    pub toolbox: Arc<ToolBox>,
    pub persona: Arc<RwLock<PersonaCard>>,
    /// Semantic route refinement router (warmed up at startup).
    pub embed_router: Arc<EmbeddingRouter>,
    /// ML prompt-injection guard client.
    pub prompt_guard: Arc<PromptGuardClient>,
    /// Per-channel conversation ring buffer (last RING_SIZE exchanges).
    pub channel_ctx: Mutex<HashMap<ChannelId, VecDeque<ContextEntry>>>,
    /// In-flight message IDs — prevents double-processing on reshard.
    pub in_flight: Mutex<HashSet<MessageId>>,
    /// Per-channel cancellation tokens — cancelled by /stop.
    pub cancel_map: Mutex<HashMap<ChannelId, CancellationToken>>,
    /// Channels with debug trace output enabled.
    pub trace_channels: Mutex<HashSet<ChannelId>>,
    /// Last message Unix timestamp per channel (for proactive scheduling).
    pub last_seen: Mutex<HashMap<ChannelId, u64>>,
    /// Per-user JPAF personality adaptation state.
    pub personality_states: Mutex<HashMap<u64, PersonalityState>>,
    /// Direct EverosMemory handle for add_turn writes (Some when EVEROS_URL is set).
    pub everos: Option<Arc<EverosMemory>>,
}

impl BotState {
    pub fn new(
        config: Config,
        provider: impl Provider + 'static,
        memory: impl Memory + 'static,
        toolbox: ToolBox,
        persona: PersonaCard,
        embed_router: EmbeddingRouter,
        everos: Option<EverosMemory>,
    ) -> Arc<Self> {
        let prompt_guard_url = config.prompt_guard_url.clone();
        Arc::new(Self {
            config: Arc::new(config),
            provider: Arc::new(provider),
            memory: Arc::new(memory),
            toolbox: Arc::new(toolbox),
            persona: Arc::new(RwLock::new(persona)),
            embed_router: Arc::new(embed_router),
            prompt_guard: Arc::new(PromptGuardClient::new(&prompt_guard_url)),
            channel_ctx: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            cancel_map: Mutex::new(HashMap::new()),
            trace_channels: Mutex::new(HashSet::new()),
            last_seen: Mutex::new(HashMap::new()),
            personality_states: Mutex::new(HashMap::new()),
            everos: everos.map(Arc::new),
        })
    }
}
