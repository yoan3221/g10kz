//! Typed configuration for g10kz, loaded from environment variables.
//!
//! L0 — no internal crate dependencies.

use std::time::Duration;

use serde::Deserialize;

/// Central, immutable configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub discord_token: String,
    pub llm_provider: String,
    pub llm_base_url: String,
    pub llm_api_key: String,
    pub llm_model_social: String,
    pub llm_model_reason: String,
    pub llm_model_judge: String,
    pub llm_fusion_drafters: Vec<String>,
    pub everos_url: String,
    pub owner_user_id: u64,
    pub request_timeout: Duration,
    pub log_level: String,
    pub blacklisted_users: Vec<u64>,
    pub proactive_inactive_secs: u64,
    pub persona_card_path: String,
    pub embed_server_url: String,
    /// URL of the ML prompt-injection guard service (e.g. http://localhost:8083).
    /// Empty string disables the guard.
    pub prompt_guard_url: String,
    /// Path to the Obscura headless browser binary for web search.
    /// Defaults to `/usr/local/bin/obscura`. Empty string disables page fetching.
    pub obscura_path: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let llm_fusion_drafters = std::env::var("LLM_FUSION_DRAFTERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            discord_token: std::env::var("DISCORD_TOKEN").unwrap_or_default(),
            llm_provider: std::env::var("LLM_PROVIDER")
                .unwrap_or_else(|_| "openrouter".into()),
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
            log_level: std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "g10kz=info,warn".into()),
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
            embed_server_url: std::env::var("EMBED_SERVER_URL")
                .unwrap_or_else(|_| "http://localhost:8082".into()),
            prompt_guard_url: std::env::var("PROMPT_GUARD_URL")
                .unwrap_or_else(|_| "http://localhost:8083".into()),
            obscura_path: std::env::var("OBSCURA_PATH")
                .unwrap_or_else(|_| "/usr/local/bin/obscura".into()),
        })
    }

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
            embed_server_url: String::new(),
            prompt_guard_url: String::new(),
            obscura_path: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_default_is_mock_provider() {
        assert_eq!(Config::mock_default().llm_provider, "mock");
    }

    #[test]
    fn mock_default_has_two_fusion_drafters() {
        assert_eq!(Config::mock_default().llm_fusion_drafters.len(), 2);
    }

    #[test]
    fn mock_default_embed_server_url_empty() {
        assert!(Config::mock_default().embed_server_url.is_empty());
    }
}
