//! Prompt windows — show, render, keyboard handling, size-confirm
//! modal, and translating overlay. Extracted from `app/mod.rs` Step 5
//! of the improvement plan.

use std::time::Duration;

use egui::{Key, ViewportCommand};

use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::error::TranslateError;
use crate::platform::Platform;
use crate::translator::Action;
use crate::ui::{custom_prompt as prompt_custom, prompt, result, size_confirm, translating};

use super::pure;
use super::translation::build_history_entry;
use super::AppState;

impl super::ClipApp {
    pub(super) fn show_window(&mut self, ctx: &egui::Context) {
        self.prompt_model.clipboard_text = self.snapshot_clipboard();
        self.capture_previous_app();
        self.show_window_with_current_prompt_text(ctx);
    }

    pub(super) fn show_window_from_selection(&mut self, ctx: &egui::Context) {
        match self.snapshot_selected_text() {
            Ok(text) => {
                self.prompt_model.clipboard_text = text;
                self.capture_previous_app();
                self.show_window_with_current_prompt_text(ctx);
            }
            Err(e) => {
                tracing::warn!(error = %e, "selected text capture failed");
                if let Err(notify_err) = crate::notify::selection_capture_failed(&e) {
                    tracing::warn!(error = %notify_err, "notification failed");
                }
            }
        }
    }

    fn snapshot_selected_text(&self) -> Result<String, TranslateError> {
        let mut cb = ArboardClipboard::new()?;
        let before = cb.read_text().unwrap_or_default();
        let platform = crate::platform::current();
        let before_change_count = platform.clipboard_change_count();
        platform.copy_selection_to_clipboard()?;
        std::thread::sleep(Duration::from_millis(
            self.cfg.hotkey.selection.copy_delay_ms,
        ));
        let after = cb.read_text()?;
        let after_change_count = platform.clipboard_change_count();
        let copy_changed = match (before_change_count, after_change_count) {
            (Some(before), Some(after)) => Some(after != before),
            _ => None,
        };
        let selected = pure::selected_text_after_copy(&before, &after, copy_changed)
            .ok_or(TranslateError::EmptyOrNonTextClipboard)?;
        restore_clipboard(&before, &mut cb);
        Ok(selected)
    }

    fn show_window_with_current_prompt_text(&mut self, ctx: &egui::Context) {
        self.pending_preview = false;
        self.prompt_model.last_slot = self.state.last_slot;

        // Compute pair-agnostic glossary hits for the chip strip preview.
        // The translator applies pair scoping correctly at execute time;
        // this preview is informational only.
        let detected_iso3 = crate::glossary::detect_source_lang(&self.prompt_model.clipboard_text);
        let source_iso2 = detected_iso3
            .as_deref()
            .and_then(crate::glossary::iso3_to_iso2);
        self.prompt_model.detected_lang = source_iso2.map(String::from);
        let g = crate::glossary::Glossary::read_shared(&self.glossary);
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

        pure::reset_focus_loss_latch(&mut self.has_been_focused);
        self.initial_focus_pending = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::Showing;
    }

