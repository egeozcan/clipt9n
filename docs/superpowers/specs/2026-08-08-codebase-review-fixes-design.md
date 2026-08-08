# Codebase Review Fixes Design

**Date:** 2026-08-08
**Status:** Approved for planning
**Base:** `main` at `d6903eb`

## Objective

Resolve every actionable finding from the 2026-08-08 current-codebase review without allowing parallel writers to share a checkout. Preserve `main` until all lane commits have been independently reviewed, integrated, and verified.

Manual release checks that require unavailable hardware, credentials, accessibility review, or operating systems will not be marked complete without evidence. The implementation will automate what can be automated and record unavailable checks as explicitly blocked with the required environment.

## Integration Strategy

Create `review-fixes-integration` from the reviewed base. Its first implementation commit adds these project-local runtime directories to `.gitignore`:

```gitignore
/.worktrees/
/.pi-subagents/
```

All implementation worktrees live under `.worktrees/<lane>`. Each lane has one writer and one independent reviewer. Writers may commit only to their own lane branch; they may not push, merge, release, or modify another lane's worktree. The parent session owns integration, conflict resolution, aggregate verification, and final review.

Use staged integration rather than unrestricted fanout:

- Wave 1 runs the desktop-I/O, setup/settings, secrets/history, runtime/platform, and release/CI lanes concurrently.
- The parent reviews and integrates Wave 1 in dependency-safe order.
- The config-security lane starts from the integrated provider-profile/configuration interface produced by Wave 1.
- A final fresh-context review evaluates the complete integration diff.

## Lane 1: Desktop I/O Safety

**Branch:** `fix/desktop-io`
**Primary ownership:** `src/app/prompt.rs`, `src/app/translation.rs`, desktop clipboard/focus/paste interfaces, and their tests.

### Required behavior

- Inline replacement captures the originating application before selection capture.
- Completion pastes only when the original destination can be verified as still active or can be safely restored.
- If destination verification is unavailable or fails, the result remains on the clipboard and the app notifies the user instead of synthesizing a global paste.
- Selection capture restores the original clipboard on every post-copy exit path.
- Empty and supported non-text clipboard states are preserved rather than converted to an unrestorable empty string.
- Standard tests never mutate the real desktop clipboard, send real copy/paste gestures, or depend on the foreground application.

### Module design

Introduce a focused desktop-I/O seam whose interface represents coordinated user operations rather than exposing unrelated platform primitives. The production adapter may compose the existing `Clipboard` and `Platform` implementations. Tests use a deterministic adapter that records writes, copy gestures, target verification, and paste attempts.

This seam must remain narrow: selection snapshot/restore, result write, destination identity, and guarded paste. It must not become a general wrapper around every platform operation.

## Lane 2: Setup, Settings, and Provider Profiles

**Branch:** `fix/setup-settings`
**Primary ownership:** `src/app/setup.rs`, `src/app/settings.rs`, `src/ui/setup.rs`, provider construction/default metadata, and focused tests.

### Required behavior

- Every setup verification attempt carries a monotonically increasing identifier.
- Results from canceled, replaced, or previous wizard sessions are ignored.
- Provider switching builds one complete candidate containing the selected kind, default model, default base URL, account, and environment-variable metadata.
- The sample translation uses that same candidate for both provider construction and `Translator` configuration.
- Setup remains open when focus moves to another application. Explicit Cancel is the credential-discard path.
- Setup and Settings validate a candidate provider before publishing live state.
- Config and secret persistence act as a recoverable commit: a failed secret write must not leave disk advertising unavailable credentials, and a failed config replacement must not publish partial live state.
- Config files are replaced atomically rather than overwritten in place.
- Environment-variable storage may not accept a typed key and silently discard it. The UI must require the configured variable to resolve or clearly reject the typed value.

### Module design

Create a provider-profile module that owns the facts currently duplicated across config normalization, setup metadata, and the provider factory: provider ID, implementation kind, default model, default URL, account, and environment variable. Callers consume profiles instead of maintaining parallel lists.

Create one configuration-commit interface shared by Setup and Settings. It accepts a fully validated candidate and credential operation, stages effects, and publishes live state only after durable state is coherent. Rollback behavior must be explicit and tested with injected failures.

## Lane 3: Secrets, History, and Notification Privacy

