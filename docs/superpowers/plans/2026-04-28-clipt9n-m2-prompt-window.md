# clipt9n M2 — Prompt Window + Global Hotkey + Design Tokens — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the app feel real for the first time. `Cmd+Shift+T` summons a centered, always-on-top prompt window; pressing 1/2/3 translates the system clipboard end-to-end and shows an OS notification. App keeps running between invocations.

**Architecture:** Add `eframe` (egui-on-winit) with the AccessKit feature for screen-reader support. Background thread polls `global-hotkey` events and forwards them to the egui app via a `crossbeam_channel`. The app holds a long-lived `tokio` runtime (the existing async LLM client lives on it) and dispatches translations as `runtime.spawn()` jobs whose results return through another channel. All OS-specific code (Accessibility-permission detection now; reduced-motion query later) lives in `src/platform/`. A11y-corrected design palette and reusable `WindowFrame` chrome live in `src/ui/theme.rs`. State persistence (`state.toml`) for last-action recall is a thin TOML read/write in `src/state.rs`.

**Tech Stack:** Rust 2021 / eframe 0.31 / egui 0.31 / accesskit (via eframe feature) / global-hotkey 0.7 / notify-rust 4 / arboard (already pinned) / tokio 1 (already pinned) / crossbeam-channel 0.5 (new) / objc2 + objc2-foundation + objc2-application-services 0.3 (macOS Accessibility detection only — guarded by `#[cfg(target_os = "macos")]` inside `src/platform/macos.rs`).

> **Branch:** This plan executes on `m2-prompt-window`, branched from `main` after M1 was fast-forwarded onto it. Working directory: `/Users/egecan/Code/clipt9n`.

---

## File structure

After M2, the tree gains the following (relative to repo root):

```
Cargo.toml                  ← MODIFIED: add M2 deps; declare windowed bin metadata
src/
├── main.rs                 ← REWRITTEN: eframe entry + hotkey registration
├── lib.rs                  ← MODIFIED: re-exports for new modules; loosen Cli ArgGroup
├── config.rs               ← MODIFIED: add HotkeyConfig + modifier-mapping helper; add UiConfig
├── state.rs                ← NEW: state.toml model + load/save (last-action recall)
├── notify.rs               ← NEW: notify-rust wrapper, "Translation copied" toast
├── ui/
│   ├── mod.rs              ← NEW: pub mod theme, prompt
│   ├── theme.rs            ← NEW: palette, Visuals builder, WindowFrame, kbd widget
│   └── prompt.rs           ← NEW: prompt-window draw + state machine
├── app.rs                  ← NEW: eframe App impl owning runtime, channels, prompt state
├── platform/
│   ├── mod.rs              ← NEW: Platform trait + cfg-gated re-export
│   ├── macos.rs            ← NEW: Accessibility-permission check; open Settings
│   ├── linux.rs            ← NEW: stub (Platform impl with no-op)
│   └── windows.rs          ← NEW: stub
README.md                   ← MODIFIED: add M2 GUI section + Accessibility-permission note
```

Boundary discipline:
- `src/platform/` is the **only** place `#[cfg(target_os = …)]` and `#[cfg(unix)]` may appear (enforced by M8 grep-lint).
- `src/ui/` knows nothing about `tokio`, `reqwest`, or platform specifics — it only paints frames and emits events.
- `src/app.rs` is the seam between egui (sync) and tokio (async); nothing else owns the runtime.

---

## Glossary of cross-cutting decisions (read once)

These come up repeatedly; agreeing on them up front prevents drift.

1. **Hotkey events ride a `crossbeam_channel::unbounded`.** A dedicated OS thread spawned in `main.rs` reads `GlobalHotKeyEvent::receiver()` (a blocking `Receiver`) and forwards events into our channel. The egui app polls `try_recv()` on each `update()`. Why crossbeam: `global-hotkey` returns its events via `crossbeam_channel::Receiver` already, and `crossbeam_channel::Select` can wake on multiple channels later. Don't use `tokio::sync::mpsc` — the hotkey thread is sync.
2. **Translation results ride `std::sync::mpsc`.** The tokio task uses `tx.send(Result<String, TranslateError>)` and the egui app polls `try_recv()`. We don't need crossbeam here. Don't use `tokio::sync::mpsc` — the receiver is sync.
3. **Tokio runtime is owned by `App`** (`tokio::runtime::Runtime`, multi-thread, default). Constructed in `App::new()`, dropped on app exit.
4. **Window visibility is driven by viewport commands**, not by closing/reopening windows. Initial state is hidden (`ViewportBuilder::with_visible(false)`). Hotkey → `Visible(true)` + `Focus`. Esc / completion → `Visible(false)`. App stays running.
5. **Daemon mode is in-process for M2**, not a launchd agent. Closing the last window does NOT exit the app — `eframe::run_native` returns only when we explicitly call `ViewportCommand::Close` on `ViewportId::ROOT`.
6. **CLI mode still works.** `clipt9n --translate-to=de` continues to exit after one translation. The action ArgGroup goes from `required = true` to `required = false`; absence of any action flag means "launch GUI." See Task 2.
7. **Slots 4/5/6 are visible but no-op in M2** (per design row `prompt-window.jsx` + design doc M2 deliverables). Their button rows render with the right number, label, and tag, and pressing 4/5/6 is a no-op (no error, no toast). M3 wires them.
8. **AccessKit is enabled via eframe feature `accesskit`.** No code changes needed beyond the feature flag — egui auto-emits accessibility nodes for `Button`, `Label`, etc. We add `egui::Response::on_hover_text` / explicit role hints only where the auto-output is wrong.
9. **The hotkey is `Cmd+Shift+T` on macOS, `Ctrl+Shift+T` elsewhere.** Single source of truth: `Config::hotkey` + `config::resolve_modifier()`. The footer keymap and any UI string showing the hotkey reads through `config::hotkey_display()` (added in Task 4) — never hard-codes `Cmd`.

---

## Pre-flight: Confirm starting state

- [ ] **Step 0.1: Verify branch and clean tree**

Run:
```bash
git rev-parse --abbrev-ref HEAD
git status --short
```
Expected: branch `m2-prompt-window`, no working-tree changes.

- [ ] **Step 0.2: Verify M1 tests pass on this branch**

Run: `cargo test --all-features 2>&1 | tail -5`
Expected: `test result: ok. 58 passed`.

If either step fails, stop and report.

---

## Task 1: Add M2 dependencies

**Files:**
- Modify: `Cargo.toml`

**Why:** The walking skeleton's deps (reqwest, arboard, tokio) cover the I/O layer. M2 layers a windowing toolkit, hotkey registration, OS notifications, and a thread-safe channel onto that base. We pin all new deps to specific minor versions per the project's pinning convention.

- [ ] **Step 1.1: Add the new dependencies**

Edit `Cargo.toml`. After the existing `async-trait = "0.1"` line and before `[dev-dependencies]`, append:

```toml
eframe = { version = "0.31", default-features = false, features = ["default_fonts", "wgpu", "accesskit"] }
egui = "0.31"
global-hotkey = "0.7"
notify-rust = "4"
crossbeam-channel = "0.5"

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-foundation = "0.3"
objc2-application-services = { version = "0.3", features = ["HIServices"] }
```

Rationale on feature flags:
- `eframe` `default-features = false`: drop `glow` (the GL backend) and use only `wgpu` for crisp rendering and consistent macOS behavior.
- `accesskit`: enables AccessKit emission for screen readers (M2 exit criterion 8).
- `default_fonts`: keeps egui's bundled Hack/Ubuntu fonts. We accept these as a documented visual deviation from the design's Inter/JetBrains Mono — bundling design fonts is deferred to M8 polish per the M2 row's "minor egui-specific deviations documented in M2 plan."

The `objc2-*` block under `[target.'cfg(target_os = "macos")'.dependencies]` keeps Linux/Windows builds free of Apple-only crates.

- [ ] **Step 1.2: Verify dependencies resolve**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` (a fresh build of `eframe` is heavy — first run may take 60–180s; that's fine).

If a dep version no longer resolves (e.g., yanked), substitute the latest 0.x in the same minor series and update this plan's deps block in a follow-up commit. **Do not** silently bump major versions — `eframe 0.31` and `egui 0.31` must match.

- [ ] **Step 1.3: Verify M1 tests still pass**

Run: `cargo test --all-features 2>&1 | tail -3`
Expected: `test result: ok. 58 passed`.

- [ ] **Step 1.4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore(M2): add eframe, global-hotkey, notify-rust, crossbeam-channel deps"
```

---

## Task 2: Loosen `Cli` ArgGroup so no-action invocation enters GUI mode

**Files:**
- Modify: `src/lib.rs:32` (the `#[command(group(...))]` attribute and `Cli::to_action`)

**Why:** M1 makes one of `--translate-to`/`--fix-grammar`/`--rewrite`/`--custom` mandatory. M2 needs `clipt9n` (no args) to launch the GUI. We keep CLI mode working — passing one of those flags still runs the one-shot pipeline and exits.

- [ ] **Step 2.1: Write the failing test**

Add to `src/lib.rs` inside the `cli_tests` mod (append after `multiple_actions_are_rejected`):

```rust
#[test]
fn no_action_is_now_accepted_for_gui_mode() {
    let cli = Cli::try_parse_from(["clipt9n"]).unwrap();
    assert!(cli.translate_to.is_none());
    assert!(!cli.fix_grammar);
    assert!(!cli.rewrite);
    assert!(cli.custom.is_none());
    assert!(cli.action_or_none().is_none());
}

#[test]
fn translate_to_still_resolves_action() {
    let cli = Cli::try_parse_from(["clipt9n", "--translate-to=de"]).unwrap();
    assert!(matches!(cli.action_or_none(), Some(Action::Translate { code }) if code == "de"));
}
```