    pub(super) fn update_showing(&mut self, ctx: &egui::Context) {
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
        // Capture Shift-held state at pick time. Shift+pick means
        // "show result in a preview window" instead of replacing
        // the clipboard immediately.
        let shift_held = ctx.input(|i| i.modifiers.shift);
        match outcome {
            Some(prompt::PromptOutcome::Pick(n)) => {
                // dispatch() may transition to any of the new states; restore
                // Showing only if dispatch didn't transition.
                self.app_state = AppState::Showing;
                self.pending_preview = shift_held && !self.prompt_model.clipboard_text.is_empty();
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

    pub(super) fn update_entering_custom(
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
            || key_outcome == Some(super::CustomKey::Submit);
        let cancel = key_outcome == Some(super::CustomKey::Cancel);

        if cancel {
            self.dismiss_to_idle(ctx);
            return;
        }
        if submit && prompt_custom::submit_enabled(&model.instruction) {
            let instruction = model.instruction.trim().to_string();
            let action = Action::Custom { instruction };
            let action_label = pure::action_label_for(&action, &self.cfg);
            let overlay_label = pure::overlay_label_for(&action);
            self.dispatch_translate(ctx, 6, action, action_label, overlay_label);
            return;
        }
        // Otherwise stay in EnteringCustom with the (possibly mutated) model.
        self.app_state = AppState::EnteringCustom { model };
    }

    pub(super) fn update_confirming_size(
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
                let preview_mode = std::mem::take(&mut self.pending_preview);
                self.start_translation(ctx, slot, pending_action, action_label, overlay_label, preview_mode);
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

    pub(super) fn update_translating(
        &mut self,
        ctx: &egui::Context,
        gen: u64,
        action_label: String,
        overlay_label: String,
        started_at: std::time::Instant,
        preview_mode: bool,
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
            self.dispatch_gen = pure::next_gen(self.dispatch_gen);
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
            preview_mode,
        };
    }

    pub(super) fn update_showing_result(
        &mut self,
        ctx: &egui::Context,
        result_text: String,
        source_text: String,
        action_label: String,
        detected_source_lang: Option<String>,
        action: Action,
        slot: u8,
    ) {
        let model = result::ResultModel {
            result_text: result_text.clone(),
            action_label: action_label.clone(),
            source_char_count: source_text.chars().count(),
        };
        let click = result::draw(ctx, &model);
        let key = ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                Some(result::ResultOutcome::Dismiss)
            } else if i.key_pressed(Key::Enter) {
                Some(result::ResultOutcome::Copy)
            } else {
                None
            }
        });
        let outcome = click.or(key);
        match outcome {
            Some(result::ResultOutcome::Copy) => {
                if let Err(e) = self.copy_to_clipboard(&result_text) {
                    tracing::error!(error = %e, "clipboard write from result preview failed");
                } else {
                    if let Err(e) =
                        crate::notify::translation_copied(&action_label, &result_text)
                    {
                        tracing::warn!(error = %e, "notification failed");
                    }
                    // History insert (best-effort, same pattern as
                    // handle_translation_done).
                    let entry = build_history_entry(
                        &source_text,
                        &result_text,
                        &action,
                        detected_source_lang.as_deref(),
                        self.cfg.history.store_text,
                    );
                    self.schedule_history_insert_from_entry(entry);
                }
                tracing::info!(
                    slot,
                    action = %action_label,
                    "result preview: copied to clipboard"
                );
                self.dismiss_to_idle(ctx);
            }
            Some(result::ResultOutcome::Dismiss) => {
                tracing::info!(
                    slot,
                    action = %action_label,
                    "result preview: dismissed without copying"
                );
                self.dismiss_to_idle(ctx);
            }
            None => {
                self.app_state = AppState::ShowingResult {
                    result_text,
                    source_text,
                    action_label,
                    detected_source_lang,
                    action,
                    slot,
                };
            }
        }
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
    ) -> Option<super::CustomKey> {
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                return Some(super::CustomKey::Cancel);
            }
            // Cmd+Enter (macOS) or Ctrl+Enter (Linux/Windows) submits.
            if i.key_pressed(Key::Enter) && (i.modifiers.command || i.modifiers.ctrl) {
                return Some(super::CustomKey::Submit);
            }
            None
        })
    }
}

/// Restore the clipboard to its previous value after a selected-text
/// capture. Called by `snapshot_selected_text` after the copy-selection
/// keystroke; ensures the user's clipboard isn't disturbed by capture.
pub(super) fn restore_clipboard(before: &str, cb: &mut dyn crate::clipboard::Clipboard) {
    if !before.is_empty() {
        if let Err(e) = cb.write_text(before) {
            tracing::warn!(error = %e, "failed to restore clipboard after selected-text capture");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::restore_clipboard;
    use crate::clipboard::MockClipboard;

    #[test]
    fn restore_writes_back_non_empty_before() {
        let mut cb = MockClipboard::with_text("current clipboard");
        restore_clipboard("saved original", &mut cb);
        assert_eq!(cb.written, Some("saved original".to_string()));
    }

    #[test]
    fn restore_skips_write_when_before_is_empty() {
        let mut cb = MockClipboard::with_text("current clipboard");
        restore_clipboard("", &mut cb);
        assert_eq!(cb.written, None, "empty before should not write");
    }

    #[test]
    fn restore_skips_write_when_before_is_whitespace_only() {
        let mut cb = MockClipboard::with_text("current clipboard");
        // before is whitespace-only; the !before.is_empty() guard fires.
        // Per the contract, whitespace-only "before" values are still
        // restored — the clipboard had text and the user might want it back.
        restore_clipboard("  \t\n  ", &mut cb);
        assert_eq!(cb.written, Some("  \t\n  ".to_string()));
    }
}
