# clipt9n M8 - Tests, Packaging, CI, and Polish - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the v0.1/v1.0 release-readiness milestone by enforcing the platform-abstraction rule in CI, backfilling the remaining spec section 11 automated tests, adding fuzz and latency harnesses, packaging the app for distribution, wiring GitHub Actions release artifacts, executing/documenting the manual smoke matrix, and polishing the small M4/M5/M7 carry-overs.

**Architecture:** M8 runs in five bands. **M8.J first** closes the current cfg exceptions by moving native-modifier and keyfile-permission dispatch into `src/platform/`, then adds a CI lint script. **M8.A/B/C** expands automated coverage in the already-existing units (`templates`, providers, translator post-processing, config, glossary, history crypto/store, secrets) and integration suites (`wiremock`, SQLite, gated keychain). **M8.E/F** adds release-readiness harnesses (`scripts/bench.sh`, `docs/benchmarks/`, `cargo-fuzz` targets) without changing runtime behavior. **M8.G/H** adds packaging metadata/scripts and GitHub Actions build/release workflows. **M8.D/I/K** lands `TESTING.md`, README/LICENSE/CHANGELOG updates, manual smoke execution notes, the setup wizard API-key link, and tray icon size cleanup.

**Tech Stack:** Rust 2021 / eframe 0.31 / egui 0.31 / tokio 1.42 / wiremock 0.6 / rusqlite bundled / keyring 3 / tray-icon 0.22. New release tooling is script/workflow based: `cargo-bundle` is installed in CI rather than added as a crate dependency; `cargo-fuzz` owns its own workspace under `fuzz/`. No new production crate is expected.

> **Branch:** This plan executes on `m8-tests-packaging-ci-polish`, branched from `main` at `e508419` (the M7->M8 handoff commit). Working directory: `/Users/egecan/Code/clipt9n`.

---

## File structure

Expected additions and modifications:

```
.github/workflows/
├── build.yml                         <- MODIFY: lint-platform step, matrix polish, macOS test job
└── release.yml                       <- CREATE: v*.*.* artifact workflow

assets/
├── icon-32.png                       <- CREATE: simple app/tray icon seed for bundling
├── icon-128.png                      <- CREATE
└── icon-256.png                      <- CREATE

docs/
├── benchmarks/
│   └── .gitkeep                      <- CREATE: benchmark output directory
└── superpowers/plans/
    └── 2026-04-29-clipt9n-m8-tests-packaging-ci-polish.md

fuzz/
├── Cargo.toml                        <- CREATE: cargo-fuzz workspace
└── fuzz_targets/
    ├── glossary_parser_fuzz.rs       <- CREATE
    ├── history_decrypt_fuzz.rs       <- CREATE
    └── template_renderer_fuzz.rs     <- CREATE

scripts/
├── bench.sh                          <- CREATE: real-provider latency harness
├── lint-platform-discipline.sh       <- CREATE: no cfg outside platform/
├── package-linux.sh                  <- CREATE: binary + desktop/icon staging
└── package-macos.sh                  <- CREATE: cargo-bundle + LSUIElement + ad-hoc signing

src/
├── config.rs                         <- MODIFY: remove target_os cfg; add validation tests
├── glossary.rs                       <- MODIFY: load_str, entry validation, matching tests
├── history/crypto.rs                 <- MODIFY: remove unix/not(unix) cfg dispatch
├── llm/client.rs                     <- MODIFY: Retry-After retry support
├── llm/templates.rs                  <- MODIFY: branch validation + from_sources helper
├── platform/mod.rs                   <- MODIFY: native cmd modifier + keyfile permission dispatch
├── tray.rs                           <- MODIFY: scale/remove icon size parameter
└── ui/setup.rs                       <- MODIFY: provider key URL outcome

tests/
├── keychain_integration.rs           <- CREATE: gated OS-keychain smoke
└── provider_wiremock.rs              <- CREATE or split from retry_policy.rs for expanded provider cases

TESTING.md                           <- CREATE: manual matrix and execution log
LICENSE                              <- CREATE: MIT license text
CHANGELOG.md                         <- CREATE: M1..v0.1 history
Cargo.toml                           <- MODIFY: bundle metadata, fuzz cfg feature if needed
README.md                            <- MODIFY: release-install docs and troubleshooting
```

---

## Cross-cutting decisions

- **M8.J is first.** The design doc says the platform abstraction lint has no exceptions. Current `main` still has `#[cfg(target_os = "macos")]` in `src/config.rs::Modifier::resolve_native` and `#[cfg(unix)]` / `#[cfg(not(unix))]` in `src/history/crypto.rs`. Those are fixed before any new cross-platform work lands.
- **No live tray assumption in CI.** Unit and kittest suites may run on macOS CI, but `TrayHandle::build_with_panic_isolation` remains a manual smoke target. NSStatusItem creation is not a stable headless CI contract.
- **Provider Retry-After semantics.** Existing 5xx behavior stays: two retries with `[1s, 2s]` in production and injected fast backoffs in tests. For `429` with a parseable `Retry-After`, retry once after that delay and then surface `RateLimited` if it still fails. `429` without the header remains immediate `RateLimited`.
- **Manual smoke is in scope.** M5/M6/M7 matrices move into `TESTING.md`; automated work can land before all manual rows are checked, but M8 cannot be called release-ready until the macOS Day-1 matrix and VoiceOver pass are filled in.
- **Benchmark uses the real provider.** `scripts/bench.sh` requires a real config/key and records network/hardware conditions. It writes Markdown to `docs/benchmarks/<date>.md`. Do not fake the latency target with wiremock.
- **Fuzz targets do not log secrets or clipboard text.** Inputs are arbitrary bytes. Success means no panic/OOM; `history_decrypt_fuzz` treats `Ok` on tampered arbitrary ciphertext as a failure unless the bytes happen to be a valid encrypted blob generated by the target itself.
- **Setup wizard provider link uses `Platform::open_path` only for files today.** For URLs, add `Platform::open_url(&self, url: &str)` or use egui `ViewportCommand::OpenUrl`. Prefer `ViewportCommand::OpenUrl` for fewer shell branches.
- **Notarization is out of scope.** Ad-hoc signing is in scope; README documents Gatekeeper "Open Anyway" for personal distribution and marks Apple notarization as a wider-distribution follow-up.

---

## Pre-flight: Verify baseline

**Files:** none.

- [ ] **Step 0.1: Verify branch and dirty tree**

Run:

```bash
git branch --show-current
git status --short
```

Expected: branch `m8-tests-packaging-ci-polish`. Existing untracked `.idea/` may remain; do not add it.

- [ ] **Step 0.2: Verify M7 baseline**

Run:

```bash
cargo test --all-features
cargo clippy --all-features --all-targets -- -D warnings
cargo fmt --check
```

Expected: all pass. If baseline tests fail before any M8 code edits, stop and record the failing command/output in this plan's execution notes.

- [ ] **Step 0.3: Verify current cfg leaks are exactly the known ones**

Run:

```bash
grep -rn '#\[cfg(target_os' src/
grep -rn '#\[cfg(unix' src/
grep -rn '#\[cfg(not(unix' src/
```

Expected: target-os matches only `src/config.rs` and `src/platform/*`; unix/not(unix) matches only `src/history/crypto.rs` and `src/platform/*`.

---

## Task 1: M8.J platform-discipline cleanup and lint script

**Files:**
- Modify: `src/config.rs`
- Modify: `src/history/crypto.rs`
- Modify: `src/platform/mod.rs`
- Create: `scripts/lint-platform-discipline.sh`
- Modify: `.github/workflows/build.yml`

**Why:** This is the safety rail for all later M8 packaging/CI work. The script must fail on any future platform branch outside `src/platform/`.

- [ ] **Step 1.1: Move `cmd` native-modifier mapping into platform**

Add this helper to `src/platform/mod.rs`:

```rust
/// Resolve logical "cmd" to the OS-native hotkey modifier.
pub fn cmd_modifier() -> crate::config::NativeModifier {
    cmd_modifier_impl()
}

#[cfg(target_os = "macos")]
fn cmd_modifier_impl() -> crate::config::NativeModifier {
    crate::config::NativeModifier::Meta
}

#[cfg(not(target_os = "macos"))]
fn cmd_modifier_impl() -> crate::config::NativeModifier {
    crate::config::NativeModifier::Ctrl
}
```

Then replace the `Self::Cmd` arm in `src/config.rs::Modifier::resolve_native` with:

```rust
Self::Cmd => crate::platform::cmd_modifier(),
```

Run:

```bash
cargo test --lib config::tests::resolve_modifier_returns_native_for_cmd
```

Expected: pass on the current macOS dev box.

- [ ] **Step 1.2: Move keyfile-permission cfg dispatch into platform**

Replace the cfg-gated functions in `src/history/crypto.rs` with one platform call:

```rust
fn set_keyfile_permissions(path: &Path) -> Result<(), TranslateError> {
    crate::platform::set_owner_only_permissions(path)
        .map_err(|e| TranslateError::History(format!("chmod 0o600 on {}: {e}", path.display())))
}
```

In `src/platform/mod.rs`, make `set_owner_only_permissions` available on all targets:

```rust
#[cfg(unix)]
pub(crate) fn set_owner_only_permissions(path: &std::path::Path) -> Result<(), std::io::Error> {
    unix::set_owner_only_permissions(path)
}

#[cfg(not(unix))]
pub(crate) fn set_owner_only_permissions(_path: &std::path::Path) -> Result<(), std::io::Error> {
    Ok(())
}
```

Run:

```bash
cargo test --lib history::crypto::tests::keyfile_creates_with_owner_only_perms_on_first_open
```

Expected: pass.

- [ ] **Step 1.3: Create the lint script**

Create `scripts/lint-platform-discipline.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

fail=0

check_pattern() {
  local pattern="$1"
  local label="$2"
  local output

  if output=$(grep -rn "$pattern" src/ | grep -v '^src/platform/'); then
    printf 'Platform discipline violation: %s outside src/platform/\n' "$label" >&2
    printf '%s\n' "$output" >&2
    fail=1
  fi
}

check_pattern '#\[cfg(target_os' '#[cfg(target_os = ...)]'
check_pattern '#\[cfg(unix' '#[cfg(unix)]'
check_pattern '#\[cfg(not(unix' '#[cfg(not(unix))]'

exit "$fail"
```

Run:

```bash
chmod +x scripts/lint-platform-discipline.sh
scripts/lint-platform-discipline.sh
```

Expected: no output and exit 0.

- [ ] **Step 1.4: Wire the script into CI**

In `.github/workflows/build.yml`, add this step after checkout/toolchain in `fmt-and-clippy`:

```yaml
      - name: platform discipline lint
        run: scripts/lint-platform-discipline.sh
```

Run:

```bash
cargo fmt --check
cargo clippy --all-features --all-targets -- -D warnings
```

Expected: both clean.

- [ ] **Step 1.5: Commit**

```bash
git add src/config.rs src/history/crypto.rs src/platform/mod.rs scripts/lint-platform-discipline.sh .github/workflows/build.yml
git commit -m "chore(M8): enforce platform cfg discipline in CI"
```

---

## Task 2: M8.A template and post-processing test backfill

**Files:**
- Modify: `src/llm/templates.rs`
- Modify: `src/translator.rs`

**Why:** Covers the deferred M5/M7 template conditional-branch validation and rounds out spec section 5.3/5.6 behavior.

- [ ] **Step 2.1: Add failing template branch tests**

In `src/llm/templates.rs`, add tests:

```rust
#[test]
fn override_validation_checks_truthy_and_falsey_conditional_branches() {
    let dir = tempdir().unwrap();
    let templates_dir = dir.path().join("templates");
    std::fs::create_dir(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("translate.j2"),
        "{% if glossary_block %}{{ glossary_block }} {{ missing_in_truthy }}{% else %}Translate to {{ target_language }}{% endif %}",
    )
    .unwrap();

    let err = Templates::load(dir.path(), &TemplatesConfig::default()).unwrap_err();
    match err {
        TranslateError::Template(msg) => assert!(msg.contains("missing_in_truthy"), "msg: {msg}"),
        other => panic!("expected Template error, got {other:?}"),
    }
}

#[test]
fn override_conditional_template_renders_both_paths_when_valid() {
    let dir = tempdir().unwrap();
    let templates_dir = dir.path().join("templates");
    std::fs::create_dir(&templates_dir).unwrap();
    std::fs::write(
        templates_dir.join("translate.j2"),
        "{% if glossary_block %}WITH {{ glossary_block }}{% else %}WITHOUT {{ target_language }}{% endif %}",
    )
    .unwrap();

    let t = Templates::load(dir.path(), &TemplatesConfig::default()).unwrap();
    let without = render(&t, TemplateKind::Translate, &TemplateContext::for_translate("German", "")).unwrap();
    let with = render(&t, TemplateKind::Translate, &TemplateContext::for_translate("German", "GLOSSARY")).unwrap();
    assert_eq!(without, "WITHOUT German");
    assert_eq!(with, "WITH GLOSSARY");
}
```

Run:

```bash
cargo test --lib llm::templates::tests::override_validation_checks_truthy_and_falsey_conditional_branches
```

Expected: first test fails before implementation because current validation renders only one stub context.

- [ ] **Step 2.2: Validate with multiple stub contexts**

Change `validate_template_source` so it renders at least these two contexts:

```rust
let contexts = [
    context! {
        source_language => "stub",
        target_language => "Stub",
        user_instruction => "stub",
        glossary_block => "",
    },
    context! {
        source_language => "stub",
        target_language => "Stub",
        user_instruction => "stub",
        glossary_block => "GLOSSARY",
    },
];

for ctx in contexts {
    if let Err(e) = tmpl.render(ctx) {
        return Err(TranslateError::Template(format!(
            "{} line {}: undefined variable or render error: {e}",
            path.display(),
            err_line(&e),
        )));
    }
}
```

