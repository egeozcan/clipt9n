# clipt9n M3 — Slot 6 + Translating Overlay + Size Confirm + Reduced Motion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the spec's six-slot menu by wiring slot 6 to a custom-prompt window; show the design's "Translating…" overlay (with reduced-motion fallback) for any in-flight action; gate oversized clipboards behind a confirmation modal; and honor the existing `[ui] show_preview` config flag.

**Architecture:** Three new pure render layers (`ui/custom_prompt.rs`, `ui/translating.rs`, `ui/size_confirm.rs`) — each is a stateless `draw(ctx, model) -> Outcome` function in the M2 pattern. The single viewport stays put; **no new viewports** are introduced (per the M2 hand-off recommendation). The state machine in `src/app.rs` grows three states (`EnteringCustom`, `ConfirmingSize`, `Translating { … payload }`) and a `dispatch_gen: u64` counter so user cancellation can drop a still-in-flight outcome. A new `Platform::reduced_motion()` trait method (macOS impl shells out to `defaults read -g NSReduceMotionEnabled`; Linux/Windows use the default `false`) is queried once at app construction and cached.

**Tech Stack:** Rust 2021 / eframe 0.31 / egui 0.31 (already pinned). No new external crates. M3 is purely additions on top of the M2 wiring.

> **Branch:** This plan executes on `m3-actions-and-overlay`, branched from `main` after M2 was fast-forwarded onto it. Working directory: `/Users/egecan/Code/clipt9n`.

---

## File structure

After M3, the tree gains the following (relative to repo root):

```
src/
├── app.rs                       ← MODIFIED: AppState extended; gen counter; new dispatch paths
├── config.rs                    ← MODIFIED: UiConfig.confirm_size_threshold (default 2000)
├── platform/
│   ├── mod.rs                   ← MODIFIED: Platform::reduced_motion() trait method (default false)
│   └── macos.rs                 ← MODIFIED: macOS reduced_motion impl
├── ui/
│   ├── mod.rs                   ← MODIFIED: pub mod custom_prompt, translating, size_confirm
│   ├── prompt.rs                ← MODIFIED: gate preview block on cfg.ui.show_preview;
│   │                                         large-paste warning reads cfg.ui.confirm_size_threshold
│   ├── custom_prompt.rs         ← NEW: slot-6 instruction window
│   ├── translating.rs           ← NEW: in-flight overlay (animated bar + static fallback)
│   └── size_confirm.rs          ← NEW: "send X chars to API?" modal
README.md                        ← MODIFIED: M3 section (custom prompt, overlay, size threshold, reduced motion)
```

Boundary discipline (unchanged from M2):
- `src/platform/` is the **only** place `#[cfg(target_os = …)]` and `#[cfg(unix)]` may appear (with the single audited exception in `config::Modifier::resolve_native`).
- `src/ui/` knows nothing about `tokio`, `reqwest`, or platform specifics — it only paints frames and emits intents.
- `src/app.rs` is the seam between egui (sync) and tokio (async).

---

## Glossary of cross-cutting decisions (read once)

These come up repeatedly; agreeing on them up front prevents drift.

1. **Single viewport, alternate UIs by `AppState`.** Per the user-confirmed approach, M3 does NOT spawn additional egui viewports. The existing `ViewportBuilder` (created in `main.rs`) renders whichever view matches the current `AppState`: prompt window, custom prompt window, size-confirm modal, or translating overlay. The `ViewportCommand::Visible(want_visible)` defensive re-assertion in `app.rs::update()` simply changes from `matches!(state, Showing)` to `!matches!(state, Idle)`.

2. **Viewport size stays at 520×470 (or 460×470 in compact density).** Both new windows fit inside this. The translating overlay paints at the top of the body with extra trailing whitespace; the custom prompt fills the body. We do NOT issue `ViewportCommand::InnerSize` because resizing has the same flakiness profile as the `Visible(false)` issue from M2.

3. **Cancellation = generation counter.** Translation requests can take seconds; users can press Esc / Cancel mid-flight. Rather than try to abort the in-flight `provider.complete().await` (which would require restructuring the trait), we increment `App.dispatch_gen: u64` on every dispatch. The spawned task captures the gen at dispatch time and tags its `TranslationOutcome` with it; on receive, mismatched gens are dropped. The HTTP request continues to its natural conclusion (≤30s) and its result is ignored. **No new cancellation crate, no new tokio plumbing.**

4. **`reduced_motion` is queried once at startup, cached on `App`.** Don't re-query per frame. The macOS impl shells out to `defaults read -g NSReduceMotionEnabled`; failures (key unset, command missing) fall back to `false`.

5. **Both new render layers stay pure.** `ui/custom_prompt.rs::draw` returns `Option<CustomPromptOutcome>` (Submit, Cancel). `ui/translating.rs::draw` returns `Option<TranslatingOutcome>` (Cancel). `ui/size_confirm.rs::draw` returns `Option<SizeConfirmOutcome>` (Confirm, Cancel). All event handling and state mutation lives in `app.rs::update()`, identically to how `prompt.rs` works in M2.

6. **The size threshold is a single source of truth.** `cfg.ui.confirm_size_threshold: usize` (default 2000) controls BOTH the prompt-window footer's `⚠ large paste` indicator AND the confirm modal's trigger. The hardcoded `2000` literal at `src/ui/prompt.rs:283` becomes `cfg.ui.confirm_size_threshold`.

7. **Slot 4/5 are already wired in M2** (`src/app.rs:155-156`). M3 does NOT touch them; this plan focuses exclusively on slot 6, the overlay, the size-confirm modal, the preview-config flag, and reduced motion.

8. **Custom prompts are NEVER persisted.** Spec privacy rule. `state.toml` already only stores `last_slot` and refuses values outside 1–6 — but for slot 6, the *slot* is recorded (so Enter → repeat repeats opening the custom prompt window with empty textarea), while the *instruction text* is dropped on dismiss. The `CustomPromptModel` is reconstructed empty on every entry to `EnteringCustom`.

9. **Hotkey while busy.** While in `EnteringCustom`, `ConfirmingSize`, or `Translating`, hotkey events bring the active window to the foreground (`ViewportCommand::Focus`) but do not change state. This matches the existing M2 behavior in `drain_channels`. Esc in any of those states transitions to `Idle`.

10. **Action-label vs overlay-label.** The OS notification on completion uses the M2-style noun phrasing ("Translate to Deutsch", "Fix grammar"). The overlay title uses verb-form per the design ("Translating to Deutsch…", "Fixing grammar…", "Rewriting for clarity…", "Running custom prompt…"). Both labels are derived once at dispatch time from the `Action` and stored in `AppState::Translating { action_label, overlay_label, … }`.

---

## Pre-flight: Confirm starting state

- [ ] **Step 0.1: Verify branch and clean tree**

Run:
```bash
git rev-parse --abbrev-ref HEAD
git status --short
```
Expected: branch `m3-actions-and-overlay`, no working-tree changes.

- [ ] **Step 0.2: Verify M2 tests pass on this branch**

Run: `cargo test --all-features 2>&1 | grep "test result:"`
Expected: lines totaling **80 passed; 0 failed** across the lib, integration, and doctest test runs.

If either step fails, stop and report.

---

## Task 1: Add `confirm_size_threshold` to `[ui]` config

**Files:**
- Modify: `src/config.rs:125-140` (`UiConfig` struct + Default impl + tests at end of file)

**Why:** The size-confirm modal (Task 6 + 9) and the prompt-window's `⚠ large paste` indicator (Task 2) both need to read this value. Exposing it as a config key follows the spec §6 convention.

- [ ] **Step 1.1: Write the failing tests**

Append to `src/config.rs`'s `tests` mod (after `default_ui_density_is_normal`, around line 335):

```rust
#[test]
fn default_confirm_size_threshold_is_2000() {
    let cfg = Config::default();
    assert_eq!(cfg.ui.confirm_size_threshold, 2000);
}

#[test]
fn loads_confirm_size_threshold_override() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(
        f,
        r#"
[ui]
confirm_size_threshold = 5000
"#
    )
    .unwrap();
    let cfg = Config::load(f.path()).unwrap();
    assert_eq!(cfg.ui.confirm_size_threshold, 5000);
}
```

- [ ] **Step 1.2: Run tests to verify failure**

Run: `cargo test --lib config 2>&1 | tail -10`
Expected: compilation error on `cfg.ui.confirm_size_threshold` (field doesn't exist).

- [ ] **Step 1.3: Add the field to `UiConfig`**

In `src/config.rs`, modify the `UiConfig` struct (around line 125) to add the new field:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// "normal" or "compact". Drives prompt window width (520 vs 460).
    pub density: String,
    pub show_preview: bool,
    /// Above this character count, dispatch shows a confirm modal before
    /// sending the clipboard to the API. Spec §6 default is 2000.
    pub confirm_size_threshold: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            density: "normal".into(),
            show_preview: true,
            confirm_size_threshold: 2000,
        }
    }
}
```

- [ ] **Step 1.4: Run tests to verify pass**

Run: `cargo test --lib config 2>&1 | tail -10`
Expected: all config tests pass (12 total in this module after the additions).

- [ ] **Step 1.5: Commit**

```bash
git add src/config.rs
git commit -m "feat(M3): [ui] confirm_size_threshold config (default 2000)"
```

---

## Task 2: Honor `[ui] show_preview` and `confirm_size_threshold` in the prompt window

**Files:**
- Modify: `src/ui/prompt.rs:191-216` (preview block) and `src/ui/prompt.rs:282-291` (large-paste warning)

**Why:** M2 hardcoded both: the preview block always renders, and the warning fires at >2000 chars regardless of config. Spec §6 promises both are config-driven; M3 connects them.

- [ ] **Step 2.1: Write the failing tests**

Append to `src/ui/prompt.rs`'s `tests` mod (around line 449):

```rust
#[test]
fn slot_strings_unchanged_when_show_preview_false() {
    // show_preview is a draw-layer concern (whether the preview block renders);
    // slot label resolution is independent. Sanity-check that toggling the
    // config flag does not change slot string output.
    let mut cfg = Config::default();
    cfg.ui.show_preview = false;
    let (label, code) = slot_strings(SLOTS[0], &cfg);
    assert_eq!(label, "English");
    assert_eq!(code, "en");
}

