# Codebase Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve every actionable finding in the 2026-08-08 codebase review through isolated, reviewed worktree lanes and integrate them into one verified branch.

**Architecture:** Five independent Wave 1 writers work in project-local Git worktrees, each owning a non-overlapping source seam. The parent reviews and cherry-picks each lane into `review-fixes-integration`; a sixth config-security writer then starts from that integrated head. Production changes use test-first development, narrow injected seams, atomic persistence, and explicit platform degradation.

**Tech Stack:** Rust 2021, Tokio, egui/eframe, reqwest/rustls, rusqlite, keyring, arboard, global-hotkey, GitHub Actions, Bash packaging scripts.

## Global Constraints

- Base revision is the reviewed `main` history containing design commit `3febbc0`.
- Main remains untouched after orchestration setup; implementation lands on `review-fixes-integration`.
- Worktrees live below `/.worktrees/`; `/.worktrees/` and `/.pi-subagents/` must be ignored before worktree creation.
- One writer owns each worktree. Writers may commit locally but may not push, merge, release, publish, or modify another lane.
- Every behavior change follows red-green-refactor and records the failing and passing commands in its handoff.
- Cross-platform app modules may not add target-OS conditional compilation; platform-specific code stays under `src/platform/`.
- Unsafe inline automation degrades to clipboard-only output rather than pasting into an unverified destination.
- Manual release checks are never marked passed without direct evidence. Unavailable checks state the missing environment and exact command or flow.
- No credentials, clipboard contents, history plaintext, review artifacts, worktrees, local databases, or secret files may enter Git history or Cargo packages.
- No new broad abstraction is allowed unless at least two real adapters use its interface or it directly replaces an untestable concrete dependency identified in the review.

---

### Task 1: Create the integration branch and ignored worktree root

**Files:**
- Modify: `.gitignore`

**Interfaces:**
- Produces: branch `review-fixes-integration` and ignored directory `.worktrees/` used by Tasks 2–6.
- Consumes: design commit `3febbc0`.

- [ ] **Step 1: Verify the orchestration base is clean apart from ignored runtime state**

Run:

```bash
git status --short --branch
git rev-parse HEAD
git check-ignore -q .worktrees && echo already-ignored || echo not-ignored
```

Expected: HEAD includes `3febbc0`; `.worktrees` reports `not-ignored`; `.pi-subagents/` may be untracked but contains no project source changes.

- [ ] **Step 2: Add project-local orchestration ignores**

Append exactly:

```gitignore
/.worktrees/
/.pi-subagents/
```

Do not add blanket ignores for all dot-directories.

- [ ] **Step 3: Verify ignore behavior**

Run:

```bash
mkdir -p .worktrees/probe .pi-subagents/probe
git check-ignore -v .worktrees/probe .pi-subagents/probe
git status --short
rmdir .worktrees/probe .pi-subagents/probe
```

Expected: both probe paths resolve to `.gitignore`; only `.gitignore` is modified.

- [ ] **Step 4: Create the integration branch and commit orchestration setup**

Run:

```bash
git switch -c review-fixes-integration
git add .gitignore
git commit -m "chore: ignore local agent worktrees"
```

Expected: branch `review-fixes-integration` contains one setup commit above `3febbc0`.

- [ ] **Step 5: Create Wave 1 worktrees**

Run:

```bash
git worktree add .worktrees/desktop-io -b fix/desktop-io
git worktree add .worktrees/setup-settings -b fix/setup-settings
git worktree add .worktrees/secrets-history -b fix/secrets-history
git worktree add .worktrees/runtime-platform -b fix/runtime-platform
git worktree add .worktrees/release-ci -b fix/release-ci
git worktree list
```

Expected: each worktree is based on the integration setup commit and has a distinct branch.

---

### Task 2: Make desktop selection and inline replacement safe and testable

**Files:**
- Create: `src/desktop_io.rs`
- Modify: `src/lib.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/prompt.rs`
- Modify: `src/app/translation.rs`
- Modify: `src/platform/mod.rs`
- Modify: `src/platform/macos.rs`
- Test: unit tests beside `src/desktop_io.rs` and focused app tests

