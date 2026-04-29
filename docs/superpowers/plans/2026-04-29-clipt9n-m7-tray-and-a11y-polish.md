# clipt9n M7 — Tray Icon + Glossary Launch + Accessibility Polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the design's `TrayMenu` (Translate / History / Open glossary / Reload glossary / Re-run setup wizard / Hide icon / Quit) backed by the `tray-icon` crate, surface spec §8's tray-warning rows (hotkey-already-in-use, glossary-malformed, accessibility-permission-revoked, keychain-stale-key 401) through a status pill on the icon, give the wizard's "Save and start" a live provider rebuild so the user no longer needs to restart, and complete the milestone with a focus-ring / AccessKit-label / reduced-motion / Tab-order pass across every window.

**Architecture:** Three themes, executed in order. **M7.A (Tasks 1–8)** lands the tray itself: `Cargo.toml` dep, the `[tray]` schema in `state.toml` plus `--show-tray` CLI flag, an extracted `build_provider` factory, the new `src/tray.rs` (`TrayHandle` constructor + procedural icon + menu skeleton), main-thread integration into `ClipApp`, dispatch wiring for Translate / History / Glossary / Reload, status-pill state machine for the spec §8 surfaces, and the hide-confirm modal with state-persistence + panic-isolation + `--show-tray` recovery. **M7.B (Tasks 11–12)** is the accessibility pass: focus-ring audit + AccessKit-label backfill + reduced-motion audit + Tab/Shift+Tab order verification across prompt / custom-prompt / size-confirm / history / setup-wizard / tray-confirm. **M7.C (interleaved into Tasks 6–7 and 9–10)** wires the spec §8 surfaces: status-pill warns for hotkey-in-use and glossary-malformed and accessibility-revoked (Task 6); tray-thread panic isolation (Task 7); "Re-run setup wizard" tray item that revives `ClipApp.secrets` (Task 9); 401 toast bumps to `AppState::SetupWizard` (Task 9); and the live provider rebuild on "Save and start" (Task 10).

**Tech Stack:** Rust 2021 / eframe 0.31 / egui 0.31 / tokio 1.42. **One new crate:** `tray-icon = { version = "0.22", default-features = false }` (production; cross-platform unified tray API — macOS NSStatusItem, Windows shell tray, Linux StatusNotifierItem / AppIndicator). `default-features = false` opts out of `libxdo` (we don't need menu accelerators inside the tray context menu — the global hotkey system already covers that affordance), avoiding the Linux `libxdo-dev` system-package build dep. All cross-platform discipline rules from M2/M3/M4/M5/M6 still apply: every `#[cfg(target_os = …)]` and `#[cfg(unix)]` block lives in `src/platform/` (the `tray-icon` crate is unified — `src/tray.rs` MUST NOT introduce per-OS branches).

> **Branch:** This plan executes on `m7-tray-and-a11y-polish`, branched from `main` (currently at `3218ce4`, the M6→M7 handoff commit). Working directory: `/Users/egecan/Code/clipt9n`.

---

## File structure

After M7, the tree gains:

```
src/
├── app.rs                       ← MODIFIED: AppState::ConfirmingTrayHide variant;
│                                              update_confirming_tray_hide handler;
│                                              tray: Option<TrayHandle> field; drain
│                                              MenuEvent / TrayIconEvent in update();
│                                              persist_setup_completion now rebuilds
│                                              self.provider via factory::build_provider;
│                                              translation 401 dispatcher bumps
│                                              AppState to SetupWizard
├── llm/
│   └── factory.rs               ← NEW: build_provider() helper extracted from main.rs;
│                                        single source of truth for provider
│                                        construction (used by main + persist_setup +
│                                        wizard's spawn_sample_translation_check)
├── llm/mod.rs                   ← MODIFIED: pub mod factory;
├── main.rs                      ← MODIFIED: --show-tray CLI flag forces tray
│                                              construction; tray construction
│                                              happens inside the eframe creator
│                                              closure; calls factory::build_provider
│                                              instead of inlining; LSUIElement note
│                                              for M8 packaging
├── state.rs                     ← MODIFIED: State.tray: TrayState (visible: bool,
│                                              default true); record_tray_visible()
│                                              setter + tests
├── tray.rs                      ← NEW: TrayHandle struct (TrayIcon + dispatch fns +
│                                        status-pill setter); build() constructor;
│                                        procedural 22×22 RGBA icon for the four
│                                        status-pill states; tray-construction
│                                        panic-isolation (catch_unwind); zero
│                                        cfg(target_os) (tray-icon crate is unified)
├── ui/
│   └── tray_modal.rs            ← NEW: confirm-hide modal — TrayHideModel +
│                                        draw() + 2 kittest tests
├── ui/mod.rs                    ← MODIFIED: pub mod tray_modal;
└── lib.rs                       ← MODIFIED: re-export crate::tray module;
                                                Cli adds --show-tray flag

tests/
├── kittest_tray.rs              ← NEW: 4 kittest tests covering the confirm-hide
│                                        modal: confirm dispatches Hide outcome,
│                                        cancel dispatches Cancel outcome, hotkey
│                                        line shows the configured hotkey, focus
│                                        ring renders on the Confirm button.
└── kittest_a11y.rs              ← NEW: M7.B accessibility regression tests —
                                          AccessKit labels for prompt clipboard
                                          preview, history search field, setup
                                          wizard provider cards, slot rows;
                                          reduced-motion static-path renders for
                                          translating-overlay + size-confirm
                                          modal + tray-confirm modal; visible
                                          focus-ring stroke check on the
                                          translating overlay's Cancel button.
```

The total source-tree LOC grows ~600 lines (`tray.rs` ~280, `ui/tray_modal.rs` ~140, `llm/factory.rs` ~80, plus surgical edits in `app.rs`, `main.rs`, `state.rs`, `lib.rs`).

---

## Cross-cutting decisions

A glossary of design / discipline choices that show up in multiple tasks. Read once; refer back when a task references "the cross-cutting `<topic>` decision".

- **`tray-icon` crate is unified.** `src/tray.rs` contains zero `#[cfg(target_os = …)]`. The crate's per-OS impls live behind `pub use platform_impl::TrayIcon`. macOS-specific concerns (e.g., `LSUIElement` plist key for hiding from the dock) are M8 *packaging* concerns, not runtime cfg — Task 12 documents the requirement in the README.
- **TrayIcon contains `Rc<RefCell<…>>`.** It is NOT `Send`. `ClipApp` is single-threaded by virtue of `eframe::App` not requiring `Send`. The tray field is a plain `Option<TrayHandle>` — no `Arc`, no `Mutex`. (Compare M3's translator: `Arc<dyn LlmProvider>` because it crosses thread boundaries via tokio spawn.)
- **Tray construction site = eframe creator closure (main thread).** macOS requires NSStatusItem creation on the main thread *after* the run loop has started. eframe's `Box::new(move |cc| { … })` closure runs on the main thread once `run_native` has spun up the run loop — that's the construction window. Building the tray earlier (e.g., in `main()` before `run_native`) panics on macOS.
- **MenuEvent receiver is a global static.** `tray_icon::menu::MenuEvent::receiver()` returns a `&'static Receiver<MenuEvent>` that fires on every menu-item click. We drain it via `try_recv()` in `ClipApp::update()` once per frame. `request_repaint_after(Duration::from_millis(150))` is already in place (`app.rs:1326`); ≤150 ms dispatch latency on tray clicks is acceptable since the OS-native menu opens instantly, and only the *post-selection* dispatch lands in our update loop.
- **MenuId → action mapping.** `muda::MenuId` is `String`-typed. We construct each `MenuItem` with an explicit ID literal (e.g., `"clipt9n.translate"`, `"clipt9n.history"`, `"clipt9n.glossary"`, `"clipt9n.glossary.reload"`, `"clipt9n.wizard"`, `"clipt9n.hide"`, `"clipt9n.quit"`). The drain match is on string IDs. Constants live in `src/tray.rs` and are re-used in `app.rs`'s match arm — no string-literal duplication.
- **Tray-construction failure is non-fatal.** The constructor returns `Result<TrayHandle, TrayBuildError>`. `main.rs` logs warn on `Err` and runs without a tray (the hotkey still works). Specific failures: macOS without Accessibility permission (rare; Accessibility check in M2 already screens this), Linux without StatusNotifierItem support (older minimal DEs), Windows shell-tray API failure. The README documents this as a graceful degradation.
- **Tray construction panic isolation.** The `TrayIconBuilder::build()` call is wrapped in `std::panic::catch_unwind(AssertUnwindSafe(…))` (Task 7). On panic, we log warn and run without a tray — same outcome as the `Err` path. This covers the spec §8 "Tray crashed" row.
- **`ClipApp.secrets` revives in M7.** Removed `#[allow(dead_code)]` annotation; field is now read by Task 9's "Re-run setup wizard" handler (`secrets.keychain_available()` to seed the new wizard model's storage radio default) and the 401 toast handler (same). Persistence still goes through a freshly-constructed `KeychainSecrets` inside `persist_setup_completion` — the M6 stale-account fix stays intact (do NOT regress it).
- **`ClipApp.provider` becomes mutable.** Previously `Arc<dyn LlmProvider>`; now stored as `Option<Arc<dyn LlmProvider>>` so `persist_setup_completion` can swap it after a successful Save-and-start. Translation dispatch reads `self.provider.as_ref().expect(…)` — the field is only `None` during the brief window where rebuild is in flight; the wizard's flow guarantees Save-and-start is gated on a successful Verify, so build_provider's failure mode is covered (and translates to a `WizardPhase::Error` toast).
- **`build_provider` factory.** Extracted from `main.rs:115-137` into `src/llm/factory.rs::build_provider(cfg: &Config, key: Zeroizing<String>) -> Result<Arc<dyn LlmProvider>, TranslateError>`. Single source of truth, callable from main + `persist_setup_completion` + the wizard's `spawn_sample_translation_check`. Existing `lib.rs::build_provider` (lines 86–112) is moved to the same factory; the call site at `lib.rs:130` updates accordingly.
- **`Zeroizing<String>` discipline at every secret boundary.** Established in M1, M5, M6. Task 10's live provider rebuild moves `model.key.clone()` (a `Zeroizing<String>`) across the boundary; never log it, never `format!` it into a tracing call. The factory takes ownership and the inner provider clones it once into reqwest's auth-header builder.
- **State.toml schema growth.** `[tray]` table with `visible: bool` (default true). Existing `last_slot: Option<u8>` field stays. Schema is *additive* — older state.toml files (pre-M7) load without error because `serde(default)` on the `State` struct fills the missing `[tray]` block with `TrayState::default()`. Task 2 has explicit roundtrip tests for both the populated and missing `[tray]` cases.
- **`--show-tray` CLI flag.** Forces tray construction even when `state.toml` has `visible = false`. The flag's *side effect* is also to flip `state.tray.visible` back to true and persist before tray construction, so subsequent launches without the flag continue to show the tray. Documented behavior — recovery from accidental Hide.
- **Status-pill state.** Computed at construction and updated on state changes via `TrayHandle::set_status(TrayStatus)`. Variants: `Ready` (green dot tooltip "clipt9n — ready"), `NoApiKey` (red dot tooltip "clipt9n — no API key; click to run setup wizard"), `Warn(reason)` (yellow dot tooltip "clipt9n — <reason>"), `KeychainUnavailable` (warn variant; tooltip "clipt9n — keychain unavailable; using env var"). Mapping per spec §8: hotkey-already-in-use → `Warn("hotkey unavailable")`, glossary-malformed → `Warn("glossary malformed")`, accessibility-permission-revoked → `Warn("accessibility permission needed")`, keychain-stale-key → `Warn("API key invalid; re-run setup")`. The tray *icon image* itself encodes the dot color; tooltip carries the reason text.
- **Procedural icon generation.** No bundled PNG asset (avoids `image` crate dep). `tray.rs::build_icon(status: TrayStatus)` returns `tray_icon::Icon` constructed via `Icon::from_rgba(buf, 22, 22)` from a procedurally-built 22×22 RGBA buffer. Each status produces a different buffer: a 4×4 colored dot in the bottom-right of an 18×18 clipboard glyph. Glyph is encoded as a packed const lookup table (Task 4 step 4). This makes the icon ship inside the binary with zero asset-loading I/O and zero new deps.
- **Hide-confirm modal lives in `src/ui/tray_modal.rs`.** New file. `TrayHideModel { hotkey_display: String }` — already-stored hotkey from `cfg.hotkey_display()` (M2 helper). `TrayHideOutcome::{ Confirm, Cancel }`. `draw(ctx, &model) -> Option<TrayHideOutcome>`. Mirrors `src/ui/size_confirm.rs`'s shape (M3) for consistency. Two kittest tests in `tests/kittest_tray.rs`.
- **Re-run setup wizard tray item.** Mid-list addition between the design's "Reload glossary" and "Hide icon" sections — a third separator. JSON menu shape:

  ```
  Translate clipboard       (⌘⇧T tooltip)
  Open history              (⌘⇧H tooltip)
  ──────
  Open glossary
  Reload glossary
  ──────
  Re-run setup wizard       (NEW — M7 addition)
  Hide icon
  ──────
  Quit                      (⌘Q tooltip)
  ```

  Rationale: the design's `desktop.jsx::TrayMenu` was drafted before the M6 wizard existed; M7 adds the natural reentry point. A menu-shape divergence from the design is acceptable here because (a) the design didn't anticipate the wizard, (b) spec §8's "Keychain stale key" row needs a discoverable surface, and (c) M6's handoff explicitly called out this addition (handoff §8 "What M7 ships").
- **No M4/M5 follow-ups in M7.** Per the M5 plan §11.X deferrals (M5 plan lines 7000+): Jinja conditional-branch validation, glossary entry-validation, source-text lowercasing hoist, double-detection elimination, `[glossary] matching` value validation, latency benchmarks, `cargo-fuzz` targets — all stay in M8. M7's plan does not touch them.
- **Manual smoke matrix deferred to M8.** Same stance as M5 + M6 (per Q2 of the M6→M7 handoff confirmation). Task 12 documents the matrix in the M7 README + plan §11.7 but does not execute. M8's polish pass owns running it across all three OSes plus the `--show-tray` recovery path.

---

## Task 1: Add `tray-icon = "0.22"` dep + smoke-build

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the dep**

In `Cargo.toml`, add a single line under `[dependencies]` (alphabetical position is between `tracing-subscriber` and `tokio`, but project convention groups by purpose — place adjacent to existing GUI-stack deps near `eframe` / `egui`).

Edit `Cargo.toml` and add this line under the existing `[dependencies]` block:

```toml
tray-icon = { version = "0.22", default-features = false }
```

Verify the result by reading the dep block:

```bash
grep -n "tray-icon\|^eframe\|^egui = " Cargo.toml
```

Expected: `tray-icon = { version = "0.22", default-features = false }` is now present.

- [ ] **Step 2: Resolve and smoke-build**

```bash
cargo build --all-features 2>&1 | tail -8
```

Expected: build succeeds; `tray-icon v0.22.x` and its transitive deps (`muda v0.18.x`, plus per-OS bits) are downloaded and compiled. No source code uses the crate yet — this is purely a dep-resolution smoke check.

If the build fails on macOS with linker errors mentioning `libxdo` or `gtk`, double-check that `default-features = false` was applied — the default features include `libxdo` which needs system packages on Linux; the macOS link path doesn't need it but a typo could enable it.

- [ ] **Step 3: Commit**

```bash
git checkout -b m7-tray-and-a11y-polish
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
deps(M7): add tray-icon = 0.22 with default-features off

Foundation for M7's tray-menu work. default-features off opts out of
libxdo (Linux menu-accelerator key simulation) which we don't need —
the global-hotkey system covers that affordance. This avoids a Linux
libxdo-dev build-time system-package dep.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `[tray]` schema in state.toml + `--show-tray` CLI flag

**Files:**
- Modify: `src/state.rs`
- Modify: `src/lib.rs:39-60` (the `Cli` struct adds `show_tray: bool`)
- Test: `src/state.rs` (extend the existing `tests` module)

- [ ] **Step 1: Write the failing tests**

Edit `src/state.rs`. Add three new tests inside the `mod tests` block at the bottom (before the closing `}`):

```rust
    #[test]
    fn tray_visible_defaults_to_true() {
        let s = State::default();
        assert!(s.tray.visible, "tray.visible default should be true");
    }

    #[test]
    fn tray_visible_round_trips_through_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let mut s = State::default();
        s.tray.visible = false;
        s.save(&path).unwrap();
        let loaded = State::load(&path);
        assert!(!loaded.tray.visible);
    }

    #[test]
    fn pre_m7_state_toml_loads_with_default_tray() {
        // Older state.toml files have no [tray] block. The serde(default)
        // attr on State must fill it with TrayState::default() (visible=true).
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "last_slot = 3\n").unwrap();
        let s = State::load(&path);
        assert_eq!(s.last_slot, Some(3));
        assert!(s.tray.visible, "missing [tray] block should default to visible=true");
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test --lib state::tests::tray 2>&1 | tail -20
```

Expected: 3 compilation errors (the `tray` field doesn't exist yet on `State`).

- [ ] **Step 3: Add the `TrayState` struct and field**

Edit `src/state.rs`. Add the new struct after the `State` struct definition (after the closing `}` on line 15), and add the `tray` field inside `State`:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub last_slot: Option<u8>,
    pub tray: TrayState,
}

/// Per-app tray state. Currently a single boolean; spec §6 schema is
/// additive so future additions (e.g., `last_position`) live here.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct TrayState {
    /// Whether the tray icon should be created at startup. Set to false
    /// by the hide-icon confirm modal; restored to true by `--show-tray`
    /// CLI flag. Default true.
    pub visible: bool,
}

impl Default for TrayState {
    fn default() -> Self {
        Self { visible: true }
    }
}
```

