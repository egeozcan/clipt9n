//! `ClipApp` is the eframe application: it owns the tokio runtime, the
//! channels to/from the hotkey thread and the translation worker, and the
//! prompt-window state machine. All UI is paint-only (`src/ui/prompt.rs`);
//! input handling lives here.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crossbeam_channel::Receiver as CrossbeamReceiver;
use eframe::CreationContext;
use egui::{Key, Vec2, ViewportCommand};
use global_hotkey::GlobalHotKeyEvent;
use tokio::runtime::Runtime;

use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::LlmProvider;
use crate::platform::Platform;
use crate::secrets::Secrets;
use crate::state::State;
use crate::translator::{Action, Translator};
#[allow(unused_imports)]
use crate::ui::{custom_prompt as prompt_custom, prompt, size_confirm, theme, translating};

/// Initial state override passed by `main.rs` to ClipApp at construction.
/// Used to land the app in the setup wizard on first launch instead of
/// the default Idle state.
pub enum InitialState {
    Idle,
    SetupWizard(crate::ui::setup::SetupWizardModel),
}

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

    runtime: Runtime,
    hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    result_tx: mpsc::Sender<TranslationOutcome>,
    result_rx: mpsc::Receiver<TranslationOutcome>,

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
    /// `EnvSecrets` (fallback). Currently dead-allocated — Task 9 of
    /// M7 revives this field as the source of `keychain_available()`
    /// for the tray menu's "Re-run setup wizard" entry. Keep the
    /// `#[allow(dead_code)]` until then; persistence in
    /// `persist_setup_completion` goes through a freshly-constructed
    /// `KeychainSecrets` (M6 stale-account fix), not through this.
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

    app_state: AppState,
    prompt_model: prompt::PromptModel,

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
    /// ISO-2 source language detected at dispatch time (carries into
    /// the history row's `source_lang` column on success). `None` if
    /// `whatlang` confidence was below the threshold.
    detected_source_lang: Option<String>,
    /// The source text we fed to the translator. The history insert
    /// path uses this to compute `char_count` and (when
    /// `[history] store_text = true`) to encrypt as the source column.
    source_text: String,
    /// Action that produced this outcome — used to fill the history
    /// row's `action` and `target_lang` columns. Cloned at dispatch
    /// time so the worker doesn't hold a `&Action` reference.
    action: Action,
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
            prompt_model: prompt::PromptModel {
                clipboard_text: String::new(),
                detected_lang: None,
                last_slot: state.last_slot,
                glossary_hits: vec![],
            },
            cfg,
            state_path,
            state,
            provider,
            templates,
            glossary,
            glossary_path,
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
            app_state: AppState::Idle,
            has_been_focused: false,
            initial_focus_pending: false,
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

        // Compute pair-agnostic glossary hits for the chip strip preview.
        // The translator applies pair scoping correctly at execute time;
        // this preview is informational only.
        let detected_iso3 = crate::glossary::detect_source_lang(&self.prompt_model.clipboard_text);
        let source_iso2 = detected_iso3
            .as_deref()
            .and_then(crate::glossary::iso3_to_iso2);
        self.prompt_model.detected_lang = source_iso2.map(String::from);
        let g = self.glossary.read().expect("glossary RwLock poisoned");
        self.prompt_model.glossary_hits = g
            .preview_entries(
                &self.prompt_model.clipboard_text,
                source_iso2,
                &self.cfg.glossary,
            )
            .into_iter()
            .map(|e| prompt::GlossaryHit {
                source: e.source.clone(),
                target: e.target.clone(),
            })
            .collect();
        drop(g); // release the read lock before mutating other state.

        self.has_been_focused = false;
        self.initial_focus_pending = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::Showing;
    }

    fn dispatch(&mut self, ctx: &egui::Context, slot: u8) {
        let Some(intent) = decide_intent(slot, &self.cfg) else {
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
            Intent::Translate {
                action,
                action_label,
                overlay_label,
            } => {
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
        // Worker: runs the translation. May panic (e.g., a malformed-UTF-8
        // bug in post-processing). On panic, the JoinHandle returns Err and
        // the watcher below converts it into a TranslationOutcome with an
        // Internal error — guaranteeing `tx.send` always fires exactly once,
        // so the overlay never gets stuck.
        let label_for_panic = action_label.clone();
        let templates = self.templates.clone();
        let glossary = self.glossary.clone();
        let detected_source = self.prompt_model.detected_lang.clone();
        let action_for_outcome = action.clone();
        let source_text_for_outcome = source_text.clone();
        let worker = self.runtime.spawn(async move {
            // Take a read snapshot of the glossary at dispatch time. If a
            // SIGHUP-driven reload arrives mid-translation, the running
            // worker uses the snapshot it captured here; the next dispatch
            // sees the new entries.
            let g_snapshot = glossary.read().expect("glossary RwLock poisoned").clone();
            let translator = Translator::new(&cfg, provider.as_ref(), &templates, &g_snapshot);
            let result = translator.execute(&action, &source_text).await;
            TranslationOutcome {
                result,
                action_label,
                slot,
                gen,
                detected_source_lang: detected_source,
                source_text: source_text_for_outcome,
                action: action_for_outcome,
            }
        });
        self.runtime.spawn(async move {
            let outcome = match worker.await {
                Ok(o) => o,
                Err(join_err) => {
                    tracing::error!(error = %join_err, "translation worker panicked");
                    TranslationOutcome {
                        result: Err(TranslateError::Internal(format!(
                            "translation worker crashed: {join_err}"
                        ))),
                        action_label: label_for_panic,
                        slot,
                        gen,
                        detected_source_lang: None,
                        source_text: String::new(),
                        action: Action::FixGrammar, // placeholder — never read on Err
                    }
                }
            };
            let _ = tx.send(outcome);
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
            Ok(ref translated) => {
                let mut cb = match ArboardClipboard::new() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(error = %e, "clipboard open failed");
                        self.app_state = AppState::Idle;
                        return;
                    }
                };
                if let Err(e) = cb.write_text(translated) {
                    tracing::error!(error = %e, "clipboard write failed");
                } else {
                    if let Err(e) = crate::notify::translation_copied(&outcome.action_label) {
                        tracing::warn!(error = %e, "notification failed");
                    }
                    // History insert: best-effort, AFTER clipboard write
                    // succeeds, NEVER blocks the user's primary outcome.
                    self.schedule_history_insert(&outcome, translated);
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
        while let Ok(event) = self.hotkey_rx.try_recv() {
            let is_prompt = event.id == self.prompt_hotkey_id;
            let is_history = self
                .history_hotkey_id
                .map(|id| event.id == id)
                .unwrap_or(false);
            if is_prompt {
                if matches!(self.app_state, AppState::Idle) {
                    self.show_window(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else if is_history {
                if matches!(self.app_state, AppState::Idle) {
                    self.summon_history(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else {
                tracing::debug!(
                    event_id = event.id,
                    "ignoring hotkey event from unregistered ID"
                );
            }
        }
        // Translation results
        while let Ok(outcome) = self.result_rx.try_recv() {
            self.handle_translation_done(outcome);
        }
        // Glossary reload requests (SIGHUP, tray menu in M7)
        let mut reload_requested = false;
        while self.glossary_reload_rx.try_recv().is_ok() {
            reload_requested = true;
        }
        if reload_requested {
            self.reload_glossary();
        }
    }

    fn reload_glossary(&mut self) {
        // Sync I/O on the egui update thread is acceptable here because
        // glossary files are small (typically <10KB) and the reload is
        // user-driven (SIGHUP / future tray menu), not periodic.
        match crate::glossary::Glossary::load(&self.glossary_path) {
            Ok(g) => {
                let entry_count = g.len();
                *self.glossary.write().expect("glossary RwLock poisoned") = g;
                tracing::info!(
                    path = %self.glossary_path.display(),
                    entries = entry_count,
                    "glossary reloaded"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %self.glossary_path.display(),
                    "glossary reload failed; keeping previous entries"
                );
            }
        }
    }

    /// Drain pending tray menu events and dispatch. Called once per
    /// frame from `update()`. The static `MenuEvent` channel is drained
    /// unconditionally — events queued from a previous tray that's
    /// since been dropped would otherwise accumulate forever and fire
    /// stale on a future re-attach (Task 7's --show-tray recovery
    /// path). Dispatch is gated on `self.tray.is_some()`.
    fn drain_tray_events(&mut self, ctx: &egui::Context) {
        // Drain the global static MenuEvent channel unconditionally —
        // events queued from a previous tray that's since been dropped
        // would otherwise accumulate forever and fire stale on a future
        // re-attach (Task 7's --show-tray recovery path).
        let Some(id) = crate::tray::TrayHandle::try_drain_menu_event() else {
            return;
        };
        if self.tray.is_none() {
            // Tray not active; silently discard.
            return;
        }
        match id.as_str() {
            crate::tray::ID_TRANSLATE => self.show_window(ctx),
            crate::tray::ID_HISTORY => self.summon_history(ctx),
            crate::tray::ID_GLOSSARY_OPEN => self.dispatch_open_glossary(),
            crate::tray::ID_GLOSSARY_RELOAD => self.dispatch_reload_glossary(),
            // Re-run wizard, Hide, Quit — wired in Tasks 7, 9.
            other => {
                tracing::debug!(id = %other, "tray menu event (handler not yet wired)");
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

    /// Compute the appropriate `TrayStatus` for the current app state.
    /// Called every frame from `update()`; the per-frame cost is one
    /// match plus a glossary-empty check (cheap RwLock read).
    ///
    /// Priority: NoApiKey > Warn(AccessibilityPermissionRevoked) >
    /// Warn(HotkeyInUse) > Warn(GlossaryMalformed) > Ready.
    /// Warn(KeychainStaleKey) is set transiently by handle_translation_done
    /// (Task 9) and persists until the next status transition.
    pub(crate) fn compute_tray_status(&self) -> crate::tray::TrayStatus {
        // Highest priority: missing API key (the wizard would be open
        // anyway, but tray status mirrors the underlying state).
        if matches!(self.app_state, AppState::SetupWizard { .. }) {
            return crate::tray::TrayStatus::NoApiKey;
        }
        if self.accessibility_revoked {
            return crate::tray::TrayStatus::Warn(
                crate::tray::WarnReason::AccessibilityPermissionRevoked,
            );
        }
        if self.hotkey_in_use {
            return crate::tray::TrayStatus::Warn(crate::tray::WarnReason::HotkeyInUse);
        }
        if self.glossary_was_malformed_at_startup() {
            return crate::tray::TrayStatus::Warn(crate::tray::WarnReason::GlossaryMalformed);
        }
        crate::tray::TrayStatus::Ready
    }

    /// Whether the M4 startup glossary load fell back to empty due to
    /// a parse error. M4 logs a warn but doesn't surface to the UI;
    /// M7 bridges that into the tray status.
    fn glossary_was_malformed_at_startup(&self) -> bool {
        // Inspect the live glossary: if it's empty AND the file
        // exists on disk AND the file is non-empty, we know the
        // startup load fell back. Reload-via-SIGHUP that succeeds
        // re-fills the live glossary, so this self-clears.
        let g = match self.glossary.read() {
            Ok(g) => g,
            Err(_) => return false, // poisoned lock — treat as not-malformed
        };
        if !g.is_empty() {
            return false;
        }
        drop(g);
        matches!(std::fs::metadata(&self.glossary_path), Ok(m) if m.len() > 0)
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

    /// Best-effort: persist a successful translation to the history
    /// store. Runs on the tokio runtime. Failures are logged at warn;
    /// panics are caught by the watcher and similarly logged. The user
    /// never sees a toast — the clipboard write is the primary outcome
    /// and has already happened by the time we get here.
    fn schedule_history_insert(&self, outcome: &TranslationOutcome, translated: &str) {
        // Short-circuit if history is disabled (config or corruption).
        let Some(history) = self.history.clone() else {
            return;
        };
        if self
            .history_disabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let history_disabled = self.history_disabled.clone();
        let max_entries = self.cfg.history.max_entries;
        let store_text = self.cfg.history.store_text;
        let source_text = outcome.source_text.clone();
        let result_text = translated.to_string();
        let action = outcome.action.clone();
        let detected = outcome.detected_source_lang.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let entry = crate::history::store::NewEntry {
            created_at: now,
            action: action_kind_str(&action).to_string(),
            source_lang: detected,
            target_lang: target_lang_for(&action),
            char_count: source_text.chars().count() as i64,
            source: if store_text { Some(source_text) } else { None },
            result: if store_text { Some(result_text) } else { None },
        };

        let inner = self.runtime.spawn(async move {
            // history is Arc<History> (already unwrapped from the Option
            // above). insert_with_cap takes &self.
            if let Err(e) = history.insert_with_cap(entry, max_entries) {
                tracing::warn!(error = %e, "history insert failed; row dropped");
                // Don't disable globally — a transient SQLite error
                // (e.g., disk full) shouldn't permanently take down
                // history. Corruption-class errors set the flag at
                // open time, not at insert time.
                let _ = history_disabled; // suppress unused warning
            }
        });
        // Watcher: catch a panic in the inner task and log it.
        self.runtime.spawn(async move {
            if let Err(join_err) = inner.await {
                tracing::warn!(
                    error = %join_err,
                    "history insert panicked; row dropped"
                );
            }
        });
    }

    fn dismiss_to_idle(&mut self, ctx: &egui::Context) {
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    fn update_showing(&mut self, ctx: &egui::Context) {
        // prompt_model was snapshotted at show_window(); we re-draw it each
        // frame against the same snapshot so the user sees a stable view
        // until they dismiss or pick a slot.
        //
        // On the first frame after summon, request_focus on the slot the
        // user last picked (or slot 1 by default). With focus pre-set,
        // Enter on that row directly fires Pick(n) via egui's built-in
        // Sense::click handling — no separate "RepeatLast" key shortcut
        // needed; the focused row IS the repeat target.
        let focus_target = if self.initial_focus_pending {
            self.initial_focus_pending = false;
            Some(self.state.last_slot.unwrap_or(1))
        } else {
            None
        };
        let click = prompt::draw(ctx, &self.cfg, &self.prompt_model, focus_target);
        let key = self.handle_keys_showing(ctx);
        let outcome = click.or(key);
        match outcome {
            Some(prompt::PromptOutcome::Pick(n)) => {
                // dispatch() may transition to any of the new states; restore
                // Showing only if dispatch didn't transition.
                self.app_state = AppState::Showing;
                self.dispatch(ctx, n);
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

    fn handle_keys_showing(&self, ctx: &egui::Context) -> Option<prompt::PromptOutcome> {
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(prompt::PromptOutcome::Cancel);
            }
            // No global Enter handler: a slot row is always focused after
            // show_window (initial_focus_pending), and egui's Sense::click
            // handling fires the focused row's clicked() on Enter. The
            // 1-6 number keys remain as direct shortcuts.
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

    /// Open the history viewer. Queries the store, builds a model,
    /// resizes the viewport to 680×540, and transitions to
    /// `ShowingHistory`. If history is disabled (config or corruption),
    /// the viewer still opens but with a warning banner; this lets the
    /// user verify the toast and explore an empty (or partially
    /// readable) database.
    fn summon_history(&mut self, ctx: &egui::Context) {
        let mut model = crate::ui::history::HistoryModel::default();
        let disabled = self
            .history_disabled
            .load(std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.history.as_ref() {
            match h.query(
                &crate::history::store::QueryFilter::default(),
                self.cfg.history.max_entries,
            ) {
                Ok(rows) => {
                    model.entries = rows;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "history query failed; viewer will show empty");
                    self.history_disabled
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        // First time? Show banner. Latch via the App-level warned flag.
        if (disabled
            || self
                .history_disabled
                .load(std::sync::atomic::Ordering::Relaxed))
            && !self
                .history_warned
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            model.show_corruption_banner = true;
            self.history_warned
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // Resize viewport for history viewer.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(680.0, 540.0)));
        self.has_been_focused = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::ShowingHistory { model };
    }

    fn update_showing_history(
        &mut self,
        ctx: &egui::Context,
        mut model: crate::ui::history::HistoryModel,
    ) {
        let click_outcome = crate::ui::history::draw(ctx, &mut model);
        // Banner is one-shot per session — clear AFTER the first draw
        // renders it, so subsequent frames don't re-render it.
        model.show_corruption_banner = false;
        let key_outcome = self.handle_keys_history(ctx, &mut model);
        let outcome = click_outcome.or(key_outcome);

        match outcome {
            Some(crate::ui::history::HistoryOutcome::Close) => {
                self.dismiss_history_to_idle(ctx);
            }
            Some(crate::ui::history::HistoryOutcome::CopyResult(id)) => {
                if let Some(entry) = model.entries.iter().find(|e| e.id == id) {
                    if let Some(result) = entry.result.as_ref() {
                        let _ = self.copy_to_clipboard(result.as_str());
                    }
                }
                self.dismiss_history_to_idle(ctx);
            }
            Some(crate::ui::history::HistoryOutcome::CopySource(id)) => {
                if let Some(entry) = model.entries.iter().find(|e| e.id == id) {
                    if let Some(source) = entry.source.as_ref() {
                        let _ = self.copy_to_clipboard(source.as_str());
                    }
                }
                // Stay open; user may want to copy more.
                self.app_state = AppState::ShowingHistory { model };
            }
            Some(crate::ui::history::HistoryOutcome::Delete(id)) => {
                if let Some(h) = self.history.as_ref() {
                    if let Err(e) = h.delete(id) {
                        tracing::warn!(error = %e, id, "history delete failed");
                    }
                }
                // Re-query so the list reflects the deletion.
                self.refresh_history_model(&mut model);
                self.app_state = AppState::ShowingHistory { model };
            }
            Some(crate::ui::history::HistoryOutcome::ClearAll) => {
                if let Some(h) = self.history.as_ref() {
                    if let Err(e) = h.clear_all() {
                        tracing::warn!(error = %e, "history clear_all failed");
                    }
                }
                self.refresh_history_model(&mut model);
                self.app_state = AppState::ShowingHistory { model };
            }
            None => {
                self.app_state = AppState::ShowingHistory { model };
            }
        }
    }

    fn handle_keys_history(
        &self,
        ctx: &egui::Context,
        model: &mut crate::ui::history::HistoryModel,
    ) -> Option<crate::ui::history::HistoryOutcome> {
        // If the modal is up, only Esc/Enter act on it.
        if model.confirm_clear {
            return ctx.input(|i| {
                if i.key_pressed(Key::Escape) {
                    model.confirm_clear = false;
                    None
                } else if i.key_pressed(Key::Enter) {
                    model.confirm_clear = false;
                    Some(crate::ui::history::HistoryOutcome::ClearAll)
                } else {
                    None
                }
            });
        }

        // Apply filter to find the focused row's id (we only act on it
        // for s/d/Enter shortcuts).
        let filtered = crate::ui::history::filter_entries(&model.entries, &model.query);
        let focused_id = filtered.get(model.selected).map(|e| e.id);

        let len = filtered.len();
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(crate::ui::history::HistoryOutcome::Close);
            }
            if i.key_pressed(Key::ArrowDown) && len > 0 {
                model.selected = (model.selected + 1).min(len - 1);
            }
            if i.key_pressed(Key::ArrowUp) && len > 0 {
                model.selected = model.selected.saturating_sub(1);
            }
            if i.key_pressed(Key::Delete) && i.modifiers.shift {
                if self.cfg.history.confirm_clear {
                    model.confirm_clear = true;
                    return None;
                }
                return Some(crate::ui::history::HistoryOutcome::ClearAll);
            }
            if i.key_pressed(Key::Enter) {
                if let Some(id) = focused_id {
                    return Some(crate::ui::history::HistoryOutcome::CopyResult(id));
                }
            }
            // For 's' / 'd', reject when a Text event of that letter
            // was emitted this frame — that means the search field
            // captured it (so the user is typing into the search box).
            let typed_letters: std::collections::HashSet<char> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(s) if s.len() == 1 => s.chars().next(),
                    _ => None,
                })
                .collect();
            if i.key_pressed(Key::S) && !typed_letters.contains(&'s') {
                if let Some(id) = focused_id {
                    return Some(crate::ui::history::HistoryOutcome::CopySource(id));
                }
            }
            if i.key_pressed(Key::D) && !typed_letters.contains(&'d') {
                if let Some(id) = focused_id {
                    return Some(crate::ui::history::HistoryOutcome::Delete(id));
                }
            }
            None
        })
    }

    fn refresh_history_model(&self, model: &mut crate::ui::history::HistoryModel) {
        if let Some(h) = self.history.as_ref() {
            match h.query(
                &crate::history::store::QueryFilter::default(),
                self.cfg.history.max_entries,
            ) {
                Ok(rows) => {
                    model.entries = rows;
                    if model.selected >= model.entries.len() {
                        model.selected = 0;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "history re-query failed"),
            }
        }
    }

    fn update_setup_wizard(
        &mut self,
        ctx: &egui::Context,
        mut model: crate::ui::setup::SetupWizardModel,
    ) {
        // First, drain any check results sitting on our channel.
        while let Ok((check, result)) = self.setup_check_rx.try_recv() {
            match (check, result) {
                (crate::ui::setup::SetupCheck::Connectivity, Ok(())) => {
                    model.check1 = crate::ui::setup::CheckStatus::Ok;
                    if !model.test_translation {
                        // Skip check2; advance to Done.
                        model.phase = crate::ui::setup::WizardPhase::Done;
                    } else {
                        // Kick off check2.
                        model.check2 = crate::ui::setup::CheckStatus::Running;
                        self.spawn_sample_translation_check(&model.provider, model.key.clone());
                    }
                }
                (crate::ui::setup::SetupCheck::Connectivity, Err(msg)) => {
                    model.check1 = crate::ui::setup::CheckStatus::Fail;
                    model.err_msg = msg;
                    model.phase = crate::ui::setup::WizardPhase::Error;
                }
                (crate::ui::setup::SetupCheck::SampleTranslation, Ok(())) => {
                    model.check2 = crate::ui::setup::CheckStatus::Ok;
                    model.phase = crate::ui::setup::WizardPhase::Done;
                }
                (crate::ui::setup::SetupCheck::SampleTranslation, Err(msg)) => {
                    model.check2 = crate::ui::setup::CheckStatus::Fail;
                    model.err_msg = msg;
                    model.phase = crate::ui::setup::WizardPhase::Error;
                }
            }
        }

        let outcome = crate::ui::setup::draw(ctx, &mut model);

        match outcome {
            Some(crate::ui::setup::SetupOutcome::Cancel) => {
                tracing::warn!("setup wizard cancelled — no API key persisted");
                self.dismiss_setup_to_idle(ctx);
            }
            Some(crate::ui::setup::SetupOutcome::Verify) => {
                model.phase = crate::ui::setup::WizardPhase::Verifying;
                model.check1 = crate::ui::setup::CheckStatus::Running;
                model.check2 = crate::ui::setup::CheckStatus::Idle;
                model.err_msg.clear();
                self.spawn_connectivity_check(&model.provider, model.key.clone());
                self.app_state = AppState::SetupWizard { model };
            }
            Some(crate::ui::setup::SetupOutcome::SaveAndStart) => {
                if let Err(e) = self.persist_setup_completion(&model) {
                    tracing::error!(error = %e, "setup wizard persist failed");
                    model.err_msg = format!("save failed: {e}");
                    model.phase = crate::ui::setup::WizardPhase::Error;
                    self.app_state = AppState::SetupWizard { model };
                    return;
                }
                self.dismiss_setup_to_idle(ctx);
            }
            Some(crate::ui::setup::SetupOutcome::OpenConfig) => {
                let cfg_path = self.state_path.parent().map(|p| p.join("config.toml"));
                if let Some(p) = cfg_path {
                    let plat = crate::platform::current();
                    if let Err(e) = plat.open_path(&p) {
                        tracing::warn!(error = %e, "open_path failed");
                    }
                }
                self.app_state = AppState::SetupWizard { model };
            }
            None => {
                self.app_state = AppState::SetupWizard { model };
            }
        }
    }

    fn spawn_connectivity_check(&self, provider: &str, key: zeroize::Zeroizing<String>) {
        // Use the wizard-selected provider's default base URL — the
        // live cfg.provider.base_url may not match the wizard's
        // selection until Save-and-start rewrites the config.
        let provider = provider.to_string();
        let base_url = crate::ui::setup::default_base_url(&provider).to_string();
        let tx = self.setup_check_tx.clone();
        let runtime = self.runtime.handle().clone();
        runtime.spawn(async move {
            let result = run_connectivity_check(&provider, &base_url, &key).await;
            // One auto-retry per spec §13.
            let final_result = if result.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                run_connectivity_check(&provider, &base_url, &key).await
            } else {
                result
            };
            let _ = tx.send((
                crate::ui::setup::SetupCheck::Connectivity,
                final_result.map_err(|e| e.to_string()),
            ));
        });
    }

    fn spawn_sample_translation_check(&self, provider_kind: &str, key: zeroize::Zeroizing<String>) {
        // The wizard's selected provider may differ from the running
        // self.provider (which was built from the cfg at startup, possibly
        // with a placeholder key). Build a fresh provider from the
        // wizard's typed key + the wizard's selected provider kind +
        // the kind-default base URL.
        let provider_kind = provider_kind.to_string();
        let cfg = self.cfg.clone();
        let templates = self.templates.clone();
        let glossary = self.glossary.clone();
        let tx = self.setup_check_tx.clone();
        let runtime = self.runtime.handle().clone();
        runtime.spawn(async move {
            let timeout = std::time::Duration::from_secs(cfg.provider.timeout_seconds);
            let base_url = crate::ui::setup::default_base_url(&provider_kind);
            // NOTE: This intentionally bypasses crate::llm::factory::build_provider
            // because the wizard wants the per-provider default base URL (from
            // setup::default_base_url) rather than cfg.provider.base_url — the user
            // might be configuring a fresh provider whose base_url hasn't been
            // persisted yet. A follow-up may extend the factory with an
            // Option<&str> base-URL override; until then this stays hand-rolled.
            let provider_result: Result<
                std::sync::Arc<dyn crate::llm::LlmProvider>,
                TranslateError,
            > = match provider_kind.as_str() {
                "anthropic" => crate::llm::anthropic::AnthropicProvider::new(
                    base_url,
                    key.clone(),
                    &cfg.provider.model,
                    timeout,
                )
                .map(|p| std::sync::Arc::new(p) as std::sync::Arc<dyn crate::llm::LlmProvider>),
                _ => crate::llm::openai::OpenAiCompatibleProvider::new(
                    base_url,
                    key.clone(),
                    &cfg.provider.model,
                    timeout,
                )
                .map(|p| std::sync::Arc::new(p) as std::sync::Arc<dyn crate::llm::LlmProvider>),
            };
            let provider = match provider_result {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send((
                        crate::ui::setup::SetupCheck::SampleTranslation,
                        Err(e.to_string()),
                    ));
                    return;
                }
            };
            let action = crate::translator::Action::Translate { code: "de".into() };
            let attempt = || async {
                let g_snapshot = glossary.read().expect("glossary RwLock poisoned").clone();
                let translator = crate::translator::Translator::new(
                    &cfg,
                    provider.as_ref(),
                    &templates,
                    &g_snapshot,
                );
                translator.execute(&action, "Hello, world.").await
            };
            let result = attempt().await;
            // One auto-retry per spec §13.
            let final_result = if result.is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                attempt().await
            } else {
                result
            };
            let _ = tx.send((
                crate::ui::setup::SetupCheck::SampleTranslation,
                final_result.map(|_| ()).map_err(|e| e.to_string()),
            ));
        });
    }

    fn dismiss_setup_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::prompt_default_inner_size(&self.cfg.ui),
        ));
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn persist_setup_completion(
        &mut self,
        model: &crate::ui::setup::SetupWizardModel,
    ) -> Result<(), TranslateError> {
        // Update in-memory config.
        self.cfg.provider.kind = model.provider.clone();
        let new_source = match model.storage {
            crate::ui::setup::Storage::Keychain => "keychain",
            crate::ui::setup::Storage::Env => "env",
        };
        self.cfg.provider.api_key.source = new_source.into();
        self.cfg.provider.api_key.account = model.provider.clone();
        // Update env-var name for the env case so the user knows what
        // to export.
        let (_, _, env_var) = crate::ui::setup::provider_meta(&model.provider);
        self.cfg.provider.api_key.env_var = env_var.into();

        // Persist config to disk.
        let cfg_path = self
            .state_path
            .parent()
            .map(|p| p.join("config.toml"))
            .ok_or_else(|| TranslateError::Config("state path has no parent".into()))?;
        self.cfg.persist(&cfg_path)?;

        // Persist the key to the chosen backend. Env-storage logs a
        // warning that the user must set the env var manually — the
        // wizard already showed them the variable name.
        match model.storage {
            crate::ui::setup::Storage::Keychain => {
                // Build a fresh KeychainSecrets from the just-updated
                // cfg so the key lands in the correct slot when the
                // user switches providers in the wizard. self.secrets
                // was constructed at startup from the OLD account and
                // would write to the stale slot.
                let fresh = crate::secrets::KeychainSecrets::new(
                    &self.cfg.provider.api_key.service,
                    &self.cfg.provider.api_key.account,
                );
                fresh.set_api_key(model.key.clone())?;
            }
            crate::ui::setup::Storage::Env => {
                tracing::warn!(
                    env_var = %env_var,
                    "setup wizard: storage=env — user must set the env var manually before next launch"
                );
            }
        }
        Ok(())
    }

    fn dismiss_history_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::prompt_default_inner_size(&self.cfg.ui),
        ));
        self.app_state = AppState::Idle;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
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
            AppState::ShowingHistory { model } => self.update_showing_history(ctx, model),
            AppState::SetupWizard { model } => self.update_setup_wizard(ctx, model),
        }

        self.refresh_tray_status();
    }
}

// -----------------------------------------------------------------------
// Async helpers
// -----------------------------------------------------------------------

async fn run_connectivity_check(
    provider: &str,
    base_url: &str,
    key: &str,
) -> Result<(), TranslateError> {
    let (url, headers) = crate::ui::setup::connectivity_request(provider, base_url, key);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| TranslateError::Network(e.to_string()))?;
    let mut req = client.get(&url);
    for (k, v) in headers {
        req = req.header(&k, &v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| TranslateError::Network(e.to_string()))?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else if status.as_u16() == 401 {
        Err(TranslateError::SetupWizard(format!(
            "{} Invalid API key",
            status.as_u16()
        )))
    } else {
        Err(TranslateError::Provider {
            status: status.as_u16(),
            message: status.canonical_reason().unwrap_or("provider error").into(),
        })
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

pub(crate) fn decide_intent(slot: u8, cfg: &Config) -> Option<Intent> {
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

/// Map an `Action` to the string we persist in `entries.action`. Must
/// match the `'translate' | 'fix_grammar' | 'rewrite' | 'custom'`
/// alphabet from spec §7.
fn action_kind_str(action: &Action) -> &'static str {
    match action {
        Action::Translate { .. } => "translate",
        Action::FixGrammar => "fix_grammar",
        Action::Rewrite => "rewrite",
        Action::Custom { .. } => "custom",
    }
}

/// Target language for the history row. `None` for fix_grammar /
/// rewrite / custom (which stay in source language); `Some(code)` for
/// translate.
fn target_lang_for(action: &Action) -> Option<String> {
    match action {
        Action::Translate { code } => Some(code.clone()),
        _ => None,
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
        let intent = decide_intent(1, &cfg).expect("slot 1 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
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
        let intent = decide_intent(2, &cfg).expect("slot 2 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
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
        let intent = decide_intent(3, &cfg).expect("slot 3 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
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
        let intent = decide_intent(4, &cfg).expect("slot 4 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        assert!(matches!(action, Action::FixGrammar));
        assert_eq!(action_label, "Fix grammar");
        assert_eq!(overlay_label, "Fixing grammar…");
    }

    #[test]
    fn slot_5_resolves_to_rewrite_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(5, &cfg).expect("slot 5 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        assert!(matches!(action, Action::Rewrite));
        assert_eq!(action_label, "Rewrite for clarity");
        assert_eq!(overlay_label, "Rewriting for clarity…");
    }

    #[test]
    fn slot_6_resolves_to_enter_custom() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(6, &cfg).expect("slot 6 is valid");
        assert!(matches!(intent, Intent::EnterCustom));
    }

    #[test]
    fn invalid_slot_returns_none() {
        let cfg = cfg_with_threshold(2000);
        assert!(decide_intent(0, &cfg).is_none());
        assert!(decide_intent(7, &cfg).is_none());
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
        assert_eq!(
            overlay_label_for(&Action::Rewrite),
            "Rewriting for clarity…"
        );
        assert_eq!(
            overlay_label_for(&Action::Custom {
                instruction: "x".into()
            }),
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
        assert_eq!(
            action_label_for(&Action::Rewrite, &cfg),
            "Rewrite for clarity"
        );
        assert_eq!(
            action_label_for(
                &Action::Custom {
                    instruction: "anything".into()
                },
                &cfg
            ),
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
        assert!(requires_size_confirm(
            "this is definitely longer than ten characters",
            &cfg
        ));
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

    #[test]
    fn action_kind_str_maps_per_spec() {
        assert_eq!(
            action_kind_str(&Action::Translate { code: "de".into() }),
            "translate"
        );
        assert_eq!(action_kind_str(&Action::FixGrammar), "fix_grammar");
        assert_eq!(action_kind_str(&Action::Rewrite), "rewrite");
        assert_eq!(
            action_kind_str(&Action::Custom {
                instruction: "x".into()
            }),
            "custom"
        );
    }

    #[test]
    fn target_lang_for_only_set_on_translate() {
        assert_eq!(
            target_lang_for(&Action::Translate { code: "de".into() }),
            Some("de".to_string())
        );
        assert_eq!(target_lang_for(&Action::FixGrammar), None);
        assert_eq!(target_lang_for(&Action::Rewrite), None);
        assert_eq!(
            target_lang_for(&Action::Custom {
                instruction: "x".into()
            }),
            None
        );
    }
}