Also delete the existing `no_action_is_rejected` test (it asserts the old behavior).

- [ ] **Step 2.2: Run tests to verify failure**

Run: `cargo test --lib cli_tests 2>&1 | tail -10`
Expected: compilation error on `cli.action_or_none()` (method doesn't exist) — that's the failing signal.

- [ ] **Step 2.3: Update `Cli` to make actions optional and add `action_or_none`**

In `src/lib.rs`, change the `#[command(group(...))]` line from:

```rust
#[command(group(ArgGroup::new("action").required(true).args(["translate_to", "fix_grammar", "rewrite", "custom"])))]
```

to:

```rust
#[command(group(ArgGroup::new("action").required(false).args(["translate_to", "fix_grammar", "rewrite", "custom"])))]
```

Replace the existing `Cli::to_action` impl with:

```rust
impl Cli {
    /// Return the explicit CLI action, or `None` if no action flag was given
    /// (which means: launch the GUI).
    pub fn action_or_none(&self) -> Option<Action> {
        if let Some(code) = &self.translate_to {
            Some(Action::Translate { code: code.clone() })
        } else if self.fix_grammar {
            Some(Action::FixGrammar)
        } else if self.rewrite {
            Some(Action::Rewrite)
        } else if let Some(instruction) = &self.custom {
            Some(Action::Custom { instruction: instruction.clone() })
        } else {
            None
        }
    }
}
```

Update `pub async fn run()` in the same file: replace `let action = cli.to_action();` (appears twice) with:

```rust
let action = cli
    .action_or_none()
    .ok_or_else(|| TranslateError::Config(
        "no CLI action; GUI mode is not yet wired in run()".into(),
    ))?;
```

(In Task 14 we replace `run()` itself for the GUI path. For now this preserves CLI semantics: a no-arg `clipt9n` invoked through `run()` errors out cleanly.)

- [ ] **Step 2.4: Run tests to verify pass**

Run: `cargo test --lib cli_tests 2>&1 | tail -10`
Expected: all `cli_tests` pass.

Run also: `cargo test --all-features 2>&1 | tail -3`
Expected: 58 passed (one removed test, two added → still 59. If integration tests in `tests/cli_smoke.rs` previously asserted error on no-action, fix them. Per `git log` cli_smoke tests use explicit actions — no change needed. Verify by running and reading output.)

- [ ] **Step 2.5: Commit**

```bash
git add src/lib.rs
git commit -m "feat(M2): make CLI action flags optional; add action_or_none()"
```

---

## Task 3: Add `[hotkey]` and `[ui]` sections to `Config`

**Files:**
- Modify: `src/config.rs`

**Why:** The hotkey and density tokens are read by M2's main loop and prompt window. Spec §6 defines both sections; we now consume them.

- [ ] **Step 3.1: Write the failing tests**

Append to the `tests` module at the bottom of `src/config.rs`:

```rust
#[test]
fn default_hotkey_is_cmd_shift_t() {
    let cfg = Config::default();
    assert_eq!(cfg.hotkey.modifier, "cmd");
    assert!(cfg.hotkey.shift);
    assert_eq!(cfg.hotkey.key, "T");
    assert!(cfg.hotkey.enabled);
}

#[test]
fn default_ui_density_is_normal() {
    let cfg = Config::default();
    assert_eq!(cfg.ui.density, "normal");
    assert!(cfg.ui.show_preview);
}

#[test]
fn loads_hotkey_override() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(
        f,
        r#"
[hotkey]
modifier = "ctrl"
shift = false
key = "Y"
enabled = true
"#
    )
    .unwrap();
    let cfg = Config::load(f.path()).unwrap();
    assert_eq!(cfg.hotkey.modifier, "ctrl");
    assert!(!cfg.hotkey.shift);
    assert_eq!(cfg.hotkey.key, "Y");
}
```

- [ ] **Step 3.2: Run tests to verify failure**

Run: `cargo test --lib config::tests 2>&1 | tail -10`
Expected: compilation error on `cfg.hotkey` and `cfg.ui` (fields don't exist).

- [ ] **Step 3.3: Add `HotkeyConfig` and `UiConfig` types and wire them into `Config`**

In `src/config.rs`, after the `LanguageSlot` struct (around line 97), add:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// "cmd" → Cmd on macOS, Ctrl on Linux/Windows. "ctrl" → Ctrl on every OS.
    /// "alt" / "super" allowed but unmapped (passthrough).
    pub modifier: String,
    pub shift: bool,
    /// Key name accepted by `global-hotkey::hotkey::Code`. Single uppercase letter
    /// like "T" maps to `Code::KeyT`.
    pub key: String,
    pub enabled: bool,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "cmd".into(),
            shift: true,
            key: "T".into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// "normal" or "compact". Drives prompt window width (520 vs 460).
    pub density: String,
    pub show_preview: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            density: "normal".into(),
            show_preview: true,
        }
    }
}
```

Update the top-level `Config` struct (around line 13) by adding the two fields:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderConfig,
    pub languages: LanguagesConfig,
    pub hotkey: HotkeyConfig,
    pub ui: UiConfig,
}
```

- [ ] **Step 3.4: Run tests to verify pass**

Run: `cargo test --lib config 2>&1 | tail -10`
Expected: all config tests pass.

- [ ] **Step 3.5: Commit**

```bash
git add src/config.rs
git commit -m "feat(M2): add [hotkey] and [ui] config sections"
```

---

## Task 4: Modifier-mapping helpers (Cmd↔Ctrl) and `hotkey_display`

**Files:**
- Modify: `src/config.rs`

**Why:** Per the cross-platform discipline invariant in the design doc, **one** function maps logical `Cmd` to the OS-correct physical modifier. Used by hotkey registration in `main.rs` AND by every UI string showing the active hotkey. No `cfg`-blocks anywhere else.

- [ ] **Step 4.1: Write the failing tests**

Append to `src/config.rs`'s tests mod:

```rust
#[test]
fn hotkey_display_uses_logical_name() {
    let cfg = Config::default();
    // The displayed string is logical, not OS-mapped (it's a UI affordance, not a key event).
    // On every OS, a default config shows "Cmd+Shift+T" because that's how the user wrote it.
    assert_eq!(cfg.hotkey_display(), "Cmd+Shift+T");
}

#[test]
fn hotkey_display_no_shift() {
    let mut cfg = Config::default();
    cfg.hotkey.shift = false;
    cfg.hotkey.modifier = "ctrl".into();
    cfg.hotkey.key = "Y".into();
    assert_eq!(cfg.hotkey_display(), "Ctrl+Y");
}

#[test]
fn resolve_modifier_returns_native_for_cmd() {
    use crate::config::Modifier;
    let resolved = Modifier::Cmd.resolve_native();
    // On macOS, Cmd resolves to Meta (the global-hotkey "super"); on Linux/Windows, to Ctrl.
    #[cfg(target_os = "macos")]
    assert_eq!(resolved, NativeModifier::Meta);
    #[cfg(not(target_os = "macos"))]
    assert_eq!(resolved, NativeModifier::Ctrl);
}
```

(The third test is gated by `cfg(target_os)` only because the *expected value* depends on the runtime OS. The function itself is callable on every OS and contains the only `cfg` outside `src/platform/` that we permit — see Step 4.3 note.)

- [ ] **Step 4.2: Run tests to verify failure**

Run: `cargo test --lib config 2>&1 | tail -10`
Expected: compilation errors on `Modifier`, `NativeModifier`, `hotkey_display`, `resolve_native`.

- [ ] **Step 4.3: Implement the modifier types and helpers**

> **Cross-platform-discipline note:** The `Modifier::resolve_native()` function below contains the **only** `#[cfg(target_os)]` block outside `src/platform/`. We accept this exception because moving it into `platform/` would require introducing a `Platform::resolve_modifier()` method that's purely a constant-folding pure function — overkill. The M8 grep-lint allowlist will document this single line. A simpler alternative — putting the helper in `platform/mod.rs` — works fine; choose either. (If the engineer chooses to put it in `platform/mod.rs`, update the imports in tests above accordingly.)

Append to `src/config.rs` after `impl Config`:

```rust
/// Logical hotkey modifier as authored by the user. Mapped to the
/// OS-appropriate physical modifier via `resolve_native()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// "cmd" → Meta on macOS, Ctrl on Linux/Windows.
    Cmd,
    /// "ctrl" → Ctrl on every OS.
    Ctrl,
    /// "alt" → Alt on every OS.
    Alt,
    /// "super" → Meta on every OS.
    Super,
}

/// Native modifier flag returned by `resolve_native()`. Mirrors
/// `global_hotkey::hotkey::Modifiers` shape so the main-loop conversion
/// is a one-liner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeModifier {
    Ctrl,
    Alt,
    Meta,
}

impl Modifier {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cmd" => Some(Self::Cmd),
            "ctrl" | "control" => Some(Self::Ctrl),
            "alt" | "option" => Some(Self::Alt),
            "super" | "meta" | "win" => Some(Self::Super),
            _ => None,
        }
    }

    pub fn resolve_native(self) -> NativeModifier {
        match self {
            Self::Cmd => {
                #[cfg(target_os = "macos")]
                {
                    NativeModifier::Meta
                }
                #[cfg(not(target_os = "macos"))]
                {
                    NativeModifier::Ctrl
                }
            }
            Self::Ctrl => NativeModifier::Ctrl,
            Self::Alt => NativeModifier::Alt,
            Self::Super => NativeModifier::Meta,
        }
    }

    /// Human-readable form for UI strings ("Cmd", "Ctrl", "Alt", "Super").
    pub fn display(self) -> &'static str {
        match self {
            Self::Cmd => "Cmd",
            Self::Ctrl => "Ctrl",
            Self::Alt => "Alt",
            Self::Super => "Super",
        }
    }
}