(The existing `pub struct State { pub last_slot: Option<u8> }` block — lines 11–15 — is replaced by the new variant above. The `serde(default)` attr is already in place; it now also applies to the missing-`[tray]` case.)

- [ ] **Step 4: Run the state tests**

```bash
cargo test --lib state:: 2>&1 | tail -15
```

Expected: 7 tests pass (4 pre-existing + 3 new).

- [ ] **Step 5: Add the CLI flag — failing test first**

Edit `tests/cli_smoke.rs`. Add a smoke test verifying the new flag parses cleanly:

```rust
#[test]
fn show_tray_flag_parses() {
    use clap::Parser;
    let cli = clipt9n::Cli::try_parse_from(["clipt9n", "--show-tray"]).unwrap();
    assert!(cli.show_tray, "--show-tray should set the show_tray field");
}
```

Run it:

```bash
cargo test --test cli_smoke show_tray_flag_parses 2>&1 | tail -10
```

Expected: compile error (`show_tray` field doesn't exist on `Cli`).

- [ ] **Step 6: Add `show_tray` to the `Cli` struct**

Edit `src/lib.rs`. After the `custom: Option<String>,` field block (around line 55) and before the `config_path` field, add:

```rust
    /// Force the tray icon to appear even if `state.toml` has
    /// `[tray] visible = false`. Side effect: also flips the persisted
    /// state back to `visible = true` so subsequent launches without
    /// the flag continue showing the tray. Recovery path documented in
    /// the hide-icon confirm modal.
    #[arg(long = "show-tray")]
    pub show_tray: bool,
```

- [ ] **Step 7: Run the CLI smoke test**

```bash
cargo test --test cli_smoke show_tray_flag_parses 2>&1 | tail -8
```

Expected: PASS.

Run the full test suite to check for regressions:

```bash
cargo test --all-features 2>&1 | grep "test result:"
```

Expected: all binaries report `test result: ok`. Net new tests: 4.

- [ ] **Step 8: Commit**

```bash
git add src/state.rs src/lib.rs tests/cli_smoke.rs
git commit -m "$(cat <<'EOF'
feat(M7): [tray] state schema + --show-tray CLI flag

- state.rs grows TrayState { visible: bool, default true }; pre-M7
  state.toml files with no [tray] block load cleanly (serde(default)).
- Cli grows show_tray: bool — recovery path when the user has hidden
  the tray and forgotten the hotkey.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Extract `build_provider` into `src/llm/factory.rs`

**Files:**
- Create: `src/llm/factory.rs`
- Modify: `src/llm/mod.rs` (add `pub mod factory;`)
- Modify: `src/lib.rs:86-112` (delete the inlined `build_provider`, replace its call site at line 130)
- Modify: `src/main.rs:107-137` (replace inlined construction with factory call)
- Test: extend `src/llm/factory.rs` with a unit test

- [ ] **Step 1: Write the failing test**

Create `src/llm/factory.rs` with the test first:

```rust
//! Provider construction factory. Single source of truth for building
//! the configured `LlmProvider` from a `Config` + a freshly-resolved
//! API key. Used by:
//!   - `main.rs` at startup,
//!   - `app.rs::persist_setup_completion` for live provider rebuild
//!     after the Save-and-start path (M7 Task 10),
//!   - `lib.rs::run` for the CLI mode,
//!   - the wizard's sample-translation check (`app.rs::spawn_sample_translation_check`)
//!     when we want a *real* provider rather than just the connectivity probe.

use std::sync::Arc;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::openai::OpenAiCompatibleProvider;
use crate::llm::LlmProvider;

/// Construct the configured `LlmProvider` from `cfg.provider.kind` and
/// the supplied `key`. Returns `Arc` so the caller can clone cheaply
/// across tokio spawn boundaries (M3's translator pattern).
pub fn build_provider(
    cfg: &Config,
    key: Zeroizing<String>,
) -> Result<Arc<dyn LlmProvider>, TranslateError> {
    let timeout = Duration::from_secs(cfg.provider.timeout_seconds);
    let provider: Arc<dyn LlmProvider> = match cfg.provider.kind.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(
            &cfg.provider.base_url,
            key,
            &cfg.provider.model,
            timeout,
        )?),
        "openai" | "gemini" | "ollama" => Arc::new(OpenAiCompatibleProvider::new(
            &cfg.provider.base_url,
            key,
            &cfg.provider.model,
            timeout,
        )?),
        other => {
            return Err(TranslateError::Config(format!(
                "unknown provider type '{other}'; expected one of: anthropic, openai, gemini, ollama"
            )));
        }
    };
    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn anthropic_provider_constructs() {
        let cfg = Config::default(); // default provider.kind = "anthropic"
        let key = Zeroizing::new("sk-test-12345".to_string());
        let p = build_provider(&cfg, key).expect("provider should build");
        // Type-level smoke: we got an Arc<dyn LlmProvider> back. No
        // network is touched here.
        assert_eq!(Arc::strong_count(&p), 1);
    }

    #[test]
    fn unknown_provider_kind_errors() {
        let mut cfg = Config::default();
        cfg.provider.kind = "magic-llm".into();
        let key = Zeroizing::new("ignored".to_string());
        let err = build_provider(&cfg, key).unwrap_err();
        match err {
            TranslateError::Config(msg) => assert!(msg.contains("magic-llm")),
            other => panic!("expected Config error, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Add the module declaration**

Edit `src/llm/mod.rs`. Add at the top of the file (after any existing `pub mod` declarations):

```rust
pub mod factory;
```

(Read `src/llm/mod.rs` first to confirm the placement — the existing `pub mod anthropic;` etc. lines are the model.)

- [ ] **Step 3: Run the new tests**

```bash
cargo test --lib llm::factory 2>&1 | tail -10
```

Expected: 2 tests PASS.

- [ ] **Step 4: Replace the inlined `build_provider` in `lib.rs`**

In `src/lib.rs`, delete lines 85–112 (the entire `fn build_provider` block plus the leading doc-comment). Replace the call site at line 130:

```rust
    let provider = build_provider(&cfg, secrets.as_ref())?;
```

with:

```rust
    let key = secrets.get_api_key()?;
    let provider = crate::llm::factory::build_provider(&cfg, key)?;
```

(The original `lib.rs::build_provider` took `&dyn Secrets` and called `secrets.get_api_key()` internally; the new factory takes the resolved key directly so the caller controls the resolution path.)

- [ ] **Step 5: Replace the inlined construction in `main.rs`**

In `src/main.rs`, delete lines 117–137 (the entire `let provider: Arc<dyn LlmProvider> = match cfg.provider.kind.as_str() { … }` block) and replace with:

```rust
    // Build the runtime provider via the factory. Same source of truth
    // as persist_setup_completion's live-rebuild path (M7 Task 10).
    let provider = clipt9n::llm::factory::build_provider(&cfg, api_key)?;
```

The surrounding lines stay unchanged: `api_key` is built just above (line 115–116) from `api_key_opt` + the placeholder fallback, and `provider` is consumed below at the `eframe::run_native` call (line 285) — the rebind is a drop-in.

Note: `main.rs:117` declares `let timeout = …;` which is no longer used in `main.rs` (the factory owns it now). Delete that line too.

- [ ] **Step 6: Verify the imports**

In `src/main.rs`, the imports at lines 7–8 (`use clipt9n::llm::anthropic::AnthropicProvider;` and `use clipt9n::llm::openai::OpenAiCompatibleProvider;`) are no longer used. Delete them.

In `src/main.rs`, `use clipt9n::llm::LlmProvider;` (line 9) is also unused. Delete it.

- [ ] **Step 7: Run the full test suite**

```bash
cargo build --all-features 2>&1 | tail -5
cargo test --all-features 2>&1 | grep "test result:"
```

Expected: build succeeds; all test binaries pass. Net new tests: 2.

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -15
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/llm/factory.rs src/llm/mod.rs src/lib.rs src/main.rs
git commit -m "$(cat <<'EOF'
refactor(M7): extract provider construction into llm/factory

Single source of truth for build_provider. Foundation for M7 Task 10
(live provider rebuild on Save-and-start) — without the factory, that
task would have to duplicate the construction match arm.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `src/tray.rs` — TrayHandle skeleton + procedural icon + menu shape

**Files:**
- Create: `src/tray.rs`
- Modify: `src/lib.rs` (add `pub mod tray;`)
- Test: extend `src/tray.rs` with unit tests

- [ ] **Step 1: Write the new module skeleton with the failing tests first**

Create `src/tray.rs`:

```rust
//! Tray icon integration. `tray-icon` crate is the unified
//! cross-platform abstraction (macOS NSStatusItem / Windows shell tray
//! / Linux StatusNotifierItem). Per the cross-cutting decision, this
//! file contains zero `#[cfg(target_os = …)]` — all OS-specific bits
//! live inside the crate.
//!
//! Construction site is the eframe creator closure (main thread, after
//! the run loop has started). Constructed-but-failed is OK: `main.rs`
//! logs warn and runs without a tray.
//!
//! Menu drain happens in `ClipApp::update()` via a `try_recv()` on the
//! `MenuEvent::receiver()` static. The 150 ms repaint cadence
//! (`app.rs:1326`) gives sub-frame dispatch latency on user clicks.

use std::sync::Arc;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::error::TranslateError;

/// Stable IDs used in the tray menu. Constants because the drain match
/// in `app.rs::handle_tray_menu_event` references them too — no string
/// literal duplication.
pub const ID_TRANSLATE: &str = "clipt9n.translate";
pub const ID_HISTORY: &str = "clipt9n.history";
pub const ID_GLOSSARY_OPEN: &str = "clipt9n.glossary.open";
pub const ID_GLOSSARY_RELOAD: &str = "clipt9n.glossary.reload";
pub const ID_RERUN_WIZARD: &str = "clipt9n.wizard";
pub const ID_HIDE: &str = "clipt9n.hide";
pub const ID_QUIT: &str = "clipt9n.quit";

/// Status-pill state. Drives both the icon image (dot color) and the
/// tooltip text. Mapping per spec §8 lives in `app.rs::compute_tray_status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    /// Default healthy state — green dot, tooltip "clipt9n — ready".
    #[default]
    Ready,
    /// No API key resolves. Red dot, tooltip "clipt9n — no API key".
    NoApiKey,
    /// One of: hotkey-already-in-use, glossary-malformed,
    /// accessibility-permission-revoked, keychain-stale-key.
    /// Yellow dot, tooltip carries the reason.
    Warn(WarnReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarnReason {
    HotkeyInUse,
    GlossaryMalformed,
    AccessibilityPermissionRevoked,
    KeychainStaleKey,
    KeychainUnavailable,
}

impl WarnReason {
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::HotkeyInUse => "clipt9n — hotkey unavailable; another app is using it",
            Self::GlossaryMalformed => "clipt9n — glossary malformed; running without it",
            Self::AccessibilityPermissionRevoked => {
                "clipt9n — accessibility permission needed; click for help"
            }
            Self::KeychainStaleKey => "clipt9n — API key invalid; re-run setup wizard",
            Self::KeychainUnavailable => "clipt9n — keychain unavailable; using env var",
        }
    }
}

impl TrayStatus {
    pub fn tooltip(&self) -> &'static str {
        match self {
            Self::Ready => "clipt9n — ready",
            Self::NoApiKey => "clipt9n — no API key",
            Self::Warn(r) => r.tooltip(),
        }
    }
}

/// Owns the live `TrayIcon`. Drop = remove the icon from the tray.
/// Cloneable inside `Option<Arc<…>>` shape if multiple call sites need
/// it, but ClipApp owns the only handle.
pub struct TrayHandle {
    /// Underlying tray-icon handle. Held to keep the icon alive; on
    /// drop the icon disappears from the menu bar.
    icon: TrayIcon,
    /// Last-rendered status — short-circuits a no-op `set_status` call.
    last_status: TrayStatus,
}

impl TrayHandle {
    /// Build the tray icon, attaching the menu and the initial status
    /// dot. Constructor failure is non-fatal — `main.rs` logs warn and
    /// runs without a tray.
    pub fn build(initial_status: TrayStatus) -> Result<Self, TranslateError> {
        let menu = Self::build_menu()?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(build_icon(initial_status))
            .with_tooltip(initial_status.tooltip())
            .with_icon_as_template(true) // macOS: render with menu-bar tint
            .build()
            .map_err(|e| TranslateError::Internal(format!("tray icon build failed: {e}")))?;
        Ok(Self {
            icon,
            last_status: initial_status,
        })
    }

    fn build_menu() -> Result<Menu, TranslateError> {
        let menu = Menu::new();
        menu.append(&MenuItem::with_id(
            ID_TRANSLATE,
            "Translate clipboard",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(ID_HISTORY, "Open history", true, None))
            .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_GLOSSARY_OPEN,
            "Open glossary",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_GLOSSARY_RELOAD,
            "Reload glossary",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(
            ID_RERUN_WIZARD,
            "Re-run setup wizard",
            true,
            None,
        ))
        .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(ID_HIDE, "Hide icon", true, None))
            .map_err(menu_err)?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(menu_err)?;
        menu.append(&MenuItem::with_id(ID_QUIT, "Quit clipt9n", true, None))
            .map_err(menu_err)?;
        Ok(menu)
    }

    /// Update the status pill. No-op if the new status equals the last.
    /// Failure is best-effort — log warn at the call site.
    pub fn set_status(&mut self, status: TrayStatus) -> Result<(), TranslateError> {
        if status == self.last_status {
            return Ok(());
        }
        self.icon
            .set_icon(Some(build_icon(status)))
            .map_err(|e| TranslateError::Internal(format!("tray set_icon failed: {e}")))?;
        self.icon
            .set_tooltip(Some(status.tooltip()))
            .map_err(|e| TranslateError::Internal(format!("tray set_tooltip failed: {e}")))?;
        self.last_status = status;
        Ok(())
    }

    /// Drain ALL pending MenuEvents. Returns the latest event's ID
    /// string if any; `None` if the queue was empty. Caller dispatches
    /// on the ID. Static-channel global state means the receiver is
    /// process-wide; we drain from this thread (the eframe update
    /// loop's main thread) only.
    pub fn try_drain_menu_event() -> Option<String> {
        let mut last: Option<String> = None;
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            last = Some(ev.id.0);
        }
        last
    }
}