#[test]
fn large_paste_threshold_uses_config_value() {
    // The threshold helper used by the footer warning reads from cfg.
    let mut cfg = Config::default();
    cfg.ui.confirm_size_threshold = 100;
    assert!(should_warn_large_paste("x".repeat(101).as_str(), &cfg));
    assert!(!should_warn_large_paste("x".repeat(99).as_str(), &cfg));
}
```

- [ ] **Step 2.2: Run tests to verify failure**

Run: `cargo test --lib ui::prompt 2>&1 | tail -10`
Expected: compilation error on `should_warn_large_paste` (function doesn't exist).

- [ ] **Step 2.3: Add the helper and gate the preview block**

In `src/ui/prompt.rs`, add this helper after the existing `preview_text` function (around line 414):

```rust
/// Returns true when the clipboard text exceeds the configured size
/// threshold and the prompt window should show a `⚠ large paste` indicator.
/// Same threshold gates the size-confirm modal in `app.rs`.
pub fn should_warn_large_paste(text: &str, cfg: &Config) -> bool {
    text.chars().count() > cfg.ui.confirm_size_threshold
}
```

In `src/ui/prompt.rs`, find the preview block (the `egui::Frame::new()` block starting at the comment `// ----- Preview block -----`, around line 191) and wrap it in a conditional:

```rust
            // ----- Preview block -----
            if cfg.ui.show_preview {
                let preview = preview_text(&model.clipboard_text);
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(12, 10))
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
            }
```

In `src/ui/prompt.rs`, find the large-paste indicator (around line 283) and replace the hardcoded `> 2000` check:

```rust
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if should_warn_large_paste(&model.clipboard_text, cfg) {
                        ui.label(
                            RichText::new("⚠ large paste")
                                .color(theme::WARN)
                                .monospace()
                                .size(11.0),
                        );
                    }
                });
```

- [ ] **Step 2.4: Run tests to verify pass**

Run: `cargo test --lib ui::prompt 2>&1 | tail -10`
Expected: all `ui::prompt` tests pass (6 total after additions).

Run a full build to confirm nothing else broke: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

- [ ] **Step 2.5: Commit**

```bash
git add src/ui/prompt.rs
git commit -m "feat(M3): honor cfg.ui.show_preview + confirm_size_threshold in prompt"
```

---

## Task 3: `Platform::reduced_motion()` trait method + macOS impl

**Files:**
- Modify: `src/platform/mod.rs:10-18` (`Platform` trait body) and `tests` mod at end
- Modify: `src/platform/macos.rs:28-39` (`MacOsPlatform` impl) and `tests` mod at end

**Why:** The translating overlay (Task 5) needs to render a static label when the user has macOS Reduce Motion enabled, per the WCAG 2.3.3 a11y baseline in the design doc. We add the query as a `Platform` trait method with a `false` default — Linux/Windows automatically get the no-op behavior.

- [ ] **Step 3.1: Write the failing tests**

Append to `src/platform/mod.rs`'s `tests` mod (around line 56):

```rust
#[test]
fn default_reduced_motion_is_false() {
    struct Stub;
    impl Platform for Stub {}
    assert!(!Stub.reduced_motion());
}

#[test]
fn current_platform_reduced_motion_does_not_panic() {
    // Whatever the OS reports, we just need a clean call.
    let _ = current().reduced_motion();
}
```

Append to `src/platform/macos.rs`'s `tests` mod (around line 60):

```rust
#[test]
fn parse_defaults_output_handles_known_values() {
    assert!(parse_reduce_motion_output("1\n"));
    assert!(parse_reduce_motion_output(" 1 "));
    assert!(!parse_reduce_motion_output("0\n"));
    assert!(!parse_reduce_motion_output("garbage"));
    assert!(!parse_reduce_motion_output(""));
}

#[test]
fn macos_reduced_motion_does_not_panic() {
    let _ = MacOsPlatform.reduced_motion();
}
```

- [ ] **Step 3.2: Run tests to verify failure**

Run: `cargo test --lib platform 2>&1 | tail -10`
Expected: compilation errors on `Stub.reduced_motion()`, `MacOsPlatform.reduced_motion()`, and `parse_reduce_motion_output` (none exist).

- [ ] **Step 3.3: Add the trait method to `Platform`**

In `src/platform/mod.rs`, modify the `Platform` trait (around line 10) to add the new method:

```rust
pub trait Platform {
    /// Verify the OS-level prerequisites for registering a global hotkey.
    /// On macOS this checks Accessibility permission. On Linux/Windows this
    /// is a no-op. Returns an error with user-actionable messaging if the
    /// prereq is missing.
    fn ensure_hotkey_permissions(&self) -> Result<(), TranslateError> {
        Ok(())
    }

    /// Whether the user has requested reduced motion at the OS level.
    /// Default `false`. macOS implements via `defaults read -g NSReduceMotionEnabled`;
    /// other OSes use the default until a per-platform query is added.
    fn reduced_motion(&self) -> bool {
        false
    }
}
```

- [ ] **Step 3.4: Implement `reduced_motion` for macOS**

In `src/platform/macos.rs`, add inside `impl Platform for MacOsPlatform` (after `ensure_hotkey_permissions`, around line 39):

```rust
    fn reduced_motion(&self) -> bool {
        match Command::new("defaults")
            .args(["read", "-g", "NSReduceMotionEnabled"])
            .output()
        {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout);
                parse_reduce_motion_output(&s)
            }
            // Any failure (key unset, defaults missing, sandbox denied)
            // → assume reduce-motion is off. Spec a11y baseline accepts
            // false-negative > false-positive here.
            _ => false,
        }
    }
```

In the same file, append the parser helper (after `open_accessibility_settings`, around line 54):

```rust
/// Parse `defaults read -g NSReduceMotionEnabled` output. Treats "1" as
/// true; anything else (including missing-key, "0", garbage) as false.
fn parse_reduce_motion_output(s: &str) -> bool {
    s.trim() == "1"
}
```

- [ ] **Step 3.5: Run tests to verify pass**

Run: `cargo test --lib platform 2>&1 | tail -10`
Expected: all platform tests pass (4 in `mod.rs`'s tests, 3 in `macos.rs`'s).

- [ ] **Step 3.6: Commit**

```bash
git add src/platform/mod.rs src/platform/macos.rs
git commit -m "feat(M3): Platform::reduced_motion() trait method + macOS defaults query"
```

---

## Task 4: New `ui/custom_prompt.rs` — slot-6 instruction window (render layer only)

**Files:**
- Create: `src/ui/custom_prompt.rs`
- Modify: `src/ui/mod.rs` to declare `pub mod custom_prompt;`

**Why:** The custom prompt window is the design's `custom-prompt.jsx`: instruction textarea, preset chips, preview block, footer with `⌘+↵ run · Esc cancel` and a `Run →` button. This task implements the render layer + outcome enum. Wiring (state machine entry/exit, focus, key dispatch) is Task 8.

- [ ] **Step 4.1: Write the failing tests**

Create `src/ui/custom_prompt.rs` with this content (test included):

```rust
//! Slot-6 custom prompt window. Renders the design's `custom-prompt.jsx`.
//! Pure render layer: no event handling — `app.rs` owns Cmd+Enter / Esc /
//! preset clicks. The view is a function of `CustomPromptModel`.

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// Hardcoded list per design `custom-prompt.jsx`. Order is meaningful
/// (rendered left-to-right, wrapped). Edit only with design approval.
pub const PRESETS: &[&str] = &[
    "translate to formal Spanish",
    "make this sound more diplomatic",
    "explain like I'm five",
    "summarize in one sentence",
    "convert to bullet points",
];

/// Mutable state of the custom prompt window. Owned by `App`; reset to
/// default on every entry to `EnteringCustom` so a previously-entered
/// instruction is never recalled (spec privacy rule).
#[derive(Debug, Clone, Default)]
pub struct CustomPromptModel {
    /// Source clipboard text (read-only; rendered in the preview block).
    pub clipboard_text: String,
    /// Live editor contents.
    pub instruction: String,
    /// Set to `true` on entry; the renderer clears it after calling
    /// `request_focus` once. Without this, the textarea would refocus on
    /// every frame and steal focus from preset buttons.
    pub focus_textarea_next_frame: bool,
}

/// Click-dispatched outcomes. Enter / Esc are caller-handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomPromptOutcome {
    /// User clicked `Run →` or pressed Cmd+Enter (caller emits the latter).
    Submit,
    /// User clicked outside / pressed a preset chip (selects it as text).
    PresetPicked(usize),
}

/// Truncate clipboard text to ≤200 chars with an ellipsis, matching the
/// design's `slice(0, 200) + "…"` rule.
pub fn preview_truncate(text: &str) -> String {
    if text.chars().count() <= 200 {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(200).collect();
        out.push('…');
        out
    }
}

/// Returns true when the instruction (after trim) is non-empty — `Run →`
/// is enabled iff this returns true.
pub fn submit_enabled(instruction: &str) -> bool {
    !instruction.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_under_limit_returns_unchanged() {
        let s = "short input";
        assert_eq!(preview_truncate(s), s);
    }

    #[test]
    fn preview_over_limit_truncates_with_ellipsis() {
        let s = "x".repeat(300);
        let p = preview_truncate(&s);
        assert_eq!(p.chars().count(), 201); // 200 + ellipsis
        assert!(p.ends_with('…'));
    }

    #[test]
    fn submit_disabled_for_empty_instruction() {
        assert!(!submit_enabled(""));
        assert!(!submit_enabled("   "));
        assert!(!submit_enabled("\n\t"));
    }

    #[test]
    fn submit_enabled_for_nontrivial_instruction() {
        assert!(submit_enabled("translate to formal Spanish"));
        assert!(submit_enabled("  hello  "));
    }

    #[test]
    fn presets_are_five_items() {
        // Keep the count locked so that any future addition is a deliberate
        // design change, not an accidental one (renderer wraps; layout is
        // calibrated for ~5 chips).
        assert_eq!(PRESETS.len(), 5);
    }

    #[test]
    fn custom_prompt_model_default_is_empty() {
        let m = CustomPromptModel::default();
        assert!(m.clipboard_text.is_empty());
        assert!(m.instruction.is_empty());
        assert!(!m.focus_textarea_next_frame);
    }
}
```

In `src/ui/mod.rs`, add `pub mod custom_prompt;` (alphabetical order):

```rust
pub mod custom_prompt;
pub mod prompt;
pub mod theme;
```

- [ ] **Step 4.2: Run tests to verify pass**

