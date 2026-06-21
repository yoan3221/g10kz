//! OpenAI-compatible HTTP provider for OpenRouter and any compatible endpoint.
//!
//! # Features
//! - `tokio::select!` cancellation via [`tokio_util::sync::CancellationToken`]
//! - Exponential backoff retry on 429 / 5xx (max 2 retries)
//! - Inline circuit breaker (consecutive failure counter → open after N failures)
//! - Prefix-cache marking on system message (forwarded to Anthropic via OpenRouter)
//! - Cost parsing from OpenRouter's `x-openrouter-cost` response header

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use futures::StreamExt;
use serde_json::json;

use crate::{
    provider::{BoxFuture, BoxStream, Provider},
    serialize::{build_request, extract_reply, StreamChunk},
    types::{CompletionParams, Message, StreamItem, Usage},
    LlmError,
};

// ─── Circuit-breaker constants ───────────────────────────────────────────────

/// Consecutive failures before the circuit opens.
const CIRCUIT_OPEN_THRESHOLD: u32 = 5;
/// Half-open probe after this many seconds since last failure.
const CIRCUIT_RESET_SECS: u64 = 60;

// ─── OpenRouterProvider ───────────────────────────────────────────────────────

/// OpenAI-compatible HTTP client.
///
/// Clone-safe: all state is in `Arc`s so cloning is cheap.
#[derive(Clone)]
pub struct OpenRouterProvider {
    client: Client,
    base_url: String,
    api_key: String,
    /// Default cancellation token.  Replaced per-call in [`complete_with_cancel`].
    cancel: CancellationToken,
    /// Consecutive failure counter for circuit breaker.
    failures: Arc<AtomicU32>,
    /// Unix timestamp of last failure (for half-open probe).
    last_failure_ts: Arc<AtomicU32>,
}

impl OpenRouterProvider {
    /// Create a new provider.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self::new_with_timeout(base_url, api_key, Duration::from_secs(120))
    }

    /// Create a provider with an explicit HTTP timeout.
    pub fn new_with_timeout(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build failed");

        Self {
            client,
            base_url: base_url.into(),
            api_key: api_key.into(),
            cancel: CancellationToken::new(),
            failures: Arc::new(AtomicU32::new(0)),
            last_failure_ts: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Construct from [`g10kz_config::Config`].
    pub fn from_config(config: &g10kz_config::Config) -> Self {
        Self::new_with_timeout(
            &config.llm_base_url,
            &config.llm_api_key,
            config.request_timeout,
        )
    }

    /// True if the circuit breaker is currently open (provider should be skipped).
    pub fn circuit_open(&self) -> bool {
        let failures = self.failures.load(Ordering::Relaxed);
        if failures < CIRCUIT_OPEN_THRESHOLD {
            return false;
        }
        // Allow half-open probe after CIRCUIT_RESET_SECS
        let last = self.last_failure_ts.load(Ordering::Relaxed) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(last) < CIRCUIT_RESET_SECS
    }

    fn record_success(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }

    fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.last_failure_ts.store(now, Ordering::Relaxed);
    }

    /// Full completion with explicit cancellation token.
    pub async fn complete_with_cancel(
        &self,
        messages: &[Message],
        params: &CompletionParams,
        cancel: CancellationToken,
    ) -> anyhow::Result<(String, Usage)> {
        if self.circuit_open() {
            return Err(LlmError::Request("circuit breaker open".into()).into());
        }

        let body = build_request(messages, params, None);
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let mut last_err: anyhow::Error = anyhow::anyhow!("no attempts made");
        let max_retries = 2usize;

        for attempt in 0..=max_retries {
            if cancel.is_cancelled() {
                return Err(LlmError::Cancelled.into());
            }

            if attempt > 0 {
                let backoff = Duration::from_millis(500u64 * (1 << (attempt - 1)));
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = cancel.cancelled() => {
                        return Err(LlmError::Cancelled.into());
                    }
                }
            }

            let request = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .header("HTTP-Referer", "https://github.com/EverMind-AI/g10kz")
                .header("X-Title", "g10kz")
                .json(&body);

            let resp_future = request.send();

            let resp = tokio::select! {
                r = resp_future => r,
                _ = cancel.cancelled() => {
                    return Err(LlmError::Cancelled.into());
                }
            };

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    warn!(attempt, error = %e, "request failed");
                    last_err = e.into();
                    self.record_failure();
                    continue;
                }
            };

            let status = resp.status();

            // Retry on 429 (rate limit) and 5xx (server errors).
            if status == 429 || status.is_server_error() {
                warn!(attempt, %status, "retryable status");
                last_err = anyhow::anyhow!("HTTP {status}");
                self.record_failure();
                continue;
            }

            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                let err = LlmError::Request(format!("HTTP {status}: {body_text}"));
                self.record_failure();
                return Err(err.into());
            }

            // Parse cost from OpenRouter header (optional).
            let cost_usd: f64 = resp
                .headers()
                .get("x-openrouter-cost")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);

            let completion = tokio::select! {
                r = resp.json::<serde_json::Value>() => r,
                _ = cancel.cancelled() => {
                    return Err(LlmError::Cancelled.into());
                }
            };

            let json_val = match completion {
                Ok(v) => v,
                Err(e) => {
                    last_err = e.into();
                    self.record_failure();
                    continue;
                }
            };

            let comp_resp: crate::serialize::CompletionResponse = serde_json::from_value(json_val)
                .map_err(|e| LlmError::Request(format!("parse: {e}")))?;

            let (text, mut usage) = extract_reply(comp_resp)?;
            usage.cost_usd = cost_usd;

            self.record_success();
            debug!(model = %params.model, ptok = usage.prompt_tokens, ctok = usage.completion_tokens, "completion ok");

            return Ok((text, usage));
        }

        self.record_failure();
        Err(last_err)
    }
}

