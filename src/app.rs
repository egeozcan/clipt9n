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
    /// Used to detect focus-loss dismiss without firing in the brief window
    /// between sending `Visible(true)+Focus` and the OS actually focusing us.
    has_been_focused: bool,
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
            has_been_focused: false,
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

    fn hide_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Map a slot number to a concrete `Action`. Returns `None` if slot 6
    /// (custom — not wired in M2) or invalid.
    fn slot_to_action(&self, slot: u8) -> Option<(Action, String)> {
        match slot {
            1 => Some((
                Action::Translate {
                    code: self.cfg.languages.slot_1.code.clone(),
                },
                format!("Translate to {}", self.cfg.languages.slot_1.label),
            )),
            2 => Some((
                Action::Translate {
                    code: self.cfg.languages.slot_2.code.clone(),
                },
                format!("Translate to {}", self.cfg.languages.slot_2.label),
            )),
            3 => Some((
                Action::Translate {
                    code: self.cfg.languages.slot_3.code.clone(),
                },
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
            let _ = tx.send(TranslationOutcome {
                result,
                action_label,
                slot,
            });
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
}

impl eframe::App for ClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Lightly throttle when idle to not burn CPU. egui repaints on
        // input + we explicitly request_repaint when async tasks finish, so
        // a slow background tick is a safety net.
        ctx.request_repaint_after(Duration::from_millis(150));

        self.drain_channels(ctx);

        // Drive viewport visibility from app state every frame.
        // ViewportBuilder::with_visible(false) is unreliable on macOS at
        // launch, so we re-assert hidden state here defensively.
        let want_visible = matches!(self.app_state, AppState::Showing);
        ctx.send_viewport_cmd(ViewportCommand::Visible(want_visible));

        if want_visible {
            // Auto-dismiss on focus loss (Spotlight-style). We require the
            // viewport to have gained focus at least once before checking,
            // so we don't immediately self-dismiss in the gap between
            // `Visible(true)` and the OS finishing the focus handoff.
            let focused = ctx.input(|i| i.focused);
            if focused {
                self.has_been_focused = true;
            } else if self.has_been_focused {
                self.app_state = AppState::Idle;
                self.hide_window(ctx);
                return;
            }

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
        } else {
            // Paint a clean PANEL-filled CentralPanel so any moment the OS
            // briefly reveals the window (focus tab, exposé, expander
            // animations) it shows clean chrome rather than GPU back-buffer
            // garbage.
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show(ctx, |_| {});
        }
    }
}
