//! Async HTTP client for the prompt-injection guard service.
//!
//! POSTs to `POST /classify` on a local ONNX-based FastAPI service.
//! **Fail-open**: any network or parse error returns `false` so the guard
//! never blocks the bot when the service is down.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Serialize)]
struct Req<'a> {
    text: &'a str,
}

#[derive(Deserialize)]
struct Resp {
    blocked: bool,
}

pub struct PromptGuardClient {
    client: Client,
    url: String,
}

impl PromptGuardClient {
    /// Create a new client pointing at `base_url` (e.g. `http://localhost:8083`).
    /// Passing an empty string disables the guard.
    pub fn new(base_url: &str) -> Self {
        let url = if base_url.is_empty() {
            String::new()
        } else {
            format!("{}/classify", base_url.trim_end_matches('/'))
        };
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_millis(1000))
                .build()
                .expect("reqwest client"),
            url,
        }
    }

    /// Returns `true` if the guard service flags this text as an injection attempt.
    /// Always returns `false` when disabled (empty URL) or on any error.
    pub async fn is_injection(&self, text: &str) -> bool {
        if self.url.is_empty() {
            return false;
        }
        match self.client.post(&self.url).json(&Req { text }).send().await {
            Ok(resp) => match resp.json::<Resp>().await {
                Ok(r) => r.blocked,
                Err(e) => {
                    warn!(error = %e, "prompt-guard: response parse error");
                    false
                }
            },
            Err(e) => {
                warn!(error = %e, "prompt-guard: service unreachable");
                false
            }
        }
    }
}