Run:

```bash
cargo test --lib llm::templates::tests::override_
```

Expected: new override tests pass.

- [ ] **Step 2.3: Add post-processing edge tests**

In `src/translator.rs`, add tests:

```rust
#[test]
fn strips_translation_preamble_after_outer_quotes_are_removed() {
    assert_eq!(
        post_process("\"Translation: Hallo\"", "Hello"),
        "Hallo"
    );
}

#[test]
fn preserves_wrapping_quotes_when_source_started_with_curly_quote() {
    assert_eq!(
        post_process("\u{201C}Hallo\u{201D}", "\u{201C}Hello\u{201D}"),
        "\u{201C}Hallo\u{201D}"
    );
}

#[test]
fn strips_preamble_with_leading_newline_after_trim() {
    assert_eq!(post_process("\n\nHere is the translation: Hallo", "Hello"), "Hallo");
}
```

Run:

```bash
cargo test --lib translator::tests::strips_translation_preamble_after_outer_quotes_are_removed translator::tests::preserves_wrapping_quotes_when_source_started_with_curly_quote translator::tests::strips_preamble_with_leading_newline_after_trim
```

Expected: all pass, or implement the smallest post-processing adjustment needed if the first test reveals ordering drift.

- [ ] **Step 2.4: Commit**

```bash
git add src/llm/templates.rs src/translator.rs
git commit -m "test(M8): expand template and post-processing coverage"
```

---

## Task 3: M8.B provider integration tests and Retry-After support

**Files:**
- Modify: `src/llm/client.rs`
- Modify: `src/llm/openai.rs`
- Modify: `src/llm/anthropic.rs`
- Modify or create: `tests/provider_wiremock.rs`

**Why:** Existing `tests/retry_policy.rs` covers basic success/retry. M8 adds provider-specific parse failures, 4xx/5xx mapping, no-content/no-choice cases, timeout, and `Retry-After`.

- [ ] **Step 3.1: Add Retry-After tests first**

Create `tests/provider_wiremock.rs` with these imports, constants, and helpers:

```rust
use std::time::Duration;

use clipt9n::error::TranslateError;
use clipt9n::llm::anthropic::AnthropicProvider;
use clipt9n::llm::openai::OpenAiCompatibleProvider;
use clipt9n::llm::LlmProvider;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

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
```

Then add:

```rust
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
    assert!(matches!(p.complete("system", "user").await.unwrap_err(), TranslateError::RateLimited));
}
```

Run:

```bash
cargo test --test provider_wiremock openai_retries_once_on_429_with_retry_after_zero_then_succeeds
```

Expected: fails before implementation; current providers return `RateLimited` immediately.

- [ ] **Step 3.2: Extend retry helper**

In `src/llm/client.rs`, add:

```rust
pub enum AttemptOutcome<T, E> {
    Done(T),
    Retry(E),
    RetryAfter(Duration, E),
    Fatal(E),
}

pub fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let value = value?.to_str().ok()?.trim();
    let seconds: u64 = value.parse().ok()?;
    Some(Duration::from_secs(seconds.min(30)))
}
```

Update `with_retry` so `RetryAfter(delay, e)` sleeps `delay`, retries if there is retry budget left, and returns `e` when budget is exhausted.

Provider 429 branch shape:

```rust
} else if status == StatusCode::TOO_MANY_REQUESTS {
    if let Some(delay) = super::client::parse_retry_after(resp.headers().get("Retry-After")) {
        AttemptOutcome::RetryAfter(delay, TranslateError::RateLimited)
    } else {
        AttemptOutcome::Fatal(TranslateError::RateLimited)
    }
}
```

Run:

```bash
cargo test --lib llm::client::tests
cargo test --test provider_wiremock
```

Expected: pass.

- [ ] **Step 3.3: Add provider parse/error cases**

Add these tests in `tests/provider_wiremock.rs`:

```rust
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
```

Run:

```bash
cargo test --test provider_wiremock
cargo test --test retry_policy
```

Expected: pass.

- [ ] **Step 3.4: Commit**

```bash
git add src/llm/client.rs src/llm/openai.rs src/llm/anthropic.rs tests/provider_wiremock.rs tests/retry_policy.rs
git commit -m "test(M8): expand provider wiremock coverage and Retry-After handling"
```

---

## Task 4: M8.A config validation and round-trip coverage

**Files:**
- Modify: `src/config.rs`

**Why:** Spec section 6 validation is still permissive in places. M8 closes `[provider.api_key].source`, `[hotkey.history]`, and `[glossary].matching` coverage.

- [ ] **Step 4.1: Add failing validation tests**

Add tests to `src/config.rs`:

```rust
#[test]
fn invalid_glossary_matching_is_config_error() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "[glossary]\nmatching = \"regex\"\n").unwrap();
    let err = Config::load(f.path()).unwrap_err();
    match err {
        TranslateError::Config(msg) => assert!(msg.contains("glossary.matching"), "msg: {msg}"),
        other => panic!("expected Config error, got {other:?}"),
    }
}

#[test]
fn invalid_api_key_source_is_config_error() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "[provider.api_key]\nsource = \"plaintext\"\n").unwrap();
    let err = Config::load(f.path()).unwrap_err();
    match err {
        TranslateError::Config(msg) => assert!(msg.contains("provider.api_key.source"), "msg: {msg}"),
        other => panic!("expected Config error, got {other:?}"),
    }
}

#[test]
fn full_config_round_trips_every_section() {
    let mut cfg = Config::default();
    cfg.provider.kind = "gemini".into();
    cfg.provider.api_key.source = "keychain".into();
    cfg.hotkey.history.enabled = false;
    cfg.glossary.matching = "substring".into();
    cfg.history.store_text = false;
    let f = NamedTempFile::new().unwrap();
    cfg.persist(f.path()).unwrap();
    let loaded = Config::load(f.path()).unwrap();
    assert_eq!(loaded.provider.kind, "gemini");
    assert_eq!(loaded.provider.api_key.source, "keychain");
    assert!(!loaded.hotkey.history.enabled);
    assert_eq!(loaded.glossary.matching, "substring");
    assert!(!loaded.history.store_text);
}
```

Run:

```bash
cargo test --lib config::tests::invalid_
```

Expected: validation tests fail before implementation.

- [ ] **Step 4.2: Implement `Config::validate`**

Add:

```rust
impl Config {
    fn validate(&self) -> Result<(), TranslateError> {
        match self.provider.api_key.source.as_str() {
            "keychain" | "env" | "prompt" => {}
            other => {
                return Err(TranslateError::Config(format!(
                    "provider.api_key.source must be keychain, env, or prompt; got {other}"
                )));
            }
        }
        match self.glossary.matching.as_str() {
            "auto" | "word_boundary" | "substring" => {}
            other => {
                return Err(TranslateError::Config(format!(
                    "glossary.matching must be auto, word_boundary, or substring; got {other}"
                )));
            }
        }
        if self.history.max_entries == 0 {
            return Err(TranslateError::Config(
                "history.max_entries must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}
```