**Interfaces:**
- Produces:
  - `DesktopTarget` as an opaque, comparable destination identity.
  - `SelectionSnapshot { selected_text, target }`.
  - `DesktopIo::capture_selection(copy_delay)`, `write_clipboard`, and `paste_if_target_current`.
  - `PasteDisposition::{Pasted, TargetChanged, Unsupported}`.
- Consumes: existing `Clipboard` and `Platform` implementations.

- [ ] **Step 1: Write failing desktop-I/O tests**

Add tests that express this interface:

```rust
#[test]
fn capture_restores_original_clipboard_when_selected_text_is_empty() {
    let mut io = FakeDesktopIo::with_text_clipboard("saved");
    io.copy_result = Ok(String::new());

    let result = io.capture_selection(Duration::ZERO);

    assert!(matches!(result, Err(TranslateError::EmptyOrNonTextClipboard)));
    assert_eq!(io.clipboard_text(), Some("saved"));
}

#[test]
fn paste_is_refused_after_target_changes() {
    let mut io = FakeDesktopIo::default();
    let original = DesktopTarget::for_test(41);
    io.current_target = Some(DesktopTarget::for_test(99));

    let result = io.paste_if_target_current(&original).unwrap();

    assert_eq!(result, PasteDisposition::TargetChanged);
    assert_eq!(io.paste_count, 0);
}
```

Also cover initial empty clipboard, a supported non-text snapshot, clipboard-read failure after Copy, and successful same-target paste.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test --lib desktop_io -- --nocapture
```

Expected: compilation/test failure because the new interface is not implemented.

- [ ] **Step 3: Implement the narrow desktop-I/O seam**

Use a shape equivalent to:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTarget(TargetIdentity);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteDisposition {
    Pasted,
    TargetChanged,
    Unsupported,
}

pub trait DesktopIo: Send {
    fn capture_selection(
        &mut self,
        copy_delay: Duration,
    ) -> Result<SelectionSnapshot, TranslateError>;
    fn write_clipboard(&mut self, text: &str) -> Result<(), TranslateError>;
    fn paste_if_target_current(
        &mut self,
        target: &DesktopTarget,
    ) -> Result<PasteDisposition, TranslateError>;
}
```

The production adapter must restore the clipboard from a guard on all post-copy returns. Preserve supported non-text contents through arboard's available image API; represent an originally empty clipboard explicitly and clear it during restoration.

- [ ] **Step 4: Carry the destination through inline translation**

Extend `AppState::TranslatingInline` and `TranslationOutcome` with the originating `DesktopTarget`. Capture before triggering Copy. On completion:

```rust
match self.desktop_io.paste_if_target_current(&target)? {
    PasteDisposition::Pasted => { /* record success */ }
    PasteDisposition::TargetChanged | PasteDisposition::Unsupported => {
        crate::notify::inline_result_ready_for_manual_paste()?;
    }
}
```

Always write the translated result to the clipboard first. Never reactivate and paste into an unverifiable target.

- [ ] **Step 5: Replace the real-side-effect app test**

Remove direct `ArboardClipboard::new()` and real `paste_from_clipboard()` use from `test_handle_translation_done_inline_writes_clipboard_and_pastes`. Inject `FakeDesktopIo`, assert its recorded clipboard write and paste disposition, and add a changed-target test that asserts zero paste attempts.

- [ ] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
cargo test --lib desktop_io app::translation app::prompt -- --nocapture
cargo test --test kittest_selection
```

Expected: all focused tests pass and no test sends a real copy/paste event.

- [ ] **Step 7: Run lane verification and commit**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
git add src tests
git commit -m "fix: guard inline paste destination"
```

---

### Task 3: Scope setup verification and make configuration commits transactional

