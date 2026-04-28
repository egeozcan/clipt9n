//! Integration tests for HTTP retry behavior across providers.
//!
//! Critical: these tests verify the resolution of the spec §8 retry-policy
//! ambiguity — exactly two retries with 1s and 2s sleeps, three attempts total
//! (we use millisecond sleeps in tests). See M1 exit criterion 4 in
//! `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md`.

use std::time::Duration;

use clipt9n::error::TranslateError;
use clipt9n::llm::anthropic::AnthropicProvider;
use clipt9n::llm::LlmProvider;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

const SUCCESS_BODY: &str = r#"{
    "content": [{"type": "text", "text": "Hallo, Welt."}]
}"#;

fn fast_backoffs() -> Vec<Duration> {
    vec![Duration::from_millis(1), Duration::from_millis(2)]
}

fn provider(server: &MockServer) -> AnthropicProvider {
    AnthropicProvider::new(
        server.uri(),
        Zeroizing::new("sk-ant-test".into()),
        "claude-haiku-4-5",
        Duration::from_secs(10),
    )
    .unwrap()
    .with_backoffs(fast_backoffs())
}

#[tokio::test]
async fn anthropic_succeeds_on_first_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SUCCESS_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server);
    let out = p.complete("you are a translator", "Hello, world.").await.unwrap();
    assert_eq!(out, "Hallo, Welt.");
}

#[tokio::test]
async fn anthropic_retries_on_503_then_succeeds_on_third_attempt() {
    let server = MockServer::start().await;

    // First two requests: 503. Third: 200.
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream is sad"))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SUCCESS_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server);
    let out = p.complete("you are a translator", "Hello, world.").await.unwrap();
    assert_eq!(out, "Hallo, Welt.");
}

#[tokio::test]
async fn anthropic_gives_up_after_three_5xx_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(503))
        .expect(3) // exactly 3 attempts
        .mount(&server)
        .await;

    let p = provider(&server);
    let err = p.complete("system", "user").await.unwrap_err();
    match err {
        TranslateError::Provider { status, .. } => assert_eq!(status, 503),
        other => panic!("expected Provider 503, got {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_does_not_retry_on_4xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Invalid API key"))
        .expect(1) // exactly 1 attempt — no retry on 4xx
        .mount(&server)
        .await;

    let p = provider(&server);
    let err = p.complete("system", "user").await.unwrap_err();
    match err {
        TranslateError::Provider { status, .. } => assert_eq!(status, 401),
        other => panic!("expected Provider 401, got {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_returns_rate_limited_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server);
    let err = p.complete("system", "user").await.unwrap_err();
    assert!(matches!(err, TranslateError::RateLimited));
}

use clipt9n::llm::openai::OpenAiCompatibleProvider;

const OPENAI_SUCCESS_BODY: &str = r#"{
    "choices": [{"message": {"role": "assistant", "content": "Hallo, Welt."}}]
}"#;

fn openai_provider(server: &MockServer) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        server.uri(),
        Zeroizing::new("sk-test".into()),
        "gpt-5",
        Duration::from_secs(10),
    )
    .unwrap()
    .with_backoffs(fast_backoffs())
}

#[tokio::test]
async fn openai_succeeds_on_first_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_SUCCESS_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let p = openai_provider(&server);
    let out = p.complete("system", "user").await.unwrap();
    assert_eq!(out, "Hallo, Welt.");
}

#[tokio::test]
async fn openai_retries_on_502_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_SUCCESS_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let p = openai_provider(&server);
    let out = p.complete("system", "user").await.unwrap();
    assert_eq!(out, "Hallo, Welt.");
}