impl Config {
    /// Render the configured hotkey for UI display (e.g., "Cmd+Shift+T").
    /// Returns "(disabled)" if `[hotkey].enabled = false`.
    pub fn hotkey_display(&self) -> String {
        if !self.hotkey.enabled {
            return "(disabled)".to_string();
        }
        let modifier = Modifier::parse(&self.hotkey.modifier)
            .map(Modifier::display)
            .unwrap_or("?");
        if self.hotkey.shift {
            format!("{modifier}+Shift+{}", self.hotkey.key)
        } else {
            format!("{modifier}+{}", self.hotkey.key)
        }
    }
}
```

Update the test imports in Step 4.1 if needed: the tests use `crate::config::Modifier` and `NativeModifier`, which are now public. Adjust the test module's `use super::*;` already covers them.

- [ ] **Step 4.4: Run tests to verify pass**

Run: `cargo test --lib config 2>&1 | tail -15`
Expected: all config tests pass (including the three new ones).

- [ ] **Step 4.5: Commit**

```bash
git add src/config.rs
git commit -m "feat(M2): Modifier::resolve_native() + Config::hotkey_display() helpers"
```

---

## Task 5: Platform module skeleton

**Files:**
- Create: `src/platform/mod.rs`
- Create: `src/platform/macos.rs` (stub for now; real impl in Task 6)
- Create: `src/platform/linux.rs`
- Create: `src/platform/windows.rs`
- Modify: `src/lib.rs` to declare `pub mod platform;`

**Why:** Constrain all OS-specific code behind one trait surface from M2 onward. The trait has `ensure_hotkey_permissions()` returning `Result<(), TranslateError>`. macOS implements it for real in Task 6; Linux and Windows return `Ok(())` (no-op).

- [ ] **Step 5.1: Write the failing test**

Create `src/platform/mod.rs` with this content (test included):

```rust
//! Cross-platform abstraction layer. Per the design doc, all
//! `#[cfg(target_os = …)]` and `#[cfg(unix)]` blocks in the codebase live
//! inside this module (M8 grep-lint enforces this with one exception
//! documented in `config::Modifier::resolve_native`).

use crate::error::TranslateError;

/// Behavior an OS may need to provide to the rest of the app. Per-OS impls
/// in `macos.rs`, `linux.rs`, `windows.rs`. Defaults are no-ops.
pub trait Platform {
    /// Verify the OS-level prerequisites for registering a global hotkey.
    /// On macOS this checks Accessibility permission. On Linux/Windows this
    /// is a no-op. Returns an error with user-actionable messaging if the
    /// prereq is missing.
    fn ensure_hotkey_permissions(&self) -> Result<(), TranslateError> {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacOsPlatform as ActivePlatform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as ActivePlatform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WindowsPlatform as ActivePlatform;

/// Construct the active platform impl for this build.
pub fn current() -> ActivePlatform {
    ActivePlatform::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_constructs() {
        let _ = current();
    }

    #[test]
    fn no_op_default_succeeds() {
        struct Stub;
        impl Platform for Stub {}
        assert!(Stub.ensure_hotkey_permissions().is_ok());
    }
}
```

Create `src/platform/linux.rs`:

```rust
use super::Platform;

#[derive(Default)]
pub struct LinuxPlatform;

impl Platform for LinuxPlatform {}
```

Create `src/platform/windows.rs`:

```rust
use super::Platform;

#[derive(Default)]
pub struct WindowsPlatform;

impl Platform for WindowsPlatform {}
```

Create `src/platform/macos.rs` (stub; replaced in Task 6):

```rust
use super::Platform;

#[derive(Default)]
pub struct MacOsPlatform;

impl Platform for MacOsPlatform {}
```

In `src/lib.rs`, add `pub mod platform;` next to the other `pub mod` lines (after `pub mod llm;`).

- [ ] **Step 5.2: Run tests to verify pass**

Run: `cargo test --lib platform 2>&1 | tail -10`
Expected: 2 tests pass.

Run: `cargo build --target x86_64-unknown-linux-gnu 2>&1 | tail -3` (only if cross toolchain is installed; otherwise skip — CI catches it.)
Expected: builds clean. (M1's CI already builds 5 targets; this task's CI run will exercise non-macOS branches.)

- [ ] **Step 5.3: Commit**

```bash
git add src/platform src/lib.rs
git commit -m "feat(M2): platform/ module skeleton with no-op defaults"
```

---

## Task 6: macOS Accessibility-permission detection

**Files:**
- Modify: `src/platform/macos.rs`
- Modify: `src/error.rs` to add a new variant if needed

**Why:** Registering a global hotkey on macOS requires the app to be granted Accessibility permission via System Settings → Privacy & Security → Accessibility. If missing, `global-hotkey` registration succeeds silently but the hotkey never fires — confusing for the user. M2's exit criterion 1 ("Hotkey opens window centered on the active display") fails silently without this. We use the public `AXIsProcessTrustedWithOptions` API from `ApplicationServices.framework` to detect permission state, and `open` to launch System Settings if missing.

- [ ] **Step 6.1: Add `AccessibilityPermissionDenied` to `TranslateError`**

In `src/error.rs`, append a new variant to the `TranslateError` enum:

```rust
#[error("macOS Accessibility permission not granted; the global hotkey cannot be registered without it. Open System Settings → Privacy & Security → Accessibility and enable clipt9n.")]
AccessibilityPermissionDenied,
```

Also bump any match-statement exhaustiveness tests (none in M1; verify with `cargo build`).

- [ ] **Step 6.2: Replace `src/platform/macos.rs` with the real impl**

Replace the file content entirely with:

```rust
//! macOS-specific platform integration.
//!
//! Currently provides Accessibility-permission detection so the user gets a
//! clear error (and a one-click open of System Settings) when the global
//! hotkey can't be registered.

use std::process::Command;

use objc2_application_services::AXIsProcessTrustedWithOptions;
use objc2_foundation::{NSDictionary, NSString};

use super::Platform;
use crate::error::TranslateError;

#[derive(Default)]
pub struct MacOsPlatform;

impl Platform for MacOsPlatform {
    fn ensure_hotkey_permissions(&self) -> Result<(), TranslateError> {
        if is_process_trusted(false) {
            return Ok(());
        }
        // Not trusted — call again with prompt=true so macOS shows its own
        // permission dialog, AND open System Settings to the right pane in
        // case the user dismissed the dialog.
        let _ = is_process_trusted(true);
        let _ = open_accessibility_settings();
        Err(TranslateError::AccessibilityPermissionDenied)
    }
}

/// Returns true if the current process has Accessibility permission. If
/// `prompt` is true and permission is missing, macOS shows its own
/// permission dialog (only does so if the binary is in System Settings's
/// list at all — the open below covers the first-launch case).
fn is_process_trusted(prompt: bool) -> bool {
    // SAFETY: AXIsProcessTrustedWithOptions accepts a NSDictionary pointer
    // (or null). We construct a dict with the documented prompt key.
    unsafe {
        if !prompt {
            return AXIsProcessTrustedWithOptions(std::ptr::null());
        }
        let key = NSString::from_str("AXTrustedCheckOptionPrompt");
        let value = objc2_foundation::NSNumber::new_bool(true);
        let dict = NSDictionary::from_slices(&[&*key], &[value.as_ref()]);
        let dict_ptr: *const NSDictionary<NSString, objc2_foundation::NSObject> = &*dict;
        AXIsProcessTrustedWithOptions(dict_ptr.cast())
    }
}

fn open_accessibility_settings() -> std::io::Result<()> {
    Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_process_trusted_does_not_panic() {
        // CI runs as a non-GUI process, so this is virtually always false.
        // We assert only that the call returns without panicking.
        let _ = is_process_trusted(false);
    }
}
```

> **Implementation note for the engineer:** The exact API surface of `objc2-application-services` 0.3 may have shifted between point releases. If the `AXIsProcessTrustedWithOptions` symbol path or signature differs from above, consult the crate's `cargo doc --open` and adapt the call site. The function ultimately calls a documented Apple API; the wrapper is mechanical. If the `objc2-application-services` route proves painful, an acceptable fallback is hand-rolled FFI:
>
> ```rust
> #[link(name = "ApplicationServices", kind = "framework")]
> extern "C" {
>     fn AXIsProcessTrusted() -> bool;
> }
> ```
>
> ...and skip the prompt-true variant (let our `open` call handle the user-facing prompt). If you take this path, drop the `objc2-*` deps from Cargo.toml.

- [ ] **Step 6.3: Run tests**

Run: `cargo test --lib platform 2>&1 | tail -10`
Expected: 3 tests pass (the 2 from Task 5 + the new `is_process_trusted_does_not_panic`).

Run: `cargo test --all-features 2>&1 | tail -3`
Expected: all green.

- [ ] **Step 6.4: Commit**

```bash
git add src/platform/macos.rs src/error.rs Cargo.toml Cargo.lock
git commit -m "feat(M2): detect macOS Accessibility permission; open Settings on miss"
```

---

## Task 7: `state.toml` last-action persistence

**Files:**
- Create: `src/state.rs`
- Modify: `src/lib.rs` to add `pub mod state;`

**Why:** Exit criterion 3 ("Pressing Enter on second invocation repeats slot from previous run") requires persisting the last action across app restarts. Spec rule: only slots are persisted, never custom prompts (privacy).

- [ ] **Step 7.1: Write the failing tests**

Create `src/state.rs`:

```rust
//! Cross-restart state. Currently persists only the last-used slot index
//! (1–6), so Enter on the prompt window can repeat it. Custom prompts are
//! never persisted (spec privacy rule).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TranslateError;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub last_slot: Option<u8>,
}

impl State {
    /// Read state from `path`. Missing file or malformed TOML returns
    /// `State::default()` — last-action recall is best-effort, never blocks.
    pub fn load(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&contents).unwrap_or_default()
    }