**Branch:** `fix/secrets-history`
**Primary ownership:** `src/secrets.rs`, `src/history/crypto.rs`, `src/history/store.rs`, `src/notify.rs`, and focused tests.

### Required behavior

- API-key and history-secret files are owner-only from creation, not chmodded after plaintext has already been written.
- Permission, ownership, type, symlink, flush, and atomic-replacement failures are reported as failures; secret storage never fails open.
- File-backed secrets are disabled on platforms where equivalent owner-only protection cannot be guaranteed, unless a secure native implementation exists.
- Existing legacy history-key migration is provisioned before history opens. When keychain readback succeeds, the active key does not need a colocated fallback file. Legacy-file deletion must preserve data recoverability and be documented.
- `Clear all` enables SQLite secure deletion, handles journal/WAL state, and vacuums as needed so deleted records are not left in ordinary free pages.
- History query health distinguishes clean partial results from authentication/corruption failures; corruption cannot silently look like an empty history while writes continue under a different key.
- Translation notifications are metadata-only by default. Result previews require an explicit opt-in setting if retained.
- Provider error text shown in logs or notifications is bounded and stripped of control characters.

### Threat model

The fixes protect credentials and retained translations from other local users, accidental backup disclosure, notification observers, and malformed/corrupted storage. They do not claim secure erasure from external backups or protection after the user's account is fully compromised. Documentation must state that boundary.

## Lane 4: Runtime, Hotkeys, Tray, State, and Platform Adapters

**Branch:** `fix/runtime-platform`
**Primary ownership:** `src/main.rs`, hotkey configuration/registration support, `src/app/channels.rs`, `src/state.rs`, `src/platform/*`, and focused tests.

### Required behavior

- Global-hotkey dispatch reacts only to `Pressed`, not `Released`.
- All four hotkeys use one validated registration adapter and one consistent result type containing ID and failure/warning state.
- Invalid modifiers or keys never silently register a fallback shortcut while claiming to be disabled.
- History-hotkey failures contribute to the same warning status as other hotkeys.
- Rewrite slot 7 is persisted as the last repeatable action. Custom slot 8 remains unpersisted.
- Tray-driven glossary reload executes immediately while the event loop is idle and refreshes tray health without requiring a visible window.
- Windows path opening uses a native non-shell mechanism; configured paths never flow through `cmd.exe`.
- Linux detects native Wayland sessions. Selection capture and inline replacement either use a supported implementation or return a direct actionable error; they never pretend that X11-only `xdotool` works on Wayland.
- Linux packaging documents or declares the `xdotool` runtime dependency for the X11 path.

### Module design

Hotkey registration becomes a deep module: callers provide a validated hotkey description and receive a registration outcome. Modifier parsing, key conversion, native flags, registration, and warning classification stay behind that interface.

Platform adapters continue to own all target-specific code. Cross-platform app modules must not add new `cfg(target_os)` branches.

## Lane 5: Release, CI, Packaging, Fuzzing, and Repository Hygiene

**Branch:** `fix/release-ci`
**Primary ownership:** `Cargo.toml`, `Cargo.lock`, `.github/workflows/*`, packaging scripts, fuzz targets, `TESTING.md`, benchmark/release documentation, README installation metadata, and package include/exclude policy.

### Required behavior

- Package, bundle, changelog, CLI, user-agent, and release tag versions derive from one version. Release preflight rejects a mismatched tag.
- The declared MSRV is truthful. Either pin compatible dependencies for Rust 1.83 or raise the declaration to the minimum required by the locked graph. CI checks the exact declared toolchain.
- `block 0.1.6` future-incompatibility is removed by a compatible dependency upgrade where feasible; otherwise the residual dependency constraint is documented and CI tracks it.
- macOS artifacts are either universal binaries verified with `lipo -info` or explicitly architecture-labeled artifacts. An architecture-neutral name may not hide a host-only build.
- Tag packaging runs only after required formatting, lint, tests, and builds pass.
- Release assets are attached to a durable GitHub Release rather than existing only as retention-bound workflow artifacts.
- Actions, Rust, and packaging tools are pinned to auditable versions or immutable references.
- Release jobs produce checksums and provenance where supported. Signing/notarization requirements that need unavailable credentials remain explicitly blocked rather than falsely reported complete.
- The history-decrypt fuzz target asserts its intended invariant and documents the required nightly toolchain.
- The keychain integration test is a real opt-in test: once enabled it fails on readback failure. CI does not report it as exercised when it was skipped.
- Source packaging excludes IDE files, worktrees, subagent artifacts, build output, local databases, and secret files.
- README cloning/install examples contain the real repository path, not placeholders.
- `TESTING.md` records automated evidence. Unavailable VoiceOver, Intel Mac, Linux/Windows runtime, credentialed provider, and latency checks are marked blocked with the exact required environment.

