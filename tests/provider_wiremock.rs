use std::time::Duration;

use clipt9n::error::TranslateError;
use clipt9n::llm::anthropic::AnthropicProvider;
use clipt9n::llm::openai::OpenAiCompatibleProvider;
use clipt9n::llm::LlmProvider;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

#[allow(dead_code)]
const ANTHROPIC_SUCCESS_BODY: &str = r#"{
    "content": [{"type": "text", "text": "Hallo, Welt."}]
}"#;

const OPENAI_SUCCESS_BODY: &str = r#"{
    "choices": [{"message": {"role": "assistant", "content": "Hallo, Welt."}}]
}"#;

fn fast_backoffs() -> Vec<Duration> {
    vec![Duration::from_millis(1), Duration::from_millis(2)]
}

fn anthropic_provider(server: &MockServer) -> AnthropicProvider {
    AnthropicProvider::new(
        server.uri(),
        Zeroizing::new("sk-ant-test".into()),
        "claude-haiku-4-5",
        Duration::from_secs(10),
    )
    .unwrap()
    .with_backoffs(fast_backoffs())
}

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
async fn openai_retries_once_on_429_with_retry_after_zero_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OPENAI_SUCCESS_BODY))
        .expect(1)
        .mount(&server)
        .await;

    let p = openai_provider(&server);
    assert_eq!(p.complete("system", "user").await.unwrap(), "Hallo, Welt.");
}

#[tokio::test]
async fn anthropic_429_without_retry_after_is_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;

    let p = anthropic_provider(&server);
    assert!(matches!(
        p.complete("system", "user").await.unwrap_err(),
        TranslateError::RateLimited
    ));
}

#[tokio::test]
async fn openai_empty_choices_is_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"choices":[]}"#))
        .expect(1)
        .mount(&server)
        .await;

    let p = openai_provider(&server);
    match p.complete("system", "user").await.unwrap_err() {
        TranslateError::Provider { status, message } => {
            assert_eq!(status, 200);
            assert!(message.contains("no choices"), "message: {message}");
        }
        other => panic!("expected Provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_malformed_json_is_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{bad"))
        .expect(1)
        .mount(&server)
        .await;

    let p = openai_provider(&server);
    match p.complete("system", "user").await.unwrap_err() {
        TranslateError::Provider { status, message } => {
            assert_eq!(status, 200);
            assert!(message.contains("parsing response"), "message: {message}");
        }
        other => panic!("expected Provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_no_text_content_is_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"content":[{"type":"tool_use","text":""}]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let p = anthropic_provider(&server);
    match p.complete("system", "user").await.unwrap_err() {
        TranslateError::Provider { status, message } => {
            assert_eq!(status, 200);
            assert!(message.contains("no text content"), "message: {message}");
        }
        other => panic!("expected Provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_403_body_is_preserved() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(header("x-api-key", "sk-ant-test"))
        .respond_with(ResponseTemplate::new(403).set_body_string("permission denied"))
        .expect(1)
        .mount(&server)
        .await;

    let p = anthropic_provider(&server);
    match p.complete("system", "user").await.unwrap_err() {
        TranslateError::Provider { status, message } => {
            assert_eq!(status, 403);
            assert!(message.contains("permission denied"), "message: {message}");
        }
        other => panic!("expected Provider 403, got {other:?}"),
    }
}

#[tokio::test]
async fn openai_gives_up_after_three_5xx_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
        .expect(3)
        .mount(&server)
        .await;

    let p = openai_provider(&server);
    match p.complete("system", "user").await.unwrap_err() {
        TranslateError::Provider { status, message } => {
            assert_eq!(status, 502);
            assert!(message.contains("bad gateway"), "message: {message}");
        }
        other => panic!("expected Provider 502, got {other:?}"),
    }
}