Run: `cargo test --lib ui::custom_prompt 2>&1 | tail -10`
Expected: 6 tests pass.

(The `draw` function is added in Step 4.3 — the tests above only cover pure helpers, which is the testable surface. The renderer is exercised manually + by Task 8's wiring.)

- [ ] **Step 4.3: Add the `draw` function**

Append to `src/ui/custom_prompt.rs` (after the helpers, before `#[cfg(test)]`):

```rust
/// Render the custom prompt window. Returns `Some(CustomPromptOutcome)` on
/// click events; keyboard handling (Cmd+Enter, Esc) lives in `App::update`.
/// Mutates `model.instruction` to reflect TextEdit contents and may set
/// `model.focus_textarea_next_frame = false` after first focus call.
pub fn draw(
    ctx: &egui::Context,
    model: &mut CustomPromptModel,
) -> Option<CustomPromptOutcome> {
    let mut clicked: Option<CustomPromptOutcome> = None;
    theme::window_frame(ctx, "Custom prompt", Some("clipt9n · slot 6"), |ui| {
        let body_padding = egui::Margin::symmetric(18, 14);
        egui::Frame::new()
            .inner_margin(body_padding)
            .show(ui, |ui| {
                // ----- "Instruction" label -----
                ui.label(
                    RichText::new("INSTRUCTION")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.add_space(4.0);

                // ----- Multi-line text editor -----
                let editor = egui::TextEdit::multiline(&mut model.instruction)
                    .hint_text("e.g. \"translate to formal Spanish\"")
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY);
                let response = ui.add(editor);
                if model.focus_textarea_next_frame {
                    response.request_focus();
                    model.focus_textarea_next_frame = false;
                }
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        true,
                        "Custom prompt instruction",
                    )
                });

                ui.add_space(8.0);

                // ----- Preset chips -----
                ui.horizontal_wrapped(|ui| {
                    for (i, preset) in PRESETS.iter().enumerate() {
                        let chip_resp = chip(ui, preset);
                        if chip_resp.clicked() {
                            clicked = Some(CustomPromptOutcome::PresetPicked(i));
                        }
                        chip_resp.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                preset,
                            )
                        });
                    }
                });

                ui.add_space(14.0);

                // ----- "Will be applied to" preview -----
                ui.label(
                    RichText::new("WILL BE APPLIED TO")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.add_space(4.0);
                let preview = preview_truncate(&model.clipboard_text);
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if preview.is_empty() { " " } else { &preview })
                                .color(theme::INK_2)
                                .monospace()
                                .size(12.0),
                        );
                    });

                ui.add_space(12.0);

                // ----- Footer: hint + Run button -----
                let sep_rect =
                    egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 1.0));
                ui.painter().hline(
                    sep_rect.x_range(),
                    sep_rect.center().y,
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::kbd(ui, "⌘");
                    ui.label(RichText::new("+").color(theme::INK_3).size(11.0));
                    theme::kbd(ui, "↵");
                    ui.label(
                        RichText::new("run ·")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    theme::kbd(ui, "Esc");
                    ui.label(
                        RichText::new("cancel")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = submit_enabled(&model.instruction);
                        if run_button(ui, enabled).clicked() && enabled {
                            clicked = Some(CustomPromptOutcome::Submit);
                        }
                    });
                });
            });
    });
    clicked
}

/// Render a single preset chip. Click selects it as the instruction text.
fn chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::monospace(11.0), theme::INK_2)
        .size();
    let padding = Vec2::new(9.0, 3.0);
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::PANEL_3
        } else {
            Color32::from_rgba_unmultiplied(0x23, 0x27, 0x2f, 0xcc)
        };
        ui.painter().rect_filled(rect, 999.0, bg);
        ui.painter().rect_stroke(
            rect,
            999.0,
            Stroke::new(1.0, theme::LINE),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(11.0),
            theme::INK_2,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Render the primary `Run →` action button. When `enabled` is false, paints
/// a disabled style and a non-click response.
fn run_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let label = "Run →";
    let padding = Vec2::new(14.0, 7.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.5), theme::ACCENT_INK)
        .size();
    let desired = galley_size + padding * 2.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(desired, sense);
    if ui.is_rect_visible(rect) {
        let (bg, fg) = if !enabled {
            (theme::PANEL_3, theme::INK_3)
        } else if response.hovered() {
            (theme::ACCENT.gamma_multiply(0.92), theme::ACCENT_INK)
        } else {
            (theme::ACCENT, theme::ACCENT_INK)
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                6.0,
                Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5).clone(),
            fg,
        );
    }
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Run custom prompt")
    });
    response
}
```

- [ ] **Step 4.4: Verify it builds**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

Run: `cargo test --lib ui::custom_prompt 2>&1 | tail -10`
Expected: 6 tests pass.

- [ ] **Step 4.5: Commit**

```bash
git add src/ui/custom_prompt.rs src/ui/mod.rs
git commit -m "feat(M3): custom prompt window render layer (slot 6)"
```

---

## Task 5: New `ui/translating.rs` — in-flight overlay (animated bar + static fallback)

**Files:**
- Create: `src/ui/translating.rs`
- Modify: `src/ui/mod.rs` to declare `pub mod translating;`

**Why:** Per design `prompt-window.jsx::TranslatingWindow` (lines 247-287 of the handoff): a 16-cell lime sweep bar plus an elapsed-time counter, with a Cancel affordance. When OS Reduce Motion is enabled, the bar is replaced with a static "Translating…" label per the WCAG 2.3.3 a11y baseline.

- [ ] **Step 5.1: Write the failing tests**

Create `src/ui/translating.rs` with this content:

```rust
//! In-flight translation overlay. Renders the design's `TranslatingWindow`
//! (animated lime sweep bar + elapsed-time counter + Cancel button), or a
//! static label when reduced motion is enabled. Pure render layer; key
//! handling (Esc → Cancel) lives in `App::update`.

use std::time::Duration;

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// Number of cells in the animated sweep bar. Matches the design's
/// `cells = 16`.
pub const BAR_CELLS: usize = 16;

/// Animation tick interval. Matches the design's `setInterval(80ms)`.
pub const TICK_MS: u64 = 80;

/// Mutable state of the translating overlay. Owned by `App`; constructed
/// at dispatch time and dropped on outcome / cancel.
#[derive(Debug, Clone)]
pub struct TranslatingModel {
    /// Verb-form label per design ("Translating to Deutsch…", "Fixing
    /// grammar…", "Rewriting for clarity…", "Running custom prompt…").
    pub overlay_label: String,
    /// Provider model identifier, shown in the title-bar subtitle.
    pub provider_model: String,
    /// Elapsed wall-time since dispatch.
    pub elapsed: Duration,
    /// User has reduced motion enabled at OS level.
    pub reduced_motion: bool,
}

/// Click-dispatched outcomes. Esc is caller-handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslatingOutcome {
    Cancel,
}

/// Compute the per-cell opacities for the sweep bar at a given elapsed time.
/// The "head" of the sweep cycles through cells; cells within 4 of the head
/// are progressively dimmer. Returns one `f32 ∈ [0.15, 1.0]` per cell.
pub fn compute_bar_opacities(elapsed: Duration, cells: usize) -> Vec<f32> {
    let tick = (elapsed.as_millis() as u64 / TICK_MS) as usize;
    let head = tick % cells.max(1);
    (0..cells)
        .map(|i| {
            let dist = (i + cells - head) % cells;
            let intensity = if dist < 4 {
                (4 - dist) as f32 / 4.0
            } else {
                0.0
            };
            0.15 + intensity * 0.85
        })
        .collect()
}

/// Format elapsed time as the design's `(tick * 80 / 1000).toFixed(1)` —
/// e.g., "0.4s", "1.2s".
pub fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_millis() as f32 / 1000.0;
    format!("{secs:.1}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacities_have_correct_length() {
        let v = compute_bar_opacities(Duration::from_millis(0), BAR_CELLS);
        assert_eq!(v.len(), BAR_CELLS);
    }

    #[test]
    fn opacities_are_clamped_in_range() {
        for ms in [0, 80, 240, 1280, 5000] {
            let v = compute_bar_opacities(Duration::from_millis(ms), BAR_CELLS);
            for o in v {
                assert!((0.15..=1.0).contains(&o), "opacity {o} out of range");
            }
        }
    }

    #[test]
    fn opacities_have_a_bright_head_at_t0() {
        let v = compute_bar_opacities(Duration::from_millis(0), BAR_CELLS);
        // At tick=0 the head is at index 0; cell 0 should be the brightest.
        assert!((v[0] - 1.0).abs() < 0.01, "expected head at index 0 to be 1.0, got {}", v[0]);
        // Cells far from head should be at minimum (0.15).
        assert!((v[8] - 0.15).abs() < 0.01);
    }

    #[test]
    fn head_advances_with_time() {
        let t0 = compute_bar_opacities(Duration::from_millis(0), BAR_CELLS);
        let t1 = compute_bar_opacities(Duration::from_millis(TICK_MS), BAR_CELLS);
        // The head moved one cell, so the brightest cell index shifted.
        let bright = |v: &[f32]| {
            v.iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap()
        };
        assert_eq!(bright(&t0), 0);
        assert_eq!(bright(&t1), 1);
    }

    #[test]
    fn format_elapsed_renders_one_decimal() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "0.0s");
        assert_eq!(format_elapsed(Duration::from_millis(450)), "0.5s"); // rounds via :.1
        assert_eq!(format_elapsed(Duration::from_secs(2)), "2.0s");
        assert_eq!(format_elapsed(Duration::from_millis(12345)), "12.3s");
    }
}
```

In `src/ui/mod.rs`, add the new module declaration alphabetically:

```rust
pub mod custom_prompt;
pub mod prompt;
pub mod theme;
pub mod translating;
```

- [ ] **Step 5.2: Run tests to verify pass**

Run: `cargo test --lib ui::translating 2>&1 | tail -10`
Expected: 5 tests pass.

- [ ] **Step 5.3: Add the `draw` function**

Append to `src/ui/translating.rs` (after `format_elapsed`, before `#[cfg(test)]`):

```rust
/// Render the overlay. Returns `Some(TranslatingOutcome::Cancel)` if the
/// user clicked Cancel; `None` otherwise.
pub fn draw(ctx: &egui::Context, model: &TranslatingModel) -> Option<TranslatingOutcome> {
    let mut clicked: Option<TranslatingOutcome> = None;
    theme::window_frame(
        ctx,
        &model.overlay_label,
        Some(&model.provider_model),
        |ui| {
            let body_padding = egui::Margin::symmetric(18, 16);
            egui::Frame::new()
                .inner_margin(body_padding)
                .show(ui, |ui| {
                    if model.reduced_motion {
                        // Static fallback per WCAG 2.3.3.
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Translating…")
                                .color(theme::INK)
                                .size(13.5),
                        );
                        ui.add_space(8.0);
                    } else {
                        draw_animated_bar(ui, model);
                    }

                    // Meta row: endpoint + elapsed.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("request → api endpoint")
                                .color(theme::INK_3)
                                .monospace()
                                .size(11.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_elapsed(model.elapsed))
                                    .color(theme::INK_3)
                                    .monospace()
                                    .size(11.0),
                            );
                        });
                    });

                    ui.add_space(14.0);
                    let sep_rect =
                        egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 1.0));
                    ui.painter().hline(
                        sep_rect.x_range(),
                        sep_rect.center().y,
                        Stroke::new(1.0, theme::LINE_SOFT),
                    );
                    ui.add_space(10.0);

                    // Footer: hint + Cancel button.
                    ui.horizontal(|ui| {
                        theme::kbd(ui, "Esc");
                        ui.label(
                            RichText::new("cancel")
                                .color(theme::INK_3)
                                .monospace()
                                .size(11.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if cancel_button(ui).clicked() {
                                clicked = Some(TranslatingOutcome::Cancel);
                            }
                        });
                    });
                });
        },
    );
    clicked
}

fn draw_animated_bar(ui: &mut egui::Ui, model: &TranslatingModel) {
    let opacities = compute_bar_opacities(model.elapsed, BAR_CELLS);
    let bar_height = 16.0;
    let total = ui.available_width();
    let gap = 3.0;
    let cell_w = (total - gap * (BAR_CELLS as f32 - 1.0)) / BAR_CELLS as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, bar_height), Sense::hover());
    if ui.is_rect_visible(rect) {
        for (i, op) in opacities.iter().enumerate() {
            let x = rect.left() + (cell_w + gap) * i as f32;
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.top()),
                Vec2::new(cell_w, bar_height),
            );
            let alpha = (op * 255.0).clamp(0.0, 255.0) as u8;
            let color = Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, alpha);
            ui.painter().rect_filled(cell_rect, 2.0, color);
        }
    }
    ui.add_space(12.0);
}

fn cancel_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Cancel";
    let padding = Vec2::new(12.0, 5.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.0), theme::INK_2)
        .size();
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::PANEL_3
        } else {
            theme::PANEL_2
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, theme::LINE),
            egui::StrokeKind::Inside,
        );
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                6.0,
                Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            theme::INK_2,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel translation")
    });
    response
}
```

- [ ] **Step 5.4: Verify it builds and tests pass**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

Run: `cargo test --lib ui::translating 2>&1 | tail -10`
Expected: 5 tests pass.

- [ ] **Step 5.5: Commit**

```bash
git add src/ui/translating.rs src/ui/mod.rs
git commit -m "feat(M3): translating overlay render layer (animated + reduced-motion)"
```

---

## Task 6: New `ui/size_confirm.rs` — "send X chars to API?" modal

**Files:**
- Create: `src/ui/size_confirm.rs`
- Modify: `src/ui/mod.rs` to declare `pub mod size_confirm;`

**Why:** Per spec §6 + design doc M3 row, oversized clipboards should require explicit confirmation before being sent to the API. The modal shows the character count, the truncated source, and Send / Cancel buttons.

- [ ] **Step 6.1: Write the failing tests + create the module**

Create `src/ui/size_confirm.rs`:

```rust
//! Pre-dispatch confirmation modal for oversized clipboards. Renders inside
//! the existing viewport (no new viewport spawned). Pure render layer; key
//! handling (Esc → Cancel, Enter → Confirm) lives in `App::update`.

use egui::{RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// Mutable view state.
#[derive(Debug, Clone)]
pub struct SizeConfirmModel {
    /// Character count of the pending clipboard text.
    pub char_count: usize,
    /// Truncated preview of the source — caller passes the result of
    /// `format_preview`, not the raw clipboard.
    pub preview: String,
    /// Verb-form label of the action that will run on confirm
    /// ("Translate to Deutsch", "Run custom prompt", etc).
    pub action_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeConfirmOutcome {
    Confirm,
    Cancel,
}

/// Truncate the preview to ≤300 chars with an ellipsis. The modal body
/// shows up to two lines; longer source is implied via the count.
pub fn format_preview(text: &str) -> String {
    if text.chars().count() <= 300 {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(300).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_under_limit_returns_unchanged() {
        let s = "short input";
        assert_eq!(format_preview(s), s);
    }

    #[test]
    fn preview_over_limit_truncates_with_ellipsis() {
        let s = "x".repeat(500);
        let p = format_preview(&s);
        assert_eq!(p.chars().count(), 301);
        assert!(p.ends_with('…'));
    }
}
```

In `src/ui/mod.rs`, add the new module:

```rust
pub mod custom_prompt;
pub mod prompt;
pub mod size_confirm;
pub mod theme;
pub mod translating;
```

- [ ] **Step 6.2: Run tests to verify pass**

Run: `cargo test --lib ui::size_confirm 2>&1 | tail -10`
Expected: 2 tests pass.

- [ ] **Step 6.3: Add the `draw` function**

Append to `src/ui/size_confirm.rs` (before `#[cfg(test)]`):

```rust
pub fn draw(ctx: &egui::Context, model: &SizeConfirmModel) -> Option<SizeConfirmOutcome> {
    let mut clicked: Option<SizeConfirmOutcome> = None;
    theme::window_frame(ctx, "Confirm send", Some("clipt9n · large clipboard"), |ui| {
        let body_padding = egui::Margin::symmetric(18, 14);
        egui::Frame::new()
            .inner_margin(body_padding)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("{} characters", model.char_count))
                        .color(theme::WARN)
                        .strong()
                        .size(14.0),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Sending this clipboard to the API for: {}.",
                        model.action_label
                    ))
                    .color(theme::INK)
                    .size(12.5),
                );
                ui.add_space(12.0);

                ui.label(
                    RichText::new("PREVIEW")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if model.preview.is_empty() {
                                " "
                            } else {
                                &model.preview
                            })
                            .color(theme::INK_2)
                            .monospace()
                            .size(12.0),
                        );
                    });

                ui.add_space(14.0);
                let sep_rect =
                    egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 1.0));
                ui.painter().hline(
                    sep_rect.x_range(),
                    sep_rect.center().y,
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    theme::kbd(ui, "↵");
                    ui.label(
                        RichText::new("send ·")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    theme::kbd(ui, "Esc");
                    ui.label(
                        RichText::new("cancel")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if confirm_button(ui).clicked() {
                            clicked = Some(SizeConfirmOutcome::Confirm);
                        }
                        ui.add_space(8.0);
                        if cancel_button(ui).clicked() {
                            clicked = Some(SizeConfirmOutcome::Cancel);
                        }
                    });
                });
            });
    });
    clicked
}

fn confirm_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Send →";
    let padding = Vec2::new(14.0, 7.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.5), theme::ACCENT_INK)
        .size();
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::ACCENT.gamma_multiply(0.92)
        } else {
            theme::ACCENT
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                6.0,
                Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            theme::ACCENT_INK,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Send"));
    response
}

fn cancel_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Cancel";
    let padding = Vec2::new(12.0, 5.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.0), theme::INK_2)
        .size();
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::PANEL_3
        } else {
            theme::PANEL_2
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, theme::LINE),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            theme::INK_2,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel"));
    response
}
```

- [ ] **Step 6.4: Verify it builds and tests pass**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

Run: `cargo test --lib ui::size_confirm 2>&1 | tail -10`
Expected: 2 tests pass.

- [ ] **Step 6.5: Commit**

```bash
git add src/ui/size_confirm.rs src/ui/mod.rs
git commit -m "feat(M3): size-confirm modal render layer"
```

---

## Task 7: Refactor `AppState` — Intent + new states + generation counter (no rendering hookup yet)

**Files:**
- Modify: `src/app.rs`

**Why:** Before any of the new windows can be wired in, the state machine has to model them. This task introduces the `Intent` enum + `decide_intent` pure function (unit-testable), extends `AppState` with `EnteringCustom`, `ConfirmingSize`, and a payload-carrying `Translating` variant, and adds a `dispatch_gen: u64` counter so cancellation can drop in-flight outcomes. **No rendering changes** — `update()` keeps drawing only the prompt window in `Showing`. Subsequent tasks (8/9/10) wire the new states into the renderer.

- [ ] **Step 7.1: Write the failing tests**

Append to `src/app.rs` (the file currently has no test module — add one at the end):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_threshold(threshold: usize) -> Config {
        let mut c = Config::default();
        c.ui.confirm_size_threshold = threshold;
        c
    }

    #[test]
    fn slot_1_resolves_to_translate_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(1, "hi", &cfg).expect("slot 1 is valid");
        let Intent::Translate { action, action_label, overlay_label } = intent else {
            panic!("expected Intent::Translate");
        };
        let Action::Translate { code } = action else {
            panic!("expected Action::Translate");
        };
        assert_eq!(code, "en");
        assert_eq!(action_label, "Translate to English");
        assert_eq!(overlay_label, "Translating to English…");
    }

    #[test]
    fn slot_4_resolves_to_fix_grammar_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(4, "hi", &cfg).expect("slot 4 is valid");
        let Intent::Translate { action, action_label, overlay_label } = intent else {
            panic!("expected Intent::Translate");
        };
        assert!(matches!(action, Action::FixGrammar));
        assert_eq!(action_label, "Fix grammar");
        assert_eq!(overlay_label, "Fixing grammar…");
    }

    #[test]
    fn slot_5_resolves_to_rewrite_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(5, "hi", &cfg).expect("slot 5 is valid");
        let Intent::Translate { action, action_label, overlay_label } = intent else {
            panic!("expected Intent::Translate");
        };
        assert!(matches!(action, Action::Rewrite));
        assert_eq!(action_label, "Rewrite for clarity");
        assert_eq!(overlay_label, "Rewriting for clarity…");
    }

    #[test]
    fn slot_6_resolves_to_enter_custom() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(6, "hi", &cfg).expect("slot 6 is valid");
        assert!(matches!(intent, Intent::EnterCustom));
    }

    #[test]
    fn invalid_slot_returns_none() {
        let cfg = cfg_with_threshold(2000);
        assert!(decide_intent(0, "hi", &cfg).is_none());
        assert!(decide_intent(7, "hi", &cfg).is_none());
    }

    #[test]
    fn requires_size_confirm_above_threshold() {
        let cfg = cfg_with_threshold(100);
        let big = "x".repeat(150);
        assert!(requires_size_confirm(&big, &cfg));
        let small = "x".repeat(50);
        assert!(!requires_size_confirm(&small, &cfg));
    }

    #[test]
    fn dispatch_gen_starts_at_zero_and_monotonically_increases() {
        // Just verify the field exists with the expected starting value.
        // We can't construct ClipApp here (requires CreationContext), so
        // this is a doc-style invariant test on a free helper.
        assert_eq!(next_gen(0), 1);
        assert_eq!(next_gen(42), 43);
        assert_eq!(next_gen(u64::MAX - 1), u64::MAX);
    }

    #[test]
    fn overlay_label_for_translate() {
        assert_eq!(overlay_label_for(&Action::FixGrammar), "Fixing grammar…");
        assert_eq!(overlay_label_for(&Action::Rewrite), "Rewriting for clarity…");
        assert_eq!(
            overlay_label_for(&Action::Custom { instruction: "x".into() }),
            "Running custom prompt…"
        );
    }

    #[test]
    fn action_label_for_translate_uses_label() {
        let cfg = Config::default();
        assert_eq!(
            action_label_for(&Action::Translate { code: "de".into() }, &cfg),
            "Translate to Deutsch"
        );
        assert_eq!(action_label_for(&Action::FixGrammar, &cfg), "Fix grammar");
        assert_eq!(action_label_for(&Action::Rewrite, &cfg), "Rewrite for clarity");
        assert_eq!(
            action_label_for(&Action::Custom { instruction: "anything".into() }, &cfg),
            "Custom prompt"
        );
    }
}
```

- [ ] **Step 7.2: Run tests to verify failure**

Run: `cargo test --lib app 2>&1 | tail -15`
Expected: compilation errors on `Intent`, `decide_intent`, `requires_size_confirm`, `next_gen`, `overlay_label_for`, `action_label_for` (none exist).

- [ ] **Step 7.3: Replace `AppState` and add the helper functions + counter**

In `src/app.rs`, **replace** the existing `AppState` enum (lines 26-34) with the extended one:

```rust
/// Top-level UI state machine.
#[derive(Debug, Clone)]
enum AppState {
    /// Window hidden. Hotkey will transition to `Showing`.
    Idle,
    /// Window visible; user is choosing an action.
    Showing,
    /// User picked slot 6; the custom prompt window is visible. The
    /// `CustomPromptModel` carries instruction state across frames.
    EnteringCustom { model: prompt_custom::CustomPromptModel },
    /// Pre-flight size confirmation. Confirm → transition to `Translating`
    /// with the carried `pending_action`; Cancel → `Idle`.
    ConfirmingSize {
        pending_action: Action,
        action_label: String,
        overlay_label: String,
        source_text: String,
        char_count: usize,
        preview: String,
    },
    /// Translation in flight. The overlay window is visible.
    Translating {
        gen: u64,
        action_label: String,
        overlay_label: String,
        started_at: std::time::Instant,
    },
}
```

Add a `use` alias for `custom_prompt` near the existing imports (top of file, around line 23):

```rust
use crate::ui::{custom_prompt as prompt_custom, prompt, size_confirm, theme, translating};
```

Replace the `ClipApp` struct (lines 36-57) so it adds the new fields:

```rust
pub struct ClipApp {
    cfg: Config,
    state_path: PathBuf,
    state: State,