    /// Write state to `path`, creating parent dirs as needed. Returns an
    /// error on failure but the caller is expected to log and continue.
    pub fn save(&self, path: &Path) -> Result<(), TranslateError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| TranslateError::Config(format!("creating state dir: {e}")))?;
        }
        let toml_str = toml::to_string(self)
            .map_err(|e| TranslateError::Config(format!("encoding state: {e}")))?;
        std::fs::write(path, toml_str)
            .map_err(|e| TranslateError::Config(format!("writing state: {e}")))
    }

    pub fn record_slot(&mut self, slot: u8) {
        if (1..=6).contains(&slot) {
            self.last_slot = Some(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_returns_default() {
        let s = State::load(Path::new("/tmp/clipt9n-nonexistent-state-12345.toml"));
        assert_eq!(s, State::default());
        assert!(s.last_slot.is_none());
    }

    #[test]
    fn round_trip_persists_slot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let mut s = State::default();
        s.record_slot(2);
        s.save(&path).unwrap();
        let loaded = State::load(&path);
        assert_eq!(loaded.last_slot, Some(2));
    }

    #[test]
    fn record_slot_rejects_out_of_range() {
        let mut s = State::default();
        s.record_slot(0);
        assert!(s.last_slot.is_none());
        s.record_slot(7);
        assert!(s.last_slot.is_none());
        s.record_slot(3);
        assert_eq!(s.last_slot, Some(3));
    }

    #[test]
    fn malformed_toml_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, "this is not :::: valid toml").unwrap();
        let s = State::load(&path);
        assert_eq!(s, State::default());
    }
}
```

In `src/lib.rs`, add `pub mod state;` next to the other `pub mod` lines.

- [ ] **Step 7.2: Run tests to verify pass**

Run: `cargo test --lib state 2>&1 | tail -10`
Expected: 4 tests pass.

- [ ] **Step 7.3: Commit**

```bash
git add src/state.rs src/lib.rs
git commit -m "feat(M2): state.toml last-slot persistence"
```

---

## Task 8: `src/ui/theme.rs` — palette, Visuals builder, WindowFrame, kbd widget

**Files:**
- Create: `src/ui/mod.rs`
- Create: `src/ui/theme.rs`
- Modify: `src/lib.rs` to add `pub mod ui;`

**Why:** The design's tokens live as CSS vars in `Clipboard Translator.html`. We lift them into Rust constants and build an `egui::Visuals` from them so all subsequent windows inherit the look automatically. The a11y-corrected `--ink-3` (`#9ca3b1` instead of `#80869294`) is applied here per the design doc's a11y baseline.

- [ ] **Step 8.1: Write the failing tests**

Create `src/ui/mod.rs`:

```rust
pub mod prompt;
pub mod theme;
```

Create `src/ui/theme.rs` with tests at the bottom and types (filled in next step):

```rust
//! Design tokens (a11y-corrected) and reusable visual primitives.
//!
//! Source palette: `handoff/clipt9n/project/Clipboard Translator.html`.
//! a11y corrections per `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md`
//! ("A11y baseline" section): `--ink-3` lifted from `#80869294` (alpha 58%,
//! ~3.5:1 contrast) to solid `#9ca3b1` (~5.1:1, AA pass). Disabled-state
//! foreground bumped to `#7a818d`.

use egui::{Color32, Stroke, Visuals};

// ----- Palette -----

pub const BG: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x17, 0x1c);
pub const PANEL_2: Color32 = Color32::from_rgb(0x1c, 0x20, 0x27);
pub const PANEL_3: Color32 = Color32::from_rgb(0x23, 0x27, 0x2f);
pub const LINE: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x39);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x20, 0x24, 0x2c);

pub const INK: Color32 = Color32::from_rgb(0xe9, 0xec, 0xf1);
pub const INK_2: Color32 = Color32::from_rgb(0xb6, 0xbc, 0xc7);
/// a11y-corrected from #80869294 (alpha 58%) to solid #9ca3b1.
pub const INK_3: Color32 = Color32::from_rgb(0x9c, 0xa3, 0xb1);
/// Decorative gutter only (line numbers in glossary view). Documents the
/// 3.6:1 ratio is acceptable for non-text UI chrome.
pub const MUTED: Color32 = Color32::from_rgb(0x6c, 0x72, 0x7d);
/// Disabled foreground on PANEL_3, ~3.2:1.
pub const DISABLED_FG: Color32 = Color32::from_rgb(0x7a, 0x81, 0x8d);

pub const ACCENT: Color32 = Color32::from_rgb(0xc8, 0xff, 0x5e);
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
pub const WARN: Color32 = Color32::from_rgb(0xff, 0xb8, 0x4d);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x76, 0x76);
pub const GOOD: Color32 = Color32::from_rgb(0x8f, 0xe3, 0xa7);

// ----- Visuals -----

/// Build the dark-mode `Visuals` for the entire app. Sets backgrounds,
/// strokes, and selection/focus colors so every interactive widget gets
/// the lime accent for selection AND a 2px ACCENT focus ring (a11y).
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.window_fill = PANEL;
    v.panel_fill = PANEL;
    v.faint_bg_color = PANEL_2;
    v.extreme_bg_color = BG;
    v.override_text_color = Some(INK);
    v.hyperlink_color = ACCENT;

    // Selection (highlighted rows, selected text)
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x18); // ~9% accent
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    // Focus ring: 2px ACCENT on every focusable widget. In egui this is
    // `widgets.active.bg_stroke` (drawn when focused/active).
    v.widgets.inactive.bg_fill = PANEL_2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, INK_2);

    v.widgets.hovered.bg_fill = PANEL_3;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, INK);

    v.widgets.active.bg_fill = PANEL_3;
    v.widgets.active.bg_stroke = Stroke::new(2.0, ACCENT); // ← focus ring
    v.widgets.active.fg_stroke = Stroke::new(1.0, INK);

    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE_SOFT);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK_3);

    v
}

// ----- Reusable widgets -----

/// Render a kbd-style key cap. Use for footer keymap hints.
pub fn kbd(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let frame = egui::Frame::none()
        .fill(PANEL_3)
        .stroke(Stroke::new(1.0, LINE))
        .rounding(3.0)
        .inner_margin(egui::Margin::symmetric(5.0, 1.0));
    frame
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .monospace()
                    .size(10.5)
                    .color(INK_2),
            );
        })
        .response
}

/// `WindowFrame` analog: title bar + body. Rendered as content inside an
/// already-borderless egui viewport (the viewport has decorations off, so
/// we paint our own title bar). Returns the inner-body `Ui` for the caller
/// to fill in.
pub fn window_frame<R>(
    ctx: &egui::Context,
    title: &str,
    subtitle: Option<&str>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(PANEL))
        .show(ctx, |ui| {
            // Title bar
            egui::Frame::none()
                .fill(Color32::from_rgba_unmultiplied(0x14, 0x16, 0x1c, 0x99))
                .inner_margin(egui::Margin::symmetric(12.0, 9.0))
                .stroke(Stroke::new(1.0, LINE_SOFT))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        // Accent dot
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 3.0, ACCENT);
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(title).color(INK).size(13.0).strong());
                        if let Some(sub) = subtitle {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(sub)
                                    .color(INK_3)
                                    .monospace()
                                    .size(11.0),
                            );
                        }
                    });
                });
            ui.add_space(4.0);
            ui.scope(|ui| {
                ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 6.0);
                body(ui)
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_3_is_a11y_corrected() {
        // Confirms the corrected color, not the original alpha-58% design value.
        assert_eq!(INK_3, Color32::from_rgb(0x9c, 0xa3, 0xb1));
    }

    #[test]
    fn visuals_use_accent_for_focus_ring() {
        let v = visuals();
        assert_eq!(v.widgets.active.bg_stroke.color, ACCENT);
        assert_eq!(v.widgets.active.bg_stroke.width, 2.0);
    }

    #[test]
    fn visuals_text_color_is_ink() {
        let v = visuals();
        assert_eq!(v.override_text_color, Some(INK));
    }
}
```

In `src/lib.rs`, add `pub mod ui;` next to the other `pub mod` lines.

- [ ] **Step 8.2: Run tests to verify pass**

Run: `cargo test --lib ui::theme 2>&1 | tail -10`
Expected: 3 tests pass.

Run: `cargo build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 8.3: Commit**

```bash
git add src/ui src/lib.rs
git commit -m "feat(M2): a11y-corrected design palette, Visuals, WindowFrame, kbd widget"
```

---

## Task 9: `src/ui/prompt.rs` — render-only (no event handling yet)

**Files:**
- Create / fully populate: `src/ui/prompt.rs`

**Why:** Split the prompt window into "draw" (this task) and "events" (Task 11) so each is small enough to reason about. This task gets the visual right; keyboard handling lands in Task 11.

The window has two states:
- **Empty clipboard:** centered icon + "Clipboard is empty or not text." + "Esc to dismiss" hint.
- **Populated:** preview block (3-line clip with monospace `›` markers, lang badge, char count), 6 numbered slot rows, glossary chip area (renders empty in M2; M4 fills it), footer keymap.