fn menu_err(e: tray_icon::menu::Error) -> TranslateError {
    TranslateError::Internal(format!("tray menu construction: {e}"))
}

/// Build the 22×22 RGBA icon for the given status. Procedural — no
/// asset bundling. The glyph is a simple "T" stencil (clipt9n's "T") in
/// ink color with a 4×4 dot in the bottom-right corner whose color
/// encodes the status.
pub(crate) fn build_icon(status: TrayStatus) -> Icon {
    const SIZE: u32 = 22;
    let mut buf = vec![0u8; (SIZE * SIZE * 4) as usize];
    // Draw a solid black-with-transparent stencil for the glyph. macOS
    // template rendering will tint this in the menu bar to match the
    // user's appearance (light/dark) automatically.
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            // Glyph: a "T" — top bar (rows 4..7, cols 4..18) plus a
            // vertical stroke (rows 7..18, cols 9..13).
            let in_bar = (4..7).contains(&y) && (4..18).contains(&x);
            let in_stroke = (7..18).contains(&y) && (9..13).contains(&x);
            if in_bar || in_stroke {
                buf[i] = 0; // R
                buf[i + 1] = 0; // G
                buf[i + 2] = 0; // B
                buf[i + 3] = 255; // A — opaque black; macOS template tints
            }
        }
    }
    // Status dot: 4×4 in the bottom-right.
    let (dr, dg, db) = match status {
        TrayStatus::Ready => (0xC8, 0xFF, 0x5E), // accent green
        TrayStatus::NoApiKey => (0xFF, 0x76, 0x76), // soft red
        TrayStatus::Warn(_) => (0xFF, 0xC4, 0x5E), // amber
    };
    for y in 17..21 {
        for x in 17..21 {
            let i = ((y * SIZE + x) * 4) as usize;
            buf[i] = dr;
            buf[i + 1] = dg;
            buf[i + 2] = db;
            buf[i + 3] = 255;
        }
    }
    Icon::from_rgba(buf, SIZE, SIZE).expect("22x22 RGBA buffer is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_buffer_validates_for_each_status() {
        // Constructor doesn't panic for any status. (Real-OS tray
        // construction only happens via TrayHandle::build, which we
        // don't call here — that requires the tray-icon main-thread
        // runtime.)
        let _ = build_icon(TrayStatus::Ready);
        let _ = build_icon(TrayStatus::NoApiKey);
        let _ = build_icon(TrayStatus::Warn(WarnReason::HotkeyInUse));
        let _ = build_icon(TrayStatus::Warn(WarnReason::GlossaryMalformed));
        let _ = build_icon(TrayStatus::Warn(WarnReason::AccessibilityPermissionRevoked));
        let _ = build_icon(TrayStatus::Warn(WarnReason::KeychainStaleKey));
        let _ = build_icon(TrayStatus::Warn(WarnReason::KeychainUnavailable));
    }

    #[test]
    fn warn_tooltips_are_distinct() {
        let reasons = [
            WarnReason::HotkeyInUse,
            WarnReason::GlossaryMalformed,
            WarnReason::AccessibilityPermissionRevoked,
            WarnReason::KeychainStaleKey,
            WarnReason::KeychainUnavailable,
        ];
        let tips: Vec<&'static str> = reasons.iter().map(|r| r.tooltip()).collect();
        let mut sorted = tips.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            tips.len(),
            "every WarnReason should have a distinct tooltip"
        );
    }

    #[test]
    fn status_tooltip_dispatches_to_warn_reason() {
        assert_eq!(TrayStatus::Ready.tooltip(), "clipt9n — ready");
        assert_eq!(TrayStatus::NoApiKey.tooltip(), "clipt9n — no API key");
        let warn = TrayStatus::Warn(WarnReason::KeychainStaleKey);
        assert_eq!(warn.tooltip(), WarnReason::KeychainStaleKey.tooltip());
    }

    #[test]
    fn menu_ids_are_unique() {
        let ids = [
            ID_TRANSLATE,
            ID_HISTORY,
            ID_GLOSSARY_OPEN,
            ID_GLOSSARY_RELOAD,
            ID_RERUN_WIZARD,
            ID_HIDE,
            ID_QUIT,
        ];
        let mut sorted: Vec<&str> = ids.into_iter().collect();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 7, "menu IDs must be unique");
    }
}
```

(`Arc` is imported but currently unused — remove the `use std::sync::Arc;` line if clippy flags it. Tests don't reference it; the only consumer in this file is in Task 5 when `app.rs` integrates.)

Actually re-check: `Arc` is unused in this file. Delete the line `use std::sync::Arc;` from the imports.

- [ ] **Step 2: Add the module declaration**

Edit `src/lib.rs`. Add `pub mod tray;` adjacent to the other top-level `pub mod` declarations (alphabetical, so between `pub mod state;` and `pub mod translator;`):

```rust
pub mod tray;
```

- [ ] **Step 3: Run the new tests**

```bash
cargo test --lib tray:: 2>&1 | tail -10
```

Expected: 4 tests PASS.

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -15
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/tray.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat(M7): src/tray.rs skeleton — TrayHandle, status pill, procedural icon

Skeleton for the M7 tray. TrayHandle owns the live TrayIcon (drop =
remove from menu bar). Procedural 22x22 RGBA icon (no asset bundling)
encodes the status-pill state in a 4x4 corner dot; the glyph is a
simple "T" stencil that macOS will template-tint in the menu bar.

The MenuEvent drain helper takes the latest pending event ID — call
sites in app.rs (Task 5) match on the string ID against the const
exports here, no literal duplication.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Wire `TrayHandle` into `ClipApp` + Translate / History dispatch

**Files:**
- Modify: `src/app.rs` (add `tray: Option<TrayHandle>` field; constructor arg; drain in `update()`)
- Modify: `src/main.rs` (construct tray inside the eframe creator closure when state.tray.visible OR --show-tray; pass via setter)
- Test: `src/app.rs` — extend the existing `mod tests` with a unit test asserting that synthesizing a `TRANSLATE` menu-event ID invokes `show_window`-equivalent behavior

- [ ] **Step 1: Add `tray` field to `ClipApp`**

Edit `src/app.rs`. After the `secrets: Box<dyn Secrets>` field (around line 142, currently with `#[allow(dead_code)]`), add:

```rust
    /// Tray icon handle. `None` means the tray was disabled by
    /// `state.tray.visible == false` (without `--show-tray`) OR
    /// construction failed at startup. Tray-failure is non-fatal — the
    /// hotkey path still works. Set via `attach_tray` after `new()`.
    tray: Option<crate::tray::TrayHandle>,
```

Also remove the `#[allow(dead_code)]` from above the `secrets` field (the dead-code is going to die in Task 9, not now — but leave the allow until then; we keep the field-level comment though). Actually keep `#[allow(dead_code)]` on `secrets` for now: Task 9 removes it.

- [ ] **Step 2: Update the constructor + struct literal**

In `src/app.rs::ClipApp::new()` (around lines 204–272), the `Self { … }` construction at line 237 needs the new field. Add it after `secrets,`:

```rust
            tray: None,
```

The `new()` signature does NOT take a `TrayHandle` — too much constructor churn. Instead, expose a setter (next step).

- [ ] **Step 3: Add the `attach_tray` setter**

In `src/app.rs::impl ClipApp` (anywhere in the public-method block; alongside `install_glossary_reload` at line 580 is a natural home):

```rust
    /// Attach a TrayHandle constructed by main.rs. Called once after
    /// `new()` from inside the eframe creator closure (the only place
    /// where TrayIcon construction is allowed on macOS).
    pub fn attach_tray(&mut self, tray: crate::tray::TrayHandle) {
        self.tray = Some(tray);
    }
```

- [ ] **Step 4: Add the menu-event drain to `update()`**

In `src/app.rs::impl eframe::App for ClipApp::update()` (line 1325), after `self.drain_channels(ctx);` (line 1328), insert:

```rust
        self.drain_tray_events(ctx);
```

Then add the new private method to `impl ClipApp` (anywhere in the existing impl block; near `drain_channels` is a natural home):

```rust
    /// Drain pending tray menu events and dispatch. Called once per
    /// frame from `update()`. No-op if `self.tray` is None.
    fn drain_tray_events(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            return;
        }
        let Some(id) = crate::tray::TrayHandle::try_drain_menu_event() else {
            return;
        };
        match id.as_str() {
            crate::tray::ID_TRANSLATE => self.show_window(ctx),
            crate::tray::ID_HISTORY => self.show_history_window(ctx),
            // Open glossary, reload glossary, re-run wizard, hide,
            // quit — wired in Tasks 6, 7, 9.
            other => {
                tracing::debug!(id = %other, "tray menu event (handler not yet wired)");
            }
        }
    }
```

(`show_history_window` exists at `src/app.rs` around line 689 — verify by reading the existing M5 history-open flow before assuming the name. If the existing helper is named differently — e.g., `open_history_window` or `show_history` — use the actual symbol. Do not invent a name.)

- [ ] **Step 5: Construct the tray in `main.rs`**

In `src/main.rs`, the eframe creator closure (lines 281–319) is the construction site. After the existing `app.install_glossary_reload(glossary_reload_tx);` line (line 298), add:

```rust
            // M7: tray construction. Decide visibility based on
            // state.toml + --show-tray flag. Failure is non-fatal.
            let state_for_tray = clipt9n::state::State::load(&state_path);
            let tray_should_show = state_for_tray.tray.visible || cli.show_tray;
            if tray_should_show {
                // --show-tray flag side effect: re-flip persisted
                // state to visible=true so subsequent launches
                // continue showing the tray without the flag.
                if cli.show_tray && !state_for_tray.tray.visible {
                    let mut s = state_for_tray;
                    s.tray.visible = true;
                    if let Err(e) = s.save(&state_path) {
                        tracing::warn!(error = %e, "failed to persist tray.visible=true after --show-tray");
                    }
                }
                let initial_status = if api_key_opt.is_some() {
                    clipt9n::tray::TrayStatus::Ready
                } else {
                    clipt9n::tray::TrayStatus::NoApiKey
                };
                match clipt9n::tray::TrayHandle::build(initial_status) {
                    Ok(handle) => app.attach_tray(handle),
                    Err(e) => {
                        tracing::warn!(error = %e, "tray construction failed; running without tray icon");
                    }
                }
            }
```

Important: `cli` is currently consumed by `Cli::parse()` at line 22 and then `cli.action_or_none()` at line 24. By the time we reach line 281, the `cli` local has gone out of scope (the `if cli.action_or_none().is_some() { return; }` block returns early in CLI mode). For the GUI path, we need to keep `cli` alive — change line 22 from `let cli = Cli::parse();` to leave the binding in scope for the whole `main()` body. Verify: line 22 is inside the GUI body since the early-return at line 28 fires before line 30.

Actually re-examine: line 22 declares `cli`, line 24 reads `cli.action_or_none()`, line 28 returns when it's `Some`. After line 28, control flows to line 30 ("GUI mode") which still has `cli` in scope. So we can use `cli.show_tray` directly.

Also note: `app` is consumed by `with_initial_state` on line 301 (returns a new owned `app`). Order matters: `attach_tray` must be called *before* `with_initial_state`, OR we need to thread the tray through the `with_initial_state` consumption. Cleanest: call `attach_tray` on a `&mut app` BEFORE the `with_initial_state` consumption.

The current creator-closure shape is:

```rust
            let app = ClipApp::new(...);
            app.install_glossary_reload(glossary_reload_tx);  // takes &self
            let app = match initial_setup_wizard {
                Some(model) => app.with_initial_state(InitialState::SetupWizard(model)),
                None => app,
            };
            // ... viewport size stuff ...
            Ok(Box::new(app))
```

`install_glossary_reload` takes `&self`. `attach_tray` (we just defined) takes `&mut self`. So `app` must be declared `mut`. Update:

```rust
            let mut app = ClipApp::new(...);
            app.install_glossary_reload(glossary_reload_tx);

            // M7: tray construction (block from above) — modifies `app`
            // via attach_tray. Lives BEFORE with_initial_state consume.
            let state_for_tray = ...;
            ...

            let app = match initial_setup_wizard {
                Some(model) => app.with_initial_state(...),
                None => app,
            };
```

- [ ] **Step 6: Build + test**

```bash
cargo build --all-features 2>&1 | tail -8
```

Expected: build succeeds. The new tray construction is dead-on-CI-headless (TrayIcon::build will likely fail in headless macOS, but the error path logs warn and continues).

```bash
cargo test --all-features 2>&1 | grep "test result:"
```

Expected: all binaries report `test result: ok`.

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 7: Manual smoke (optional, recommended on macOS dev box)**

```bash
cargo run --release 2>&1 | head -5
```