## Lane 6: Configuration and Provider Boundary Security

**Branch:** `fix/config-security`
**Start point:** integrated Wave 1 branch after provider-profile work.

**Primary ownership:** configuration validation, template/glossary path resolution, provider HTTP clients, and focused tests.

### Required behavior

- Template and glossary paths reject absolute paths, parent traversal, symlink escape, and canonical targets outside their permitted config subdirectories.
- Provider URLs are parsed during validation.
- Remote providers require HTTPS.
- Plain HTTP is allowed only for explicit loopback/local endpoints intended for local providers.
- Changing a provider host through the GUI requires an explicit user-visible confirmation before the next credentialed request.
- Redirects may not forward credentials or clipboard content to a different origin. Prefer disabled redirects or a same-origin-only policy.
- Provider response/error bodies are bounded before allocation into user-facing errors and sanitized before logging or notification display.
- Unknown configuration fields are rejected with actionable errors so misspelled privacy settings cannot silently fall back to permissive defaults.
- Configuration changes remain compatible with the provider-profile and transactional-commit interfaces integrated from Wave 1.

## Testing Strategy

Every behavioral fix follows red-green-refactor:

1. Add the smallest regression test that demonstrates the reviewed failure.
2. Run it and confirm it fails for the expected reason.
3. Implement the minimum fix.
4. Run the focused test and adjacent module tests.
5. Commit only after focused verification succeeds.

Tests must assert observable behavior rather than implementation details. Desktop tests use injected adapters. Persistence tests inject write, rename, permission, keychain, and rollback failures. Async setup tests explicitly deliver stale and current generation results. Security tests cover traversal, absolute paths, symlinks, insecure URLs, redirect origins, oversized/control-character errors, and unknown fields.

Each lane must run before handoff:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
scripts/lint-platform-discipline.sh
```

Platform- or release-specific lanes also run their focused package, target-build, workflow-lint, or fuzz-build checks. A lane may report an environment-bound check as blocked only when it identifies the missing environment and supplies the command to run there.

After integration, the parent runs the full verification suite from a clean integration worktree, inspects the aggregate diff, and launches fresh-context correctness, security/privacy, tests/CI, and maintainability reviews. One integration fix worker addresses accepted final findings, followed by one scoped re-review.

## Error Handling and User Experience

- Unsafe automation degrades to a non-destructive result: keep translated text on the clipboard and notify instead of pasting into an unverified destination.
- Persistence failures preserve the previously working durable and live configuration and leave the form open with an actionable error.
- Stale async results are silently discarded and cannot change current wizard state.
- Security validation failures name the invalid field/path/URL and block the operation before reading a file or sending a request.
- Platform limitations are explicit. Wayland or unavailable secure file storage produces a targeted message rather than a generic internal error.
- Manual verification remains evidence-based: unchecked or blocked is preferable to a false pass.

## Integration Order

1. Create `review-fixes-integration`; add ignore rules.
2. Launch Wave 1 worktrees.
3. Review each Wave 1 lane independently.
4. Integrate provider/setup, secrets/history, desktop-I/O, runtime/platform, then release/CI; resolve only integration conflicts in the parent.
5. Create `fix/config-security` from the integrated Wave 1 head.
6. Review and integrate configuration security.
7. Run aggregate verification and final review.
8. Present the finished integration branch for user-selected merge/PR handling. No push, PR, merge to `main`, release, or publication occurs without explicit user authorization.

## Non-Goals

- Rewriting the entire `ClipApp` state machine.
- Replacing egui, Tokio, SQLite, or provider protocols.
- Adding unsupported Wayland automation through brittle shell-command guessing.
- Claiming secure deletion from external backups.
- Marking manual release checks complete without direct evidence.
- Publishing, signing, notarizing, pushing, or releasing artifacts automatically.