- [ ] **Step 9.1: Define `PromptModel` and the slot list**

Create `src/ui/prompt.rs`:

```rust
//! The hotkey-summoned prompt window. Renders the design's
//! `prompt-window.jsx` (M2 wires slots 1–3 end-to-end via the caller; slots
//! 4–6 render but are no-ops in M2). The view is a pure function of
//! `PromptModel`; event handling lives in `update()` (Task 11).

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::config::Config;
use crate::ui::theme;

/// What the prompt window currently knows.
#[derive(Debug, Clone)]
pub struct PromptModel {
    /// Current clipboard text (already filtered to text-only). Empty string
    /// → render the empty state.
    pub clipboard_text: String,
    /// Auto-detected language code for the clipboard, if any (M4 sets this;
    /// M2 always passes `None`).
    pub detected_lang: Option<String>,
    /// 1-based slot index of the most recently used action ("last used" badge
    /// + Enter-to-repeat affordance). `None` on first run.
    pub last_slot: Option<u8>,
}

/// Picked action from the prompt window. The caller maps the slot to a
/// concrete `translator::Action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    /// User clicked a slot button or pressed 1–6.
    Pick(u8),
    /// User pressed Esc.
    Cancel,
    /// User pressed Enter while `last_slot` was set.
    RepeatLast,
}

/// Static slot definitions (matches `data.jsx` SLOTS).
#[derive(Debug, Clone, Copy)]
pub struct SlotDef {
    pub n: u8,
    pub kind: SlotKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Slot 1, 2, 3 — language slots; label/code come from `Config::languages`.
    Lang,
    /// Slot 4 — fix grammar.
    FixGrammar,
    /// Slot 5 — rewrite.
    Rewrite,
    /// Slot 6 — custom (M3 wires; M2 is no-op).
    Custom,
}

pub const SLOTS: [SlotDef; 6] = [
    SlotDef { n: 1, kind: SlotKind::Lang },
    SlotDef { n: 2, kind: SlotKind::Lang },
    SlotDef { n: 3, kind: SlotKind::Lang },
    SlotDef { n: 4, kind: SlotKind::FixGrammar },
    SlotDef { n: 5, kind: SlotKind::Rewrite },
    SlotDef { n: 6, kind: SlotKind::Custom },
]
;

/// Resolve a slot's display label and trailing tag/code. Returns
/// (label, trailing) where `trailing` is the right-aligned hint text
/// (lang code or descriptive tag).
pub fn slot_strings<'a>(slot: SlotDef, cfg: &'a Config) -> (&'a str, &'a str) {
    match (slot.n, slot.kind) {
        (1, SlotKind::Lang) => (&cfg.languages.slot_1.label, &cfg.languages.slot_1.code),
        (2, SlotKind::Lang) => (&cfg.languages.slot_2.label, &cfg.languages.slot_2.code),
        (3, SlotKind::Lang) => (&cfg.languages.slot_3.label, &cfg.languages.slot_3.code),
        (4, SlotKind::FixGrammar) => ("Fix grammar", "conservative"),
        (5, SlotKind::Rewrite) => ("Rewrite for clarity", "aggressive"),
        (6, SlotKind::Custom) => ("Custom prompt…", "type instruction"),
        _ => ("(invalid slot)", ""),
    }
}

/// Draw the prompt window into `ctx`. Returns `Some(PromptOutcome)` if the
/// user clicked a slot button this frame; `None` otherwise. Keyboard
/// handling lives in `App::update` and is not the responsibility of this
/// function.
pub fn draw(ctx: &egui::Context, cfg: &Config, model: &PromptModel) -> Option<PromptOutcome> {
    let mut clicked: Option<PromptOutcome> = None;
    theme::window_frame(ctx, "Translate clipboard", Some("clipt9n · prompt"), |ui| {
        if model.clipboard_text.is_empty() {
            draw_empty(ui);
        } else {
            draw_populated(ui, cfg, model, &mut clicked);
        }
    });
    clicked
}

fn draw_empty(ui: &mut egui::Ui) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("⎚").size(28.0).color(theme::BAD));
        ui.add_space(8.0);
        ui.label(RichText::new("Clipboard is empty or not text.").color(theme::INK).size(14.0));
        ui.label(RichText::new("Copy something and try again.").color(theme::INK_3).size(12.0));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 40.0);
            theme::kbd(ui, "Esc");
            ui.label(RichText::new("to dismiss").color(theme::INK_3).size(11.0).monospace());
        });
    });
    ui.add_space(20.0);
}

fn draw_populated(
    ui: &mut egui::Ui,
    cfg: &Config,
    model: &PromptModel,
    clicked: &mut Option<PromptOutcome>,
) {
    let body_padding = egui::Margin::symmetric(18.0, 14.0);
    egui::Frame::none().inner_margin(body_padding).show(ui, |ui| {
        // ----- Preview header -----
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("CLIPBOARD")
                    .color(theme::INK_3)
                    .size(11.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("· {} chars", model.clipboard_text.chars().count()))
                        .color(theme::INK_3)
                        .monospace()
                        .size(11.0),
                );
                let lang = model.detected_lang.as_deref().unwrap_or("??").to_uppercase();
                let lang_frame = egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x1f))
                    .rounding(3.0)
                    .inner_margin(egui::Margin::symmetric(6.0, 1.0));
                lang_frame.show(ui, |ui| {
                    ui.label(
                        RichText::new(lang)
                            .color(theme::ACCENT)
                            .monospace()
                            .size(10.0)
                            .strong(),
                    );
                });
            });
        });
        ui.add_space(6.0);

        // ----- Preview block -----
        let preview = preview_text(&model.clipboard_text);
        egui::Frame::none()
            .fill(theme::PANEL_2)
            .stroke(Stroke::new(1.0, theme::LINE_SOFT))
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                for line in preview.lines().take(3) {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("›")
                                .color(theme::ACCENT.linear_multiply(0.6))
                                .monospace(),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(if line.is_empty() { "\u{00A0}" } else { line })
                                .color(theme::INK_2)
                                .monospace()
                                .size(12.5),
                        );
                    });
                }
            });
        ui.add_space(14.0);

        // ----- Slot rows -----
        for slot in SLOTS {
            let (label, trailing) = slot_strings(slot, cfg);
            let is_last = model.last_slot == Some(slot.n);
            if draw_slot_row(ui, slot, label, trailing, is_last) {
                *clicked = Some(PromptOutcome::Pick(slot.n));
            }
        }

        // ----- Glossary chip area (M2 always empty; M4 fills it) -----
        // Empty placeholder reserved so the layout doesn't shift when M4
        // adds chips. Render nothing; the gap above the footer is enough.

        ui.add_space(12.0);
        // ----- Footer -----
        egui::Frame::none()
            .stroke(Stroke {
                width: 1.0,
                color: theme::LINE_SOFT,
            })
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::kbd(ui, "1");
                    ui.label(RichText::new("–").color(theme::INK_3).size(11.0));
                    theme::kbd(ui, "6");
                    ui.label(RichText::new("pick ·").color(theme::INK_3).monospace().size(11.0));
                    theme::kbd(ui, "↵");
                    let enter_label = if model.last_slot.is_some() {
                        "repeat last ·"
                    } else {
                        "— ·"
                    };
                    ui.label(RichText::new(enter_label).color(theme::INK_3).monospace().size(11.0));
                    theme::kbd(ui, "Esc");
                    ui.label(RichText::new("cancel").color(theme::INK_3).monospace().size(11.0));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if model.clipboard_text.chars().count() > 2000 {
                            ui.label(
                                RichText::new("⚠ large paste")
                                    .color(theme::WARN)
                                    .monospace()
                                    .size(11.0),
                            );
                        }
                    });
                });
            });
    });
}

fn draw_slot_row(
    ui: &mut egui::Ui,
    slot: SlotDef,
    label: &str,
    trailing: &str,
    is_last: bool,
) -> bool {
    let bg = if is_last {
        Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x10)
    } else {
        Color32::TRANSPARENT
    };
    let border = if is_last {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x2e))
    } else {
        Stroke::new(1.0, Color32::TRANSPARENT)
    };

    let mut clicked = false;
    let response = egui::Frame::none()
        .fill(bg)
        .stroke(border)
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(10.0, 9.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Number badge
                let (badge_rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
                ui.painter().rect_filled(badge_rect, 4.0, theme::PANEL_3);
                ui.painter().rect_stroke(badge_rect, 4.0, Stroke::new(1.0, theme::LINE));
                ui.painter().text(
                    badge_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", slot.n),
                    egui::FontId::monospace(11.5),
                    theme::INK_2,
                );
                ui.add_space(8.0);
                ui.label(RichText::new(label).color(theme::INK).size(13.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_last {
                        let badge = egui::Frame::none()
                            .fill(Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x29))
                            .rounding(999.0)
                            .inner_margin(egui::Margin::symmetric(7.0, 2.0));
                        badge.show(ui, |ui| {
                            ui.label(
                                RichText::new("LAST USED")
                                    .color(theme::ACCENT)
                                    .size(10.0)
                                    .strong(),
                            );
                        });
                        ui.add_space(6.0);
                    }
                    ui.label(
                        RichText::new(trailing)
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                });
            });
        })
        .response
        .interact(Sense::click());

    if response.clicked() {
        clicked = true;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    clicked
}