Change `Config::load` to parse into `cfg`, call `cfg.validate()?`, then return `Ok(cfg)`.

Run:

```bash
cargo test --lib config::tests
```

Expected: pass.

- [ ] **Step 4.3: Commit**

```bash
git add src/config.rs
git commit -m "test(M8): validate config edge cases and full round trip"
```

---

## Task 5: M8.A glossary validation and matching coverage

**Files:**
- Modify: `src/glossary.rs`

**Why:** Closes deferred glossary entry validation, pair scoping, case sensitivity, and source-text lowercasing coverage.

- [ ] **Step 5.1: Extract `Glossary::load_str`**

Refactor `Glossary::load` to call:

```rust
pub fn load_str(contents: &str) -> Result<Self, TranslateError> {
    let mut g: Self = toml::from_str(contents)
        .map_err(|e| TranslateError::Glossary(format!("parsing glossary: {e}")))?;
    for entry in g.entries.iter_mut() {
        if entry.languages.is_empty() {
            entry.languages.push("*".into());
        }
        validate_entry(entry)?;
    }
    Ok(g)
}
```

Keep `load(path)` responsible only for missing-file handling, read errors, and adding the path to error strings.

- [ ] **Step 5.2: Add entry-validation tests**

Add tests:

```rust
#[test]
fn empty_source_is_glossary_error() {
    let err = Glossary::load_str("[[entry]]\nsource = \"\"\ntarget = \"Fall\"\n").unwrap_err();
    assert!(matches!(err, TranslateError::Glossary(msg) if msg.contains("source")));
}

#[test]
fn invalid_language_pair_is_glossary_error() {
    let err = Glossary::load_str("[[entry]]\nsource = \"case\"\ntarget = \"Fall\"\nlanguages = [\"english-to-german\"]\n").unwrap_err();
    assert!(matches!(err, TranslateError::Glossary(msg) if msg.contains("languages")));
}

#[test]
fn load_str_normalizes_empty_languages_to_wildcard() {
    let g = Glossary::load_str("[[entry]]\nsource = \"GIP\"\ntarget = \"GIP\"\n").unwrap();
    assert_eq!(g.entries()[0].languages, vec!["*"]);
}
```

Implement:

```rust
fn validate_entry(entry: &GlossaryEntry) -> Result<(), TranslateError> {
    if entry.source.trim().is_empty() {
        return Err(TranslateError::Glossary("entry.source must not be empty".into()));
    }
    if entry.target.trim().is_empty() {
        return Err(TranslateError::Glossary("entry.target must not be empty".into()));
    }
    for pair in &entry.languages {
        if pair == "*" {
            continue;
        }
        let Some((src, dst)) = pair.split_once("->") else {
            return Err(TranslateError::Glossary(format!("entry.languages value '{pair}' must be '*' or '<src>-><target>'")));
        };
        if src.len() != 2 || dst.len() != 2 {
            return Err(TranslateError::Glossary(format!("entry.languages value '{pair}' must use 2-letter ISO codes")));
        }
    }
    Ok(())
}
```

Run:

```bash
cargo test --lib glossary::tests
```

Expected: pass.

- [ ] **Step 5.3: Add pair/case/auto strategy regression tests**

Add:

```rust
#[test]
fn matching_entries_respects_case_sensitive_config() {
    let g = Glossary::load_str("[[entry]]\nsource = \"Smart Table\"\ntarget = \"Smart Table\"\nlanguages = [\"*\"]\n").unwrap();
    let mut cfg = crate::config::GlossaryConfig::default();
    cfg.case_sensitive = true;
    assert!(g.matching_entries("SMART TABLE", Some("en"), Some("de"), &cfg).is_empty());
    assert_eq!(g.matching_entries("Smart Table", Some("en"), Some("de"), &cfg).len(), 1);
}

#[test]
fn auto_uses_substring_for_japanese_iso2_without_redetecting_text() {
    let g = Glossary::load_str("[[entry]]\nsource = \"東京\"\ntarget = \"Tokyo\"\nlanguages = [\"ja->en\"]\n").unwrap();
    let cfg = crate::config::GlossaryConfig::default();
    assert_eq!(g.matching_entries("東京都", Some("ja"), Some("en"), &cfg).len(), 1);
}
```

If this reveals repeated lowercasing cost, introduce a private `NormalizedSource` helper:

```rust
struct NormalizedSource<'a> {
    original: &'a str,
    lower: Option<String>,
}
```

Use it inside `matching_entries` / `preview_entries` so case-insensitive matching computes the lowercased source once per query.

- [ ] **Step 5.4: Commit**

```bash
git add src/glossary.rs
git commit -m "test(M8): validate glossary entries and matching edges"
```

---

## Task 6: M8.C history SQLite and crypto edge coverage

**Files:**
- Modify: `src/history/crypto.rs`
- Modify: `src/history/store.rs`

**Why:** Existing M5 tests are strong; M8 adds the corruption and invariant cases called out in spec sections 8, 9, and 11.

- [ ] **Step 6.1: Add tampered nonce and half-null row tests**

In `src/history/crypto.rs`, add:

```rust
#[test]
fn decrypt_rejects_tampered_nonce() {
    let secret = Zeroizing::new([9u8; 32]);
    let key = derive_key(&secret).unwrap();
    let (ct, mut nonce) = encrypt(&key, b"nonce protected").unwrap();
    nonce[0] ^= 0x01;
    assert!(matches!(decrypt(&key, &ct, &nonce), Err(TranslateError::History(_))));
}
```

In `src/history/store.rs`, add a test that inserts a raw half-null row and verifies `query` skips it or returns an error only for that row:

```rust
#[test]
fn query_skips_half_null_ciphertext_rows() {
    let h = History::in_memory(test_key()).unwrap();
    {
        let conn = h.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO entries (created_at, action, char_count, source_ciphertext)
             VALUES (1, 'translate', 4, X'01020304')",
            [],
        )
        .unwrap();
    }
    let rows = h.query(&QueryFilter::default(), 10).unwrap();
    assert!(rows.is_empty());
}
```

If the private `conn` field blocks the test, add a `#[cfg(test)] fn raw_conn_for_test(&self) -> MutexGuard<'_, Connection>` helper inside the module.

- [ ] **Step 6.2: Add corrupt DB open integration note**

Keep the current `open_with_corrupt_file_returns_history_error` unit test, and add a doc note in Task 12's `TESTING.md` history section that `main.rs` maps this open error to `history_disabled_initial = true`. Do not over-extract main startup just to test a boolean.

Run:

```bash
cargo test --lib history::crypto::tests history::store::tests
```

Expected: pass.

- [ ] **Step 6.3: Commit**

```bash
git add src/history/crypto.rs src/history/store.rs
git commit -m "test(M8): strengthen history corruption and crypto invariants"
```

---

## Task 7: M8.A gated keychain integration smoke

**Files:**
- Create: `tests/keychain_integration.rs`
- Modify: `.github/workflows/build.yml`