    /// Boxed for shared ownership across async tasks. We keep the `Arc` form
    /// to allow cheap clones into the spawn closure.
    provider: std::sync::Arc<dyn LlmProvider>,

    runtime: Runtime,
    hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    result_tx: mpsc::Sender<TranslationOutcome>,
    result_rx: mpsc::Receiver<TranslationOutcome>,

    app_state: AppState,
    prompt_model: prompt::PromptModel,

    /// Set to true once the viewport has gained focus after a `show_window`.
    has_been_focused: bool,

    /// Monotonically-increasing dispatch counter. Each translation captures
    /// this value at dispatch time; outcomes whose gen ≠ current are dropped
    /// (used for cancellation).
    dispatch_gen: u64,

    /// Whether the user has reduced motion enabled at OS level. Queried
    /// once at construction; the translating overlay reads this to decide
    /// between animated and static rendering.
    reduced_motion: bool,
}
```

Update the `TranslationOutcome` struct (around line 59) to carry `gen`:

```rust
#[derive(Debug)]
struct TranslationOutcome {
    result: Result<String, TranslateError>,
    action_label: String,
    slot: u8,
    /// Dispatch-generation that produced this outcome. If `App.dispatch_gen`
    /// has advanced since dispatch, this outcome is stale (user cancelled).
    gen: u64,
}
```

Update `ClipApp::new` (around lines 67-107) to query reduced motion + initialize the new fields. **Replace** the existing constructor:

```rust
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

        // Cache the OS reduced-motion preference once at startup. Spec
        // a11y baseline accepts a one-shot read.
        let reduced_motion = crate::platform::current().reduced_motion();

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
            has_been_focused: false,
            dispatch_gen: 0,
            reduced_motion,
        }
    }
