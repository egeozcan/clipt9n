//! `ClipApp` is the eframe application: it owns the tokio runtime, the
//! channels to/from the hotkey thread and the translation worker, and the
//! prompt-window state machine. All UI is paint-only (`src/ui/prompt.rs`);
//! input handling lives in the sub-modules.

mod channels;
mod history;
mod prompt;
mod pure;
mod setup;
mod translation;
mod tray;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crossbeam_channel::Receiver as CrossbeamReceiver;
use eframe::CreationContext;
use egui::ViewportCommand;
use global_hotkey::GlobalHotKeyEvent;
use tokio::runtime::Runtime;

use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::LlmProvider;
use crate::platform::Platform;
use crate::secrets::Secrets;
use crate::state::State;
use crate::translator::Action;
use crate::ui::{custom_prompt as prompt_custom, theme};

/// Initial state override passed by `main.rs` to ClipApp at construction.
/// Used to land the app in the setup wizard on first launch instead of
/// the default Idle state.
pub enum InitialState {
    Idle,
    SetupWizard(crate::ui::setup::SetupWizardModel),
}

/// Top-level UI state machine.
#[derive(Debug, Clone)]
enum AppState {
    /// Window hidden. Hotkey will transition to `Showing`.
    Idle,
    /// Window visible; user is choosing an action.
    Showing,
    /// User picked slot 6; the custom prompt window is visible. The
    /// `CustomPromptModel` carries instruction state across frames.
    EnteringCustom {
        model: prompt_custom::CustomPromptModel,
    },
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
        /// Whether the user held Shift when picking the slot —
        /// result will be shown in a preview window instead of
        /// written directly to the clipboard.
        preview_mode: bool,
    },
    /// Translation completed in preview mode. The result is
    /// displayed with Copy and Dismiss actions.
    ShowingResult {
        result_text: String,
        source_text: String,
        action_label: String,
        detected_source_lang: Option<String>,
        action: crate::translator::Action,
        slot: u8,
    },
    /// Encrypted history viewer is open. The model holds the
    /// most-recent query results plus search/selection state.
    ShowingHistory {
        model: crate::ui::history::HistoryModel,
    },
    /// First-launch setup wizard is open. Persists the API key into
    /// the keychain (or env-var hint), runs connectivity + sample-
    /// translation checks, and persists the resulting config back to
    /// disk before transitioning to `Idle`.
    SetupWizard {
        model: crate::ui::setup::SetupWizardModel,
    },
    /// Hide-icon confirmation modal is open. The model carries the
    /// active hotkey display so the modal can show users their actual
    /// keyboard shortcut, not a hardcoded one.
    ConfirmingTrayHide {
        model: crate::ui::tray_modal::TrayHideModel,
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

    /// Live LLM provider, swappable so the wizard's Save-and-start can
    /// replace it without a restart (M7 Task 10 — Q6 brainstorming).
    ///
    /// `None` only if `build_provider` failed after a Save-and-start
    /// (near-impossible: the wizard's Verify gate already proved the
    /// key works at the network level, so only a config-shape error
    /// can cause this). When None, the wizard stays in
    /// `WizardPhase::Error` and translation dispatch is unreachable
    /// via normal UI flow — both the hotkey path and the tray
    /// `ID_TRANSLATE` path gate on `AppState::Idle`. Translation
    /// dispatch sites `.as_ref().expect(...)` rely on this invariant.
    provider: Option<std::sync::Arc<dyn LlmProvider>>,

    /// Compiled templates (built-in + user overrides). Immutable for the
    /// lifetime of the app — overrides are validated at startup and
    /// changes require a restart.
    templates: std::sync::Arc<crate::llm::templates::Templates>,

    /// Glossary loaded from `<config_dir>/<cfg.glossary.file>`. Wrapped
    /// in `Arc<RwLock<_>>` so a SIGHUP handler (Task 10) can swap it
    /// atomically without touching anything else. Read access in
    /// `start_translation` is uncontended (no concurrent writes during
    /// rendering since the render path takes a read lock and the SIGHUP
    /// handler takes a write lock).
    glossary: std::sync::Arc<std::sync::RwLock<crate::glossary::Glossary>>,

    /// Path to the glossary file on disk. The SIGHUP handler reuses
    /// this for re-reads.
    glossary_path: PathBuf,

    /// Set to true at startup iff `Glossary::load` returned an error
    /// (file existed but was malformed). Cleared by `reload_glossary`
    /// when a SIGHUP-triggered reload succeeds with at least one entry.
    /// Drives the tray's `Warn(GlossaryMalformed)` status pill.
    glossary_malformed: std::sync::atomic::AtomicBool,

    runtime: Runtime,
    hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    result_tx: mpsc::Sender<translation::TranslationOutcome>,
    result_rx: mpsc::Receiver<translation::TranslationOutcome>,

    /// SIGHUP / glossary-reload signal receiver. A unit value is sent
    /// each time a reload is requested (Task 10 wires the sender).
    glossary_reload_rx: CrossbeamReceiver<()>,

    /// Sender clone for the glossary-reload channel. Tray menu's
    /// "Reload glossary" item sends `()` here; SIGHUP listener also
    /// sends here. The receiver (`glossary_reload_rx` above) is drained
    /// once per frame in `update()`.
    glossary_reload_tx: Option<crossbeam_channel::Sender<()>>,

    /// Encrypted history store. `None` means history was disabled at
    /// config time (`[history] enabled = false`) OR open failed at
    /// startup. The `Arc<History>` lets worker tasks clone the handle
    /// cheaply; `Option` is the "no store" shortcut.
    history: Option<std::sync::Arc<crate::history::store::History>>,

    /// Set to true if history-side errors should short-circuit. Read
    /// by the insert path (atomic check, no lock) and the viewer path
    /// (which surfaces the corruption toast). The flag persists for
    /// the life of the app — the user must restart after fixing the DB
    /// to re-enable history.
    history_disabled: std::sync::Arc<std::sync::atomic::AtomicBool>,

    /// Set to true the first time the corruption toast has been shown
    /// to the user. Mirrors M4's "warned once per session" pattern.
    history_warned: std::sync::atomic::AtomicBool,

    /// API-key resolver. `KeychainSecrets` (preferred) or
    /// `EnvSecrets` (fallback). Read by:
    ///   - `dispatch_rerun_wizard` (tray menu's "Re-run setup wizard"
    ///     item) to seed the wizard model's storage radio default.
    ///   - The 401 toast handler (auto-rerun on stale key).
    ///
    /// Persistence in `persist_setup_completion` goes through a
    /// freshly-constructed `KeychainSecrets` (the M6 stale-account
    /// fix), not through this field — don't regress that.
    /// Currently unread (M8: `keychain_probe` replaced the only
    /// consumer) but kept so future flows can resolve secrets
    /// through the configured backend without re-plumbing the
    /// constructor.
    #[allow(dead_code)]
    secrets: Box<dyn Secrets>,

    /// Tray icon handle. `None` means the tray was disabled by
    /// `state.tray.visible == false` (without `--show-tray`) OR
    /// construction failed at startup. Tray-failure is non-fatal — the
    /// hotkey path still works. Set via `attach_tray` after `new()`.
    tray: Option<crate::tray::TrayHandle>,

    /// Setup-wizard check results channel. The connectivity + sample-
    /// translation tasks send `(SetupCheck, Result<(), String>)` here;
    /// `update_setup_wizard` drains it on every frame.
    setup_check_tx: std::sync::mpsc::Sender<crate::ui::setup::SetupCheckResult>,
    setup_check_rx: std::sync::mpsc::Receiver<crate::ui::setup::SetupCheckResult>,

    /// Captured at startup. The runtime tray-status computation
    /// preserves these warning surfaces unless they're superseded by
    /// higher-priority states (NoApiKey, KeychainStaleKey).
    accessibility_revoked: bool,
    hotkey_in_use: bool,

    /// `global-hotkey` ID for the prompt hotkey. Always set (the prompt
    /// hotkey is always registered).
    prompt_hotkey_id: u32,
    /// `global-hotkey` ID for the history hotkey. `None` if the user
    /// disabled it via `[hotkey.history] enabled = false`.
    history_hotkey_id: Option<u32>,
    /// `global-hotkey` ID for the selected-text hotkey. `None` if the user
    /// disabled it via `[hotkey.selection] enabled = false` or registration
    /// failed.
    selection_hotkey_id: Option<u32>,

    app_state: AppState,
    prompt_model: crate::ui::prompt::PromptModel,

    /// Set to true once the viewport has gained focus after a `show_window`.
    has_been_focused: bool,

    /// Set to true by `show_window`; the next `update_showing` frame
    /// consumes it to call `request_focus` on the last-selected slot row
    /// (or slot 1 if no last slot). After consume, egui's normal Tab
    /// navigation takes over.
    initial_focus_pending: bool,

    /// Monotonically-increasing dispatch counter. Each translation captures
    /// this value at dispatch time; outcomes whose gen ≠ current are dropped
    /// (used for cancellation).
    dispatch_gen: u64,

    /// Timestamp of the most recent translation dispatch. Used to enforce
    /// a minimum interval between consecutive translations (rate limiting).
    last_translation_at: Option<std::time::Instant>,

    /// Whether the user has reduced motion enabled at OS level. Queried
    /// once at construction; the translating overlay reads this to decide
    /// between animated and static rendering.
    reduced_motion: bool,

    /// Set to `true` when the user picks a slot while holding Shift in
    /// the prompt window. Consumed by `start_translation` and carried
    /// through to `AppState::Translating.preview_mode`. Cleared by
    /// `std::mem::take` at consumption.
    pending_preview: bool,

    /// PID of the application that was frontmost before clipt9n showed its
    /// window. Captured in `show_window` / `show_window_from_selection`;
    /// restored in `dismiss_to_idle` so the user lands back in their
    /// previous app after the translation completes or is cancelled.
    previous_app_pid: Option<i32>,
}

