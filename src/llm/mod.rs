//! LLM provider abstraction, built-in templates, and HTTP client retry helper.

pub mod client;
pub mod prompts;
pub mod templates;

use async_trait::async_trait;

use crate::error::TranslateError;

/// Provider-agnostic LLM completion.
///
/// Implementations:
///   - `crate::llm::anthropic::AnthropicProvider` (Task 10)
///   - `crate::llm::openai::OpenAiCompatibleProvider` (Task 11)
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Run a completion. `system` is the rendered template (system prompt).
    /// `user` is the clipboard text.
    async fn complete(&self, system: &str, user: &str) -> Result<String, TranslateError>;
}
