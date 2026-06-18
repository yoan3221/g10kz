//! `Provider` trait — the single abstraction for all LLM backends.

use std::future::Future;
use std::pin::Pin;

use futures::Stream;
use tokio_util::sync::CancellationToken;

use crate::types::{CompletionParams, Message, StreamItem, Usage};

/// `Pin<Box<dyn Future<Output = T> + Send + 'a>>` — shorthand used by the trait.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `Pin<Box<dyn Stream<Item = T> + Send + 'static>>` — streaming shorthand.
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send + 'static>>;

/// Async LLM completion provider. Object-safe via boxed futures/streams.
pub trait Provider: Send + Sync {
    /// Request a (non-streaming) completion. Returns `(reply_text, usage)`.
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        params: &'a CompletionParams,
    ) -> BoxFuture<'a, anyhow::Result<(String, Usage)>>;

    /// Streaming completion. Yields incremental `StreamItem::Token` deltas, then
    /// a final `StreamItem::Done` carrying token usage. `cancel` aborts mid-flight.
    /// Default impl errors — only HTTP providers override it.
    fn complete_stream(
        &self,
        messages: &[Message],
        params: &CompletionParams,
        cancel: CancellationToken,
    ) -> BoxStream<anyhow::Result<StreamItem>> {
        let _ = (messages, params, cancel);
        Box::pin(futures::stream::once(async {
            Err(anyhow::anyhow!("streaming not supported by this provider"))
        }))
    }
}
