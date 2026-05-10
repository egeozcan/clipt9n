//! Translation worker — spawn, outcome handling, history insertion.
//! Extracted from `app/mod.rs` Step 2 of the improvement plan.

use std::time::Duration;

use super::pure;
use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::error::TranslateError;
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
            let preview = crate::ui::size_confirm::format_preview(&self.prompt_model.clipboard_text);
            let char_count = self.prompt_model.clipboard_text.chars().count();
            self.app_state = super::AppState::ConfirmingSize {
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

    pub(super) fn start_translation(
        &mut self,
        ctx: &egui::Context,
        slot: u8,
        action: Action,
        action_label: String,
        overlay_label: String,
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

    pub(super) fn handle_translation_done(
        &mut self,
        outcome: TranslationOutcome,
        ctx: &egui::Context,
    ) {
        // Stale outcome from a cancelled translation; drop silently.
        let current_gen = match &self.app_state {
            super::AppState::Translating { gen, .. } => Some(*gen),
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
            action: pure::action_kind_str(&action).to_string(),
            source_lang: detected,
            target_lang: pure::target_lang_for(&action),
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
}