**Why:** Spec section 11 calls for OS keychain integration tests. They must be opt-in locally and non-destructive in CI.

- [ ] **Step 7.1: Add gated test**

Create `tests/keychain_integration.rs`:

```rust
use clipt9n::secrets::{KeychainSecrets, Secrets};
use zeroize::Zeroizing;

fn enabled() -> bool {
    std::env::var("CLIPT9N_KEYCHAIN_INTEGRATION").as_deref() == Ok("1")
}

#[test]
fn keychain_round_trip_when_enabled() {
    if !enabled() {
        eprintln!("skipping: set CLIPT9N_KEYCHAIN_INTEGRATION=1 to run");
        return;
    }

    let account = format!("integration-{}", std::process::id());
    let secrets = KeychainSecrets::new("clipt9n-test", &account);
    assert!(secrets.keychain_available(), "OS keychain must be available");

    secrets
        .set_api_key(Zeroizing::new("sk-test-keychain-roundtrip".to_string()))
        .unwrap();
    let key = secrets.get_api_key().unwrap();
    assert_eq!(&*key, "sk-test-keychain-roundtrip");
}
```

Run:

```bash
cargo test --test keychain_integration
```

Expected: skipped-by-return and pass.

- [ ] **Step 7.2: Add optional CI job**

Add a macOS-only CI job with `CLIPT9N_KEYCHAIN_INTEGRATION=1`, allowed to run only on `workflow_dispatch` or tag builds if the runner keychain proves stable. If GitHub's hosted keychain prompts or denies access, keep the test gated and document manual execution in `TESTING.md` instead of making PR CI flaky.

Run:

```bash
cargo test --test keychain_integration
```

Expected locally without env: pass.

- [ ] **Step 7.3: Commit**

```bash
git add tests/keychain_integration.rs .github/workflows/build.yml
git commit -m "test(M8): add gated keychain integration smoke"
```

---

## Task 8: M8.F cargo-fuzz targets

**Files:**
- Modify: `src/glossary.rs`
- Modify: `src/llm/templates.rs`
- Create: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/glossary_parser_fuzz.rs`
- Create: `fuzz/fuzz_targets/template_renderer_fuzz.rs`
- Create: `fuzz/fuzz_targets/history_decrypt_fuzz.rs`

**Why:** Spec section 11 requires fuzzing glossary parsing, template rendering, and history decryption.

- [ ] **Step 8.1: Add fuzz workspace**

Create `fuzz/Cargo.toml`:

```toml
[package]
name = "clipt9n-fuzz"
version = "0.0.0"
publish = false
edition = "2021"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
clipt9n = { path = "..", features = ["internal-test-helpers"] }
zeroize = "1.8"

[[bin]]
name = "glossary_parser_fuzz"
path = "fuzz_targets/glossary_parser_fuzz.rs"
test = false
doc = false

[[bin]]
name = "template_renderer_fuzz"
path = "fuzz_targets/template_renderer_fuzz.rs"
test = false
doc = false

[[bin]]
name = "history_decrypt_fuzz"
path = "fuzz_targets/history_decrypt_fuzz.rs"
test = false
doc = false
```

- [ ] **Step 8.2: Add targets**

`glossary_parser_fuzz.rs`:

```rust
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = clipt9n::glossary::Glossary::load_str(s);
    }
});
```

`template_renderer_fuzz.rs`:

```rust
#![no_main]

use clipt9n::llm::templates::{render, TemplateContext, TemplateKind, Templates};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    if let Ok(source) = std::str::from_utf8(data) {
        if let Ok(t) = Templates::from_sources_for_test(source, "", "", "") {
            let ctx = TemplateContext::for_translate("German", "GLOSSARY");
            let _ = render(&t, TemplateKind::Translate, &ctx);
        }
    }
});
```

`history_decrypt_fuzz.rs`:

```rust
#![no_main]

use clipt9n::history::crypto::{decrypt, derive_key};
use libfuzzer_sys::fuzz_target;
use zeroize::Zeroizing;

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let key = derive_key(&Zeroizing::new([7u8; 32])).unwrap();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&data[..12]);
    let ciphertext = &data[12..];
    let _ = decrypt(&key, ciphertext, &nonce).err();
});
```

Add `Templates::from_sources_for_test` behind `#[cfg(any(test, feature = "internal-test-helpers"))]`.

- [ ] **Step 8.3: Smoke-build fuzz targets**

Run:

```bash
cargo fuzz build glossary_parser_fuzz
cargo fuzz build template_renderer_fuzz
cargo fuzz build history_decrypt_fuzz
```

If `cargo fuzz` is not installed, run:

```bash
cargo install cargo-fuzz
```

Then repeat the builds. Expected: all fuzz binaries build.

- [ ] **Step 8.4: Commit**

```bash
git add src/glossary.rs src/llm/templates.rs fuzz
git commit -m "test(M8): add fuzz targets for glossary templates and history decrypt"
```

---

## Task 9: M8.E latency benchmark harness

**Files:**
- Create: `scripts/bench.sh`
- Create: `docs/benchmarks/.gitkeep`
- Modify: `README.md`

**Why:** Spec section 10 calls for p50 < 800ms and p95 < 2000ms across 20 real snippets with Haiku 4.5.

- [ ] **Step 9.1: Create benchmark script**

Create `scripts/bench.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${ANTHROPIC_API_KEY:-}" && -z "${CLIPT9N_BENCH_CONFIG:-}" ]]; then
  echo "Set ANTHROPIC_API_KEY or CLIPT9N_BENCH_CONFIG before running benchmarks." >&2
  exit 2
fi

bin="${CLIPT9N_BIN:-target/release/clipt9n}"
out_dir="docs/benchmarks"
mkdir -p "$out_dir"
date_slug="$(date +%Y-%m-%d)"
out="$out_dir/${date_slug}.md"

snippets=(
  "Hello, world."
  "Please review the attached invoice and send feedback by Friday."
  "Guten Tag, ich moechte den Termin auf morgen verschieben."
  "Bu metni daha resmi ve kisa hale getirir misin?"
  "Here is a markdown list:\n- first item\n- second item"
  "The API request failed because the token expired after deployment."
  "Ich brauche eine kurze Zusammenfassung fuer das Standup."
  "Lutfen bu cumledeki yazim hatalarini duzelt."
  "Translate this code comment without touching `snake_case` identifiers."
  "A longer paragraph about product onboarding, error recovery, and user trust that should still feel quick."
  "The customer said the onboarding screen felt confusing, but the core workflow was useful."
  "Bitte formuliere diese Nachricht diplomatischer, ohne die Aussage zu veraendern."
  "Toplanti notlarini tek cumlelik bir ozet haline getir."
  "Preserve URLs like https://example.com and email names@example.com."
  "A sentence with smart quotes: \u201cHello\u201d and guillemets: \u00abbonjour\u00bb."
  "Short"
  "This is already English."
  "Das ist bereits Deutsch."
  "Bu zaten Turkce."
  "Final representative snippet for p95 measurement across normal text."
)

durations=()
cargo build --release >/dev/null

for i in "${!snippets[@]}"; do
  start_ns="$(date +%s%N)"
  CLIPT9N_TEST_INPUT="${snippets[$i]}" CLIPT9N_TEST_PRINT_RESULT=1 "$bin" --translate-to=de ${CLIPT9N_BENCH_CONFIG:+--config "$CLIPT9N_BENCH_CONFIG"} >/dev/null
  end_ns="$(date +%s%N)"
  ms=$(((end_ns - start_ns) / 1000000))
  durations+=("$ms")
  printf 'snippet %02d: %sms\n' "$((i + 1))" "$ms"
done

sorted="$(printf '%s\n' "${durations[@]}" | sort -n)"
p50="$(printf '%s\n' "$sorted" | awk 'NR==10 {print}')"
p95="$(printf '%s\n' "$sorted" | awk 'NR==19 {print}')"

{
  echo "# clipt9n latency benchmark - ${date_slug}"
  echo
  echo "- Binary: \`$bin\`"
  echo "- Provider: Anthropic Haiku 4.5 or configured equivalent"
  echo "- Network: record Wi-Fi/Ethernet/VPN status here before committing"
  echo
  echo "| Metric | Target | Actual |"
  echo "|---|---:|---:|"
  echo "| p50 | <800 ms | ${p50} ms |"
  echo "| p95 | <2000 ms | ${p95} ms |"
  echo
  echo "| Sample | Duration |"
  echo "|---:|---:|"
  for i in "${!durations[@]}"; do
    echo "| $((i + 1)) | ${durations[$i]} ms |"
  done
} > "$out"

echo "wrote $out"
```