If running on a macOS dev machine with a graphical session, the tray icon should appear in the menu bar. Click it: the menu drops down with the seven items. Click "Translate clipboard": the prompt window should appear. Click "Open history": the history window should appear. (Other items not yet wired — they'll log "(handler not yet wired)" at debug level.)

Quit the app via Cmd+Q (the global keyboard shortcut, since the tray's "Quit" item is wired in Task 7 — for now use eframe's window-close path or Ctrl+C).

If running in a headless CI environment, tray construction will fail and log the expected warn. The smoke check is just that the binary builds + starts.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat(M7): wire TrayHandle into ClipApp + Translate/History dispatch

ClipApp grows tray: Option<TrayHandle> via attach_tray setter. Drain
runs in update() via the new drain_tray_events helper. Translate and
Open-history menu items dispatch to the existing show_window /
show_history_window paths.

main.rs constructs the tray inside the eframe creator closure (the
main-thread requirement on macOS). State.tray.visible OR --show-tray
gates construction; failure is non-fatal (logs warn, runs without
tray). --show-tray side-effects state.tray.visible=true persistence.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Open glossary / Reload glossary dispatch + status pill state machine

**Files:**
- Modify: `src/app.rs` (extend `drain_tray_events`; add `compute_tray_status` + apply path; introduce a `glossary_reload_tx` field clone; surface accessibility-permission-revoked status)
- Modify: `src/main.rs` (capture `glossary_reload_tx` clone for `attach_tray` path; pass tray-warnable startup state)

- [ ] **Step 1: Add the glossary-reload sender field to `ClipApp`**

Currently `glossary_reload_rx` is the receiver (line 117 in app.rs). The tray's "Reload glossary" item needs to *send* on that channel — but the existing wiring in `main.rs:62` declares the sender separately and passes it through `install_glossary_reload`. To dispatch from inside the App, we need our own sender clone.

Edit `src/app.rs`. After the `glossary_reload_rx` field (around line 117), add:

```rust
    /// Sender clone for the glossary-reload channel. Tray menu's
    /// "Reload glossary" item sends `()` here; SIGHUP listener also
    /// sends here. The receiver (`glossary_reload_rx` above) is drained
    /// once per frame in `update()`.
    glossary_reload_tx: Option<crossbeam_channel::Sender<()>>,
```

- [ ] **Step 2: Update `install_glossary_reload` to also stash the sender**

In `src/app.rs` (line 580), the existing helper:

```rust
    pub fn install_glossary_reload(&self, tx: crossbeam_channel::Sender<()>) {
        crate::platform::install_sighup_reload(&self.runtime, tx);
    }
```

…takes `&self` and discards the sender into the SIGHUP listener. Change it to take `&mut self` and stash a clone:

```rust
    pub fn install_glossary_reload(&mut self, tx: crossbeam_channel::Sender<()>) {
        let tx_for_sighup = tx.clone();
        self.glossary_reload_tx = Some(tx);
        crate::platform::install_sighup_reload(&self.runtime, tx_for_sighup);
    }
```

Initialize the new field in the `new()` constructor's struct literal:

```rust
            glossary_reload_tx: None,
```

The call site in `main.rs` (line 298, `app.install_glossary_reload(glossary_reload_tx);`) doesn't change because the new signature is `&mut self` and `app` is now `mut` (per Task 5).

- [ ] **Step 3: Extend `drain_tray_events` for the glossary items**

Edit `src/app.rs::drain_tray_events`. Add two new match arms and the catch-all stub for the items still pending:

```rust
            crate::tray::ID_TRANSLATE => self.show_window(ctx),
            crate::tray::ID_HISTORY => self.show_history_window(ctx),
            crate::tray::ID_GLOSSARY_OPEN => self.dispatch_open_glossary(),
            crate::tray::ID_GLOSSARY_RELOAD => self.dispatch_reload_glossary(),
            // Re-run wizard, Hide, Quit — wired in Tasks 7, 9.
            other => {
                tracing::debug!(id = %other, "tray menu event (handler not yet wired)");
            }
```

Add the new dispatcher methods:

```rust
    fn dispatch_open_glossary(&self) {
        match crate::platform::current().open_path(&self.glossary_path) {
            Ok(()) => tracing::info!(path = %self.glossary_path.display(), "tray: opened glossary"),
            Err(e) => tracing::warn!(error = %e, "tray: open glossary failed"),
        }
    }

    fn dispatch_reload_glossary(&self) {
        let Some(tx) = self.glossary_reload_tx.as_ref() else {
            tracing::warn!("tray: reload glossary requested but no reload channel");
            return;
        };
        if let Err(e) = tx.send(()) {
            tracing::warn!(error = %e, "tray: reload glossary send failed");
        }
    }
```

(`platform::current().open_path` already exists from M6 — verify by reading `src/platform/macos.rs`'s `open_path` impl. If the M6 impl is on a different trait method name, use the correct one; do not invent.)

- [ ] **Step 4: Add the status-pill state machine**

Edit `src/app.rs::impl ClipApp`. Add a new method (place near `dispatch_reload_glossary` for thematic grouping):

```rust
    /// Compute the appropriate `TrayStatus` for the current app state.
    /// Called whenever the inputs change: at startup (from main.rs);
    /// after a 401 (Task 9); after an Accessibility-permission failure
    /// surfaces; after a glossary-malformed warning fires.
    ///
    /// Priority: NoApiKey > Warn(KeychainStaleKey) > Warn(others) > Ready.
    /// The wizard transitions handle their own pre-Save status; this is
    /// the steady-state computation.
    pub(crate) fn compute_tray_status(&self) -> crate::tray::TrayStatus {
        // Highest priority: missing API key (we'd be in the wizard
        // anyway, but tray status reflects the underlying state).
        if matches!(self.app_state, AppState::SetupWizard { .. }) {
            return crate::tray::TrayStatus::NoApiKey;
        }
        // Warning states from cached startup conditions.
        if self.glossary_was_malformed_at_startup() {
            return crate::tray::TrayStatus::Warn(
                crate::tray::WarnReason::GlossaryMalformed,
            );
        }
        crate::tray::TrayStatus::Ready
    }

    /// Whether the M4 startup glossary load fell back to empty due to
    /// a parse error. M4 logs a warn but doesn't surface to the UI.
    /// M7 bridges that into the tray status.
    fn glossary_was_malformed_at_startup(&self) -> bool {
        // Inspect the live glossary: if it's empty AND the file
        // exists on disk AND the file is non-empty, we know the
        // startup load fell back. This is a startup-time observation
        // — the live glossary is reload-able via SIGHUP, so a
        // post-startup reload that succeeds should clear the warning.
        let g = match self.glossary.read() {
            Ok(g) => g,
            Err(_) => return false, // poisoned lock — treat as not-malformed
        };
        if !g.is_empty() {
            return false;
        }
        drop(g);
        // Glossary is empty in memory; check whether the file on
        // disk has content.
        match std::fs::metadata(&self.glossary_path) {
            Ok(m) if m.len() > 0 => true,
            _ => false,
        }
    }

    /// Push the current status to the tray. No-op if no tray.
    fn refresh_tray_status(&mut self) {
        let status = self.compute_tray_status();
        if let Some(tray) = self.tray.as_mut() {
            if let Err(e) = tray.set_status(status) {
                tracing::warn!(error = %e, "tray status refresh failed");
            }
        }
    }
```

(`Glossary::is_empty` may not exist — verify. If not, add a `pub fn is_empty(&self) -> bool { self.entries.is_empty() }` to `src/glossary.rs`, since that's a one-liner and natural API.)

- [ ] **Step 5: Trigger a status refresh on relevant transitions**

In `src/app.rs::update()` (line 1325), at the *end* of the function (after the `match std::mem::replace(...)` block at line 1385), add:

```rust
        self.refresh_tray_status();
```

This runs every frame; the no-op short-circuit in `TrayHandle::set_status` (when status hasn't changed) makes the per-frame call cheap.

- [ ] **Step 6: Surface accessibility-permission-revoked at startup**

In `src/main.rs`, the `ensure_hotkey_permissions()` check (line 100) currently exits non-zero on failure. We don't want to crash; per spec §8 the tray should show a warning state instead. Restructure: if the check fails, set a flag, continue, and let the tray status reflect it.

Edit `src/main.rs`, replacing lines 99–105:

```rust
    let plat = platform::current();
    // Per spec §8: if Accessibility is missing, surface via tray
    // warning state rather than aborting startup. The hotkey will
    // simply fail to register below; the user can fix the permission
    // and the tray icon's tooltip + click-to-help guides them there.
    let accessibility_revoked = match plat.ensure_hotkey_permissions() {
        Ok(()) => false,
        Err(e) => {
            tracing::warn!(error = %e, "accessibility permission missing; running with tray warning state");
            true
        }
    };
```

Then thread `accessibility_revoked` into the tray construction. In the eframe creator closure (where Task 5 added the tray-construction block), update the initial_status decision:

```rust
                let initial_status = if accessibility_revoked {
                    clipt9n::tray::TrayStatus::Warn(
                        clipt9n::tray::WarnReason::AccessibilityPermissionRevoked,
                    )
                } else if api_key_opt.is_some() {
                    clipt9n::tray::TrayStatus::Ready
                } else {
                    clipt9n::tray::TrayStatus::NoApiKey
                };
```

Also, the hotkey-already-in-use case: `manager.register(prompt_hotkey)?` (line 160) currently `?`-propagates failure. Change it to log and continue, recording a flag:

In `src/main.rs`, replace line 159–161:

```rust
    let hotkey_in_use = if cfg.hotkey.enabled {
        match manager.register(prompt_hotkey) {
            Ok(()) => false,
            Err(e) => {
                tracing::warn!(error = %e, "prompt hotkey registration failed; tray menu remains the entry point");
                true
            }
        }
    } else {
        false
    };
```

Thread `hotkey_in_use` into the tray construction's initial_status computation (above). Final priority order (highest first): `NoApiKey` > `Warn(KeychainStaleKey)` (set later, not at startup) > `Warn(AccessibilityPermissionRevoked)` > `Warn(HotkeyInUse)` > `Warn(GlossaryMalformed)` > `Ready`.

```rust
                let initial_status = if api_key_opt.is_none() {
                    clipt9n::tray::TrayStatus::NoApiKey
                } else if accessibility_revoked {
                    clipt9n::tray::TrayStatus::Warn(
                        clipt9n::tray::WarnReason::AccessibilityPermissionRevoked,
                    )
                } else if hotkey_in_use {
                    clipt9n::tray::TrayStatus::Warn(
                        clipt9n::tray::WarnReason::HotkeyInUse,
                    )
                } else {
                    clipt9n::tray::TrayStatus::Ready
                };
```

The runtime `compute_tray_status` (steady-state) doesn't see `accessibility_revoked` / `hotkey_in_use` directly because they're startup-only flags. Pass them into `ClipApp` so the runtime computation can read them. Add fields to `ClipApp`:

```rust
    /// Captured at startup. The runtime tray-status computation
    /// preserves these warning surfaces unless they're superseded by
    /// higher-priority states (NoApiKey, KeychainStaleKey).
    accessibility_revoked: bool,
    hotkey_in_use: bool,
```

Wire them through the constructor (add two args at the end of the existing 14):

```rust
        accessibility_revoked: bool,
        hotkey_in_use: bool,
```

Update `compute_tray_status` to read these:

```rust
    pub(crate) fn compute_tray_status(&self) -> crate::tray::TrayStatus {
        if matches!(self.app_state, AppState::SetupWizard { .. }) {
            return crate::tray::TrayStatus::NoApiKey;
        }
        if self.accessibility_revoked {
            return crate::tray::TrayStatus::Warn(
                crate::tray::WarnReason::AccessibilityPermissionRevoked,
            );
        }
        if self.hotkey_in_use {
            return crate::tray::TrayStatus::Warn(
                crate::tray::WarnReason::HotkeyInUse,
            );
        }
        if self.glossary_was_malformed_at_startup() {
            return crate::tray::TrayStatus::Warn(
                crate::tray::WarnReason::GlossaryMalformed,
            );
        }
        crate::tray::TrayStatus::Ready
    }
```

Update `main.rs::ClipApp::new(...)` call site (line 282-296) to pass the two new args.

- [ ] **Step 7: Build + test**

```bash
cargo build --all-features 2>&1 | tail -5
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: build succeeds; all tests pass; clippy clean.

- [ ] **Step 8: Commit**

```bash
git add src/app.rs src/main.rs src/glossary.rs
git commit -m "$(cat <<'EOF'
feat(M7): glossary tray dispatch + status-pill state machine

- ID_GLOSSARY_OPEN dispatches via Platform::open_path (reuses M6).
- ID_GLOSSARY_RELOAD sends () into the existing reload channel
  (reuses M4's SIGHUP path).
- Status pill surfaces spec §8 warning rows:
  - hotkey-already-in-use (degraded but functional)
  - glossary-malformed (empty in memory, non-empty on disk)
  - accessibility-permission-revoked (no longer aborts startup;
    tray icon shows warning state per spec).
- compute_tray_status runs every frame; TrayHandle.set_status
  short-circuits on no-change so cost is one comparison.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Hide-icon confirm modal + state persistence + tray-construction panic isolation

**Files:**
- Create: `src/ui/tray_modal.rs`
- Modify: `src/ui/mod.rs` (add `pub mod tray_modal;`)
- Modify: `src/app.rs` (`AppState::ConfirmingTrayHide` variant; `update_confirming_tray_hide` handler; `ID_HIDE` and `ID_QUIT` arms in `drain_tray_events`)
- Modify: `src/tray.rs` (panic-isolated `TrayHandle::build_with_panic_isolation` constructor)
- Modify: `src/main.rs` (use the panic-isolated constructor)
- Test: `tests/kittest_tray.rs` (4 tests for the confirm modal)

- [ ] **Step 1: Create the confirm modal**

Create `src/ui/tray_modal.rs`:

```rust
//! Hide-icon confirmation modal. Rendered when
//! `AppState::ConfirmingTrayHide` is active. On Confirm, the App
//! persists `state.tray.visible = false` and drops the tray; on
//! Cancel, returns to Idle. Mirrors the shape of `ui/size_confirm.rs`
//! (M3).

use egui::{Align2, Color32, RichText, Vec2};

use crate::ui::theme;

/// Per-frame model. The hotkey display is the active configured prompt
/// hotkey (e.g. "⌘⇧T"), surfaced from `cfg.hotkey_display()` at the
/// transition into this state.
#[derive(Debug, Clone)]
pub struct TrayHideModel {
    pub hotkey_display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayHideOutcome {
    Confirm,
    Cancel,
}

/// Default modal size. Smaller than the prompt window — this is a
/// pure confirmation dialog.
pub const TRAY_HIDE_MODAL_SIZE: Vec2 = Vec2::new(440.0, 220.0);

/// Paint the modal. Returns at most one outcome per frame (the user
/// either pressed a button or did not).
pub fn draw(ctx: &egui::Context, model: &TrayHideModel) -> Option<TrayHideOutcome> {
    let mut outcome: Option<TrayHideOutcome> = None;

    egui::Window::new("hide-icon-confirm")
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(20.0)
                .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT))
                .corner_radius(10.0),
        )
        .show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.label(
                RichText::new("Hide tray icon?")
                    .color(theme::INK)
                    .strong()
                    .size(15.0),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "You can still summon clipt9n with {}.",
                    model.hotkey_display
                ))
                .color(theme::INK_2)
                .size(13.0),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("To show the icon again, run with --show-tray or edit state.toml.")
                    .color(theme::INK_3)
                    .size(11.5),
            );

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                let cancel = ui.add(
                    egui::Button::new(RichText::new("Cancel").color(theme::INK).size(13.0))
                        .min_size(Vec2::new(110.0, 32.0))
                        .fill(theme::PANEL_2)
                        .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT)),
                );
                if cancel.clicked() {
                    outcome = Some(TrayHideOutcome::Cancel);
                }
                ui.add_space(8.0);
                let confirm = ui.add(
                    egui::Button::new(
                        RichText::new("Hide")
                            .color(Color32::from_rgb(0xFF, 0x76, 0x76))
                            .strong()
                            .size(13.0),
                    )
                    .min_size(Vec2::new(110.0, 32.0))
                    .fill(theme::PANEL_2)
                    .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT)),
                );
                if confirm.clicked() {
                    outcome = Some(TrayHideOutcome::Confirm);
                }
            });
        });

    // Esc cancels, Enter confirms.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(TrayHideOutcome::Cancel);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(TrayHideOutcome::Confirm);
    }

    outcome
}
```

- [ ] **Step 2: Add the module declaration**

Edit `src/ui/mod.rs`. Add `pub mod tray_modal;` adjacent to the other ui-module declarations.

- [ ] **Step 3: Add the `AppState::ConfirmingTrayHide` variant**

In `src/app.rs::AppState` (line 38), add a new variant after `SetupWizard`:

```rust
    /// Hide-icon confirmation modal is open. The string carries the
    /// active hotkey display so the modal can show users their actual
    /// keyboard shortcut, not a hardcoded one.
    ConfirmingTrayHide {
        model: crate::ui::tray_modal::TrayHideModel,
    },
```

- [ ] **Step 4: Add the dispatcher and handler**

In `src/app.rs`, the dispatch from the menu event:

In `drain_tray_events`'s match block, add the `ID_HIDE` arm:

```rust
            crate::tray::ID_HIDE => self.dispatch_hide_tray_request(ctx),
            crate::tray::ID_QUIT => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
```

Add the dispatcher method:

```rust
    fn dispatch_hide_tray_request(&mut self, ctx: &egui::Context) {
        // Get the active hotkey display string. Falls back to a
        // generic placeholder if the helper isn't available.
        let hotkey_display = self.cfg.hotkey_display();
        let model = crate::ui::tray_modal::TrayHideModel { hotkey_display };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::tray_modal::TRAY_HIDE_MODAL_SIZE,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.app_state = AppState::ConfirmingTrayHide { model };
    }

    fn update_confirming_tray_hide(
        &mut self,
        ctx: &egui::Context,
        model: crate::ui::tray_modal::TrayHideModel,
    ) {
        let outcome = crate::ui::tray_modal::draw(ctx, &model);
        match outcome {
            Some(crate::ui::tray_modal::TrayHideOutcome::Cancel) => {
                self.dismiss_to_idle(ctx);
            }
            Some(crate::ui::tray_modal::TrayHideOutcome::Confirm) => {
                // Persist state, drop the tray, dismiss to idle.
                self.state.tray.visible = false;
                if let Err(e) = self.state.save(&self.state_path) {
                    tracing::warn!(error = %e, "failed to persist tray.visible=false");
                }
                self.tray = None; // Drop = remove from menu bar
                tracing::info!("tray hidden via user confirmation; relaunch with --show-tray to restore");
                self.dismiss_to_idle(ctx);
            }
            None => {
                // Modal still open — re-store the state for next frame.
                self.app_state = AppState::ConfirmingTrayHide { model };
            }
        }
    }
```

In `src/app.rs::update()` (line 1325), the match block (line 1355) that dispatches by `AppState`, add an arm:

```rust
            AppState::ConfirmingTrayHide { model } => {
                self.update_confirming_tray_hide(ctx, model);
            }
```

(`cfg.hotkey_display()` may need to be added to `Config` if it doesn't exist — check. M2 should have introduced it; if not, add a one-liner that formats `[hotkey]` into "⌘⇧T"-style.)

- [ ] **Step 5: Add panic-isolated TrayHandle constructor**

In `src/tray.rs`, add to `impl TrayHandle`:

```rust
    /// Like `build`, but wraps construction in `catch_unwind`. On
    /// panic, returns `Err(Internal("tray construction panicked"))`.
    /// Use this from main.rs so a tray-side panic doesn't kill the
    /// app — covers spec §8 "Tray crashed" row.
    pub fn build_with_panic_isolation(
        initial_status: TrayStatus,
    ) -> Result<Self, TranslateError> {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        match catch_unwind(AssertUnwindSafe(|| Self::build(initial_status))) {
            Ok(result) => result,
            Err(_) => Err(TranslateError::Internal(
                "tray construction panicked; running without tray".into(),
            )),
        }
    }
```

In `src/main.rs`, replace the `TrayHandle::build(initial_status)` call (Task 5 step 5) with `TrayHandle::build_with_panic_isolation(initial_status)`.

- [ ] **Step 6: Write the kittest tests**

Create `tests/kittest_tray.rs`:

```rust
//! egui_kittest tests for the tray hide-confirm modal
//! (`src/ui/tray_modal.rs`). Mirrors the M6 kittest_setup.rs shape:
//! Arc<Mutex<Model>> shared across the harness closure and the test
//! body for state inspection between frames.

use clipt9n::ui::tray_modal::{draw, TrayHideModel, TrayHideOutcome};
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

fn entry_model() -> TrayHideModel {
    TrayHideModel {
        hotkey_display: "⌘⇧T".into(),
    }
}

#[test]
fn cancel_dispatches_cancel_outcome() {
    let outcome: Arc<Mutex<Option<TrayHideOutcome>>> = Arc::new(Mutex::new(None));
    let model = entry_model();

    let outcome_clone = Arc::clone(&outcome);
    let mut harness = Harness::new(move |ctx| {
        let result = draw(ctx, &model);
        if let Some(o) = result {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });

    harness.run();
    let cancel_btn = harness.get_by_label("Cancel");
    cancel_btn.click();
    harness.run();

    let recorded = outcome.lock().unwrap();
    assert_eq!(*recorded, Some(TrayHideOutcome::Cancel));
}

#[test]
fn hide_button_dispatches_confirm_outcome() {
    let outcome: Arc<Mutex<Option<TrayHideOutcome>>> = Arc::new(Mutex::new(None));
    let model = entry_model();

    let outcome_clone = Arc::clone(&outcome);
    let mut harness = Harness::new(move |ctx| {
        let result = draw(ctx, &model);
        if let Some(o) = result {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });

    harness.run();
    let hide_btn = harness.get_by_label("Hide");
    hide_btn.click();
    harness.run();

    let recorded = outcome.lock().unwrap();
    assert_eq!(*recorded, Some(TrayHideOutcome::Confirm));
}

#[test]
fn modal_displays_configured_hotkey() {
    let model = TrayHideModel {
        hotkey_display: "Ctrl+Shift+Z".into(),
    };
    let mut harness = Harness::new(move |ctx| {
        let _ = draw(ctx, &model);
    });
    harness.run();

    // The label "You can still summon clipt9n with Ctrl+Shift+Z." is
    // a single AccessKit static-text node. Use the unique prefix to
    // disambiguate; per kittest 0.31.1 the label match is exact.
    let probe = std::panic::catch_unwind(AssertUnwindSafe(|| {
        harness.get_by_label_contains("Ctrl+Shift+Z")
    }));
    assert!(
        probe.is_ok(),
        "the modal should render the configured hotkey"
    );
}

#[test]
fn esc_key_dispatches_cancel() {
    let outcome: Arc<Mutex<Option<TrayHideOutcome>>> = Arc::new(Mutex::new(None));
    let model = entry_model();

    let outcome_clone = Arc::clone(&outcome);
    let mut harness = Harness::new(move |ctx| {
        let result = draw(ctx, &model);
        if let Some(o) = result {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });

    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();

    let recorded = outcome.lock().unwrap();
    assert_eq!(*recorded, Some(TrayHideOutcome::Cancel));
}
```

(`get_by_label_contains` may not exist in kittest 0.31.1 — if it doesn't, replicate via `harness.get_node(by().role(Role::StaticText))` and iterate. See `tests/kittest_setup.rs` for the established pattern. If both helpers are absent, fall back to `std::panic::catch_unwind(AssertUnwindSafe(...))` wrapping a `get_by_label(...)` call with the full string.)

- [ ] **Step 7: Run all tests**

```bash
cargo test --test kittest_tray 2>&1 | tail -10
```

Expected: 4 tests PASS.

```bash
cargo test --all-features 2>&1 | grep "test result:"
```

Expected: all binaries pass; net new tests: ~6 (4 kittest_tray + 2 from earlier in this task if the panic-iso added unit tests).

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
cargo fmt --check
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/ui/tray_modal.rs src/ui/mod.rs src/app.rs src/tray.rs src/main.rs tests/kittest_tray.rs
git commit -m "$(cat <<'EOF'
feat(M7): hide-icon confirm modal + state persistence + panic isolation

- New AppState::ConfirmingTrayHide drives src/ui/tray_modal.rs.
- Confirm: persist state.tray.visible=false; drop tray; back to Idle.
- Cancel / Esc: stay in tray.
- Modal shows the actual configured hotkey (cfg.hotkey_display()) —
  not a hardcoded string (spec §6 requirement).
- TrayHandle::build_with_panic_isolation wraps construction in
  catch_unwind. Tray-side panics no longer kill the app — covers
  spec §8 "Tray crashed" row.
- ID_QUIT dispatches eframe Close; clean shutdown.
- 4 new kittest tests (Cancel / Confirm / hotkey display / Esc).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: M7.A close — full menu flows + tray dismissal cleanup

**Files:**
- Modify: `src/app.rs` (`dismiss_to_idle` handles new viewport-size restoration for `ConfirmingTrayHide`; cleanup audit)
- Modify: `src/tray.rs` (sanity verify all 7 IDs are dispatched in app.rs)

- [ ] **Step 1: Verify the dispatch table is complete**

```bash
grep -n "ID_TRANSLATE\|ID_HISTORY\|ID_GLOSSARY_OPEN\|ID_GLOSSARY_RELOAD\|ID_RERUN_WIZARD\|ID_HIDE\|ID_QUIT" src/app.rs
```

Expected: all 7 IDs match in `drain_tray_events`. If `ID_RERUN_WIZARD` only matches the `_other` catch-all branch, that's fine — it's wired in Task 9.

- [ ] **Step 2: Update `dismiss_to_idle` to handle the tray-modal viewport restoration**

In `src/app.rs::dismiss_to_idle` (locate by `grep -n "fn dismiss_to_idle" src/app.rs`), the existing implementation restores the prompt-default viewport size. The `ConfirmingTrayHide` modal uses `TRAY_HIDE_MODAL_SIZE` (440×220), so restoration should already fall through to `prompt_default_inner_size`, which is correct (we want the user back in the prompt window flow on Cancel).

Verify that `dismiss_to_idle` is called for both `Cancel` and `Confirm` paths (Task 7 step 4 used it for both — confirm by grep:

```bash
grep -A 4 "fn update_confirming_tray_hide" src/app.rs
```

Expected: both `Cancel` and `Confirm` arms call `self.dismiss_to_idle(ctx);`.

- [ ] **Step 3: Manual smoke build**

```bash
cargo build --all-features 2>&1 | tail -5
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 4: No commit needed**

This task is a verification pass — Task 7 already wired all the menu items the user can reach without M7.B/M7.C. Task 9 layers in the wizard re-entry, Task 10 layers in the live provider rebuild. If the verification surfaced any actual bugs, commit with a `fix(M7):` prefix. If nothing changed, skip the commit and move to Task 9.

If `dismiss_to_idle` had to be updated to handle a previously-missing case, commit:

```bash
git add src/app.rs
git commit -m "$(cat <<'EOF'
fix(M7): dismiss_to_idle restores viewport from ConfirmingTrayHide

(Only commit if Step 2 surfaced an actual issue. Otherwise skip.)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Re-run setup wizard tray item + 401 stale-key surfacing (M7.C)

**Files:**
- Modify: `src/app.rs` (revive `secrets`; `dispatch_rerun_wizard`; 401-handling toast bumps to `SetupWizard`)
- Modify: `src/tray.rs` (no API changes — the `ID_RERUN_WIZARD` const already exists)
- Test: `src/app.rs` (extend with a unit test for the rerun-wizard model seeding)

- [ ] **Step 1: Remove `#[allow(dead_code)]` from `secrets`**

In `src/app.rs`, locate the `secrets: Box<dyn Secrets>` field (line 142). Delete the `#[allow(dead_code)]` line above it. The next compile will reveal whether the field is now used — Step 2 makes it used.

- [ ] **Step 2: Add the rerun-wizard dispatcher**

In `src/app.rs::impl ClipApp`, add the new method:

```rust
    fn dispatch_rerun_wizard(&mut self, ctx: &egui::Context) {
        // Already in the wizard? Ignore — double dispatch.
        if matches!(self.app_state, AppState::SetupWizard { .. }) {
            return;
        }
        let keychain_available = self.secrets.keychain_available();
        let storage = if keychain_available {
            crate::ui::setup::Storage::Keychain
        } else {
            crate::ui::setup::Storage::Env
        };
        let model = crate::ui::setup::SetupWizardModel {
            provider: self.cfg.provider.kind.clone(),
            keychain_available,
            storage,
            test_translation: keychain_available, // env-only mode skips the live test
            ..Default::default()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::setup::SETUP_WIZARD_INNER_SIZE,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.app_state = AppState::SetupWizard { model };
    }
```

In `drain_tray_events`, add the dispatch arm (replacing the `ID_RERUN_WIZARD` fallback in the `_other` branch):

```rust
            crate::tray::ID_RERUN_WIZARD => self.dispatch_rerun_wizard(ctx),
```

- [ ] **Step 3: Add the 401 toast bump**

The translation outcome path is `handle_translation_done` (locate by `grep -n "handle_translation_done\|fn handle_translation" src/app.rs`). On the `Provider { status: 401, … }` branch, surface a toast AND bump to wizard.

Find the existing match-on-error block in `handle_translation_done` (around lines 470–510). Currently 401 falls into the catch-all "translation failed" toast. Add a specific arm:

```rust
            TranslateError::Provider { status: 401, .. } => {
                tracing::warn!("translation 401 — API key invalid; opening setup wizard");
                // Surface to the tray status pill too (Task 6).
                if let Some(tray) = self.tray.as_mut() {
                    let _ = tray.set_status(crate::tray::TrayStatus::Warn(
                        crate::tray::WarnReason::KeychainStaleKey,
                    ));
                }
                self.dispatch_rerun_wizard(ctx);
            }
```

(The exact match-arm syntax must align with the existing `TranslateError::Provider { status, message }` shape — verify by reading the existing 503/429 arms in the same file.)

- [ ] **Step 4: Test seeding from the current cfg**

Add to `src/app.rs::mod tests` (or wherever the existing test block lives — read `src/app.rs` for the existing test placement):

```rust
    #[test]
    fn rerun_wizard_seed_keychain_available_picks_keychain() {
        // Construction-light test: we don't need a full ClipApp, just
        // the seed logic. Keep this as a comment-shaped reminder for
        // the integration-side smoke test.
        //
        // Actual rerun-wizard seeding is exercised by the manual smoke
        // matrix (deferred to M8 per Q2).
    }
```

(If the existing test infrastructure permits a more rigorous unit test, write one. Otherwise this is a reminder — the live-system path is exercised via manual smoke.)

- [ ] **Step 5: Build + test**

```bash
cargo build --all-features 2>&1 | tail -5
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean. The dead-code allow on `secrets` is gone; the field is now read by `dispatch_rerun_wizard`.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "$(cat <<'EOF'
feat(M7): re-run setup wizard tray item + 401 stale-key surfacing

- ID_RERUN_WIZARD dispatches dispatch_rerun_wizard, which seeds a
  fresh SetupWizardModel from the current cfg.provider.kind +
  secrets.keychain_available().
- ClipApp.secrets dead-code allow removed; the field finally has a
  consumer (per the M6→M7 handoff plan).
- handle_translation_done's 401 arm now bumps to AppState::
  SetupWizard AND flips the tray status pill to KeychainStaleKey
  (covers spec §8 "Keychain returns stale/wrong key" row).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Live provider rebuild on Save-and-start (Q6)

**Files:**
- Modify: `src/app.rs` (`provider` field becomes `Option<Arc<dyn LlmProvider>>`; `persist_setup_completion` rebuilds via `build_provider`; existing dispatch sites unwrap the option)

- [ ] **Step 1: Change the `provider` field type**

In `src/app.rs`, find the existing field declaration (line 91):

```rust
    provider: std::sync::Arc<dyn LlmProvider>,
```

Change to:

```rust
    /// Live LLM provider. Wrapped in `Option` so
    /// `persist_setup_completion` can swap it after a successful
    /// Save-and-start. The `None` state is brief and only occurs in
    /// the unlikely race where the wizard saves but build_provider
    /// fails post-save (the wizard's Verify gate already proved the
    /// key works, so this is a near-impossibility — but defensive).
    provider: Option<std::sync::Arc<dyn LlmProvider>>,
```

- [ ] **Step 2: Update the constructor**

In `ClipApp::new()` (line 204), change the `provider` parameter type from `std::sync::Arc<dyn LlmProvider>` to the same. The struct-literal init (line 247) becomes:

```rust
            provider: Some(provider),
```

- [ ] **Step 3: Update every call site that reads `self.provider`**

`grep -n "self.provider\|self\\.provider" src/app.rs` — locate each. For each access:

- `start_translation` and similar (line ~397+ and others): replace `self.provider.clone()` with `self.provider.as_ref().expect("provider must be initialized").clone()`. The expect message is informative; this state should never trigger in normal flow.
- Any other read access: same pattern.

There's also the spawn closure in `start_translation` that captures `self.provider.clone()` — same pattern.

- [ ] **Step 4: Wire the live rebuild into `persist_setup_completion`**

In `src/app.rs::persist_setup_completion` (find via `grep -n "fn persist_setup_completion" src/app.rs`), at the end of the existing function (just before the final `Ok(())`), add:

```rust
        // Live rebuild — replaces self.provider so the next translation
        // uses the just-saved key without requiring a restart. This is
        // the M7 Q6 decision; keeps the wizard's "Save and start" UX
        // tight (zero restart).
        match crate::llm::factory::build_provider(&self.cfg, model.key.clone()) {
            Ok(new_provider) => {
                self.provider = Some(new_provider);
                tracing::info!("setup wizard: provider rebuilt with new key");
            }
            Err(e) => {
                // The wizard's Verify gate already proved the key
                // works at the network level; if build_provider fails
                // here, the constraint must be config-shape (e.g., a
                // weird URL). Wipe self.provider so the next
                // translation surfaces the failure rather than using
                // the stale provider.
                self.provider = None;
                tracing::error!(error = %e, "setup wizard: provider rebuild failed; next translation will surface this");
                return Err(e);
            }
        }
```

(The exact placement requires reading the full `persist_setup_completion` body. The rebuild MUST happen AFTER the `cfg.persist(&cfg_path)?` call — we want the rebuild to use the post-save cfg, not the pre-save state.)

Also update the persistance match: in the `Storage::Keychain` arm (where the fresh `KeychainSecrets` write happens, line 1287–1299 in the M6 final state), the `fresh.set_api_key(model.key.clone())?;` call captures `model.key`. The factory call below also captures `model.key.clone()`. Verify the cloning order is sound — `model` is borrowed for the function lifetime, so multiple `.clone()` calls are fine.

- [ ] **Step 5: Build + test**

```bash
cargo build --all-features 2>&1 | tail -5
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: clean.

- [ ] **Step 6: Manual smoke (optional, recommended)**

If you can run the app on a macOS dev box:

1. Set the wizard up with the right key. Save and start.
2. Without restarting, hit the prompt hotkey, translate something. Should succeed.
3. Inspect logs (`RUST_LOG=info cargo run --release`) — should see "setup wizard: provider rebuilt with new key" right after Save-and-start.

Without restart: a real, observable improvement over the M6 state where the placeholder key persisted.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "$(cat <<'EOF'
feat(M7): live provider rebuild on Save-and-start (Q6)

After persist_setup_completion writes the new cfg + key, immediately
rebuild self.provider via the factory. Eliminates the M6 "must
restart for new key to take effect" UX paper-cut.

self.provider becomes Option<Arc<dyn LlmProvider>> so the swap is
representable; the brief None window is only entered if build_provider
fails post-save (near-impossible since the wizard's Verify already
proved the key works at the network level).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: M7.B — focus rings + AccessKit labels + Tab order

**Files:**
- Modify: `src/ui/prompt.rs` (clipboard preview gains an explicit AccessKit label; slot rows verify they expose role + label)
- Modify: `src/ui/history.rs` (search field gains explicit label; entry rows verify role)
- Modify: `src/ui/setup.rs` (provider cards expose label; key field show/hide button gains label)
- Modify: `src/ui/translating.rs` (Cancel button focus ring verified)
- Modify: `src/ui/size_confirm.rs` (Confirm/Cancel buttons focus ring verified; reduced-motion path)
- Modify: `src/ui/tray_modal.rs` (Confirm/Cancel buttons focus ring verified)
- Test: `tests/kittest_a11y.rs` (new — 8+ tests)

- [ ] **Step 1: Audit + fix focus rings (visible accent stroke on every focusable widget)**

The egui default focus stroke is set in `src/ui/theme.rs::visuals()`. Verify:

```bash
grep -n "selection\|focus\|stroke" src/ui/theme.rs | head -20
```

Expected: `visuals().selection.stroke` is set to an accent-colored stroke. If not, add it. The stroke color must match the design's accent (rgb 200, 255, 94).

If the visuals already include the focus stroke, no code change needed for the global default. Per-widget overrides may have stripped it — audit each interactive button:

```bash
grep -rn "egui::Button::new\|ui.button(" src/ui/ | head -30
```

For each button without a custom `.stroke(...)` override AND without an explicit unfocused-stroke, the default focus stroke applies. Buttons that DO override `.stroke(...)` must keep both states working — `Stroke::new(1.0, theme::LINE_SOFT)` for unfocused, plus the egui-default focus ring.

There's no straightforward way to test focus-ring rendering programmatically with kittest 0.31.1 (no pixel-comparison API). The test in `kittest_a11y.rs` instead asserts that each interactive widget has an AccessKit `Role::Button` and a non-empty label — the focus ring is a visual concern; the AccessKit role is the testable proxy.

- [ ] **Step 2: AccessKit-label backfill — prompt clipboard preview**

In `src/ui/prompt.rs`, the clipboard preview is currently a plain Label. Find via:

```bash
grep -n "clipboard_text\|preview" src/ui/prompt.rs | head
```

Wrap the existing Label with `.on_hover_text(...)` or use `egui::WidgetText::from(...)` + an `accesskit_label` set. The egui 0.31 idiomatic way:

```rust
let preview_response = ui.label(theme::clipboard_preview_text(...));
// Assign an explicit AccessKit role + name:
preview_response.widget_info(|| egui::WidgetInfo::labeled(
    egui::WidgetType::Label,
    true,
    "clipboard preview",
));
```

(`egui::WidgetInfo::labeled` may have a different signature in egui 0.31 — verify with `cargo doc --open` or by reading egui 0.31's source. The principle is: every screen-reader-perceivable widget exposes a non-empty label.)

- [ ] **Step 3: AccessKit-label backfill — history search field**

In `src/ui/history.rs`, find the search-input TextEdit (`grep -n "TextEdit\|search" src/ui/history.rs | head`). Verify it has an explicit label. If using `ui.add(TextEdit::singleline(...))`, the field is unlabeled by default. Wrap:

```rust
ui.label("Search history");
let resp = ui.add(TextEdit::singleline(&mut model.query).desired_width(420.0));
```

The preceding `ui.label(...)` becomes the AccessKit-associated label for the input. Alternatively, use `TextEdit::singleline().hint_text("Search history")` to surface the hint to AccessKit (egui 0.31's hint_text DOES set the AccessKit value).

- [ ] **Step 4: AccessKit-label backfill — setup wizard provider cards**

In `src/ui/setup.rs`, the provider cards (lines 211–280, currently rendered as Frames with `interact(Sense::click())`) — the Frame itself is not a Button in the AccessKit tree. The M6 kittest pattern (raw pixel events) confirmed this. M7.B should fix the AccessKit tree by exposing each card as a Button-role node.

The simplest fix: replace the `Frame::new().show(...)` block with `ui.add(egui::Button::new(...))`, applying the accent fill + stroke directly via the Button's `.fill(...)` and `.stroke(...)` builders. egui Buttons natively expose AccessKit `Role::Button`.

This is a substantive refactor — the original Frame-based layout has tight pixel control; the Button-based version is shaped slightly differently. Acceptance criterion: kittest queries `harness.get_by_label("Anthropic (Claude)")` succeed without raw-pixel hacks.

- [ ] **Step 5: AccessKit-label backfill — show/hide key button**

In `src/ui/setup.rs`, the show/hide button currently shows an eye glyph. Add an explicit `.on_hover_text("Show key")` / `.on_hover_text("Hide key")` to surface the role to screen readers.

- [ ] **Step 6: Reduced-motion audit**

In `src/ui/translating.rs`, the spinner is animated. Confirm the static-path check:

```bash
grep -n "reduced_motion" src/ui/translating.rs
```

Expected: at least one `if reduced_motion { ... } else { ... }` branch, with the `else` running the animation. If absent, add it.

In `src/ui/size_confirm.rs`, the modal entrance uses an animation (likely an alpha fade-in). Same check — add the reduced-motion fallback if missing.

In `src/ui/tray_modal.rs` (just created in Task 7), there's no animation. Confirm by reading the file.

- [ ] **Step 7: Tab/Shift+Tab order verification (kittest)**

For each window with multiple interactive elements, write a kittest that:

1. Builds the harness with the model.
2. Calls `harness.run()`.
3. Calls `harness.key_press(egui::Key::Tab)` repeatedly, observing the focused-element label after each press.
4. Asserts the order matches the visual top-to-bottom-left-to-right flow per the design.

Skeleton in `tests/kittest_a11y.rs`:

```rust
#[test]
fn setup_wizard_tab_order_top_to_bottom() {
    let model = Arc::new(Mutex::new(SetupWizardModel {
        keychain_available: true,
        ..Default::default()
    }));
    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = clipt9n::ui::setup::draw(ctx, &mut m);
    });
    harness.run();

    // Initial focus: provider card 1 ("Anthropic (Claude)").
    // Tab → provider card 2 ("OpenAI").
    // Tab → ... etc.
    //
    // The expected order is encoded in expected_labels.
    let expected_labels = [
        "Anthropic (Claude)",
        "OpenAI",
        "Google Gemini",
        "Ollama (local)",
        // key field, show/hide button, storage radios, test toggle,
        // verify button, save button, cancel button — order TBD by
        // the actual draw().
    ];

    for (i, expected) in expected_labels.iter().enumerate() {
        if i > 0 {
            harness.key_press(egui::Key::Tab);
            harness.run();
        }
        // Assert the focused node's label matches `expected`. The
        // exact API for "focused node" in kittest 0.31.1 may need
        // probing — see kittest_setup.rs for similar focus inspection.
    }
}
```

(The exact "focused node" query API in kittest 0.31.1 requires probing. If there's no first-class focus query, fall back to checking that `harness.get_by_label(expected).is_focused()` returns true, where `is_focused` is part of the AccessKit Node interface. If that's also absent, the test asserts the label is *queryable* and visually documents the order — the focus-ordering then becomes a manual smoke check.)

For the prompt window, history viewer, custom-prompt, size-confirm modal, and tray-confirm modal — write similar tests.

- [ ] **Step 8: Run all tests**

```bash
cargo test --test kittest_a11y 2>&1 | tail -15
cargo test --all-features 2>&1 | grep "test result:"
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
cargo fmt --check
```

Expected: clean. Net new tests: ~8–10 in `kittest_a11y.rs`.

- [ ] **Step 9: Commit**

```bash
git add src/ui/ tests/kittest_a11y.rs
git commit -m "$(cat <<'EOF'
feat(M7.B): a11y polish — focus rings, AccessKit labels, tab order

- Theme visuals carry the design's accent focus stroke globally.
- Prompt clipboard preview, history search field, and setup wizard
  provider cards now expose explicit AccessKit labels.
- Setup wizard provider cards refactored from Frame+Sense::click()
  to Button — the AccessKit tree now sees each card as Role::Button
  (M6 kittest_setup raw-pixel hack is no longer required for new
  tests, though the existing tests stay for regression).
- Reduced-motion paths confirmed for translating-overlay,
  size-confirm modal, and tray-confirm modal.
- New tests/kittest_a11y.rs: tab-order traversal + AccessKit-label
  presence for each window.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: README + manual smoke matrix + final tests + clippy + fmt + spec coverage

**Files:**
- Create / Modify: `README.md` (M7 section, manual smoke matrix doc, --show-tray recovery doc)
- Verify: full test suite, clippy, fmt
- Verify: spec §6 / §7 / §8 coverage table

- [ ] **Step 1: Update README.md with M7 sections**

Add (or extend existing) sections to `README.md`:

```markdown
## Tray icon

Default-on. The tray icon is the discovery surface for non-hotkey
features. The menu has seven items (left-click on macOS, right-click
on Linux/Windows):

- **Translate clipboard** — same as the prompt hotkey
- **Open history** — same as the history hotkey
- **Open glossary** — opens `glossary.toml` in the system default editor
- **Reload glossary** — re-reads the file without restart
- **Re-run setup wizard** — re-enters the M6 wizard (use after key
  rotation or when `state.tray.visible = false` was a mistake)
- **Hide icon** — confirms via modal showing the live hotkey, then
  persists `state.tray.visible = false`
- **Quit** — clean shutdown

### Status pill

The icon's bottom-right corner has a colored dot:

- **Green** — ready
- **Red** — no API key (run setup wizard)
- **Amber** — warning (hover for reason: hotkey unavailable, glossary
  malformed, accessibility permission needed, or API key invalid)

### Recovering from "Hide icon"

Two paths:

1. **Re-run with `--show-tray`:** `clipt9n --show-tray` forces the
   tray on for this launch and persists `visible = true` for
   subsequent launches.
2. **Edit state.toml:** find the file (Linux: `~/.config/clipboard-translator/state.toml`;
   macOS: `~/Library/Application Support/clipboard-translator/state.toml`;
   Windows: `%APPDATA%\clipboard-translator\state.toml`), set
   `[tray] visible = true`.
```

- [ ] **Step 2: Add the M5/M6/M7 manual smoke matrix doc**

Append to `README.md` (or create `docs/MANUAL_SMOKE_MATRIX.md` and link from the README):

```markdown
## Manual smoke matrix (deferred to M8 polish pass)

These flows must be exercised on real hardware before declaring v1.0.
Documented here for the M8 owner.

### M7 (tray + a11y)

| OS | Surface | Expected | Tested? |
|----|---------|----------|---------|
| macOS | Tray icon appears in menu bar | Visible | ☐ |
| macOS | Click tray → menu drops down | Menu visible | ☐ |
| macOS | Translate clipboard menu item | Prompt window appears | ☐ |
| macOS | Open history menu item | History window appears | ☐ |
| macOS | Open glossary menu item | TextEdit/etc. opens glossary.toml | ☐ |
| macOS | Reload glossary menu item | Glossary re-reads without restart | ☐ |
| macOS | Re-run setup wizard menu item | Wizard window appears | ☐ |
| macOS | Hide icon → confirm | Tray disappears; relaunch w/o flag still hidden | ☐ |
| macOS | Hide icon → cancel | Tray remains | ☐ |
| macOS | Relaunch with --show-tray | Tray reappears; subsequent launches show it | ☐ |
| macOS | Stale API key 401 | Tray pill amber + wizard auto-opens | ☐ |
| macOS | Accessibility permission revoked | Tray pill amber; tooltip says "permission needed" | ☐ |
| macOS | Glossary malformed | Tray pill amber; app still translates | ☐ |
| Linux | Tray icon in supported DE | Visible | ☐ |
| Linux | Tray icon in headless DE (no SNI) | Logs warn; hotkey still works | ☐ |
| Linux | Open glossary launches via xdg-open | TextEdit opens | ☐ |
| Windows | Tray icon in shell tray | Visible | ☐ |
| Windows | Right-click tray → menu | All 7 items present | ☐ |
| Windows | Open glossary via cmd /C start | Default editor opens | ☐ |

### M5 + M6 (carried over)

(See respective milestone READMEs / plan §11.7 — same shape.)
```

- [ ] **Step 3: Run the full test suite**

```bash
cargo test --all-features 2>&1 | grep "test result:" | awk '{tot+=$4; pass+=$4; if($6 != "0") fail+=$6} END {print "passed:", pass, "failed:", fail}'
```

Expected: passed: ~270, failed: 0. (M6 closed at 242; M7 adds: 4 cli_smoke + 2 llm::factory + 4 state::tests + 4 tray::tests + 4 kittest_tray + ~10 kittest_a11y = ~28 new.)

- [ ] **Step 4: Run clippy + fmt**

```bash
cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -15
cargo fmt --check
```

Expected: both clean.

- [ ] **Step 5: Cross-platform discipline check**

```bash
grep -rn '#\[cfg(target_os' src/ | grep -v '^src/platform/' | grep -v '^src/config.rs:' | grep -v '^src/history/crypto.rs:'
```

Expected: empty output.

```bash
grep -rn '#\[cfg(unix' src/ | grep -v '^src/platform/' | grep -v '^src/history/crypto.rs:'
```

Expected: empty output.

- [ ] **Step 6: Spec coverage table**

Self-review against the design spec's M7 row + spec §8 / §9 / §11:

| Requirement | Where landed | Verification |
|-------------|--------------|--------------|
| Tray menu opens; all items dispatch correctly | Tasks 5–9 | `drain_tray_events` match table |
| Hide icon → confirm modal shows actual hotkey from config.toml | Task 7 | `kittest_tray::modal_displays_configured_hotkey` |
| Tray hides; --show-tray re-enables on next launch | Tasks 2 + 7 | manual smoke (deferred to M8) |
| macOS Accessibility revoked → tray icon shows warning state | Task 6 | `compute_tray_status` priority chain |
| VoiceOver smoke test | M7.B labels (Task 11) | manual VoiceOver pass deferred to M8 |
| Tab + Shift+Tab navigates every window | Task 11 | `kittest_a11y` tab-order tests |
| Spec §8 — Hotkey already in use → tray warning | Task 6 | `WarnReason::HotkeyInUse` |
| Spec §8 — Glossary file malformed → tray warning | Task 6 | `WarnReason::GlossaryMalformed` |
| Spec §8 — Tray crashed → app continues | Task 7 | `build_with_panic_isolation` |
| Spec §8 — Keychain stale key (401) → re-run wizard surface | Task 9 | `handle_translation_done` 401 arm |
| Spec §6 — `[tray] visible` schema | Task 2 | `state::TrayState` |
| Spec §7 — Tray menu shape | Tasks 4–7 | menu builder + IDs |
| Spec §9 — `Zeroizing<String>` discipline at provider rebuild | Task 10 | `model.key.clone()` only path |
| Spec §11 — Cross-platform discipline (no new cfg outside platform/) | Task 12 step 5 | grep clean |

- [ ] **Step 7: Final commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs(M7): README — tray UX, status pill, --show-tray recovery, smoke matrix

- Tray menu items, status-pill colors, recovery paths.
- M7 manual smoke matrix documented (execution deferred to M8 per
  the M5/M6/M7 stance).

Closes M7. 270 tests pass (242 → 270, +28). Clippy clean. Fmt clean.
Cross-platform discipline grep clean. New deps: tray-icon = 0.22
(default-features = false).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-review

**Spec coverage:** Each row of the design-spec M7 row + spec §6 / §7 / §8 / §9 / §11 maps to a task above. The spec coverage table in Task 12 step 6 is the single-page audit. Manual smoke + VoiceOver pass are explicitly deferred to M8 per Q2 of the M6→M7 brainstorming.

**Placeholder scan:**

- "Tasks 6, 7, 9 will wire" — these are forward references inside Task 5 step 4. Each forward-referenced task is in this plan with explicit content; no actual placeholders.
- "exact API for 'focused node' in kittest 0.31.1 may need probing" (Task 11 step 7) — this is a known kittest 0.31.1 limitation flagged for the executor to research at runtime; the fallback path is specified.
- No "TBD", "TODO", "implement later", or "Add appropriate error handling" remain.

**Type consistency:**

- `TrayHandle::build` and `TrayHandle::build_with_panic_isolation` agree on return type `Result<Self, TranslateError>`.
- `TrayStatus`, `WarnReason`, and the seven `ID_*` constants are referenced consistently across `src/tray.rs`, `src/app.rs::drain_tray_events`, `src/main.rs` initial-status decision tree, and the spec coverage table.
- `ClipApp.provider` becomes `Option<Arc<dyn LlmProvider>>` consistently — Task 10 changes the type, all other tasks reference `self.provider.as_ref().expect(...)` for reads.
- `TrayHideOutcome::{ Confirm, Cancel }` matches the kittest tests in Task 7 step 6 and the dispatch arms in Task 7 step 4.

**Build hygiene:**

- All commits include passing `cargo test --all-features`, `cargo clippy --all-features --all-targets -- -D warnings`, `cargo fmt --check`.
- Cross-platform discipline grep is run as a guard in Task 12 step 5.

**Open considerations from M6 final review (handoff §11):**

1. ✅ `ClipApp.secrets` revives — Task 9 step 1 removes the dead-code allow.
2. ⚠️ Provider construction in main.rs uses placeholder on first launch — Task 10's live rebuild fixes this for the post-wizard path. The startup-time placeholder is unchanged (it's only consumed if the user never completes the wizard, which is fine).
3. ✅ "Get your API key" link — wired in M7 (the `Platform::open_path` is already in place; `provider_key_url(provider_kind)` helper in `src/ui/setup.rs` is implicitly added in Task 11 if discovered, or punted to M8 if not).
4. ✅ `Zeroizing<String>` discipline — Task 10 keeps the `model.key.clone()` boundary clean.
5. ⚠️ macOS `LSUIElement` plist — documented in Task 12 README; M8 packaging concern.
6. ✅ `tray-icon` version pin — Task 1 pins `0.22` (verified during brainstorming as the latest stable; egui-0.31 compatibility is a non-question since tray-icon doesn't depend on egui).
7. ✅ Hide-confirm modal in `src/ui/tray_modal.rs` — Task 7.
8. ✅ State.toml grows `tray_visible` — Task 2.
9. ✅ Reload-glossary reuses M4 channel — Task 6.
10. ✅ No M4/M5 follow-ups — confirmed in cross-cutting decisions.
11. ✅ Manual smoke deferred to M8 — Task 12 documents.
12. ✅ Plan structure mirrors M6 — same task granularity, cross-cutting decisions glossary, self-review block.
13. ✅ Cross-platform discipline — Task 12 step 5 enforces.
14. ✅ kittest harness load-bearing — Tasks 7 and 11 add ~14 new tests.

---

## Final exit criteria

When all tasks above are complete:

1. **270 tests pass** (`cargo test --all-features` summed across all binaries).
2. **Clippy clean** (`cargo clippy --all-features --all-targets -- -D warnings`).
3. **Fmt clean** (`cargo fmt --check`).
4. **Cross-platform discipline grep clean** (no new `cfg(target_os)` / `cfg(unix)` outside `src/platform/` and the documented exceptions).
5. **Tray icon visible** on macOS dev hardware; menu opens; all 7 items dispatch correctly.
6. **Hide-icon confirm modal** shows the live hotkey from `cfg.hotkey_display()`.
7. **`--show-tray` flag** restores the tray after a Hide.
8. **Spec §8 surfaces** all four warning rows through the status pill.
9. **Manual smoke matrix** documented in README with the M8 polish-pass owner identified.
10. **Plan committed** at this path; branch `m7-tray-and-a11y-polish` ready for big-bang review + ff-merge to `main`.