```

Now add the testable helper functions. Append to `src/app.rs` (above `#[cfg(test)]`, after the `impl eframe::App for ClipApp` block):

```rust
// -----------------------------------------------------------------------
// Pure helpers (testable in isolation; no egui Context required)
// -----------------------------------------------------------------------

/// What the user implicitly asked for by picking a slot. The state machine
/// in `update()` switches on this to decide whether to enter custom-prompt
/// mode, show the size-confirm modal, or dispatch immediately.
#[derive(Debug, Clone)]
pub(crate) enum Intent {
    /// Run the action against the current clipboard.
    Translate {
        action: Action,
        action_label: String,
        overlay_label: String,
    },
    /// Slot 6 — open the custom prompt window first, the action is built
    /// from user input.
    EnterCustom,
}

pub(crate) fn decide_intent(slot: u8, _source_text: &str, cfg: &Config) -> Option<Intent> {
    match slot {
        1 => Some(translate_intent(
            Action::Translate {
                code: cfg.languages.slot_1.code.clone(),
            },
            &cfg.languages.slot_1.label,
        )),
        2 => Some(translate_intent(
            Action::Translate {
                code: cfg.languages.slot_2.code.clone(),
            },
            &cfg.languages.slot_2.label,
        )),
        3 => Some(translate_intent(
            Action::Translate {
                code: cfg.languages.slot_3.code.clone(),
            },
            &cfg.languages.slot_3.label,
        )),
        4 => Some(Intent::Translate {
            action: Action::FixGrammar,
            action_label: "Fix grammar".into(),
            overlay_label: "Fixing grammar…".into(),
        }),
        5 => Some(Intent::Translate {
            action: Action::Rewrite,
            action_label: "Rewrite for clarity".into(),
            overlay_label: "Rewriting for clarity…".into(),
        }),
        6 => Some(Intent::EnterCustom),
        _ => None,
    }
}

fn translate_intent(action: Action, lang_label: &str) -> Intent {
    Intent::Translate {
        action,
        action_label: format!("Translate to {lang_label}"),
        overlay_label: format!("Translating to {lang_label}…"),
    }
}

pub(crate) fn requires_size_confirm(source: &str, cfg: &Config) -> bool {
    source.chars().count() > cfg.ui.confirm_size_threshold
}

pub(crate) fn next_gen(current: u64) -> u64 {
    current.wrapping_add(1)
}

pub(crate) fn overlay_label_for(action: &Action) -> String {
    match action {
        Action::Translate { .. } => unreachable!(
            "Translate overlay labels are built at slot resolution; \
             callers must not pass Action::Translate here without a label"
        ),
        Action::FixGrammar => "Fixing grammar…".into(),
        Action::Rewrite => "Rewriting for clarity…".into(),
        Action::Custom { .. } => "Running custom prompt…".into(),
    }
}

pub(crate) fn action_label_for(action: &Action, cfg: &Config) -> String {
    match action {
        Action::Translate { code } => match cfg.label_for_code(code) {
            Ok(label) => format!("Translate to {label}"),
            Err(_) => format!("Translate to {code}"),
        },
        Action::FixGrammar => "Fix grammar".into(),
        Action::Rewrite => "Rewrite for clarity".into(),
        Action::Custom { .. } => "Custom prompt".into(),
    }
}
```

Update the existing `slot_to_action` method (lines 135-160). It's no longer the dispatch entry-point — it's used only by `dispatch()`. **Delete** it entirely; `dispatch()` will be rewritten in Task 8/9 to use `decide_intent` instead. For Task 7's compile, replace the body of `dispatch` (around line 162) with this minimal placeholder that preserves M2 behavior for slots 1–5 and is rewired in Tasks 8–10:

```rust
    fn dispatch(&mut self, ctx: &egui::Context, slot: u8) {
        let Some(intent) = decide_intent(slot, &self.prompt_model.clipboard_text, &self.cfg) else {
            tracing::info!(slot, "invalid slot ignored");
            return;
        };
        match intent {
            Intent::Translate { action, action_label, overlay_label } => {
                self.start_translation(ctx, slot, action, action_label, overlay_label);
            }
            Intent::EnterCustom => {
                // Wired in Task 8 — for now the state simply changes so the
                // user gets visible feedback (window stays open with prompt;
                // Task 8 swaps in the custom prompt UI).
                self.app_state = AppState::EnteringCustom {
                    model: prompt_custom::CustomPromptModel {
                        clipboard_text: self.prompt_model.clipboard_text.clone(),
                        instruction: String::new(),
                        focus_textarea_next_frame: true,
                    },
                };
                tracing::info!(slot, "entering custom prompt mode");
            }
        }
    }

    fn start_translation(
        &mut self,
        ctx: &egui::Context,
        slot: u8,
        action: Action,
        action_label: String,
        overlay_label: String,
    ) {
        self.dispatch_gen = next_gen(self.dispatch_gen);
        let gen = self.dispatch_gen;
        let cfg = self.cfg.clone();
        let provider = self.provider.clone();
        let tx = self.result_tx.clone();
        let source_text = self.prompt_model.clipboard_text.clone();

        // Note: `state.last_slot` was recorded by the caller (`dispatch()`
        // for slot keys 1–6, or implicitly slot=6 for the custom-prompt
        // submit path). Don't double-write here.

        self.app_state = AppState::Translating {
            gen,
            action_label: action_label.clone(),
            overlay_label,
            started_at: std::time::Instant::now(),
        };
        // Hide for now; Task 10 swaps in the overlay rendering. Until then
        // the user sees the cleared CentralPanel for ≤30s — acceptable for
        // an intermediate commit.
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));

        let ctx_for_repaint = ctx.clone();
        self.runtime.spawn(async move {
            let translator = Translator::new(&cfg, provider.as_ref());
            let result = translator.execute(&action, &source_text).await;
            let _ = tx.send(TranslationOutcome {
                result,
                action_label,
                slot,
                gen,
            });
            ctx_for_repaint.request_repaint();
        });
    }
```