Run:

```bash
chmod +x scripts/bench.sh
shellcheck scripts/bench.sh
```

If `shellcheck` is unavailable, run `bash -n scripts/bench.sh`. Expected: no syntax errors.

- [ ] **Step 9.2: Document benchmark execution**

Add a README section explaining:

```markdown
### Latency benchmark

Run `scripts/bench.sh` with a real provider key. The script writes a Markdown report to `docs/benchmarks/<date>.md`. M8's release target is p50 < 800 ms and p95 < 2000 ms with Anthropic Haiku 4.5 on the maintainer's macOS dev hardware.
```

- [ ] **Step 9.3: Commit**

```bash
git add scripts/bench.sh docs/benchmarks/.gitkeep README.md
git commit -m "chore(M8): add real-provider latency benchmark harness"
```

---

## Task 10: M8.G packaging scripts and bundle metadata

**Files:**
- Modify: `Cargo.toml`
- Create: `assets/icon-32.png`
- Create: `assets/icon-128.png`
- Create: `assets/icon-256.png`
- Create: `scripts/package-macos.sh`
- Create: `scripts/package-linux.sh`
- Modify: `README.md`

**Why:** Produces the `.app`, Linux binary/desktop/icon bundle, and documents Windows `.exe` distribution.

- [ ] **Step 10.1: Add cargo-bundle metadata**

In `Cargo.toml`, add:

```toml
[package.metadata.bundle]
name = "clipt9n"
identifier = "dev.egecan.clipt9n"
version = "0.1.0"
category = "public.app-category.utilities"
short_description = "Keyboard-driven clipboard translator"
long_description = "clipt9n is a menu-bar clipboard translator with hotkeys, glossary support, encrypted history, and setup wizard."
icon = ["assets/icon-32.png", "assets/icon-128.png", "assets/icon-256.png"]
osx_minimum_system_version = "13.0"
linux_use_terminal = false
```

The `identifier` becomes macOS `CFBundleIdentifier`. `LSUIElement` is patched by `scripts/package-macos.sh` after cargo-bundle generates `Info.plist`.

- [ ] **Step 10.2: Add macOS packaging script**

Create `scripts/package-macos.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cargo bundle --release --format osx

app_path="$(find target/release/bundle/osx -maxdepth 1 -name 'clipt9n.app' -type d | head -n 1)"
if [[ -z "$app_path" ]]; then
  echo "clipt9n.app not found under target/release/bundle/osx" >&2
  exit 1
fi

plist="$app_path/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Delete :LSUIElement' "$plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c 'Add :LSUIElement bool true' "$plist"
/usr/libexec/PlistBuddy -c 'Delete :LSBackgroundOnly' "$plist" >/dev/null 2>&1 || true
/usr/libexec/PlistBuddy -c 'Add :LSBackgroundOnly bool false' "$plist"

codesign --force --deep --sign - "$app_path"
echo "$app_path"
```

Run:

```bash
chmod +x scripts/package-macos.sh
bash -n scripts/package-macos.sh
```

Expected: syntax clean. If `cargo bundle` is not installed, install with `cargo install cargo-bundle` before executing the script in the packaging verification step.

- [ ] **Step 10.3: Add Linux staging script**

Create `scripts/package-linux.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

cargo build --release
stage="target/release/package-linux/clipt9n"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/icons/hicolor/256x256/apps"
cp target/release/clipt9n "$stage/bin/clipt9n"
cp assets/icon-256.png "$stage/share/icons/hicolor/256x256/apps/clipt9n.png"
cat > "$stage/share/applications/dev.egecan.clipt9n.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=clipt9n
Comment=Keyboard-driven clipboard translator
Exec=clipt9n
Icon=clipt9n
Categories=Utility;
Terminal=false
DESKTOP
echo "$stage"
```

Run:

```bash
chmod +x scripts/package-linux.sh
bash -n scripts/package-linux.sh
```

Expected: syntax clean.

- [ ] **Step 10.4: Generate simple PNG assets**

Use the existing tray stencil colors or a tiny script/tool to create three PNGs. Verify:

```bash
file assets/icon-32.png assets/icon-128.png assets/icon-256.png
```

Expected: PNG images at the named sizes.

- [ ] **Step 10.5: Commit**

```bash
git add Cargo.toml assets scripts/package-macos.sh scripts/package-linux.sh README.md
git commit -m "chore(M8): add packaging metadata and bundle scripts"
```

---

## Task 11: M8.H GitHub Actions build and release workflows

**Files:**
- Modify: `.github/workflows/build.yml`
- Create: `.github/workflows/release.yml`

**Why:** Spec section 10 requires five-target builds and tag-triggered release artifacts.

- [ ] **Step 11.1: Tighten build workflow**

Keep the existing five-target matrix. Ensure these jobs exist:

- `fmt-and-clippy`: checkout, stable toolchain with rustfmt/clippy, platform lint, `cargo fmt`, `cargo clippy`.
- `build`: five target compile matrix.
- `test-macos`: `cargo test --all-features` on macOS.

Run locally:

```bash
cargo fmt --check
scripts/lint-platform-discipline.sh
```

Expected: pass.

- [ ] **Step 11.2: Add release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: release

on:
  push:
    tags:
      - "v*.*.*"

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"