**Files:**
- Create: `src/llm/profiles.rs`
- Create: `src/config_commit.rs`
- Modify: `src/llm/mod.rs`
- Modify: `src/llm/factory.rs`
- Modify: `src/config.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/setup.rs`
- Modify: `src/app/settings.rs`
- Modify: `src/ui/setup.rs`
- Modify: `src/ui/settings.rs`
- Test: unit tests beside new modules and setup/settings tests

**Interfaces:**
- Produces:
  - `ProviderProfile { id, implementation, default_model, default_base_url, account, env_var }`.
  - `provider_profile(id) -> Result<&'static ProviderProfile, TranslateError>`.
  - `VerificationId(u64)` included in every `SetupCheckResult`.
  - `ConfigCommitter::commit(candidate, credential) -> Result<CommittedConfig, TranslateError>`.
- Consumes: existing `Config`, `Secrets`, and provider factory.

- [ ] **Step 1: Write failing provider-profile tests**

Cover every provider and assert setup/factory defaults come from one profile:

```rust
#[test]
fn openai_profile_contains_coherent_defaults() {
    let profile = provider_profile("openai").unwrap();
    assert_eq!(profile.default_model, "gpt-4o-mini");
    assert_eq!(profile.default_base_url, "https://api.openai.com/v1");
    assert_eq!(profile.account, "openai");
    assert_eq!(profile.env_var, "OPENAI_API_KEY");
}
```

Add a provider-switch sample-request test that starts with Anthropic config, selects OpenAI, and asserts the serialized request model is `gpt-4o-mini`, not the prior Anthropic model.

- [ ] **Step 2: Verify provider-profile tests fail**

Run:

```bash
cargo test --lib llm::profiles app::setup -- --nocapture
```

Expected: RED because profiles and coherent sample configuration do not exist.

- [ ] **Step 3: Implement provider profiles and remove duplicated lists**

Define an implementation discriminator:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderImplementation {
    Anthropic,
    OpenAiCompatible,
}
```

Move provider defaults from `Config::normalize`, setup helper matches, and factory kind matching into one static profile table. Setup UI may derive display copy separately, but provider IDs and operational defaults must come from profiles.

- [ ] **Step 4: Write stale-verification regression tests**

Construct a wizard with active ID 2, deliver successful results for ID 1 and ID 2, and assert only ID 2 changes state:

```rust
assert_eq!(model.phase, WizardPhase::Verifying);
apply_setup_result(&mut model, VerificationId(1), stale_ok);
assert_eq!(model.phase, WizardPhase::Verifying);
apply_setup_result(&mut model, VerificationId(2), current_ok);
assert_eq!(model.check1, CheckStatus::Ok);
```

Also assert Cancel invalidates the active ID and a reopened wizard ignores the prior result.

- [ ] **Step 5: Verify stale-result tests fail, then implement IDs**

Run the focused test before and after implementation:

```bash
cargo test --lib app::setup -- --nocapture
```

Extend `SetupCheckResult` to include `VerificationId`. Increment on Verify, Cancel, and every newly seeded wizard. Async tasks capture and return their ID. Drain logic discards mismatches before touching the model.

- [ ] **Step 6: Keep setup open across focus changes**

Add a pure focus-dismiss test that demonstrates Settings and SetupWizard are exempt while transient prompt states still dismiss. Update the state predicate in `app/mod.rs`; keep explicit Cancel unchanged.

- [ ] **Step 7: Write transactional persistence failure tests**

Use injected filesystem and credential adapters to cover:

```rust
#[test]
fn failed_secret_write_preserves_old_config_file() {
    let store = FailingCredentialStore::new("denied");
    let fs = MemoryAtomicConfig::containing(old_toml());
    let result = ConfigCommitter::new(fs.clone(), store).commit(candidate(), credential());
    assert!(result.is_err());
    assert_eq!(fs.contents(), old_toml());
}

