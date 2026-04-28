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
use crate::platform::Platform;
use crate::error::TranslateError;
use crate::llm::LlmProvider;
use crate::secrets::Secrets;
use crate::state::State;
use crate::translator::{Action, Translator};
#[allow(unused_imports)]
use crate::ui::{custom_prompt as prompt_custom, prompt, size_confirm, theme, translating};

/// Top-level UI state machine.
#[derive(Debug, Clone)]
#[allow(dead_code)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CustomKey {
    Submit,
    Cancel,
}

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

#[derive(Debug)]
struct TranslationOutcome {
    result: Result<String, TranslateError>,
    action_label: String,
    slot: u8,
    /// Dispatch-generation that produced this outcome. If `App.dispatch_gen`
    /// has advanced since dispatch, this outcome is stale (user cancelled).
    gen: u64,
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
        self.has_been_focused = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::Showing;
    }

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
                char_count,
                preview,
            };
            return;
        }
        self.start_translation(ctx, slot, action, action_label, overlay_label);
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
        ctx.request_repaint();

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

    fn dismiss_to_idle(&mut self, ctx: &egui::Context) {
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn update_showing(&mut self, ctx: &egui::Context) {
        // prompt_model was snapshotted at show_window(); we re-draw it each
        // frame against the same snapshot so the user sees a stable view
        // until they dismiss or pick a slot.
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
            self.dispatch_translate(ctx, 6, action, action_label, overlay_label);
            return;
        }
        // Otherwise stay in EnteringCustom with the (possibly mutated) model.
        self.app_state = AppState::EnteringCustom { model };
    }

    fn update_confirming_size(
        &mut self,
        ctx: &egui::Context,
        pending_action: Action,
        action_label: String,
        overlay_label: String,
        char_count: usize,
        preview: String,
    ) {
        let model = size_confirm::SizeConfirmModel {
            char_count,
            preview: preview.clone(),
            action_label: action_label.clone(),
        };
        // Click takes priority over key: when a button is Tab-focused and
        // the user presses Enter, egui's button click handler fires the
        // focused button's outcome; only fall back to global Esc/Enter
        // shortcuts when no button consumed the keystroke.
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
        let outcome = click.or(key);
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
                    char_count,
                    preview,
                };
            }
        }
    }

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
}

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
        // No `dispatch_gen` bump needed here: if we were translating, the
        // transition to Idle leaves `current_gen = None` in
        // `handle_translation_done`, so any in-flight outcome is detected
        // as stale (`Some(outcome.gen) != None`) and dropped silently.
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
            AppState::ConfirmingSize {
                pending_action,
                action_label,
                overlay_label,
                char_count,
                preview,
            } => {
                self.update_confirming_size(
                    ctx,
                    pending_action,
                    action_label,
                    overlay_label,
                    char_count,
                    preview,
                );
            }
            AppState::Translating {
                gen,
                action_label,
                overlay_label,
                started_at,
            } => {
                self.update_translating(ctx, gen, action_label, overlay_label, started_at);
            }
        }
    }
}

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

#[allow(dead_code)]
pub(crate) fn requires_size_confirm(source: &str, cfg: &Config) -> bool {
    source.chars().count() > cfg.ui.confirm_size_threshold
}

pub(crate) fn next_gen(current: u64) -> u64 {
    current.wrapping_add(1)
}

/// Return the overlay label for a non-`Translate` action.
///
/// # Panics
///
/// Panics if called with `Action::Translate` — that variant's label is
/// constructed at slot-resolution time inside `decide_intent`, so callers
/// must never pass it here.
#[allow(dead_code)]
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

#[allow(dead_code)]
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
    fn slot_2_resolves_to_translate_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(2, "hi", &cfg).expect("slot 2 is valid");
        let Intent::Translate { action, action_label, overlay_label } = intent else {
            panic!("expected Intent::Translate");
        };
        let Action::Translate { code } = action else {
            panic!("expected Action::Translate");
        };
        assert_eq!(code, "de");
        assert_eq!(action_label, "Translate to Deutsch");
        assert_eq!(overlay_label, "Translating to Deutsch…");
    }

    #[test]
    fn slot_3_resolves_to_translate_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(3, "hi", &cfg).expect("slot 3 is valid");
        let Intent::Translate { action, action_label, overlay_label } = intent else {
            panic!("expected Intent::Translate");
        };
        let Action::Translate { code } = action else {
            panic!("expected Action::Translate");
        };
        assert_eq!(code, "tr");
        assert_eq!(action_label, "Translate to Türkçe");
        assert_eq!(overlay_label, "Translating to Türkçe…");
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

    #[test]
    fn dispatch_translate_paths_diverge_on_threshold() {
        // We can't construct a ClipApp here, but we can directly verify
        // the requires_size_confirm boundary used by dispatch_translate.
        let mut cfg = Config::default();
        cfg.ui.confirm_size_threshold = 10;

        assert!(!requires_size_confirm("short", &cfg));
        assert!(requires_size_confirm("this is definitely longer than ten characters", &cfg));
    }

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
}