jobs:
  macos-app:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: install cargo-bundle
        run: cargo install cargo-bundle --locked
      - name: package app
        run: scripts/package-macos.sh
      - name: zip app
        run: ditto -c -k --keepParent target/release/bundle/osx/clipt9n.app clipt9n-macos.app.zip
      - uses: actions/upload-artifact@v4
        with:
          name: clipt9n-macos-app
          path: clipt9n-macos.app.zip

  linux-binary:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: package linux
        run: scripts/package-linux.sh
      - name: archive linux package
        run: tar -C target/release/package-linux -czf clipt9n-linux-x86_64.tar.gz clipt9n
      - uses: actions/upload-artifact@v4
        with:
          name: clipt9n-linux-x86_64
          path: clipt9n-linux-x86_64.tar.gz

  windows-exe:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: build release
        run: cargo build --release
      - uses: actions/upload-artifact@v4
        with:
          name: clipt9n-windows-x86_64
          path: target/release/clipt9n.exe
```

Use GitHub Releases upload if the repository already has a release-action preference; otherwise uploaded artifacts satisfy the M8 release workflow foundation.

- [ ] **Step 11.3: Commit**

```bash
git add .github/workflows/build.yml .github/workflows/release.yml
git commit -m "ci(M8): add release packaging workflow"
```

---

## Task 12: M8.D/I manual matrix and docs final pass

**Files:**
- Create: `TESTING.md`
- Create: `LICENSE`
- Create: `CHANGELOG.md`
- Modify: `README.md`

**Why:** v0.1 needs user-facing install/troubleshooting docs and a permanent manual QA log.

- [ ] **Step 12.1: Create `TESTING.md`**

Include these sections exactly:

```markdown
# clipt9n manual testing

## Release target

M8 release readiness requires:
- macOS Day-1 pass complete.
- VoiceOver pass complete for prompt, history, setup wizard, and tray-confirm modal.
- Latency benchmark recorded in `docs/benchmarks/<date>.md`.
- Linux and Windows smoke rows attempted or explicitly marked blocked with environment details.

## Translation matrix

| Source | Action | Target/instruction | Expected | Result |
|---|---|---|---|---|
| EN | Translate | EN | unchanged | [ ] |
| EN | Translate | DE | German output | [ ] |
| EN | Translate | TR | Turkish output | [ ] |
| DE | Translate | EN | English output | [ ] |
| DE | Translate | DE | unchanged | [ ] |
| DE | Translate | TR | Turkish output | [ ] |
| TR | Translate | EN | English output | [ ] |
| TR | Translate | DE | German output | [ ] |
| TR | Translate | TR | unchanged | [ ] |
| EN | Fix grammar | - | minimal edits, stays English | [ ] |
| DE | Fix grammar | - | minimal edits, stays German | [ ] |
| TR | Fix grammar | - | minimal edits, stays Turkish | [ ] |
| EN | Rewrite | - | clearer, stays English | [ ] |
| DE | Rewrite | - | clearer, stays German | [ ] |
| TR | Rewrite | - | clearer, stays Turkish | [ ] |
| EN | Custom | make formal | follows instruction | [ ] |
| DE | Custom | summarize | follows instruction | [ ] |
| TR | Custom | bullet list | follows instruction | [ ] |

## Setup wizard matrix

| Scenario | Expected | Result |
|---|---|---|
| No key, keychain available | wizard opens | [ ] |
| Invalid key | 401 shown, key retained | [ ] |
| Network down | network error shown, key retained | [ ] |
| Sample translation unchecked | save allowed after connectivity warning | [ ] |
| Provider switch mid-wizard | rows reset, key retained | [ ] |
| Keychain unavailable | env-only mode | [ ] |
| Stale key during translation | wizard auto-opens | [ ] |

## Tray and history matrix

Copy the M7 README rows here and mark each with date, OS, and result.

## Accessibility matrix

| Surface | Check | Result |
|---|---|---|
| Prompt | VoiceOver announces slots and focus order | [ ] |
| History | VoiceOver announces search/list/detail/buttons | [ ] |
| Setup wizard | VoiceOver announces provider cards as buttons | [ ] |
| Tray hide confirm | VoiceOver announces confirm/cancel and hotkey | [ ] |
| macOS Display Accommodations | contrast remains readable | [ ] |
```

- [ ] **Step 12.2: Add MIT `LICENSE`**

Use the standard MIT license text with copyright holder:

```text
MIT License

Copyright (c) 2026 Egecan

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Do not alter `Cargo.toml` license (`MIT` already present).

- [ ] **Step 12.3: Create `CHANGELOG.md`**

Include M1 through M8:

```markdown
# Changelog

## 0.1.0 - 2026-04-29

- M1: CLI walking skeleton and provider abstraction.
- M2: prompt window, global hotkey, state file.
- M3: all actions, custom prompt, translating overlay, size confirm.
- M4: glossary, template overrides, SIGHUP reload.
- M5: encrypted history and viewer.
- M6: setup wizard, keychain, kittest infrastructure.
- M7: tray icon, glossary launch, live provider rebuild, accessibility polish.
- M8: comprehensive tests, fuzz/bench harnesses, packaging, CI release workflow, manual QA docs.
```

- [ ] **Step 12.4: README final pass**

Update README status to M8/v0.1 readiness. Add install sections:

- macOS `.app` from release artifact, Gatekeeper right-click/Open workaround, `--show-tray`.
- Linux binary + `.desktop` installation and StatusNotifierItem caveat.
- Windows `.exe` caveat and tray right-click.
- Troubleshooting for Accessibility, keychain unavailable, malformed glossary/templates, history corruption, stale key.

Run:

```bash
rg -n "M3|M4 limitations|M7|deferred to M8" README.md
```

Expected: no stale "M8 will" statements remain unless they explicitly describe post-M8 work such as notarization.

- [ ] **Step 12.5: Commit**

```bash
git add TESTING.md LICENSE CHANGELOG.md README.md
git commit -m "docs(M8): add release testing matrix and final user docs"
```

---

## Task 13: M8.K carry-over polish

**Files:**
- Modify: `src/ui/setup.rs`
- Modify: `src/app.rs`
- Modify: `src/tray.rs`
- Modify: `tests/kittest_setup.rs`

**Why:** These are small deferred edges from M6/M7 that should not survive the release milestone.

- [ ] **Step 13.1: Wire provider API-key link**

In `src/ui/setup.rs`, add:

```rust
pub fn provider_key_url(provider_kind: &str) -> &'static str {
    match provider_kind {
        "anthropic" => "https://console.anthropic.com/settings/keys",
        "openai" => "https://platform.openai.com/api-keys",
        "gemini" => "https://aistudio.google.com/app/apikey",
        "ollama" => "https://ollama.com/download",
        _ => "https://console.anthropic.com/settings/keys",
    }
}
```

Extend `SetupOutcome`:

```rust
OpenProviderKeyUrl(&'static str),
```

Replace the informational label with a link:

```rust
if ui.link("Get your API key").clicked() {
    outcome = Some(SetupOutcome::OpenProviderKeyUrl(provider_key_url(&model.provider)));
}
```

In `src/app.rs::update_setup_wizard`, handle it with:

```rust
ctx.open_url(egui::OpenUrl {
    url: url.to_string(),
    new_tab: true,
});
```

Add kittest coverage that selecting Gemini changes the URL outcome to the Gemini URL.

- [ ] **Step 13.2: Fix tray icon size parameter**

Current `build_icon_buffer(status, size)` hardcodes dot coordinates for 22px. Either remove the `size` parameter and make it `build_icon_buffer(status)` or scale the dot:

```rust
let dot_size = (size / 6).max(3);
let dot_start = size.saturating_sub(dot_size + 1);
let in_dot = x >= dot_start && y >= dot_start && x < dot_start + dot_size && y < dot_start + dot_size;
```

Prefer scaling because tests can assert 22px behavior remains stable and future bundle icons can reuse the helper.

Run:

```bash
cargo test --lib tray::tests
cargo test --test kittest_setup
```

Expected: pass.

- [ ] **Step 13.3: Commit**

```bash
git add src/ui/setup.rs src/app.rs src/tray.rs tests/kittest_setup.rs
git commit -m "fix(M8): finish setup link and tray icon polish"
```

---

## Task 14: Final verification and release-readiness audit

**Files:**
- Modify: `TESTING.md` with actual manual results.
- Create: `docs/benchmarks/<date>.md` after running the benchmark.

- [ ] **Step 14.1: Automated verification**

Run:

```bash
scripts/lint-platform-discipline.sh
cargo fmt --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test --all-features
```

Expected: all pass. Sum test output with:

```bash
cargo test --all-features 2>&1 | grep "test result:" | awk '{passed+=$4; failed+=$6} END {print "passed=" passed " failed=" failed}'
```

Expected target: approximately 360-390 passed, 0 failed.

- [ ] **Step 14.2: Packaging verification**

Run on macOS:

```bash
scripts/package-macos.sh
/usr/libexec/PlistBuddy -c 'Print :LSUIElement' target/release/bundle/osx/clipt9n.app/Contents/Info.plist
codesign --verify --deep --strict target/release/bundle/osx/clipt9n.app
```

Expected: `LSUIElement` prints `true`; codesign verifies.

Run:

```bash
scripts/package-linux.sh
```

Expected: staged package directory exists with `bin/clipt9n`, `.desktop`, and icon.

- [ ] **Step 14.3: Fuzz smoke**

Run:

```bash
cargo fuzz run glossary_parser_fuzz -- -runs=1000
cargo fuzz run template_renderer_fuzz -- -runs=1000
cargo fuzz run history_decrypt_fuzz -- -runs=1000
```

Expected: no crashes.

- [ ] **Step 14.4: Latency benchmark**

Run:

```bash
scripts/bench.sh
```

Expected: `docs/benchmarks/<date>.md` exists and records p50/p95. If p50 or p95 misses the target, keep the file and add an explanation of provider/network conditions rather than hiding the result.

- [ ] **Step 14.5: Manual smoke execution**

Fill in `TESTING.md` macOS Day-1 rows:

- 9 translate combinations.
- 3 fix-grammar rows.
- 3 rewrite rows.
- 3 custom rows.
- setup wizard failure modes.
- tray hide/show recovery.
- VoiceOver pass.
- contrast check.

Linux/Windows rows may be marked blocked only with concrete environment notes.

- [ ] **Step 14.6: Final commit**

```bash
git add TESTING.md docs/benchmarks
git commit -m "test(M8): record release smoke and benchmark results"
```

---

## Spec coverage table

| Spec/design requirement | Task(s) | Verification |
|---|---:|---|
| Spec section 11 unit tests for templates/post-processing/config/glossary/crypto/history | 2, 4, 5, 6 | `cargo test --all-features` |
| Spec section 11 wiremock provider tests | 3 | `cargo test --test provider_wiremock --test retry_policy` |
| Spec section 11 SQLite integration and corruption path | 6, 12, 14 | history store tests + `TESTING.md` |
| Spec section 11 keychain integration | 7 | gated `CLIPT9N_KEYCHAIN_INTEGRATION=1` |
| Spec section 10 latency targets | 9, 14 | `docs/benchmarks/<date>.md` |
| Spec section 11 fuzz targets | 8, 14 | `cargo fuzz run ... -runs=1000` |
| Spec section 12 macOS `.app`, LSUIElement, ad-hoc signing | 10, 14 | `scripts/package-macos.sh`, PlistBuddy, codesign |
| Spec section 12 Linux binary/desktop/icon | 10, 14 | `scripts/package-linux.sh` |
| Spec section 10/12 GitHub release artifacts | 11 | `.github/workflows/release.yml` |
| Cross-platform cfg discipline | 1 | `scripts/lint-platform-discipline.sh` in CI |
| M5/M6/M7 deferred manual smoke | 12, 14 | `TESTING.md` completed rows |
| M7 carry-over setup link and tray icon polish | 13 | kittest + tray tests |

---

## Self-review

**Spec coverage:** Every deliverable in the design doc's M8 row maps to at least one task above. The only intentionally non-blocking item is Apple notarization, which the M8 handoff marks out of scope; README must document that boundary.

**Placeholder scan:** The implementation snippets name concrete files, functions, tests, and commands. Phrases like "If GitHub's hosted keychain prompts..." are explicit fallback decisions, not unspecified work. No `TBD`, `TODO`, or "fill in later" placeholders are present.

**Type consistency:**

- `AttemptOutcome::RetryAfter(Duration, E)` is referenced consistently from `llm/client.rs`, `openai.rs`, and `anthropic.rs`.
- `Glossary::load_str(&str) -> Result<Self, TranslateError>` is used by unit tests and fuzz target.
- `Templates::from_sources_for_test(...)` is gated with `#[cfg(any(test, feature = "internal-test-helpers"))]` so fuzz can use it without exposing a production API.
- `SetupOutcome::OpenProviderKeyUrl(&'static str)` matches `provider_key_url(...) -> &'static str` and the `ctx.open_url(...)` call site.
- Platform helpers keep all `cfg(target_os)`, `cfg(unix)`, and `cfg(not(unix))` inside `src/platform/`, satisfying the lint's no-exception rule.

**Execution risk:** M8 is larger than prior milestones. The riskiest tasks are Retry-After semantics, cargo-bundle packaging details, and manual VoiceOver. Land Task 1 first, then keep commits narrow so any regression has a small revert surface.

**Final exit criteria:**

1. `scripts/lint-platform-discipline.sh` passes locally and in CI.
2. `cargo fmt --check`, `cargo clippy --all-features --all-targets -- -D warnings`, and `cargo test --all-features` pass.
3. Fuzz targets build and pass `-runs=1000` smoke.
4. `scripts/package-macos.sh` produces a signed `.app` with `LSUIElement = true`.
5. GitHub Actions build matrix and release workflow are green.
6. `docs/benchmarks/<date>.md` records p50/p95 against the spec targets.
7. `TESTING.md` has completed macOS/VoiceOver rows and concrete notes for Linux/Windows rows.
8. README, LICENSE, and CHANGELOG are release-ready.