#[test]
fn failed_config_replace_does_not_publish_candidate() {
    let fs = MemoryAtomicConfig::fail_rename();
    let result = ConfigCommitter::new(fs, RecordingCredentialStore::default())
        .commit(candidate(), credential());
    assert!(result.is_err());
}
```

- [ ] **Step 8: Implement atomic config replacement and shared commit flow**

Write same-directory temporary files, flush, sync, and atomically rename. Setup and Settings build a candidate and provider before invoking the committer. Publish `self.cfg` and `self.provider` only after commit succeeds. Preserve the form and previous live state on error.

For environment storage, reject a typed key and require `env_var` resolution before Save. The user-facing error must name the variable.

- [ ] **Step 9: Run focused and full lane verification**

Run:

```bash
cargo test --lib llm::profiles config_commit app::setup app::settings -- --nocapture
cargo test --test kittest_setup --test kittest_settings
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
```

- [ ] **Step 10: Commit coherent increments**

Create two commits:

```bash
git add src/llm src/config.rs src/ui/setup.rs
git commit -m "refactor: centralize provider profiles"
git add src/app src/ui/settings.rs src/config_commit.rs src/lib.rs tests
git commit -m "fix: make setup and settings commits transactional"
```

---

### Task 4: Harden secrets, history integrity, deletion, and notifications

**Files:**
- Modify: `src/secrets.rs`
- Modify: `src/history/crypto.rs`
- Modify: `src/history/store.rs`
- Modify: `src/history/mod.rs`
- Modify: `src/notify.rs`
- Modify: `src/config.rs` only for notification-preview configuration if necessary
- Modify: `src/main.rs` only for history-key provisioning order
- Test: focused secret/history/notification tests

**Interfaces:**
- Produces:
  - secure atomic secret-file helper with owner-only creation semantics.
  - `HistoryQueryResult { entries, health }` or equivalent explicit corruption signal.
  - metadata-only notification body by default.
- Consumes: platform permission support and existing keyring/history interfaces.

- [ ] **Step 1: Write secure-file regression tests**

On Unix, assert the file mode is `0600` immediately after creation and that symlinks/non-regular destinations are rejected. Add injected permission and rename failures asserting `Err`, never `Ok` with a warning.

```rust
#[test]
fn api_key_write_rejects_symlink_destination() {
    let target = tempdir.path().join("target");
    let link = tempdir.path().join("api-key");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let err = FileSecrets::new(link).set_api_key(Zeroizing::new("secret".into())).unwrap_err();
    assert!(err.to_string().contains("symlink"));
}
```

Place target-specific test setup inside `src/platform/` test helpers to preserve platform discipline.

- [ ] **Step 2: Verify secure-file tests fail, then implement atomic owner-only creation**

Run:

```bash
cargo test --lib secrets history::crypto -- --nocapture
```

Use same-directory temporary files, no-follow semantics where supported, owner-only mode at open time, flush/sync, and atomic rename. On unsupported platforms, return an actionable error for file-backed secret storage.

- [ ] **Step 3: Write history corruption-health tests**

Tamper one row and assert query reports corruption rather than silently returning a clean empty/partial list. Assert the app disables subsequent writes after the first integrity failure.

- [ ] **Step 4: Implement explicit history health**

Use:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryHealth {
    Healthy,
    Corrupt { skipped_rows: usize },
}

pub struct HistoryQueryResult {
    pub entries: Vec<HistoryEntry>,
    pub health: HistoryHealth,
}
```

App history handling surfaces the corruption banner and flips the existing disabled flag when health is corrupt.

- [ ] **Step 5: Write and implement secure-clear tests**

After inserting recognizable plaintext-encrypted records, call `clear_all`, verify count zero, `PRAGMA secure_delete` enabled, journal/WAL checkpoint completed where applicable, and `VACUUM` succeeds. Do not claim deletion from backups.

- [ ] **Step 6: Provision keychain history keys before opening history**

Add tests for keychain-present, keychain-empty-with-legacy-file, and keychain-unavailable cases. Verify readback before removing a migrated legacy file. If safe removal is not available, rename it to a documented recovery file with owner-only permissions and report that state explicitly.

- [ ] **Step 7: Make notifications metadata-only**

Change the default body test to:

```rust
#[test]
fn translation_notification_omits_result_text_by_default() {
    let body = translation_copied_body("Translate to Deutsch", "private medical text", false);
    assert_eq!(body, "Translate to Deutsch");
    assert!(!body.contains("medical"));
}
```

If preview opt-in is retained, add a default-false config field and retain the existing bounded preview test only for `true`.

- [ ] **Step 8: Bound and sanitize error presentation**

Add tests with a 100 KiB provider message and ANSI/control characters. User-visible/loggable text must have a fixed character cap and contain no non-whitespace control characters.

- [ ] **Step 9: Run lane verification and commit**

Run:

```bash
cargo test --lib secrets history notify -- --nocapture
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
git add src tests
git commit -m "fix: harden secrets and retained history"
```

---

### Task 5: Unify hotkeys and fix runtime/platform behavior

**Files:**
- Create: `src/hotkeys.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Modify: `src/app/channels.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/state.rs`
- Modify: `src/platform/mod.rs`
- Modify: `src/platform/linux.rs`
- Modify: `src/platform/windows.rs`
- Modify: `scripts/package-linux.sh`
- Test: hotkey, state, tray-channel, and platform tests

**Interfaces:**
- Produces:
  - `HotkeyBinding` validated description.
  - `RegistrationOutcome { id: Option<u32>, warning: Option<HotkeyWarning> }`.
  - non-shell Windows opener.
  - explicit Linux session capability.
- Consumes: `global-hotkey` and existing platform trait.

- [ ] **Step 1: Write hotkey validation/registration tests**

Cover invalid modifier, invalid key, disabled binding, registration conflict, and history warning aggregation. Assert invalid input produces no registration attempt and no fallback shortcut.

- [ ] **Step 2: Verify RED and implement the hotkey module**

Run:

```bash
cargo test --lib hotkeys -- --nocapture
```

Define:

```rust
pub struct HotkeyBinding<'a> {
    pub name: &'a str,
    pub modifier: &'a str,
    pub option: bool,
    pub shift: bool,
    pub key: &'a str,
    pub enabled: bool,
}