/// Truncate clipboard text to ~110 chars with an ellipsis (matches
/// `prompt-window.jsx` preview rules). Splits to lines for the caller.
pub fn preview_text(text: &str) -> String {
    if text.chars().count() <= 110 {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(110).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_under_limit_returns_unchanged() {
        let s = "hello world";
        assert_eq!(preview_text(s), s);
    }

    #[test]
    fn preview_over_limit_truncates_with_ellipsis() {
        let s = "x".repeat(200);
        let p = preview_text(&s);
        assert_eq!(p.chars().count(), 111); // 110 + ellipsis
        assert!(p.ends_with('…'));
    }

    #[test]
    fn slot_strings_for_slot_1_uses_config_label() {
        let cfg = Config::default();
        let (label, code) = slot_strings(SLOTS[0], &cfg);
        assert_eq!(label, "English");
        assert_eq!(code, "en");
    }

    #[test]
    fn slot_strings_for_slot_4_returns_fix_grammar_tag() {
        let cfg = Config::default();
        let (label, tag) = slot_strings(SLOTS[3], &cfg);
        assert_eq!(label, "Fix grammar");
        assert_eq!(tag, "conservative");
    }
}
```

- [ ] **Step 9.2: Run tests to verify pass**

Run: `cargo test --lib ui::prompt 2>&1 | tail -10`
Expected: 4 tests pass.

Run: `cargo build 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 9.3: Commit**

```bash
git add src/ui/prompt.rs
git commit -m "feat(M2): prompt window draw layer (no event handling yet)"
```

---

## Task 10: `src/notify.rs` — translation-complete toast

**Files:**
- Create: `src/notify.rs`
- Modify: `src/lib.rs` to add `pub mod notify;`

**Why:** Spec §3 — every successful translation triggers an OS notification ("Translation copied"). `notify-rust` is cross-platform.

- [ ] **Step 10.1: Implement `notify::translation_copied`**

Create `src/notify.rs`:

```rust
//! OS notifications. Currently used only for the post-translation
//! "Translation copied" toast.

use crate::error::TranslateError;

/// Show a "Translation copied" toast. The body is a short identifier of
/// the action just performed (e.g., "Translate to Deutsch", "Fix grammar").
/// Failures are non-fatal — caller logs and continues.
pub fn translation_copied(action_label: &str) -> Result<(), TranslateError> {
    notify_rust::Notification::new()
        .summary("Translation copied")
        .body(action_label)
        .appname("clipt9n")
        .timeout(notify_rust::Timeout::Milliseconds(2500))
        .show()
        .map(|_| ())
        .map_err(|e| TranslateError::Config(format!("notification failed: {e}")))
}

#[cfg(test)]
mod tests {
    // Notifications are inherently a side-effect on the user's session;
    // there's no headless way to assert delivery. We only verify that the
    // function compiles and the call doesn't panic when invoked from a
    // headless test runner. (`show()` may fail in CI; we accept that.)
    #[test]
    fn translation_copied_does_not_panic() {
        let _ = super::translation_copied("Fix grammar");
    }
}
```

In `src/lib.rs`, add `pub mod notify;` next to the other `pub mod` lines.

- [ ] **Step 10.2: Run tests**

Run: `cargo test --lib notify 2>&1 | tail -5`
Expected: pass.

- [ ] **Step 10.3: Commit**

```bash
git add src/notify.rs src/lib.rs
git commit -m "feat(M2): translation-complete OS toast via notify-rust"
```

---

## Task 11: `src/app.rs` — eframe `App` impl with hotkey, async dispatch, prompt event handling

**Files:**
- Create: `src/app.rs`
- Modify: `src/lib.rs` to add `pub mod app;`

**Why:** This is the seam — egui event loop on one side, tokio runtime + LLM provider on the other, hotkey events from a third thread. Owning the runtime as a struct field keeps lifetimes simple. The state machine has three states: `Idle` (window hidden), `Showing` (window visible, waiting for user input), `Translating` (window hidden during M2 — translating overlay is M3's job, so for M2 we just hide and toast on completion).

- [ ] **Step 11.1: Implement `ClipApp`**

Create `src/app.rs`:

```rust
//! `ClipApp` is the eframe application: it owns the tokio runtime, the
//! channels to/from the hotkey thread and the translation worker, and the
//! prompt-window state machine. All UI is paint-only (`src/ui/prompt.rs`);
//! input handling lives here.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crossbeam_channel::Receiver as CrossbeamReceiver;
use eframe::CreationContext;
use egui::{Key, ViewportCommand};
use global_hotkey::GlobalHotKeyEvent;
use tokio::runtime::Runtime;

use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::LlmProvider;
use crate::secrets::Secrets;
use crate::state::State;
use crate::translator::{Action, Translator};
use crate::ui::{prompt, theme};

/// Top-level UI state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AppState {
    /// Window hidden. Hotkey will transition to `Showing`.
    Idle,
    /// Window visible; user is choosing an action.
    Showing,
    /// Translation in flight. Window hidden in M2 (overlay is M3).
    Translating,
}

pub struct ClipApp {
    cfg: Config,
    state_path: PathBuf,
    state: State,

    /// Boxed for shared ownership across async tasks. Recreated per call to
    /// avoid `Send` issues with `Box<dyn Trait>`; we keep the `Arc` form to
    /// allow cheap clones into the spawn closure.
    provider: std::sync::Arc<dyn LlmProvider>,

    runtime: Runtime,
    hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    result_tx: mpsc::Sender<TranslationOutcome>,
    result_rx: mpsc::Receiver<TranslationOutcome>,

    app_state: AppState,
    prompt_model: prompt::PromptModel,
}

#[derive(Debug)]
struct TranslationOutcome {
    result: Result<String, TranslateError>,
    action_label: String,
    slot: u8,
}

impl ClipApp {
    pub fn new(
        cc: &CreationContext<'_>,
        cfg: Config,
        provider: std::sync::Arc<dyn LlmProvider>,
        _secrets: Box<dyn Secrets>,
        state_path: PathBuf,
        hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    ) -> Self {
        cc.egui_ctx.set_visuals(theme::visuals());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("clipt9n-async")
            .build()
            .expect("tokio runtime");

        let (result_tx, result_rx) = mpsc::channel();
        let state = State::load(&state_path);
        // `cfg.hotkey_display()` is intentionally unused here — the prompt
        // window's footer shows literal kbd badges ("1", "↵", "Esc"), not
        // the configurable summon hotkey. The display helper is kept for
        // M7 (tray menu) where the active hotkey IS shown.

        Self {
            prompt_model: prompt::PromptModel {
                clipboard_text: String::new(),
                detected_lang: None,
                last_slot: state.last_slot,
            },
            cfg,
            state_path,
            state,
            provider,
            runtime,
            hotkey_rx,
            result_tx,
            result_rx,
            app_state: AppState::Idle,
        }
    }

    /// Read the system clipboard (text only). Returns the text or empty
    /// string if non-text/unreadable. Errors are swallowed so the prompt
    /// window can still show its empty state.
    fn snapshot_clipboard(&self) -> String {
        let mut cb = match ArboardClipboard::new() {
            Ok(c) => c,
            Err(_) => return String::new(),
        };
        cb.read_text().unwrap_or_default()
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.prompt_model.clipboard_text = self.snapshot_clipboard();
        self.prompt_model.last_slot = self.state.last_slot;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::Showing;
    }

    fn hide_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Map a slot number to a concrete `Action`. Returns `None` if slot 6
    /// (custom — not wired in M2) or invalid.
    fn slot_to_action(&self, slot: u8) -> Option<(Action, String)> {
        match slot {
            1 => Some((
                Action::Translate { code: self.cfg.languages.slot_1.code.clone() },
                format!("Translate to {}", self.cfg.languages.slot_1.label),
            )),
            2 => Some((
                Action::Translate { code: self.cfg.languages.slot_2.code.clone() },
                format!("Translate to {}", self.cfg.languages.slot_2.label),
            )),
            3 => Some((
                Action::Translate { code: self.cfg.languages.slot_3.code.clone() },
                format!("Translate to {}", self.cfg.languages.slot_3.label),
            )),
            4 => Some((Action::FixGrammar, "Fix grammar".into())),
            5 => Some((Action::Rewrite, "Rewrite for clarity".into())),
            6 => None, // Custom prompt — M3.
            _ => None,
        }
    }

    fn dispatch(&mut self, ctx: &egui::Context, slot: u8) {
        let Some((action, action_label)) = self.slot_to_action(slot) else {
            tracing::info!(slot, "slot is no-op in M2");
            return;
        };
        let cfg = self.cfg.clone();
        let provider = self.provider.clone();
        let tx = self.result_tx.clone();
        let source_text = self.prompt_model.clipboard_text.clone();

        // Persist last-action immediately. State write failures are logged
        // but never block the user.
        self.state.record_slot(slot);
        if let Err(e) = self.state.save(&self.state_path) {
            tracing::warn!(error = %e, "state.toml save failed");
        }
        self.app_state = AppState::Translating;
        self.hide_window(ctx);

        let ctx_for_repaint = ctx.clone();
        self.runtime.spawn(async move {
            let translator = Translator::new(&cfg, provider.as_ref());
            let result = translator.execute(&action, &source_text).await;
            let _ = tx.send(TranslationOutcome { result, action_label, slot });
            ctx_for_repaint.request_repaint();
        });
    }

    fn handle_translation_done(&mut self, outcome: TranslationOutcome) {
        match outcome.result {
            Ok(translated) => {
                let mut cb = match ArboardClipboard::new() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(error = %e, "clipboard open failed");
                        self.app_state = AppState::Idle;
                        return;
                    }
                };
                if let Err(e) = cb.write_text(&translated) {
                    tracing::error!(error = %e, "clipboard write failed");
                } else if let Err(e) = crate::notify::translation_copied(&outcome.action_label) {
                    tracing::warn!(error = %e, "notification failed");
                }
                tracing::info!(slot = outcome.slot, action = %outcome.action_label, "translation complete");
            }
            Err(e) => {
                tracing::error!(error = %e, "translation failed");
                let _ = notify_rust::Notification::new()
                    .summary("Translation failed")
                    .body(&format!("{e}"))
                    .appname("clipt9n")
                    .timeout(notify_rust::Timeout::Milliseconds(4000))
                    .show();
            }
        }
        self.app_state = AppState::Idle;
    }

    fn drain_channels(&mut self, ctx: &egui::Context) {
        // Hotkey events
        while let Ok(_event) = self.hotkey_rx.try_recv() {
            // Any hotkey event = "summon prompt" in M2 (we register one).
            if matches!(self.app_state, AppState::Idle) {
                self.show_window(ctx);
            } else {
                // If translating, ignore. If already showing, just refocus.
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
        }
        // Translation results
        while let Ok(outcome) = self.result_rx.try_recv() {
            self.handle_translation_done(outcome);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) -> Option<prompt::PromptOutcome> {
        if !matches!(self.app_state, AppState::Showing) {
            return None;
        }
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(prompt::PromptOutcome::Cancel);
            }
            if i.key_pressed(Key::Enter) && self.state.last_slot.is_some() {
                return Some(prompt::PromptOutcome::RepeatLast);
            }
            for (key, n) in [
                (Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3),
                (Key::Num4, 4), (Key::Num5, 5), (Key::Num6, 6),
            ] {
                if i.key_pressed(key) && !self.prompt_model.clipboard_text.is_empty() {
                    return Some(prompt::PromptOutcome::Pick(n));
                }
            }
            None
        })
    }
}

impl eframe::App for ClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Lightly throttle when idle to not burn CPU. egui repaints on
        // input + we explicitly request_repaint when async tasks finish, so
        // a slow background tick is a safety net.
        ctx.request_repaint_after(Duration::from_millis(150));

        self.drain_channels(ctx);

        if matches!(self.app_state, AppState::Showing) {
            // Draw first (so click hits register), then process keyboard.
            let click = prompt::draw(ctx, &self.cfg, &self.prompt_model);
            let key = self.handle_keys(ctx);
            let outcome = key.or(click);
            match outcome {
                Some(prompt::PromptOutcome::Pick(n)) => self.dispatch(ctx, n),
                Some(prompt::PromptOutcome::RepeatLast) => {
                    if let Some(n) = self.state.last_slot {
                        self.dispatch(ctx, n);
                    }
                }
                Some(prompt::PromptOutcome::Cancel) => {
                    self.app_state = AppState::Idle;
                    self.hide_window(ctx);
                }
                None => {}
            }
        }
    }
}
```

In `src/lib.rs`, add `pub mod app;` next to the other `pub mod` lines.

- [ ] **Step 11.2: Verify the build**

Run: `cargo build 2>&1 | tail -5`
Expected: clean build. (No tests added — the app integrates many subsystems and is verified manually in Task 14.)

If `LlmProvider` requires `Send + Sync` for the `Arc<dyn LlmProvider>` to work in `runtime.spawn`, audit the trait. M1 declared it as `pub trait LlmProvider: Send + Sync` (verify by reading `src/llm/mod.rs`). If `Sync` is missing there, add it before continuing this task.

- [ ] **Step 11.3: Commit**

```bash
git add src/app.rs src/lib.rs
git commit -m "feat(M2): ClipApp event loop with hotkey, async dispatch, prompt state"
```

---

## Task 12: `src/main.rs` — wire eframe + global hotkey + platform check

**Files:**
- Rewrite: `src/main.rs`

**Why:** This is the new entry point. It distinguishes CLI mode (`Cli::action_or_none().is_some()` → reuse M1's `lib::run` path) from GUI mode (`None` → eframe app). For GUI mode it builds the platform impl, checks Accessibility permission (macOS), registers the global hotkey, spawns the hotkey-forwarder thread, and hands off to `eframe::run_native`.

- [ ] **Step 12.1: Replace `src/main.rs`**

Replace `src/main.rs` entirely with:

```rust
use std::sync::Arc;

use clap::Parser;
use clipt9n::app::ClipApp;
use clipt9n::config::{Config, Modifier, NativeModifier};
use clipt9n::error::TranslateError;
use clipt9n::llm::anthropic::AnthropicProvider;
use clipt9n::llm::openai::OpenAiCompatibleProvider;
use clipt9n::llm::LlmProvider;
use clipt9n::platform::{self, Platform};
use clipt9n::secrets::{EnvSecrets, Secrets};
use clipt9n::Cli;
use directories::ProjectDirs;
use eframe::NativeOptions;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    if cli.action_or_none().is_some() {
        // CLI mode (M1 behavior): one-shot translation, then exit.
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(clipt9n::run())?;
        return Ok(());
    }

    // GUI mode.
    let cfg_path = ProjectDirs::from("", "", "clipboard-translator")
        .map(|d| d.config_dir().join("config.toml"))
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let cfg = Config::load(&cfg_path)?;
    let state_path = ProjectDirs::from("", "", "clipboard-translator")
        .map(|d| d.config_dir().join("state.toml"))
        .ok_or_else(|| anyhow::anyhow!("could not determine state path"))?;

    // Platform precondition (Accessibility on macOS, no-op elsewhere).
    let plat = platform::current();
    if let Err(e) = plat.ensure_hotkey_permissions() {
        tracing::error!(error = %e, "hotkey permission check failed");
        // Continue running so the user sees the System Settings prompt and
        // can grant + relaunch. Exit with non-zero so launchd doesn't loop.
        return Err(anyhow::anyhow!(e));
    }

    // Secrets resolution (M1 behavior: env-var only).
    let secrets: Box<dyn Secrets> = Box::new(EnvSecrets::new(cfg.provider.api_key.env_var.clone()));
    let api_key = secrets.get_api_key()?;
    let timeout = std::time::Duration::from_secs(cfg.provider.timeout_seconds);

    let provider: Arc<dyn LlmProvider> = match cfg.provider.kind.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(
            &cfg.provider.base_url,
            api_key,
            &cfg.provider.model,
            timeout,
        )?),
        "openai" | "gemini" | "ollama" => Arc::new(OpenAiCompatibleProvider::new(
            &cfg.provider.base_url,
            api_key,
            &cfg.provider.model,
            timeout,
        )?),
        other => return Err(anyhow::anyhow!(TranslateError::Config(format!(
            "unknown provider type '{other}'"
        )))),
    };

    // Hotkey registration.
    let manager = GlobalHotKeyManager::new()?;
    let modifier = Modifier::parse(&cfg.hotkey.modifier)
        .ok_or_else(|| anyhow::anyhow!("unknown hotkey modifier: {}", cfg.hotkey.modifier))?;
    let mut mods = match modifier.resolve_native() {
        NativeModifier::Ctrl => Modifiers::CONTROL,
        NativeModifier::Alt => Modifiers::ALT,
        NativeModifier::Meta => Modifiers::META,
    };
    if cfg.hotkey.shift {
        mods |= Modifiers::SHIFT;
    }
    let key_code = letter_to_code(&cfg.hotkey.key)
        .ok_or_else(|| anyhow::anyhow!("unsupported hotkey key: {}", cfg.hotkey.key))?;
    let hotkey = HotKey::new(Some(mods), key_code);
    if cfg.hotkey.enabled {
        manager.register(hotkey)?;
    }

    // Forward hotkey events from the global-hotkey channel into ours.
    // (Both are crossbeam — the global-hotkey crate's Receiver is the same
    // type our app polls. We don't need a separate forwarder if we hand
    // the receiver directly to the app. Use the global-hotkey receiver.)
    let hotkey_rx = GlobalHotKeyEvent::receiver().clone();

    // eframe options: hidden, undecorated, always-on-top, centered window.
    let inner_w = if cfg.ui.density == "compact" { 460.0 } else { 520.0 };
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([inner_w, 380.0])
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(false)
        .with_visible(false)
        .with_always_on_top()
        .with_active(true);
    let native_options = NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "clipt9n",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(ClipApp::new(
                cc,
                cfg,
                provider,
                secrets,
                state_path,
                hotkey_rx,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;

    // Keep `manager` alive until eframe returns. (Implicit; explicit drop
    // here for clarity.)
    drop(manager);
    Ok(())
}

fn letter_to_code(key: &str) -> Option<Code> {
    match key.to_ascii_uppercase().as_str() {
        "A" => Some(Code::KeyA), "B" => Some(Code::KeyB), "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD), "E" => Some(Code::KeyE), "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG), "H" => Some(Code::KeyH), "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ), "K" => Some(Code::KeyK), "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM), "N" => Some(Code::KeyN), "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP), "Q" => Some(Code::KeyQ), "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS), "T" => Some(Code::KeyT), "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV), "W" => Some(Code::KeyW), "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY), "Z" => Some(Code::KeyZ),
        _ => None,
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let _ = fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .try_init();
}
```

- [ ] **Step 12.2: Add `anyhow` to deps**

We use `anyhow` for `main()`'s error type because it composes cleanly with `eframe::run_native`'s string-y errors and avoids `From` impl boilerplate.

In `Cargo.toml`, add to `[dependencies]`:

```toml
anyhow = "1"
```

- [ ] **Step 12.3: Verify the build**

Run: `cargo build 2>&1 | tail -5`
Expected: clean build.

Run: `cargo test --all-features 2>&1 | tail -3`
Expected: all green (unit tests don't exercise `main`).

- [ ] **Step 12.4: Commit**

```bash
git add src/main.rs Cargo.toml Cargo.lock
git commit -m "feat(M2): main.rs eframe entry + global-hotkey registration"
```

---

## Task 13: Manual smoke test — the M2 exit criteria

**Files:**
- Modify: `README.md` (add a short "running the GUI" section; document Accessibility-permission prompt)

**Why:** The eight M2 exit criteria are inherently UI-dependent and cannot be unit-tested. We walk through each one manually and document any deviations.

> **For the engineer:** If a step fails, stop and either fix or file a TODO comment in the relevant module marking the deviation. Do not paper over failures.

- [ ] **Step 13.1: Build a release binary for testing**

Run: `cargo build --release 2>&1 | tail -3`
Expected: clean build.

- [ ] **Step 13.2: First-run Accessibility check**

If macOS hasn't seen this binary before, on first run the macOS Accessibility prompt should appear AND System Settings → Privacy & Security → Accessibility should open. Verify by:

```bash
ANTHROPIC_API_KEY="$(security find-generic-password -s anthropic-test -w 2>/dev/null || echo "$ANTHROPIC_API_KEY")" \
  ./target/release/clipt9n
```

Expected on first run:
- macOS shows "clipt9n would like to control this computer" dialog.
- System Settings opens to Accessibility pane.
- The terminal prints the `AccessibilityPermissionDenied` error and exits with non-zero.

Grant permission, relaunch.

**Exit criterion 6 verified.**

- [ ] **Step 13.3: Hotkey opens the window centered**

With `ANTHROPIC_API_KEY` set and the binary running:
1. Copy any text to clipboard (`pbcopy <<< "Hello, world."`).
2. Press Cmd+Shift+T.
3. **Expected:** centered, undecorated window appears, focused, with the "Translate clipboard" title and the clipboard preview.

**Exit criterion 1 verified.**

- [ ] **Step 13.4: Slot 1/2/3 runs end-to-end**

With the prompt window open and "Hello, world." on the clipboard:
1. Press `2` (translate to Deutsch).
2. **Expected:** window disappears, OS notification "Translation copied — Translate to Deutsch" appears within ~2s.
3. Run `pbpaste` — should print "Hallo, Welt." (or similar German rendering).

Repeat for slot 1 (en) and slot 3 (tr).

**Exit criterion 2 verified.**

- [ ] **Step 13.5: Enter repeats last action**

1. After Step 13.4 (slot 2 was last used), press Cmd+Shift+T again.
2. **Expected:** the prompt window shows the "LAST USED" badge on slot 2.
3. Press `Enter`.
4. **Expected:** translation runs against the new clipboard contents using slot 2.

**Exit criterion 3 verified.**

- [ ] **Step 13.6: Esc closes window**

1. Press Cmd+Shift+T.
2. Press `Esc`.
3. **Expected:** window disappears; no translation runs; binary still running.

**Exit criterion 4 verified.**

- [ ] **Step 13.7: Empty clipboard**

1. `pbcopy < /dev/null` (clear clipboard).
2. Press Cmd+Shift+T.
3. **Expected:** the empty-state UI renders ("Clipboard is empty or not text. Esc to dismiss"). Pressing 1–6 has no effect.

**Exit criterion 5 verified.**

- [ ] **Step 13.8: Visual fidelity check**

Compare the running window side-by-side with `handoff/clipt9n/project/Clipboard Translator.html` (open in browser, Storybook → Prompt Window). Document any deviations beyond:
- Font (Hack/Ubuntu vs Inter/JetBrains Mono — known M2 deviation).
- macOS traffic-light buttons (we emit our own title bar without traffic lights — close-with-Esc is the affordance).

Pixel-level deviations should match the design unless explicitly listed above. If anything more substantive is off (wrong color, wrong padding, wrong row order), file a fix before committing the README update.

**Exit criterion 6 verified.**

- [ ] **Step 13.9: Focus ring check**

With the prompt window open:
1. Press `Tab` to move focus through slots.
2. **Expected:** every focused slot row shows a 2px lime accent ring (the `widgets.active.bg_stroke` we set in `theme::visuals`).

**Exit criterion 7 verified.**

- [ ] **Step 13.10: AccessKit smoke (VoiceOver)**

1. Enable VoiceOver (Cmd+F5).
2. Open the prompt window.
3. **Expected:** VoiceOver announces "Translate clipboard, window" and reads each slot row label as you arrow through.

If VoiceOver reads nothing, AccessKit may not be reaching the system. Re-check `eframe`'s `accesskit` feature is enabled in `Cargo.toml` and confirm the build picks up the feature.

**Exit criterion 8 verified.**

- [ ] **Step 13.11: Update README**

Edit `README.md`. After the existing "Run" section (CLI usage), add:

```markdown
## Running the GUI (M2)

When invoked with no action flag, `clipt9n` launches in GUI mode:

```bash
ANTHROPIC_API_KEY=sk-ant-... clipt9n
```

The app stays running in the background. Press **Cmd+Shift+T** (default; configurable in `[hotkey]`) to summon the prompt window. Pick a numbered slot (1–3 for translation, 4 for grammar fix, 5 for rewrite, 6 is custom — wired in M3). Press `Enter` to repeat your last action; `Esc` to dismiss.

### macOS Accessibility permission

Global hotkey registration on macOS requires Accessibility permission. On first launch, clipt9n triggers a one-time grant flow:

1. macOS shows the system permission dialog.
2. System Settings opens to Privacy & Security → Accessibility.
3. Toggle **clipt9n** on.
4. Relaunch the binary.

Without this permission, `Cmd+Shift+T` will not be detected and the app exits with `AccessibilityPermissionDenied`.

### Limitations (M2)

- Slot 6 (custom prompt) renders but is no-op; M3 wires it.
- Translation in flight is signaled only by the OS notification on completion — the in-window "Translating…" overlay arrives in M3.
- Bundled fonts (Inter / JetBrains Mono per the design handoff) are deferred to M8 polish; M2 uses egui's defaults.
```

- [ ] **Step 13.12: Commit**

```bash
git add README.md
git commit -m "docs(M2): GUI usage, Accessibility prompt, and known limitations"
```

---

## Task 14: Final verification + push readiness

- [ ] **Step 14.1: Run full test + lint suite**

Run, in parallel where possible:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all three green.

- [ ] **Step 14.2: Verify cross-platform discipline**

Run: `grep -rn "cfg(target_os" src/ | grep -v "src/platform/"`

Expected output: a single line in `src/config.rs` (the documented exception in `Modifier::resolve_native`). If anything else appears, refactor it into `src/platform/`.

If the engineer chose to put `Modifier::resolve_native` inside `platform/mod.rs` (alternative noted in Task 4 Step 4.3), this grep should return zero lines.

- [ ] **Step 14.3: Inspect commit history**

Run: `git log --oneline main..m2-prompt-window`

Expected: ~12–14 commits, one per task with conventional `feat(M2): …`, `chore(M2): …`, `docs(M2): …` prefixes.

- [ ] **Step 14.4: Ready for handoff**

M2 is complete on `m2-prompt-window`. Recommended next steps (do **not** execute unless asked):
1. Push branch: `git push -u origin m2-prompt-window`.
2. Open PR against `main` titled `M2: prompt window + global hotkey + design tokens`.
3. CI runs the 5-target compile matrix + macOS test job. Wait for green before merging.
4. Begin M3 from `main` after merge.

---

## Spec coverage check

Mapping each M2 exit criterion (from `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md` lines 79–102) and each deliverable to a task in this plan:

| M2 deliverable | Tasks |
|---|---|
| `src/main.rs` event loop with `eframe`, `accesskit` enabled | Task 1 (feature), Task 12 |
| `src/ui/theme.rs` (a11y-corrected palette, `Visuals`, `kbd`, `WindowFrame`) | Task 8 |
| `src/ui/prompt.rs` (460/520px, preview, slots, footer, glossary chip area) | Tasks 9, 11 |
| Number keys 1–6, Enter, Esc | Task 11 |
| `src/platform/mod.rs` + `platform/macos.rs` (Accessibility detection) | Tasks 5, 6 |
| `global-hotkey` with config-driven mods + Cmd↔Ctrl helper | Tasks 3, 4, 12 |
| `notify-rust` "Translation copied" toast | Tasks 10, 11 |
| Always-on-top, no-decorations, centered | Task 12 (`NativeOptions`) |
| `state.toml` last-action persistence (slots only) | Tasks 7, 11 |

| M2 exit criterion | Verified by |
|---|---|
| 1. Hotkey opens window centered, focused | Step 13.3 |
| 2. Slots 1/2/3 run end-to-end with toast | Step 13.4 |
| 3. Enter repeats last slot | Step 13.5 |
| 4. Esc closes window | Step 13.6 |
| 5. Empty clipboard shows empty state | Step 13.7 |
| 6. Visual matches design | Step 13.8 |
| 7. Visible focus ring on every focusable element | Step 13.9 + Task 8 (`visuals` builder) |
| 8. AccessKit reports labels for slots + close | Step 13.10 + Task 1 (eframe feature) |

---

## Open implementation-level questions

These are flagged in the design doc and **do not block M2**:

- Bundled Inter / JetBrains Mono fonts via `include_bytes!` — deferred to M8 polish (documented in Task 13.11 README update).
- Reduced-motion handling — only relevant when the translating overlay lands in M3.
- `whatlang` confidence threshold — M4 concern.

## Things M2 deliberately does not do

(Save the M3 engineer ten minutes of "wait, why isn't this here.")

- No translating overlay (M3).
- No custom prompt window (M3).
- No glossary chip render — area reserved but always empty (M4).
- No history viewer (M5).
- No setup wizard / keychain (M6).
- No tray (M7).
- No bundled design fonts (M8).
- No oversized-clipboard confirmation modal (M3).
- No reduced-motion handling (M3).
