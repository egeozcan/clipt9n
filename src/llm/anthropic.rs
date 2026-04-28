//! Anthropic Messages API provider. Spec §5.5 request shape.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::client::{default_backoffs, with_retry, AttemptOutcome};
use super::LlmProvider;
use crate::error::TranslateError;

pub struct AnthropicProvider {
    http: Client,
    base_url: String,
    api_key: Zeroizing<String>,
    model: String,
    backoffs: Vec<Duration>,
}

impl AnthropicProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Zeroizing<String>,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, TranslateError> {
        let http = Client::builder()
            .timeout(timeout)
            .user_agent(concat!("clipt9n/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| TranslateError::Network(format!("building HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            api_key,
            model: model.into(),
            backoffs: default_backoffs(),
        })
    }

    /// Tests inject custom backoffs to keep the test suite fast.
    #[doc(hidden)]
    pub fn with_backoffs(mut self, backoffs: Vec<Duration>) -> Self {
        self.backoffs = backoffs;
        self
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<AnthropicMessage<'a>>,
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, system: &str, user: &str) -> Result<String, TranslateError> {
        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: 4096,
            system,
            messages: vec![AnthropicMessage { role: "user", content: user }],
        };
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| TranslateError::Provider { status: 0, message: format!("serialising request: {e}") })?;

        with_retry(&self.backoffs, || {
            let body_bytes = body_bytes.clone();
            let url = url.clone();
            let api_key = self.api_key.clone();
            let http = self.http.clone();
            async move {
                match http
                    .post(&url)
                    .header("x-api-key", &**api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .body(body_bytes)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() {
                            match resp.json::<AnthropicResponse>().await {
                                Ok(parsed) => match parsed.content.into_iter().find(|c| c.kind == "text") {
                                    Some(c) => AttemptOutcome::Done(c.text),
                                    None => AttemptOutcome::Fatal(TranslateError::Provider {
                                        status: status.as_u16(),
                                        message: "no text content in response".into(),
                                    }),
                                },
                                Err(e) => AttemptOutcome::Fatal(TranslateError::Provider {
                                    status: status.as_u16(),
                                    message: format!("parsing response: {e}"),
                                }),
                            }
                        } else if status == StatusCode::TOO_MANY_REQUESTS {
                            AttemptOutcome::Fatal(TranslateError::RateLimited)
                        } else if status.is_server_error() {
                            AttemptOutcome::Retry(TranslateError::Provider {
                                status: status.as_u16(),
                                message: resp.text().await.unwrap_or_default(),
                            })
                        } else {
                            AttemptOutcome::Fatal(TranslateError::Provider {
                                status: status.as_u16(),
                                message: resp.text().await.unwrap_or_default(),
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
