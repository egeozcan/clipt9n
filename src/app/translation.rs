//! Translation worker — spawn, outcome handling, history insertion.
//! Extracted from `app/mod.rs` Step 2 of the improvement plan.

use std::time::Duration;

use super::pure;
use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::error::TranslateError;
use crate::history::store::NewEntry;
use crate::platform::Platform;
use crate::translator::{Action, Translator};

/// Outcome from a translation worker. Sent from the tokio runtime to the
/// main thread via `result_tx` / `result_rx`.
#[derive(Debug)]
pub(super) struct TranslationOutcome {
    pub(super) result: Result<String, TranslateError>,
    pub(super) action_label: String,
    pub(super) slot: u8,
    /// Dispatch-generation that produced this outcome. If `App.dispatch_gen`
    /// has advanced since dispatch, this outcome is stale (user cancelled).
    pub(super) gen: u64,
    /// ISO-2 source language detected at dispatch time (carries into
    /// the history row's `source_lang` column on success). `None` if
    /// `whatlang` confidence was below the threshold.
    pub(super) detected_source_lang: Option<String>,
    /// The source text we fed to the translator. The history insert
    /// path uses this to compute `char_count` and (when
    /// `[history] store_text = true`) to encrypt as the source column.
    pub(super) source_text: String,
    /// Action that produced this outcome — used to fill the history
    /// row's `action` and `target_lang` columns. Cloned at dispatch
    /// time so the worker doesn't hold a `&Action` reference.
    pub(super) action: Action,
}

impl super::ClipApp {
    /// Single fork point for "we know what to translate; should we ask the
    /// user to confirm first?". Either transitions to ConfirmingSize or
    /// calls start_translation directly.
    pub(super) fn dispatch_translate(
        &mut self,
        ctx: &egui::Context,
        slot: u8,
        action: Action,
        action_label: String,
        overlay_label: String,
    ) {
        // Rate limit: enforce a minimum interval between consecutive
        // translations to prevent rapid-fire dispatch bursts (e.g.,
        // alternating hotkey presses and dismissals).
        const MIN_TRANSLATION_INTERVAL: Duration = Duration::from_millis(500);
        if let Some(last) = self.last_translation_at {
            if last.elapsed() < MIN_TRANSLATION_INTERVAL {
                return;
            }
        }
        self.last_translation_at = Some(std::time::Instant::now());

        if pure::requires_size_confirm(&self.prompt_model.clipboard_text, &self.cfg) {
            let preview =
                crate::ui::size_confirm::format_preview(&self.prompt_model.clipboard_text);
            let char_count = self.prompt_model.clipboard_text.chars().count();
            // Don't consume pending_preview here — the user may still
            // want preview mode when they confirm the size.
            self.app_state = super::AppState::ConfirmingSize {
                pending_action: action,
                action_label,
                overlay_label,
                char_count,
                preview,
            };
            return;
        }
        let preview_mode = std::mem::take(&mut self.pending_preview);
        self.start_translation(ctx, slot, action, action_label, overlay_label, preview_mode);
    }

