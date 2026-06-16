//! `Provider` trait — the single abstraction for all LLM backends.
//!
//! Uses `BoxFuture` (= `Pin<Box<dyn Future + Send>>`) so the trait is
//! **object-safe**: callers can hold `&dyn Provider` or `Box<dyn Provider>`
//! and swap between mock / real / Fusion at runtime.

use std::future::Future;
use std::pin::Pin;

use crate::types::{CompletionParams, Message, Usage};

/// `Pin<Box<dyn Future<Output = T> + Send + 'a>>` — shorthand used by the trait.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Async LLM completion provider.
///
/// **Object-safe**: use `&dyn Provider` or `Box<dyn Provider>` for dynamic
/// dispatch between `MockProvider`, `OpenRouterProvider`, and `FusionProvider`.
///
/// Implementors must wrap their async body in `Box::pin(async move { ... })`.
pub trait Provider: Send + Sync {
    /// Request a completion.
    ///
    /// Returns `(reply_text, usage)`.  On failure returns [`anyhow::Error`].
    /// The engine layer applies retry / fallback / circuit-breaker logic.
    fn complete<'a>(
        &'a self,
        messages: &'a [Message],
        params: &'a CompletionParams,
    ) -> BoxFuture<'a, anyhow::Result<(String, Usage)>>;
}
