//! LLM provider abstraction, built-in templates, and HTTP client retry helper.

pub mod anthropic;
pub mod client;
pub mod openai;
pub mod prompts;
pub mod templates;

use async_trait::async_trait;

use crate::error::TranslateError;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String, TranslateError>;
}