impl ClipApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cc: &CreationContext<'_>,
        cfg: Config,
        provider: std::sync::Arc<dyn LlmProvider>,
        templates: std::sync::Arc<crate::llm::templates::Templates>,
        glossary: std::sync::Arc<std::sync::RwLock<crate::glossary::Glossary>>,
        glossary_path: PathBuf,
        glossary_reload_rx: CrossbeamReceiver<()>,
        history: Option<std::sync::Arc<crate::history::store::History>>,
        history_disabled_initial: bool,
        secrets: Box<dyn Secrets>,
        state_path: PathBuf,
        hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
        prompt_hotkey_id: u32,
        history_hotkey_id: Option<u32>,
        selection_hotkey_id: Option<u32>,
        accessibility_revoked: bool,
        hotkey_in_use: bool,
    ) -> Self {
        cc.egui_ctx.set_visuals(theme::visuals());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("clipt9n-async")
            .build()
            .expect("tokio runtime");

        let (result_tx, result_rx) = mpsc::channel();
        let (setup_check_tx, setup_check_rx) =
            std::sync::mpsc::channel::<crate::ui::setup::SetupCheckResult>();
        let state = State::load(&state_path);

        // Cache the OS reduced-motion preference once at startup. Spec
        // a11y baseline accepts a one-shot read.
        let reduced_motion = crate::platform::current().reduced_motion();

        Self {
            prompt_model: crate::ui::prompt::PromptModel {
                clipboard_text: String::new(),
                detected_lang: None,
                last_slot: state.last_slot,
                glossary_hits: vec![],
            },
            cfg,
            state_path,
            state,
            provider: Some(provider),
            templates,
            glossary,
            glossary_path,
            glossary_malformed: std::sync::atomic::AtomicBool::new(false),
            runtime,
            hotkey_rx,
            result_tx,
            result_rx,
            glossary_reload_rx,
            glossary_reload_tx: None,
            history,
            history_disabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                history_disabled_initial,
            )),
            history_warned: std::sync::atomic::AtomicBool::new(false),
            secrets,
            tray: None,
            setup_check_tx,
            setup_check_rx,
            accessibility_revoked,
            hotkey_in_use,
            prompt_hotkey_id,
            history_hotkey_id,
            selection_hotkey_id,
            app_state: AppState::Idle,
            has_been_focused: false,
            initial_focus_pending: false,
            dispatch_gen: 0,
            last_translation_at: None,
            reduced_motion,
            pending_preview: false,
            previous_app_pid: None,
        }
    }

    /// Capture the PID of the currently frontmost application. Called
    /// before showing the prompt window so `dismiss_to_idle` can
    /// restore focus to the user's previous app.
    fn capture_previous_app(&mut self) {
        self.previous_app_pid = crate::platform::current().frontmost_app_pid();
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

    /// Route a slot-pick to the correct intent. Pure logic (slot→action
    /// mapping) lives in `pure::decide_intent`; this method handles the
    /// state-machine transition and persistence side-effects.
    fn dispatch(&mut self, ctx: &egui::Context, slot: u8) {
        let Some(intent) = pure::decide_intent(slot, &self.cfg) else {
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
            pure::Intent::Translate {
                action,
                action_label,
                overlay_label,
            } => {
                self.dispatch_translate(ctx, slot, action, action_label, overlay_label);
            }
            pure::Intent::EnterCustom => {
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

    /// Install the SIGHUP-driven glossary-reload listener against the
    /// app's tokio runtime. Stashes a sender clone so the tray menu's
    /// "Reload glossary" item can trigger reloads via `dispatch_reload_glossary`.
    pub fn install_glossary_reload(&mut self, tx: crossbeam_channel::Sender<()>) {
        let tx_for_sighup = tx.clone();
        self.glossary_reload_tx = Some(tx);
        crate::platform::install_sighup_reload(&self.runtime, tx_for_sighup);
    }

    /// Attach a TrayHandle constructed by main.rs. Called once after
    /// `new()` from inside the eframe creator closure (the only place
    /// where TrayIcon construction is allowed on macOS).
    pub fn attach_tray(&mut self, tray: crate::tray::TrayHandle) {
        self.tray = Some(tray);
    }

    /// Set the malformed flag. Called once at startup from main.rs if
    /// `Glossary::load` returned `Err` (and main.rs fell back to empty).
    pub fn set_glossary_malformed(&self, malformed: bool) {
        self.glossary_malformed
            .store(malformed, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the app is currently displaying the setup wizard.
    /// Used by main.rs to decide whether to show the viewport at startup.
    pub fn is_setup_wizard(&self) -> bool {
        matches!(self.app_state, AppState::SetupWizard { .. })
    }

    /// Override the initial AppState. Used by main.rs to land in
    /// `SetupWizard` instead of `Idle` on first launch.
    pub fn with_initial_state(mut self, state_kind: InitialState) -> Self {
        match state_kind {
            InitialState::Idle => {} // already the default
            InitialState::SetupWizard(model) => {
                self.app_state = AppState::SetupWizard { model };
            }
        }
        self
    }

    /// Dismiss the current overlay and return to `Idle` (window hidden).
    /// Restores focus to the user's previous application.
    fn dismiss_to_idle(&mut self, ctx: &egui::Context) {
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        // Restore focus to the application the user was in before
        // clipt9n summoned its window (e.g. after a translation
        // completes or the user cancels).
        if let Some(pid) = self.previous_app_pid.take() {
            crate::platform::current().activate_pid(pid);
        }
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<(), TranslateError> {
        let mut cb = ArboardClipboard::new()?;
        cb.write_text(text)
    }
}

impl eframe::App for ClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(150));

        self.drain_channels(ctx);
        self.drain_tray_events(ctx);

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
        //
        // Clear `previous_app_pid` before dismissing — the user deliberately
        // clicked away to another app, so we must not yank them back.
        let focused = ctx.input(|i| i.focused);
        if pure::update_focus_loss_latch(focused, &mut self.has_been_focused) {
            self.previous_app_pid = None;
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
                preview_mode,
            } => {
                self.update_translating(
                    ctx,
                    gen,
                    action_label,
                    overlay_label,
                    started_at,
                    preview_mode,
                );
            }
            AppState::ShowingHistory { model } => self.update_showing_history(ctx, model),
            AppState::SetupWizard { model } => self.update_setup_wizard(ctx, model),
            AppState::ShowingResult {
                result_text,
                source_text,
                action_label,
                detected_source_lang,
                action,
                slot,
            } => {
                self.update_showing_result(
                    ctx,
                    result_text,
                    source_text,
                    action_label,
                    detected_source_lang,
                    action,
                    slot,
                );
            }
            AppState::ConfirmingTrayHide { model } => {
                self.update_confirming_tray_hide(ctx, model);
            }
        }

        self.refresh_tray_status();
    }
}
