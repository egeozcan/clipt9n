//! OpenAI-compatible Chat Completions provider.
//!
//! Works with: OpenAI, Google Gemini (via OpenAI-compat endpoint), DeepSeek,
//! local Ollama. Distinct from Anthropic's `/messages` shape — uses
//! `/chat/completions` with messages-array system+user split.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::client::{default_backoffs, with_retry, AttemptOutcome};
use super::LlmProvider;
use crate::config::ProviderEndpoint;
use crate::error::TranslateError;

pub struct OpenAiCompatibleProvider {
    http: Client,
    endpoint: ProviderEndpoint,
    api_key: Zeroizing<String>,
    model: String,
    backoffs: Vec<Duration>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        endpoint: ProviderEndpoint,
        api_key: Zeroizing<String>,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, TranslateError> {
        let http = super::client::provider_http_client(timeout)?;
        Ok(Self {
            http,
            endpoint,
            api_key,
            model: model.into(),
            backoffs: default_backoffs(),
        })
    }

    /// Tests inject custom backoffs to keep the test suite fast.
    /// `#[doc(hidden)]` (not `#[cfg(test)]`) so integration tests in `tests/`
    /// can call this — those compile without `cfg(test)`.
    #[doc(hidden)]
    pub fn with_backoffs(mut self, backoffs: Vec<Duration>) -> Self {
        self.backoffs = backoffs;
        self
    }
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(&self, system: &str, user: &str) -> Result<String, TranslateError> {
        let url = self.endpoint.request_url("chat/completions");
        let body = OpenAiRequest {
            model: &self.model,
            messages: vec![
                OpenAiMessage {
                    role: "system",
                    content: system,
                },
                OpenAiMessage {
                    role: "user",
                    content: user,
                },
            ],
        };
        let body_bytes = serde_json::to_vec(&body).map_err(|e| TranslateError::Provider {
            status: 0,
            message: format!("serialising request: {e}"),
        })?;

        with_retry(&self.backoffs, || {
            let body_bytes = body_bytes.clone();
            let url = url.clone();
            let api_key = self.api_key.clone();
            let http = self.http.clone();
            async move {
                match http
                    .post(url)
                    .header("authorization", format!("Bearer {}", &**api_key))
                    .header("content-type", "application/json")
                    .body(body_bytes)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            match resp.json::<OpenAiResponse>().await {
                                Ok(parsed) => match parsed.choices.into_iter().next() {
                                    Some(c) => AttemptOutcome::Done(c.message.content),
                                    None => AttemptOutcome::Fatal(TranslateError::Provider {
                                        status: status.as_u16(),
                                        message: "no choices in response".into(),
                                    }),
                                },
                                Err(e) => AttemptOutcome::Fatal(TranslateError::Provider {
                                    status: status.as_u16(),
                                    message: format!("parsing response: {e}"),
                                }),
                            }
                        } else if status == StatusCode::TOO_MANY_REQUESTS {
                            if let Some(delay) =
                                super::client::parse_retry_after(resp.headers().get("Retry-After"))
                            {
                                AttemptOutcome::RetryAfter(delay, TranslateError::RateLimited)
                            } else {
                                AttemptOutcome::Fatal(TranslateError::RateLimited)
                            }
                        } else if status.is_server_error() {
                            AttemptOutcome::Retry(TranslateError::Provider {
                                status: status.as_u16(),
                                message: super::client::bounded_provider_error(resp).await,
                            })
                        } else {
                            AttemptOutcome::Fatal(TranslateError::Provider {
                                status: status.as_u16(),
                                message: super::client::bounded_provider_error(resp).await,
                            })
                        }
                    }
                    Err(e) if e.is_timeout() => AttemptOutcome::Fatal(TranslateError::Timeout),
                    Err(e) => AttemptOutcome::Fatal(TranslateError::Network(e.to_string())),
                }
            }
        })
        .await
    }
}
