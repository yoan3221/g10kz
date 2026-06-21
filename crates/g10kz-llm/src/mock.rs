//! `MockProvider` — scripted, deterministic LLM for offline testing and P1.
//!
//! Returns pre-configured responses in round-robin order.
//! No network I/O, no timing — fully synchronous under the async interface.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::{
    provider::{BoxFuture, Provider},
    types::{CompletionParams, Message, Usage},
};

// ─── MockProvider ────────────────────────────────────────────────────────────

/// Returns scripted replies in round-robin order.
///
/// # Example
/// ```
/// use g10kz_llm::{MockProvider, Provider, types::{Message, Role, CompletionParams}};
///
/// # tokio_test::block_on(async {
/// let mock = MockProvider::new(vec!["哼，隨便你。".into()]);
/// let msgs = vec![Message::text(Role::User, "你好")];
/// let (reply, _usage) = mock.complete(&msgs, &CompletionParams::social("mock")).await.unwrap();
/// assert_eq!(reply, "哼，隨便你。");
/// # });
/// ```
#[derive(Clone)]
pub struct MockProvider {
    replies: Arc<Vec<String>>,
    counter: Arc<AtomicUsize>,
}

impl MockProvider {
    /// Create a provider that cycles through `replies`.
    /// Panics if `replies` is empty.
    pub fn new(replies: Vec<String>) -> Self {
        assert!(
            !replies.is_empty(),
            "MockProvider requires at least one reply"
        );
        Self {
            replies: Arc::new(replies),
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Convenience: single reply repeated forever.
    pub fn with_reply(reply: impl Into<String>) -> Self {
        Self::new(vec![reply.into()])
    }

    /// Default social-path mock reply for the walking skeleton.
    pub fn social_default() -> Self {
        Self::with_reply("哼⋯你問這個幹嘛，又不是說不可以告訴你。".to_owned())
    }
}

impl Provider for MockProvider {
    fn complete<'a>(
        &'a self,
        _messages: &'a [Message],
        _params: &'a CompletionParams,
    ) -> BoxFuture<'a, anyhow::Result<(String, Usage)>> {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % self.replies.len();
        let reply = self.replies[idx].clone();
        Box::pin(async move { Ok((reply, Usage::default())) })
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};

    #[tokio::test]
    async fn round_robin() {
        let mock = MockProvider::new(vec!["a".into(), "b".into()]);
        let msgs = vec![Message::text(Role::User, "hi")];
        let params = CompletionParams::social("mock");

        let (r1, _) = mock.complete(&msgs, &params).await.unwrap();
        let (r2, _) = mock.complete(&msgs, &params).await.unwrap();
        let (r3, _) = mock.complete(&msgs, &params).await.unwrap();

        assert_eq!(r1, "a");
        assert_eq!(r2, "b");
        assert_eq!(r3, "a"); // wraps
    }

    #[tokio::test]
    async fn social_default_is_non_empty() {
        let mock = MockProvider::social_default();
        let msgs = vec![Message::text(Role::User, "你好")];
        let (reply, _) = mock
            .complete(&msgs, &CompletionParams::social("mock"))
            .await
            .unwrap();
        assert!(!reply.is_empty());
    }
}
