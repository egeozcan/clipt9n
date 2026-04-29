//! End-to-end CLI smoke test.
//!
//! Spawns `clipt9n` as a subprocess against a wiremock-backed Anthropic
//! endpoint, asserting that the binary exits 0 on a successful translation.
//! This is M1's exit criterion 1 verified in CI.
//!
//! NOTE: this test does NOT exercise the real system clipboard. M1 ships with
//! `arboard`-based clipboard reads/writes which require a desktop session;
//! exercising those is a manual macOS test (documented at the bottom of the M1
//! plan). This integration test exists to verify the wiring above the
//! clipboard layer and the actual binary entry point.
//!
//! To make the binary clipboard-free for this test, we rely on a test-only
//! environment variable `CLIPT9N_TEST_INPUT` that, when set, bypasses arboard
//! and uses the variable's contents as the source text. Result is written to
//! the env var `CLIPT9N_TEST_OUTPUT` (read after subprocess exit via the
//! parent's `--print-result` CLI flag).
//!
//! This test-only path is activated at runtime: when `CLIPT9N_TEST_INPUT` is
//! set, lib.rs skips arboard and uses the env var value instead; if the env var
//! is absent, the production clipboard path runs.

use std::io::Write;

use tempfile::NamedTempFile;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn show_tray_flag_parses() {
    use clap::Parser;
    let cli = clipt9n::Cli::try_parse_from(["clipt9n", "--show-tray"]).unwrap();
    assert!(cli.show_tray, "--show-tray should set the show_tray field");
}

const SUCCESS_BODY: &str = r#"{
    "content": [{"type": "text", "text": "Hallo, Welt."}]
}"#;

#[tokio::test]
async fn cli_translate_succeeds_end_to_end() {
    // 1. Start mock Anthropic server
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SUCCESS_BODY))
        .mount(&server)
        .await;

    // 2. Write a temp config pointing at the mock server
    let mut cfg_file = NamedTempFile::new().unwrap();
    writeln!(
        cfg_file,
        r#"
[provider]
type = "anthropic"
model = "claude-haiku-4-5"
base_url = "{}"
timeout_seconds = 5

[provider.api_key]
source = "env"
env_var = "CLIPT9N_E2E_KEY"
"#,
        server.uri()
    )
    .unwrap();
    cfg_file.flush().unwrap();

    // 3. Run the binary as a subprocess
    let bin = env!("CARGO_BIN_EXE_clipt9n");
    let output = tokio::process::Command::new(bin)
        .arg("--translate-to=de")
        .arg("--config")
        .arg(cfg_file.path())
        .env("CLIPT9N_E2E_KEY", "sk-ant-fake")
        .env("CLIPT9N_TEST_INPUT", "Hello, world.")
        .env("CLIPT9N_TEST_PRINT_RESULT", "1")
        .output()
        .await
        .expect("failed to run clipt9n");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "clipt9n failed: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("Hallo, Welt."),
        "expected translation in stdout, got {stdout:?}"
    );
}

#[tokio::test]
async fn cli_exits_with_error_on_missing_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SUCCESS_BODY))
        .mount(&server)
        .await;

    let mut cfg_file = NamedTempFile::new().unwrap();
    writeln!(
        cfg_file,
        r#"
[provider]
type = "anthropic"
base_url = "{}"

[provider.api_key]
source = "env"
env_var = "CLIPT9N_E2E_MISSING_KEY"
"#,
        server.uri()
    )
    .unwrap();
    cfg_file.flush().unwrap();

    let bin = env!("CARGO_BIN_EXE_clipt9n");
    let output = tokio::process::Command::new(bin)
        .arg("--translate-to=de")
        .arg("--config")
        .arg(cfg_file.path())
        .env_remove("CLIPT9N_E2E_MISSING_KEY")
        .env("CLIPT9N_TEST_INPUT", "Hello.")
        .env("CLIPT9N_TEST_PRINT_RESULT", "1")
        .output()
        .await
        .expect("failed to run clipt9n");

    assert!(
        !output.status.success(),
        "clipt9n should fail without an API key"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("API key not found"),
        "expected MissingApiKey error, got {stderr:?}"
    );
}