impl Provider for OpenRouterProvider {
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        params: &'a CompletionParams,
    ) -> BoxFuture<'a, anyhow::Result<(String, Usage)>> {
        let cancel = self.cancel.clone();
        Box::pin(async move { self.complete_with_cancel(messages, params, cancel).await })
    }

    /// Streaming completion over OpenAI-compatible SSE (`stream: true`).
    /// Clones inputs into a spawned task and bridges the byte stream into a
    /// `StreamItem` channel. No retry/circuit-breaker on the streaming path.
    fn complete_stream(
        &self,
        messages: &[Message],
        params: &CompletionParams,
        cancel: CancellationToken,
    ) -> BoxStream<anyhow::Result<StreamItem>> {
        // Build the owned request body up front (borrows end here).
        let body = build_request(
            messages,
            params,
            Some(json!({ "stream": true, "stream_options": { "include_usage": true } })),
        );
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<StreamItem>>(64);

        tokio::spawn(async move {
            let request = client
                .post(&url)
                .bearer_auth(&api_key)
                .header("HTTP-Referer", "https://github.com/EverMind-AI/g10kz")
                .header("X-Title", "g10kz")
                .json(&body);

            let resp = tokio::select! {
                r = request.send() => r,
                _ = cancel.cancelled() => return,
            };
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Err(e.into())).await;
                    return;
                }
            };
            let status = resp.status();
            if !status.is_success() {
                let txt = resp.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(
                        LlmError::Request(format!("HTTP {status}: {txt}")).into()
                    ))
                    .await;
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut usage = Usage::default();

            loop {
                let chunk = tokio::select! {
                    c = byte_stream.next() => c,
                    _ = cancel.cancelled() => break,
                };
                let Some(chunk) = chunk else { break };
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx.send(Err(e.into())).await;
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&bytes));

                // Drain complete SSE lines.
                while let Some(nl) = buf.find('\n') {
                    let line: String = buf[..nl].trim().to_string();
                    buf.drain(..=nl);
                    let Some(payload) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let payload = payload.trim();
                    if payload.is_empty() {
                        continue;
                    }
                    if payload == "[DONE]" {
                        let _ = tx.send(Ok(StreamItem::Done(usage.clone()))).await;
                        return;
                    }
                    if let Ok(sc) = serde_json::from_str::<StreamChunk>(payload) {
                        if let Some(u) = sc.usage {
                            usage.prompt_tokens = u.prompt_tokens;
                            usage.completion_tokens = u.completion_tokens;
                            usage.cost_usd = u.cost.unwrap_or(usage.cost_usd);
                            usage.cached = u
                                .prompt_tokens_details
                                .as_ref()
                                .is_some_and(|d| d.cached_tokens > 0);
                        }
                        if let Some(choice) = sc.choices.into_iter().next() {
                            if let Some(txt) = choice.delta.content {
                                if !txt.is_empty()
                                    && tx.send(Ok(StreamItem::Token(txt))).await.is_err()
                                {
                                    return; // receiver dropped
                                }
                            }
                        }
                    }
                }
            }
            let _ = tx.send(Ok(StreamItem::Done(usage))).await;
        });

        Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)))
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider() -> OpenRouterProvider {
        OpenRouterProvider::new("https://openrouter.ai/api/v1", "test-key")
    }

    #[test]
    fn circuit_starts_closed() {
        let p = make_provider();
        assert!(!p.circuit_open());
    }

    #[test]
    fn circuit_opens_after_threshold() {
        let p = make_provider();
        for _ in 0..CIRCUIT_OPEN_THRESHOLD {
            p.record_failure();
        }
        assert!(p.circuit_open());
    }

    #[test]
    fn success_resets_circuit() {
        let p = make_provider();
        for _ in 0..CIRCUIT_OPEN_THRESHOLD {
            p.record_failure();
        }
        p.record_success();
        assert!(!p.circuit_open());
    }

    #[tokio::test]
    async fn cancellation_returns_cancelled_error() {
        let p = make_provider();
        let msgs = vec![crate::types::Message::text(crate::types::Role::User, "hi")];
        let params = crate::types::CompletionParams::social("mock");
        let cancel = CancellationToken::new();
        cancel.cancel(); // pre-cancel

        let result = p.complete_with_cancel(&msgs, &params, cancel).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("cancel") || err.to_string().contains("Cancel"),
            "expected cancellation error, got: {err}"
        );
    }

    #[tokio::test]
    #[ignore] // requires live OpenRouter endpoint
    async fn live_complete_returns_text() {
        let key = std::env::var("LLM_API_KEY").expect("LLM_API_KEY required");
        let p = OpenRouterProvider::new("https://openrouter.ai/api/v1", key);
        let msgs = vec![crate::types::Message::text(crate::types::Role::User, "hi")];
        let params = crate::types::CompletionParams::social("openai/gpt-4o-mini");
        let (reply, _) = p.complete(&msgs, &params).await.unwrap();
        assert!(!reply.is_empty());
    }
}