pub struct RegistrationOutcome {
    pub id: Option<u32>,
    pub warning: Option<HotkeyWarning>,
}
```

Move letter conversion, modifier resolution, native flags, and manager registration out of `main.rs`. Register all four bindings through the same function.

- [ ] **Step 3: Filter Released events**

Add a channel-dispatch test containing Pressed then Released events and assert one action. Implement an early continue for every non-Pressed event before ID routing.

- [ ] **Step 4: Persist Rewrite but not Custom**

Update state tests first:

```rust
state.record_slot(7);
assert_eq!(state.last_slot, Some(7));
state.record_slot(8);
assert_eq!(state.last_slot, Some(7));
```

Then change the accepted range to `1..=7`.

- [ ] **Step 5: Make tray glossary reload immediate while idle**

Add a test that dispatches the tray reload from Idle and asserts the glossary loader runs in the same update cycle and tray status refreshes. Call the reload operation directly or reorder draining so no second repaint is required. Move `refresh_tray_status` before the hidden-window early return.

- [ ] **Step 6: Replace Windows shell opening**

Implement `ShellExecuteW` or an equivalent native crate/API under `src/platform/windows.rs`. Add a pure argument/path test containing `&`, `|`, spaces, and quotes; no code path may invoke `cmd.exe`.

- [ ] **Step 7: Detect unsupported Wayland automation**

Add a platform capability enum:

```rust
pub enum SelectionAutomation {
    Supported,
    Unsupported(&'static str),
}
```

Native Wayland returns an actionable unsupported message until a real portal/compositor adapter exists. X11 uses `xdotool`; check command status and document the runtime dependency in the Linux package/README.

- [ ] **Step 8: Run lane verification and commit**

Run:

```bash
cargo test --lib hotkeys state app::channels platform -- --nocapture
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
git add src scripts tests
git commit -m "fix: unify hotkeys and platform runtime behavior"
```

---

### Task 6: Repair release metadata, CI, packaging, fuzzing, and evidence

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `.github/workflows/build.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `scripts/package-macos.sh`
- Modify: `scripts/package-linux.sh`
- Modify: `fuzz/fuzz_targets/history_decrypt_fuzz.rs`
- Modify: `tests/keychain_integration.rs`
- Modify: `.gitignore` if package-local secret patterns are not already covered
- Modify: `README.md`
- Modify: `TESTING.md`
- Create: `docs/benchmarks/README.md`

**Interfaces:**
- Produces: one version source, truthful MSRV gate, test-gated durable releases, architecture-explicit artifacts, meaningful fuzz and keychain checks.
- Consumes: current GitHub Actions and packaging scripts.

- [ ] **Step 1: Add release preflight checks**

Create scriptable checks in workflow steps that fail when:

```bash
tag="${GITHUB_REF_NAME#v}"
package_version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
test "$tag" = "$package_version"
```

Remove the separate bundle version override or set it from the package version. Update changelog metadata consistently.

- [ ] **Step 2: Make MSRV truthful and tested**

Inspect locked dependency `rust-version` fields. Prefer pinning compatible transitive versions only when supported without security regressions; otherwise set `package.rust-version` to the actual minimum. Add a CI job that installs exactly that toolchain and runs `cargo check --locked`.

Run locally with an installed matching toolchain when available; otherwise record the exact CI command as environment-bound.

- [ ] **Step 3: Address the future-incompatible `block` dependency**

Upgrade the egui/eframe/wgpu chain to the smallest compatible release that removes `block 0.1.6` when the public interfaces remain compatible. If no compatible release exists within scope, add a CI `cargo report future-incompatibilities` check and document the upstream blocker rather than suppressing it.

- [ ] **Step 4: Gate release packaging on verification**

Add a preflight job running formatting, platform lint, clippy with denied warnings, full tests, and release builds. All platform packaging jobs use `needs: preflight`.

Pin Rust and third-party actions/tools to immutable or exact audited versions. Grant minimal workflow `permissions`; only the release-upload job receives `contents: write`.

- [ ] **Step 5: Produce architecture-honest macOS artifacts**

Either:

```bash
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create \
  target/aarch64-apple-darwin/release/clipt9n \
  target/x86_64-apple-darwin/release/clipt9n \
  -output "$app_path/Contents/MacOS/clipt9n"
lipo -info "$app_path/Contents/MacOS/clipt9n"
```

or publish two explicitly architecture-labeled bundles. Never publish host-only output as generic `clipt9n-macos-app`.

- [ ] **Step 6: Create durable release assets**

Create or update the tag's GitHub Release and attach platform archives plus SHA-256 checksums. Emit provenance/attestations where GitHub supports it. Signing/notarization stays blocked unless credentials are present and must not be represented as complete.

- [ ] **Step 7: Fix fuzz and keychain test semantics**

For fuzzing, generate a valid ciphertext then mutate nonce/ciphertext and assert decryption fails; arbitrary bytes alone may theoretically form a valid ciphertext and are not an unconditional rejection invariant. Document `cargo +nightly fuzz run history_decrypt_fuzz`.

Change the keychain integration test to `#[ignore]` by default. When explicitly run, every readback error fails the test. Do not print “skipping” from a test reported as passed.

- [ ] **Step 8: Tighten package and repository hygiene**

Add Cargo `exclude` entries for `.idea`, `.pi-subagents`, `.worktrees`, `target`, `fuzz/target`, local config, `history.db`, `.history-key`, and `api-key`. Add matching Git ignores where safe. Verify:

```bash
cargo package --allow-dirty --list | rg '(\.idea|\.pi-subagents|\.worktrees|history\.db|api-key|\.history-key)' && exit 1 || true
```

Replace the README clone placeholder with the repository's actual remote URL derived from `git remote get-url origin`.

- [ ] **Step 9: Record evidence honestly**

Update `TESTING.md` with automated command/date/results. Mark unavailable VoiceOver, Intel Mac, Linux/Windows runtime, real-provider latency, signing, and notarization rows as `BLOCKED` with their exact environment requirements. Add `docs/benchmarks/README.md` describing the benchmark command and report format without inventing results.

- [ ] **Step 10: Run lane verification and commit**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
cargo package --allow-dirty --list
cargo build --release
```

Create focused commits:

```bash
git add Cargo.toml Cargo.lock .github scripts
git commit -m "ci: gate versioned cross-platform releases"
git add fuzz tests .gitignore README.md TESTING.md docs/benchmarks
git commit -m "test: make release evidence explicit"
```

---

### Task 7: Review and integrate Wave 1 lanes

**Files:**
- Modify only through reviewed cherry-picks and explicit conflict resolutions on `review-fixes-integration`.
- Create reports under the plan's ignored SDD workspace, not in project source.

**Interfaces:**
- Consumes: completed branches from Tasks 2–6 and their command evidence.
- Produces: one integrated Wave 1 head from which Task 8 branches.

- [ ] **Step 1: Independently review every lane**

For each branch in `fix/desktop-io`, `fix/setup-settings`, `fix/secrets-history`, `fix/runtime-platform`, and `fix/release-ci`, generate `git diff review-fixes-integration...BRANCH` and dispatch a fresh reviewer scoped to that branch's task. Require separate verdicts for spec compliance and code quality. Any Important finding returns to that lane's writer for a fix and scoped re-review.

- [ ] **Step 2: Cherry-pick provider/setup first**

Run:

```bash
git switch review-fixes-integration
git rev-list --reverse review-fixes-integration..fix/setup-settings | xargs git cherry-pick
```

Resolve no unrelated conflicts. Run focused provider/setup tests after the pick.

- [ ] **Step 3: Integrate secrets/history, desktop I/O, runtime/platform, and release/CI**

Cherry-pick reviewed commits one lane at a time. After each lane, run its focused commands. For overlapping `Cargo.toml`, `config.rs`, `main.rs`, or platform files, preserve both reviewed behaviors and add an integration regression test when conflict resolution changes executable code.

- [ ] **Step 4: Run Wave 1 aggregate verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
cargo build --release
git diff --check 3febbc0...HEAD
```

Expected: all commands exit zero. Record any environment-bound release checks as blocked, not passed.

- [ ] **Step 5: Create config-security worktree from integrated head**

Run:

```bash
git worktree add .worktrees/config-security -b fix/config-security
git -C .worktrees/config-security rev-parse HEAD
git rev-parse HEAD
```

Expected: both revisions match the Wave 1 integration head.

---

### Task 8: Constrain configuration paths, URLs, redirects, and error bodies

**Files:**
- Modify: `src/config.rs`
- Modify: `src/llm/templates.rs`
- Modify: `src/glossary.rs` or add a shared confined-path helper
- Modify: `src/llm/anthropic.rs`
- Modify: `src/llm/openai.rs`
- Modify: provider-profile/config UI code integrated from Task 3
- Test: config, templates, glossary, provider wiremock, and settings tests

**Interfaces:**
- Produces:
  - confined config path resolver.
  - validated `ProviderEndpoint` or equivalent parsed URL.
  - same-origin/no-redirect HTTP policy.
  - bounded sanitized provider error.
- Consumes: integrated provider profiles and transactional configuration commit.

- [ ] **Step 1: Write path-confinement tests**

Cover absolute paths, `..`, symlink escape, valid nested files, and missing optional defaults:

```rust
#[test]
fn template_override_rejects_parent_escape() {
    let err = resolve_confined(config_dir, "templates", "../secret.txt").unwrap_err();
    assert!(err.to_string().contains("outside"));
}

#[test]
fn template_override_rejects_symlink_escape() {
    // templates/escape points outside config_dir
    assert!(resolve_confined(config_dir, "templates", "escape/private.txt").is_err());
}
```

Apply the same policy to glossary paths with the config directory as the allowed root.

- [ ] **Step 2: Verify RED and implement confined resolution**

Run:

```bash
cargo test --lib llm::templates glossary -- --nocapture
```

Reject absolute and parent components before I/O. Canonicalize the allowed root and existing target, then require `target.starts_with(root)`. For missing optional template overrides, preserve built-in fallback without canonicalizing a nonexistent target.

- [ ] **Step 3: Write endpoint-policy tests**

Cover HTTPS remote endpoints, HTTP loopback (`localhost`, `127.0.0.1`, `[::1]`), rejected remote HTTP, embedded credentials, invalid URLs, and host changes.

```rust
assert!(ProviderEndpoint::parse("https://api.openai.com/v1", false).is_ok());
assert!(ProviderEndpoint::parse("http://127.0.0.1:11434/v1", true).is_ok());
assert!(ProviderEndpoint::parse("http://example.com/v1", false).is_err());
```

- [ ] **Step 4: Implement endpoint validation and confirmation state**

Parse URLs during config validation. Permit HTTP only for loopback/local-provider profiles. Reject embedded username/password. Settings detects an origin change from the original config and requires a dedicated confirmation checkbox before Save.

- [ ] **Step 5: Write redirect credential-leak tests**

Use wiremock servers A and B. Server A returns a redirect to B. Assert B receives no request. Configure reqwest with redirects disabled or a same-origin policy shared by both provider adapters.

- [ ] **Step 6: Bound and sanitize response errors before reading/displaying**

Read at most a fixed 8 KiB response-body budget, convert to a maximum 2,000-character sanitized message, and strip control characters except newline/tab. Add wiremock tests for oversized and ANSI-bearing errors.

- [ ] **Step 7: Reject unknown configuration fields**

Add `#[serde(deny_unknown_fields)]` to user-authored configuration sections after adding tests for misspelled `store_txt`, provider fields, nested hotkeys, templates, and glossary settings. Preserve backward compatibility only for fields currently documented in the repository.

- [ ] **Step 8: Run lane verification and commit**

Run:

```bash
cargo test --lib config llm::templates glossary -- --nocapture
cargo test --test provider_wiremock --test kittest_settings
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
git add src tests
git commit -m "fix: constrain configuration trust boundaries"
```

---

### Task 9: Integrate configuration security and run final review

**Files:**
- Integration branch only; no new feature scope.

**Interfaces:**
- Consumes: reviewed `fix/config-security` and Wave 1 integration head.
- Produces: final reviewed integration branch ready for user-selected merge/PR handling.

- [ ] **Step 1: Review and cherry-pick config security**

Require independent spec and quality approval, fix Important findings in its worktree, then:

```bash
git switch review-fixes-integration
git rev-list --reverse review-fixes-integration..fix/config-security | xargs git cherry-pick
```

- [ ] **Step 2: Run complete fresh verification**

Run without relying on lane reports:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
cargo build --release
cargo package --allow-dirty --list
git diff --check 3febbc0...HEAD
git status --short --branch
```

Inspect package output for `.idea`, `.pi-subagents`, `.worktrees`, local databases, and secret names.

- [ ] **Step 3: Launch broad final review**

Dispatch fresh-context reviewers for:

1. correctness, concurrency, state transitions, and regressions;
2. security/privacy, credential flow, path/URL trust, and history guarantees;
3. tests, CI, packaging, cross-platform behavior, and manual-evidence honesty;
4. module depth, duplicated policy, and unnecessary complexity.

Each finding must include severity and file/line evidence. Reviewers do not edit.

- [ ] **Step 4: Apply one synthesized final fix wave**

One writer receives all accepted final findings, writes regression tests first, commits fixes, and runs focused verification. One scoped re-review verifies only those findings and new breakage.

- [ ] **Step 5: Run final verification after the fix wave**

Repeat the complete Step 2 command set. Record exact pass counts, blocked environment checks, final commits, and residual risks.

- [ ] **Step 6: Present integration options without publishing**

Report the integration branch and offer: merge locally, open a PR, keep the branch, or discard it. Do not push, merge to `main`, open a PR, sign, notarize, or release without explicit user authorization.