    pub(super) fn start_translation(
        &mut self,
        ctx: &egui::Context,
        slot: u8,
        action: Action,
        action_label: String,
        overlay_label: String,
        preview_mode: bool,
    ) {
        self.dispatch_gen = pure::next_gen(self.dispatch_gen);
        let gen = self.dispatch_gen;
        let cfg = self.cfg.clone();
        let provider = self
            .provider
            .as_ref()
            .expect(
                "provider must be Some; None is unreachable from start_translation \
                 (hotkey + tray ID_TRANSLATE both gate on AppState::Idle, which \
                 implies the wizard completed successfully and provider was rebuilt)",
            )
            .clone();
        let tx = self.result_tx.clone();
        let source_text = self.prompt_model.clipboard_text.clone();

        // Note: `state.last_slot` was recorded by the caller (`dispatch()`
        // for slot keys 1–6, or implicitly slot=6 for the custom-prompt
        // submit path). Don't double-write here.

        self.app_state = super::AppState::Translating {
            gen,
            action_label: action_label.clone(),
            overlay_label,
            started_at: std::time::Instant::now(),
            preview_mode,
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
            let g_snapshot = crate::glossary::Glossary::read_shared(&glossary).clone();
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

    pub(super) fn start_translation_inline(
        &mut self,
        source_text: String,
        action: Action,
        action_label: String,
        slot: u8,
        ctx: &egui::Context,
    ) {
        self.dispatch_gen = pure::next_gen(self.dispatch_gen);
        let gen = self.dispatch_gen;
        let cfg = self.cfg.clone();
        let provider = self
            .provider
            .as_ref()
            .expect(
                "provider must be Some; None is unreachable from start_translation_inline",
            )
            .clone();
        let tx = self.result_tx.clone();

        self.app_state = super::AppState::TranslatingInline {
            gen,
            action_label: action_label.clone(),
            started_at: std::time::Instant::now(),
        };
        ctx.request_repaint();

        let ctx_for_repaint = ctx.clone();
        let label_for_panic = action_label.clone();
        let templates = self.templates.clone();
        let glossary = self.glossary.clone();
        let detected_source = crate::glossary::detect_source_lang(&source_text)
            .as_deref()
            .and_then(crate::glossary::iso3_to_iso2)
            .map(String::from);
        let action_for_outcome = action.clone();
        let source_text_for_outcome = source_text.clone();

        let worker = self.runtime.spawn(async move {
            let g_snapshot = crate::glossary::Glossary::read_shared(&glossary).clone();
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

    pub(super) fn handle_translation_done(
        &mut self,
        outcome: TranslationOutcome,
        ctx: &egui::Context,
    ) {
        // Stale outcome from a cancelled translation; drop silently.
        let (current_gen, is_inline) = match &self.app_state {
            super::AppState::Translating { gen, .. } => (Some(*gen), false),
            super::AppState::TranslatingInline { gen, .. } => (Some(*gen), true),
            _ => (None, false),
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
                if is_inline {
                    let mut cb = match ArboardClipboard::new() {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = %e, "clipboard open failed");
                            self.dismiss_to_idle(ctx);
                            return;
                        }
                    };
                    if let Err(e) = cb.write_text(translated) {
                        tracing::error!(error = %e, "clipboard write failed");
                    } else {
                        // Delay briefly before pasting to allow target application to process the clipboard change notification.
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        if let Err(e) = crate::platform::current().paste_from_clipboard() {
                            tracing::error!(error = %e, "inline replacement paste failed");
                        }
                        // History insert: best-effort, AFTER clipboard write
                        // succeeds, NEVER blocks the user's primary outcome.
                        self.schedule_history_insert(&outcome, translated);
                    }
                    tracing::info!(
                        slot = outcome.slot,
                        action = %outcome.action_label,
                        "inline replacement complete"
                    );
                } else {
                    // If preview mode is active, show the result window
                    // instead of writing to clipboard directly.
                    let preview_mode = matches!(
                        &self.app_state,
                        super::AppState::Translating {
                            preview_mode: true,
                            ..
                        }
                    );
                    if preview_mode {
                        self.app_state = super::AppState::ShowingResult {
                            result_text: translated.clone(),
                            source_text: outcome.source_text.clone(),
                            action_label: outcome.action_label.clone(),
                            detected_source_lang: outcome.detected_source_lang.clone(),
                            action: outcome.action.clone(),
                            slot: outcome.slot,
                        };
                        tracing::info!(
                            slot = outcome.slot,
                            action = %outcome.action_label,
                            "translation complete — showing result preview"
                        );
                        return;
                    }

                    let mut cb = match ArboardClipboard::new() {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = %e, "clipboard open failed");
                            self.dismiss_to_idle(ctx);
                            return;
                        }
                    };
                    if let Err(e) = cb.write_text(translated) {
                        tracing::error!(error = %e, "clipboard write failed");
                    } else {
                        if let Err(e) =
                            crate::notify::translation_copied(&outcome.action_label, translated)
                        {
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
            }
            Err(crate::error::TranslateError::Provider { status: 401, .. }) => {
                tracing::warn!("translation 401 — API key invalid; opening setup wizard");
                // Best-effort flip the tray pill to KeychainStaleKey.
                // `refresh_tray_status()` runs later in the same update()
                // frame; once we transition to SetupWizard below it sees
                // `SetupWizard` and overwrites back to NoApiKey, so this
                // flip is effectively a same-frame breadcrumb for the OS
                // tray's event log rather than a user-visible state.
                if let Some(tray) = self.tray.as_mut() {
                    if let Err(e) = tray.set_status(crate::tray::TrayStatus::Warn(
                        crate::tray::WarnReason::KeychainStaleKey,
                    )) {
                        tracing::warn!(error = %e, "tray status flip on 401 failed");
                    }
                }
                self.dispatch_rerun_wizard(ctx);
                // Return early — dispatch_rerun_wizard sets app_state to
                // SetupWizard; we must not overwrite it with Idle below.
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "translation failed");
                if let Err(notify_err) = crate::notify::translation_failed(&e) {
                    tracing::warn!(error = %notify_err, "notification failed");
                }
            }
        }
        self.dismiss_to_idle(ctx);
    }

    fn schedule_history_insert(&self, outcome: &TranslationOutcome, translated: &str) {
        let entry = build_history_entry(
            &outcome.source_text,
            translated,
            &outcome.action,
            outcome.detected_source_lang.as_deref(),
            self.cfg.history.store_text,
        );
        self.schedule_history_insert_from_entry(entry);
    }

    pub(super) fn schedule_history_insert_from_entry(&self, entry: NewEntry) {
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
        let max_entries = self.cfg.history.max_entries;

        let inner = self.runtime.spawn(async move {
            if let Err(e) = history.insert_with_cap(entry, max_entries) {
                tracing::warn!(error = %e, "history insert failed; row dropped");
            }
        });
        self.runtime.spawn(async move {
            if let Err(join_err) = inner.await {
                tracing::warn!(
                    error = %join_err,
                    "history insert panicked; row dropped"
                );
            }
        });
    }
}

/// Build a `NewEntry` for history insertion from explicit fields.
/// Shared by `schedule_history_insert` (direct clipboard path) and
/// `update_showing_result` (preview → Copy path).
pub(super) fn build_history_entry(
    source_text: &str,
    result_text: &str,
    action: &Action,
    detected_source_lang: Option<&str>,
    store_text: bool,
) -> NewEntry {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    NewEntry {
        created_at: now,
        action: pure::action_kind_str(action).to_string(),
        source_lang: detected_source_lang.map(String::from),
        target_lang: pure::target_lang_for(action),
        char_count: source_text.chars().count() as i64,
        source: if store_text {
            Some(source_text.to_string())
        } else {
            None
        },
        result: if store_text {
            Some(result_text.to_string())
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::llm::LlmProvider;
    use crate::state::State;
    use async_trait::async_trait;
    use std::path::PathBuf;

    struct MockProvider;

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String, TranslateError> {
            Ok("Hola Mundo".to_string())
        }
    }

    #[test]
    fn test_start_translation_inline_transitions_state_and_spawns_worker() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let (_hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();
        let (_reload_tx, reload_rx) = crossbeam_channel::unbounded();
        let (setup_tx, setup_rx) = std::sync::mpsc::channel();

        let provider = std::sync::Arc::new(MockProvider);
        let templates = std::sync::Arc::new(crate::llm::templates::Templates::built_in());
        let glossary = std::sync::Arc::new(std::sync::RwLock::new(crate::glossary::Glossary::empty()));

        let mut app = super::super::ClipApp {
            cfg: Config::default(),
            state_path: PathBuf::from("state.toml"),
            state: State::default(),
            provider: Some(provider),
            templates,
            glossary,
            glossary_path: PathBuf::from("glossary.toml"),
            glossary_malformed: std::sync::atomic::AtomicBool::new(false),
            runtime: rt,
            hotkey_rx,
            result_tx,
            result_rx,
            glossary_reload_rx: reload_rx,
            glossary_reload_tx: None,
            history: None,
            history_disabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            history_warned: std::sync::atomic::AtomicBool::new(false),
            secrets: Box::new(crate::secrets::EnvSecrets::new("DUMMY")),
            tray: None,
            setup_check_tx: setup_tx,
            setup_check_rx: setup_rx,
            accessibility_revoked: false,
            hotkey_in_use: false,
            prompt_hotkey_id: 0,
            history_hotkey_id: None,
            selection_hotkey_id: None,
            replace_hotkey_id: None,
            app_state: super::super::AppState::Idle,
            prompt_model: crate::ui::prompt::PromptModel {
                clipboard_text: String::new(),
                detected_lang: None,
                last_slot: None,
                glossary_hits: vec![],
            },
            has_been_focused: false,
            initial_focus_pending: false,
            dispatch_gen: 0,
            last_translation_at: None,
            reduced_motion: false,
            pending_preview: false,
            previous_app_pid: None,
            repaint_ctx: egui::Context::default(),
            last_sent_visible: None,
        };

        app.cfg.languages.slot_1 = crate::config::LanguageSlot {
            label: "Spanish".to_string(),
            code: "es".to_string(),
        };

        let ctx = egui::Context::default();

        app.start_translation_inline(
            "Hello World".to_string(),
            Action::Translate { code: "es".to_string() },
            "Translate to Spanish".to_string(),
            1,
            &ctx,
        );

        // Assert state transitioned
        assert!(matches!(app.app_state, super::super::AppState::TranslatingInline { .. }));
        if let super::super::AppState::TranslatingInline { gen, action_label, .. } = &app.app_state {
            assert_eq!(*gen, app.dispatch_gen);
            assert_eq!(action_label, "Translate to Spanish");
        } else {
            panic!("Expected AppState::TranslatingInline");
        }

        // Wait for the tokio worker to finish and send the result
        let outcome = app.result_rx.recv_timeout(std::time::Duration::from_secs(2)).expect("outcome received");
        assert_eq!(outcome.gen, app.dispatch_gen);
        assert_eq!(outcome.slot, 1);
        assert_eq!(outcome.result.unwrap(), "Hola Mundo");
    }

    #[test]
    fn test_handle_translation_done_inline_writes_clipboard_and_pastes() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let (_hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();
        let (_reload_tx, reload_rx) = crossbeam_channel::unbounded();
        let (setup_tx, setup_rx) = std::sync::mpsc::channel();

        let provider = std::sync::Arc::new(MockProvider);
        let templates = std::sync::Arc::new(crate::llm::templates::Templates::built_in());
        let glossary = std::sync::Arc::new(std::sync::RwLock::new(crate::glossary::Glossary::empty()));

        let mut app = super::super::ClipApp {
            cfg: Config::default(),
            state_path: PathBuf::from("state.toml"),
            state: State::default(),
            provider: Some(provider),
            templates,
            glossary,
            glossary_path: PathBuf::from("glossary.toml"),
            glossary_malformed: std::sync::atomic::AtomicBool::new(false),
            runtime: rt,
            hotkey_rx,
            result_tx,
            result_rx,
            glossary_reload_rx: reload_rx,
            glossary_reload_tx: None,
            history: None,
            history_disabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            history_warned: std::sync::atomic::AtomicBool::new(false),
            secrets: Box::new(crate::secrets::EnvSecrets::new("DUMMY")),
            tray: None,
            setup_check_tx: setup_tx,
            setup_check_rx: setup_rx,
            accessibility_revoked: false,
            hotkey_in_use: false,
            prompt_hotkey_id: 0,
            history_hotkey_id: None,
            selection_hotkey_id: None,
            replace_hotkey_id: None,
            app_state: super::super::AppState::TranslatingInline {
                gen: 42,
                action_label: "Translate to Spanish".to_string(),
                started_at: std::time::Instant::now(),
            },
            prompt_model: crate::ui::prompt::PromptModel {
                clipboard_text: String::new(),
                detected_lang: None,
                last_slot: None,
                glossary_hits: vec![],
            },
            has_been_focused: false,
            initial_focus_pending: false,
            dispatch_gen: 42,
            last_translation_at: None,
            reduced_motion: false,
            pending_preview: false,
            previous_app_pid: None,
            repaint_ctx: egui::Context::default(),
            last_sent_visible: None,
        };

        let ctx = egui::Context::default();

        let outcome = TranslationOutcome {
            result: Ok("Inline Translation Result".to_string()),
            action_label: "Translate to Spanish".to_string(),
            slot: 1,
            gen: 42,
            detected_source_lang: None,
            source_text: "Hello".to_string(),
            action: Action::Translate { code: "es".to_string() },
        };

        app.handle_translation_done(outcome, &ctx);

        // State should be dismissed to Idle
        assert!(matches!(app.app_state, super::super::AppState::Idle));

        // Clipboard should contain the translated text
        let mut cb = ArboardClipboard::new().expect("clipboard open");
        let cb_text = cb.read_text().expect("clipboard read");
        assert_eq!(cb_text, "Inline Translation Result");
    }
}

