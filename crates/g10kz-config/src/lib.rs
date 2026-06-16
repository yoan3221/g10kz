//! Typed configuration for g10kz, loaded from environment variables.
//!
//! L0 — no internal crate dependencies.

use std::time::Duration;

use serde::Deserialize;

// ─── Config ─────────────────────────────────────────────────────────────────

/// Central, immutable configuration.
/// Construct once at startup via [`Config::from_env`] or [`Config::mock_default`].
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Discord bot token.
    pub discord_token: String,

    /// Active LLM provider: `"mock"` | `"openrouter"` | `"openai-compat"`.
    pub llm_provider: String,

    /// Base URL for the OpenAI-compatible API endpoint.
    pub llm_base_url: String,

    /// API key forwarded in `Authorization: Bearer …`.
    pub llm_api_key: String,

    // ── per-path model selection ──────────────────────────────
    pub llm_model_social: String,
    pub llm_model_reason: String,
    pub llm_model_judge: String,

    /// Comma-separated model ids used as Fusion drafters.
    pub llm_fusion_drafters: Vec<String>,

    /// EverOS HTTP sidecar base URL.
    pub everos_url: String,

    /// Discord snowflake of the bot owner (trusted for owner-only commands).
    pub owner_user_id: u64,

    /// Timeout applied to every outbound HTTP request.
    pub request_timeout: Duration,

    /// `RUST_LOG` filter string passed to `tracing-subscriber`.
    pub log_level: String,

    /// Discord snowflake IDs of blacklisted users.
    /// Blacklisted users receive restricted-mode responses (no media/search/memory).
    pub blacklisted_users: Vec<u64>,

    /// Minimum inactive seconds before proactive messaging fires.
    /// Default: 86400 (24 hours).
    pub proactive_inactive_secs: u64,

    /// Path to SillyTavern V2 character card JSON.
    /// Falls back to built-in stub when empty or file not found.
    pub persona_card_path: String,

    /// Cloudflare account ID for AI Search tool.
    pub cf_account_id: String,

    /// Cloudflare API token for AI Search.
    pub cf_api_token: String,
}

impl Config {
    /// Load from environment variables.
    /// Calls [`dotenvy::dotenv`] first so a `.env` file is picked up automatically.
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let llm_fusion_drafters = std::env::var("LLM_FUSION_DRAFTERS")
            .unwrap_or_else(|_| {
                "openai/gpt-4o,anthropic/claude-3-5-sonnet,google/gemini-2.0-flash".into()
            })
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            discord_token: std::env::var("DISCORD_TOKEN").unwrap_or_default(),
            llm_provider: std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "openrouter".into()),
            llm_base_url: std::env::var("LLM_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".into()),
            llm_api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
            llm_model_social: std::env::var("LLM_MODEL_SOCIAL")
                .unwrap_or_else(|_| "openai/gpt-4o-mini".into()),
            llm_model_reason: std::env::var("LLM_MODEL_REASON")
                .unwrap_or_else(|_| "openai/gpt-4o".into()),
            llm_model_judge: std::env::var("LLM_MODEL_JUDGE")
                .unwrap_or_else(|_| "anthropic/claude-3-5-haiku".into()),
            llm_fusion_drafters,
            everos_url: std::env::var("EVEROS_URL")
                .unwrap_or_else(|_| "http://localhost:7700".into()),
            owner_user_id: std::env::var("OWNER_USER_ID")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            request_timeout: Duration::from_secs(
                std::env::var("REQUEST_TIMEOUT_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            ),
            log_level: std::env::var("RUST_LOG").unwrap_or_else(|_| "g10kz=info,warn".into()),
            blacklisted_users: std::env::var("BLACKLISTED_USERS")
                .unwrap_or_default()
                .split(',')
                .filter_map(|s| s.trim().parse::<u64>().ok())
                .collect(),
            proactive_inactive_secs: std::env::var("PROACTIVE_INACTIVE_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(86400),
            persona_card_path: std::env::var("PERSONA_CARD_PATH").unwrap_or_default(),
            cf_account_id: std::env::var("CF_ACCOUNT_ID").unwrap_or_default(),
            cf_api_token: std::env::var("CF_API_TOKEN").unwrap_or_default(),
        })
    }

    /// Offline-safe defaults for unit tests and `LLM_PROVIDER=mock` runs.
    pub fn mock_default() -> Self {
        Self {
            discord_token: String::new(),
            llm_provider: "mock".into(),
            llm_base_url: String::new(),
            llm_api_key: String::new(),
            llm_model_social: "mock-social".into(),
            llm_model_reason: "mock-reason".into(),
            llm_model_judge: "mock-judge".into(),
            llm_fusion_drafters: vec!["mock-a".into(), "mock-b".into()],
            everos_url: "http://localhost:7700".into(),
            owner_user_id: 0,
            request_timeout: Duration::from_secs(5),
            log_level: "debug".into(),
            blacklisted_users: vec![],
            proactive_inactive_secs: 86400,
            persona_card_path: String::new(),
            cf_account_id: String::new(),
            cf_api_token: String::new(),
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_default_is_mock_provider() {
        let cfg = Config::mock_default();
        assert_eq!(cfg.llm_provider, "mock");
    }

    #[test]
    fn mock_default_has_two_fusion_drafters() {
        let cfg = Config::mock_default();
        assert_eq!(cfg.llm_fusion_drafters.len(), 2);
    }
}