Update `handle_translation_done` (around line 194) to drop stale outcomes:

```rust
    fn handle_translation_done(&mut self, outcome: TranslationOutcome) {
        // Stale outcome from a cancelled translation; drop silently.
        let current_gen = match &self.app_state {
            AppState::Translating { gen, .. } => Some(*gen),
            _ => None,
        };
        if Some(outcome.gen) != current_gen {
            tracing::debug!(
                outcome_gen = outcome.gen,
                current_gen = ?current_gen,
                "dropping stale translation outcome"
            );
            return;
        }
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
                tracing::info!(
                    slot = outcome.slot,
                    action = %outcome.action_label,
                    "translation complete"
                );
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
```

Update the `update()` method's `want_visible` calculation (around line 286) to cover all non-Idle states:

```rust
        // Drive viewport visibility from app state every frame.
        let want_visible = !matches!(self.app_state, AppState::Idle);
        ctx.send_viewport_cmd(ViewportCommand::Visible(want_visible));
```

(Note: silence the `unused import` warning for `prompt_custom`, `size_confirm`, `translating` — they'll be used in Tasks 8–10. To keep this commit warning-clean, mark them `#[allow(unused_imports)]` on the `use` line:)

```rust
#[allow(unused_imports)]
use crate::ui::{custom_prompt as prompt_custom, prompt, size_confirm, theme, translating};
```

The `update()` body's match on `Showing` stays as-is; the new states fall through to the `else` branch (clean panel) until Tasks 8–10 wire them. The state machine compiles, the prompt window keeps working, and slots 1–5 still translate end-to-end.

- [ ] **Step 7.4: Run tests to verify pass**

Run: `cargo test --lib app 2>&1 | tail -15`
Expected: 9 new tests pass (the helpers are pure).

Run: `cargo test --all-features 2>&1 | grep "test result:" | head -5`
Expected: total ≈89 (M2's 80 + 9 new). All pass.

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean (no warnings beyond the pre-existing baseline).

- [ ] **Step 7.5: Commit**

```bash
git add src/app.rs
git commit -m "refactor(M3): AppState extends with EnteringCustom/ConfirmingSize/Translating-payload + dispatch_gen"
```

---

## Task 8: Wire slot 6 → custom prompt window

**Files:**
- Modify: `src/app.rs`

**Why:** Task 7 set the state to `EnteringCustom` but `update()` still falls through to the cleared panel. This task draws the custom prompt window in that state, dispatches Cmd+Enter / Esc / preset clicks, and hands off to `start_translation` on submit.

- [ ] **Step 8.1: Update `update()` to render `EnteringCustom`**

In `src/app.rs`, replace the body of `eframe::App for ClipApp::update` starting after the `if want_visible {` branch's existing `Showing` rendering. The cleanest structure is to switch on `app_state` AFTER the visibility/focus-loss handling:

```rust
impl eframe::App for ClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(150));

        self.drain_channels(ctx);

        let want_visible = !matches!(self.app_state, AppState::Idle);
        ctx.send_viewport_cmd(ViewportCommand::Visible(want_visible));

        if !want_visible {
            // Idle: paint clean chrome.
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ctx, |_| {});
            return;
        }

        // Auto-dismiss on focus loss (Spotlight-style).
        let focused = ctx.input(|i| i.focused);
        if focused {
            self.has_been_focused = true;
        } else if self.has_been_focused {
            self.dismiss_to_idle(ctx);
            return;
        }

        // Render the active state and process keyboard.
        match std::mem::replace(&mut self.app_state, AppState::Idle) {
            AppState::Idle => unreachable!("handled above"),
            AppState::Showing => self.update_showing(ctx),
            AppState::EnteringCustom { model } => self.update_entering_custom(ctx, model),
            AppState::ConfirmingSize { .. } => {
                // Wired in Task 9. Until then, treat as no-op cancellation.
                self.dismiss_to_idle(ctx);
            }
            AppState::Translating { gen, action_label, overlay_label, started_at } => {
                // Restore — Task 10 wires the overlay rendering.
                self.app_state = AppState::Translating {
                    gen,
                    action_label,
                    overlay_label,
                    started_at,
                };
            }
        }
    }
}
```

The `std::mem::replace` lets us pattern-match by-value on the state without fighting the borrow checker. Each handler is responsible for restoring `self.app_state` (or transitioning it).

Add the new helper methods to `impl ClipApp`. Insert above `fn update`:

```rust
    fn dismiss_to_idle(&mut self, ctx: &egui::Context) {
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn update_showing(&mut self, ctx: &egui::Context) {
        // Refresh the prompt model in case the clipboard changed since the
        // hotkey fired (e.g., user copied something else then re-summoned).
        let click = prompt::draw(ctx, &self.cfg, &self.prompt_model);
        let key = self.handle_keys_showing(ctx);
        let outcome = key.or(click);
        match outcome {
            Some(prompt::PromptOutcome::Pick(n)) => {
                // dispatch() may transition to any of the new states; restore
                // Showing only if dispatch didn't transition.
                self.app_state = AppState::Showing;
                self.dispatch(ctx, n);
            }
            Some(prompt::PromptOutcome::RepeatLast) => {
                self.app_state = AppState::Showing;
                if let Some(n) = self.state.last_slot {
                    self.dispatch(ctx, n);
                }
            }
            Some(prompt::PromptOutcome::Cancel) => {
                self.dismiss_to_idle(ctx);
            }
            None => {
                self.app_state = AppState::Showing;
            }
        }
    }

    fn update_entering_custom(
        &mut self,
        ctx: &egui::Context,
        mut model: prompt_custom::CustomPromptModel,
    ) {
        // Refresh clipboard text in case the user pasted something else
        // between summoning and entering custom mode. Cheap.
        if model.clipboard_text != self.prompt_model.clipboard_text {
            model.clipboard_text = self.prompt_model.clipboard_text.clone();
        }

        let click = prompt_custom::draw(ctx, &mut model);
        let key_outcome = self.handle_keys_entering_custom(ctx, &model);

        // Apply preset click before dispatch so the user sees the chip's
        // text in the textarea even if they then press Esc.
        if let Some(prompt_custom::CustomPromptOutcome::PresetPicked(i)) = click {
            model.instruction = prompt_custom::PRESETS[i].into();
            self.app_state = AppState::EnteringCustom { model };
            return;
        }

        let submit = matches!(click, Some(prompt_custom::CustomPromptOutcome::Submit))
            || key_outcome == Some(CustomKey::Submit);
        let cancel = key_outcome == Some(CustomKey::Cancel);

        if cancel {
            self.dismiss_to_idle(ctx);
            return;
        }
        if submit && prompt_custom::submit_enabled(&model.instruction) {
            let instruction = model.instruction.trim().to_string();
            let action = Action::Custom { instruction };
            let action_label = action_label_for(&action, &self.cfg);
            let overlay_label = overlay_label_for(&action);
            // Custom-prompt slot index is 6 (used for last-slot persistence).
            self.start_translation(ctx, 6, action, action_label, overlay_label);
            return;
        }
        // Otherwise stay in EnteringCustom with the (possibly mutated) model.
        self.app_state = AppState::EnteringCustom { model };
    }

    fn handle_keys_showing(&mut self, ctx: &egui::Context) -> Option<prompt::PromptOutcome> {
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(prompt::PromptOutcome::Cancel);
            }
            if i.key_pressed(Key::Enter) && self.state.last_slot.is_some() {
                return Some(prompt::PromptOutcome::RepeatLast);
            }
            for (key, n) in [
                (Key::Num1, 1u8),
                (Key::Num2, 2),
                (Key::Num3, 3),
                (Key::Num4, 4),
                (Key::Num5, 5),
                (Key::Num6, 6),
            ] {
                if i.key_pressed(key) && !self.prompt_model.clipboard_text.is_empty() {
                    return Some(prompt::PromptOutcome::Pick(n));
                }
            }
            None
        })
    }

    fn handle_keys_entering_custom(
        &self,
        ctx: &egui::Context,
        _model: &prompt_custom::CustomPromptModel,
    ) -> Option<CustomKey> {
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(CustomKey::Cancel);
            }
            // Cmd+Enter (macOS) or Ctrl+Enter (Linux/Windows) submits.
            if i.key_pressed(Key::Enter) && (i.modifiers.command || i.modifiers.ctrl) {
                return Some(CustomKey::Submit);
            }
            None
        })
    }
```

Add the `CustomKey` helper enum near the other enums (right above `pub struct ClipApp`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CustomKey {
    Submit,
    Cancel,
}
```

**Delete** the now-orphaned `handle_keys` method (the original M2 method, which is replaced by `handle_keys_showing` above). It was the function around lines 246-271 of M2's `app.rs`.

Drop the `#[allow(unused_imports)]` from the `use crate::ui::…` line (now `prompt_custom` is used).

- [ ] **Step 8.2: Verify it builds**

Run: `cargo build 2>&1 | tail -5`
Expected: `Finished` clean.

- [ ] **Step 8.3: Run all tests**

Run: `cargo test --all-features 2>&1 | grep "test result:" | head -5`
Expected: still passes (no test changes; we wired existing helpers).

- [ ] **Step 8.4: Manual smoke (visual)**

Run: `cargo run --release 2>&1 | head -5` (in a separate terminal — wait for the binary to launch as a background-residing app).

Then in another terminal: copy some text and trigger Cmd+Shift+T. The prompt should appear; press 6. Expected: custom prompt window appears with empty textarea, 5 preset chips, the truncated clipboard preview, and the `⌘+↵ run · Esc cancel` footer with a Run button. Type text → Run becomes enabled. Click a preset → textarea fills. Press Esc → window dismisses. Press Cmd+Enter with text → window dismisses (and silently fails the translation in this commit; Task 10 makes it visible). The OS notification will fire on success.

- [ ] **Step 8.5: Commit**

```bash
git add src/app.rs
git commit -m "feat(M3): wire slot 6 → custom prompt window with Cmd+Enter / Esc"
```

---

## Task 9: Wire size-confirm modal — intent dispatcher + ConfirmingSize state

**Files:**
- Modify: `src/app.rs`

**Why:** `start_translation` currently kicks off the API call unconditionally. M3 must intercept oversized clipboards via the size-confirm modal. We add an indirection: `dispatch_translate(ctx, slot, action, …)` checks `requires_size_confirm` and either transitions to `ConfirmingSize` or calls `start_translation`.

- [ ] **Step 9.1: Refactor `dispatch` and `update_entering_custom` to go through the new gate**

In `src/app.rs`, **add** a new method `dispatch_translate` to `impl ClipApp` (right above `start_translation`):

```rust
    /// Single fork point for "we know what to translate; should we ask the
    /// user to confirm first?". Either transitions to ConfirmingSize or
    /// calls start_translation directly.
    fn dispatch_translate(
        &mut self,
        ctx: &egui::Context,
        slot: u8,
        action: Action,
        action_label: String,
        overlay_label: String,
    ) {
        if requires_size_confirm(&self.prompt_model.clipboard_text, &self.cfg) {
            let preview = size_confirm::format_preview(&self.prompt_model.clipboard_text);
            let char_count = self.prompt_model.clipboard_text.chars().count();
            self.app_state = AppState::ConfirmingSize {
                pending_action: action,
                action_label,
                overlay_label,
                source_text: self.prompt_model.clipboard_text.clone(),
                char_count,
                preview,
            };
            return;
        }
        self.start_translation(ctx, slot, action, action_label, overlay_label);
    }
```

Modify `dispatch` (the slot-driven entry) — replace the `Intent::Translate { … }` arm to route through `dispatch_translate`:

```rust
    fn dispatch(&mut self, ctx: &egui::Context, slot: u8) {
        let Some(intent) = decide_intent(slot, &self.prompt_model.clipboard_text, &self.cfg) else {
            tracing::info!(slot, "invalid slot ignored");
            return;
        };
        // Record the slot press immediately. Single source of truth for
        // last-action persistence; downstream functions never re-record.
        // Matches M2 semantic: "press = recorded, even if cancelled later."
        self.state.record_slot(slot);
        if let Err(e) = self.state.save(&self.state_path) {
            tracing::warn!(error = %e, "state.toml save failed");
        }
        match intent {
            Intent::Translate { action, action_label, overlay_label } => {
                self.dispatch_translate(ctx, slot, action, action_label, overlay_label);
            }
            Intent::EnterCustom => {
                self.app_state = AppState::EnteringCustom {
                    model: prompt_custom::CustomPromptModel {
                        clipboard_text: self.prompt_model.clipboard_text.clone(),
                        instruction: String::new(),
                        focus_textarea_next_frame: true,
                    },
                };
                tracing::info!(slot, "entering custom prompt mode");
            }
        }
    }
```

In `update_entering_custom`, replace the `start_translation` call with `dispatch_translate` so custom prompts also go through the size gate:

```rust
        if submit && prompt_custom::submit_enabled(&model.instruction) {
            let instruction = model.instruction.trim().to_string();
            let action = Action::Custom { instruction };
            let action_label = action_label_for(&action, &self.cfg);
            let overlay_label = overlay_label_for(&action);
            self.dispatch_translate(ctx, 6, action, action_label, overlay_label);
            return;
        }
```

Add the renderer arm to `update`. **Replace** the placeholder `ConfirmingSize { .. } =>` arm:

```rust
            AppState::ConfirmingSize {
                pending_action,
                action_label,
                overlay_label,
                source_text,
                char_count,
                preview,
            } => {
                self.update_confirming_size(
                    ctx,
                    pending_action,
                    action_label,
                    overlay_label,
                    source_text,
                    char_count,
                    preview,
                );
            }
```

Add the method:

```rust
    fn update_confirming_size(
        &mut self,
        ctx: &egui::Context,
        pending_action: Action,
        action_label: String,
        overlay_label: String,
        source_text: String,
        char_count: usize,
        preview: String,
    ) {
        let model = size_confirm::SizeConfirmModel {
            char_count,
            preview: preview.clone(),
            action_label: action_label.clone(),
        };
        let click = size_confirm::draw(ctx, &model);
        let key = ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                Some(size_confirm::SizeConfirmOutcome::Cancel)
            } else if i.key_pressed(Key::Enter) {
                Some(size_confirm::SizeConfirmOutcome::Confirm)
            } else {
                None
            }
        });
        let outcome = key.or(click);
        match outcome {
            Some(size_confirm::SizeConfirmOutcome::Confirm) => {
                // Use the persisted `last_slot` to identify which slot owned
                // this dispatch. Custom prompts use slot 6.
                let slot = match &pending_action {
                    Action::Custom { .. } => 6,
                    _ => self.state.last_slot.unwrap_or(0),
                };
                self.start_translation(ctx, slot, pending_action, action_label, overlay_label);
            }
            Some(size_confirm::SizeConfirmOutcome::Cancel) => {
                self.dismiss_to_idle(ctx);
            }
            None => {
                self.app_state = AppState::ConfirmingSize {
                    pending_action,
                    action_label,
                    overlay_label,
                    source_text,
                    char_count,
                    preview,
                };
            }
        }
    }
```

- [ ] **Step 9.2: Add tests for the gate**

Append to the `tests` mod in `src/app.rs`:

```rust
    #[test]
    fn dispatch_translate_paths_diverge_on_threshold() {
        // We can't construct a ClipApp here, but we can directly verify
        // the requires_size_confirm boundary used by dispatch_translate.
        let mut cfg = Config::default();
        cfg.ui.confirm_size_threshold = 10;

        assert!(!requires_size_confirm("short", &cfg));
        assert!(requires_size_confirm("this is definitely longer than ten characters", &cfg));
    }
```

- [ ] **Step 9.3: Run all tests**

Run: `cargo test --all-features 2>&1 | grep "test result:" | head -5`
Expected: total ≈90 (one new test added). All pass.

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

- [ ] **Step 9.4: Manual smoke**

Run: `cargo run --release` in a terminal. Copy >2000 chars of text (e.g., `seq 1 1000 | tr '\n' ' ' | pbcopy` on macOS). Trigger Cmd+Shift+T. Press 1.
Expected: size-confirm modal appears, showing "N characters", "Sending this clipboard to the API for: Translate to English.", a 300-char preview, and Send/Cancel buttons. Press Esc → returns to Idle. Re-trigger, press 1, click Send → translation proceeds (the OS notification fires on completion; the overlay is still TBD until Task 10).

Copy a short string (<2000 chars), trigger, press 1.
Expected: no modal — translation proceeds directly.

- [ ] **Step 9.5: Commit**

```bash
git add src/app.rs
git commit -m "feat(M3): size-confirm modal gates oversized clipboards"
```

---

## Task 10: Wire translating overlay rendering + cancel + reduced-motion

**Files:**
- Modify: `src/app.rs`

**Why:** `start_translation` currently hides the viewport entirely. M3's promise is the design's animated lime-bar overlay during the API call, with a Cancel button that drops the in-flight result via the gen counter, and a static label when reduce-motion is enabled.

- [ ] **Step 10.1: Replace the `Visible(false)` in `start_translation` with a transition that keeps the viewport visible**

In `src/app.rs::start_translation`, **delete** the line:

```rust
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
```

(The viewport visibility is now driven entirely by `update()`'s `want_visible` calculation, which is `!matches!(state, AppState::Idle)`. `AppState::Translating` is non-Idle, so the viewport stays visible.)

Trigger an immediate repaint to show the overlay on the next frame, even before async movement. After the `self.app_state = AppState::Translating { … }` block in `start_translation`, add:

```rust
        ctx.request_repaint();
```

- [ ] **Step 10.2: Render the overlay in `update`**

**Replace** the placeholder `AppState::Translating { … }` restoration arm in `update()` with a real handler:

```rust
            AppState::Translating {
                gen,
                action_label,
                overlay_label,
                started_at,
            } => {
                self.update_translating(ctx, gen, action_label, overlay_label, started_at);
            }
```

Add the method to `impl ClipApp`:

```rust
    fn update_translating(
        &mut self,
        ctx: &egui::Context,
        gen: u64,
        action_label: String,
        overlay_label: String,
        started_at: std::time::Instant,
    ) {
        // Tighter repaint cadence so the bar animates smoothly.
        if !self.reduced_motion {
            ctx.request_repaint_after(Duration::from_millis(translating::TICK_MS));
        }

        let model = translating::TranslatingModel {
            overlay_label: overlay_label.clone(),
            provider_model: self.cfg.provider.model.clone(),
            elapsed: started_at.elapsed(),
            reduced_motion: self.reduced_motion,
        };
        let click = translating::draw(ctx, &model);
        let cancelled_by_key = ctx.input(|i| i.key_pressed(Key::Escape));

        if click == Some(translating::TranslatingOutcome::Cancel) || cancelled_by_key {
            // Bump gen so the in-flight outcome is dropped on arrival.
            self.dispatch_gen = next_gen(self.dispatch_gen);
            tracing::info!(
                cancelled_gen = gen,
                new_gen = self.dispatch_gen,
                "user cancelled translation"
            );
            self.dismiss_to_idle(ctx);
            return;
        }

        // No event — restore the state.
        self.app_state = AppState::Translating {
            gen,
            action_label,
            overlay_label,
            started_at,
        };
    }
```

- [ ] **Step 10.3: Add a test for the gen-bump cancellation invariant**

Append to `src/app.rs`'s `tests` mod:

```rust
    #[test]
    fn cancellation_increments_gen_so_outcome_is_stale() {
        // Simulates: dispatch at gen=N, user cancels (bump to N+1), outcome
        // arrives tagged gen=N — must be considered stale.
        let mut current = 5_u64;
        let dispatched_gen = current;
        current = next_gen(current);
        // Outcome from the dispatched generation:
        let outcome_gen = dispatched_gen;
        // Stale check (mirrors handle_translation_done):
        assert_ne!(current, outcome_gen);
    }
```

- [ ] **Step 10.4: Run all tests**

Run: `cargo test --all-features 2>&1 | grep "test result:" | head -5`
Expected: total ≈91 (one more test). All pass.

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

- [ ] **Step 10.5: Manual smoke (full M3 path)**

Run: `cargo run --release`. Copy text, trigger Cmd+Shift+T, press any of 1–5.
Expected: overlay appears with the verb-form title (e.g., "Translating to Deutsch…"), provider model in the subtitle, animated lime sweep bar, elapsed-time counter incrementing in 0.1s steps, and a Cancel button. On API success: overlay closes, OS notification fires, clipboard is replaced.

Trigger again, press 6. In the custom prompt window, type "make this sound diplomatic" and Cmd+Enter.
Expected: overlay shows "Running custom prompt…". Same completion behavior.

Trigger again, press 1, then click Cancel mid-translation.
Expected: overlay closes immediately, no notification fires when the API request eventually completes (`tracing::debug!` log records "dropping stale translation outcome"). Clipboard is unchanged.

If the user has macOS Reduce Motion enabled (`defaults write -g NSReduceMotionEnabled 1`), the bar is replaced with a static "Translating…" label. Reverting (`defaults write -g NSReduceMotionEnabled 0`) restores the animation on next launch.

- [ ] **Step 10.6: Commit**

```bash
git add src/app.rs
git commit -m "feat(M3): translating overlay rendering + cancel via gen counter"
```

---

## Task 11: README updates + final verification + summary commit

**Files:**
- Modify: `README.md`

**Why:** Document M3's new features and limitations for users. Verify the full test suite + grep-lint precursor (no new `cfg(target_os)` outside `src/platform/`).

- [ ] **Step 11.1: Locate the M2 GUI section in `README.md`**

Run: `grep -n "Cmd+Shift+T" README.md` to find the GUI section's anchor lines.

- [ ] **Step 11.2: Add an M3 features sub-section to the GUI section**

Append a new sub-section in `README.md` — placement depends on the existing M2 section structure; add it directly after the M2 "GUI usage" content:

```markdown
### M3: All actions + custom prompt + size confirmation

In addition to the M2 quick-actions (1–3 translation slots), M3 wires the
remaining slots:

- **4 — Fix grammar**: minimum-changes proofreading, in the source language.
- **5 — Rewrite for clarity**: more aggressive editing, in the source language.
- **6 — Custom prompt…**: opens a small editor window where you can type a
  free-form instruction (e.g., "make this sound diplomatic") or click one
  of the built-in presets. `⌘+↵` runs it; `Esc` cancels. Custom instructions
  are never persisted.

While a translation is in flight, the **translating overlay** appears with
the active provider model, an elapsed-time counter, and a Cancel button.
Press `Esc` or click Cancel to drop the in-flight result.

#### Reduced motion

When macOS "Reduce Motion" is enabled (System Settings → Accessibility →
Display → Reduce Motion), the overlay's animated bar is replaced with a
static "Translating…" label per WCAG 2.3.3.

#### Large-clipboard confirmation

To prevent surprise API costs on accidental large pastes, clipboards
exceeding `[ui].confirm_size_threshold` characters (default `2000`) trigger
a confirmation modal showing the character count and a 300-character
preview before the request is sent. To raise or lower this threshold, edit
`config.toml`:

```toml
[ui]
confirm_size_threshold = 5000
```

Set to `0` to confirm every clipboard (mostly useful for debugging).

#### M3 limitations (carried forward)

- Bundled fonts are still egui's default Hack/Ubuntu, not the design's
  Inter/JetBrains Mono — deferred to M8.
- Cancellation drops the *result* but not the in-flight HTTP request —
  the API call continues to its 30-second natural timeout. The user
  experience is fine; this is a billing nuance, not a blocking issue.
- The custom prompt window auto-focuses the textarea on entry. Tab still
  navigates to preset chips and the Run button.
```

- [ ] **Step 11.3: Run the full test suite**

Run: `cargo test --all-features 2>&1 | grep "test result:" | head -5`
Expected: ≥91 passed; 0 failed.

- [ ] **Step 11.4: Cross-platform discipline check**

The M8 grep-lint isn't in CI yet, but we verify manually that M3 added no new `cfg(target_os)` blocks outside `src/platform/`:

```bash
grep -rn '#\[cfg(target_os' src/ | grep -v '^src/platform/' | grep -v '^src/config.rs:'
```

Expected: empty output. (The single allowed exception is `src/config.rs::Modifier::resolve_native`; that line is the only match grep-blocked above.)

```bash
grep -rn '#\[cfg(unix' src/ | grep -v '^src/platform/'
```

Expected: empty output.

If either grep returns results outside the documented allowlist, stop and route the offending code into `src/platform/`.

- [ ] **Step 11.5: Verify CI files unchanged**

Run: `git diff main..HEAD -- .github/workflows/ Cargo.toml` and confirm:
- `Cargo.toml` shows no new dependency lines (M3 used existing crates only).
- `.github/workflows/build.yml` is unchanged from M2.

If `Cargo.toml` did pick up a new dep, that's a sign of scope creep — review and either remove or document in this plan.

- [ ] **Step 11.6: Commit and merge plan**

```bash
git add README.md
git commit -m "docs(M3): GUI usage for slots 4-6, overlay, reduced-motion, size-confirm"
```

Once all M3 commits are on `m3-actions-and-overlay`:

```bash
git log --oneline main..m3-actions-and-overlay
```

Expected output: ~11 commits, each starting with `feat(M3):`, `refactor(M3):`, `docs(M3):`, or `chore(M3):` (no merge commits inside the branch).

The branch is now ready for user review. Merge strategy mirrors M2: fast-forward to `main` once approved.

---

## Self-Review

Run this checklist after writing the plan; fix issues inline.

**1. Spec coverage (M3 row of design doc):**

| Spec deliverable | Plan task |
|---|---|
| Slots 4 (fix grammar), 5 (rewrite) wired | Already done in M2 (`src/app.rs:155-156`); plan documents this in Task 7's `decide_intent` and explicit decision #7 of the glossary. |
| Slot 6: opens custom prompt window | Tasks 4 (render layer) + 8 (state-machine wiring). |
| `ui/custom_prompt.rs` modeled on `custom-prompt.jsx` | Task 4 with PRESETS constant matching the JSX five entries verbatim. |
| Translating overlay (`TranslatingWindow` design) | Tasks 5 (render layer) + 10 (wiring + cancel). |
| Animated progress with reduced-motion fallback | Task 5's `compute_bar_opacities` + `draw`'s `model.reduced_motion` branch. Task 3 adds the platform query. |
| Elapsed-time counter | Task 5's `format_elapsed` + Task 10's `started_at.elapsed()`. |
| Cancel button on overlay | Task 5 renders it; Task 10 wires it (gen-counter cancellation). |
| `confirm_size_threshold` guard before sending oversized clipboards | Tasks 1 (config), 6 (render), 9 (wiring). |
| Source-preview truncation respects `[ui].show_preview` | Task 2. |
| Glossary chip preview area still empty | Existing M2 prompt window already reserves the gap; no plan task needed (M4 fills it). Documented in Task 2's "no glossary changes" implicit scope. |

**Exit criteria from the design doc, M3 row:**

| Exit criterion | Plan coverage |
|---|---|
| 1. All 4 actions produce correct outputs end-to-end | M2 already proves slots 1–5; Task 8/9/10 manual smoke proves slot 6 + size-confirmed and reduced-motion paths. |
| 2. Custom prompt accepts presets and free-form instruction; clears on close (does not persist) | Task 4 (PRESETS, submit_enabled), Task 8 (model recreated each entry, never persisted to state). Glossary item 8 documents the privacy rule. |
| 3. Translating overlay appears for any action >150ms; cancellable with Esc | Task 10 — appears immediately on dispatch (no 150ms threshold required, simpler and matches design). Cancel via Esc + Cancel button. |
| 4. Reduced-motion: when macOS "Reduce Motion" is on, progress is a static label | Tasks 3 + 5 (parser + render branch) + 10 (cached on `App.reduced_motion`). |
| 5. Size confirmation: pasting 2500 chars triggers modal; 1500 chars does not | Tasks 1 + 9 — `requires_size_confirm` test asserts the threshold boundary; manual smoke verifies. |

**2. Placeholder scan:** No "TBD", "implement later", "etc.", "similar to Task N", or naked "add error handling" appearances. Each step contains the actual code or actual command. ✓

**3. Type consistency:**
- `Intent::Translate { action, action_label, overlay_label }` — same field names in Tasks 7, 8, 9, 10 ✓
- `AppState::Translating { gen, action_label, overlay_label, started_at }` — consistent across Tasks 7 (introduced), 10 (consumed) ✓
- `AppState::ConfirmingSize { pending_action, action_label, overlay_label, source_text, char_count, preview }` — Tasks 7 (introduced), 9 (consumed/destructured) ✓
- `TranslationOutcome { result, action_label, slot, gen }` — added `gen` in Task 7; consumed in Task 7's `handle_translation_done` ✓
- `TranslatingModel { overlay_label, provider_model, elapsed, reduced_motion }` — Tasks 5 (defined), 10 (constructed) ✓
- `CustomPromptModel { clipboard_text, instruction, focus_textarea_next_frame }` — Tasks 4 (defined), 8 (constructed and mutated) ✓
- `SizeConfirmModel { char_count, preview, action_label }` — Tasks 6 (defined), 9 (constructed) ✓
- `requires_size_confirm`, `decide_intent`, `next_gen`, `overlay_label_for`, `action_label_for` — names consistent across declaration (Task 7) and call sites (Tasks 8, 9, 10) ✓
- `compute_bar_opacities`, `format_elapsed`, `BAR_CELLS`, `TICK_MS` — Task 5 declarations; Task 10 uses `TICK_MS` only ✓
- `format_preview` (size_confirm) vs `preview_truncate` (custom_prompt) — different functions on different types, intentionally not unified (300 vs 200 limit per their respective designs) ✓

No drift. Plan is consistent end-to-end.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-28-clipt9n-m3-actions-and-overlay.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration. Mirrors M1 and M2 execution flow.
2. **Inline Execution** — execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?
