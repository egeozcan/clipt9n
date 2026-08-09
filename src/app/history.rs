//! Encrypted history viewer — summon, render, keyboard navigation,
//! and dismissal. Extracted from `app/mod.rs` Step 3 of the improvement
//! plan.

use egui::{Key, Vec2, ViewportCommand};

use super::pure;
use crate::platform::Platform;
use crate::ui::prompt_default_inner_size;

impl super::ClipApp {
    /// Open the history viewer. Queries the store, builds a model,
    /// resizes the viewport to 680×540, and transitions to
    /// `ShowingHistory`. If history is disabled (config or corruption),
    /// the viewer still opens but with a warning banner; this lets the
    /// user verify the toast and explore an empty (or partially
    /// readable) database.
    ///
    /// Only opens from `Idle`. The gate lives here rather than at the
    /// call sites so no summon path can forget it — the tray's "Open
    /// history…" did, and would replace an open settings or glossary
    /// editor along with the work in it.
    pub(super) fn summon_history(&mut self, ctx: &egui::Context) {
        // Re-entrant: a second summon refocuses the viewer it already
        // has instead of re-querying, which would drop the user's
        // selection and scroll position.
        if matches!(self.app_state, super::AppState::ShowingHistory { .. }) {
            pure::reset_focus_loss_latch(&mut self.has_been_focused);
            self.set_window_visible(ctx, true);
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            crate::platform::current().activate_app();
            return;
        }
        if !matches!(self.app_state, super::AppState::Idle) {
            // Mid-wizard / mid-editor / mid-translate: bring focus to
            // whatever is open rather than destroying it.
            tracing::info!(
                state = self.app_state.label(),
                "history viewer request ignored"
            );
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            return;
        }

        let mut model = crate::ui::history::HistoryModel::default();
        let disabled = self
            .history_disabled
            .load(std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.history.as_ref() {
            match h.query(
                &crate::history::store::QueryFilter::default(),
                self.cfg.history.max_entries,
            ) {
                Ok(result) => {
                    model.entries = result.entries;
                    if matches!(
                        result.health,
                        crate::history::store::HistoryHealth::Corrupt { .. }
                    ) {
                        self.history_disabled
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
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
        pure::reset_focus_loss_latch(&mut self.has_been_focused);
        self.set_window_visible(ctx, true);
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = super::AppState::ShowingHistory { model };
    }

    pub(super) fn update_showing_history(
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
                self.app_state = super::AppState::ShowingHistory { model };
            }
            Some(crate::ui::history::HistoryOutcome::Delete(id)) => {
                if let Some(h) = self.history.as_ref() {
                    if let Err(e) = h.delete(id) {
                        tracing::warn!(error = %e, id, "history delete failed");
                    }
                }
                // Re-query so the list reflects the deletion.
                self.refresh_history_model(&mut model);
                self.app_state = super::AppState::ShowingHistory { model };
            }
            Some(crate::ui::history::HistoryOutcome::ClearAll) => {
                if let Some(h) = self.history.as_ref() {
                    if let Err(e) = h.clear_all() {
                        tracing::warn!(error = %e, "history clear_all failed");
                    }
                }
                self.refresh_history_model(&mut model);
                self.app_state = super::AppState::ShowingHistory { model };
            }
            None => {
                self.app_state = super::AppState::ShowingHistory { model };
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
                Ok(result) => {
                    model.entries = result.entries;
                    if matches!(
                        result.health,
                        crate::history::store::HistoryHealth::Corrupt { .. }
                    ) {
                        self.history_disabled
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        model.show_corruption_banner = true;
                    }
                    if model.selected >= model.entries.len() {
                        model.selected = 0;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "history re-query failed"),
            }
        }
    }

    fn dismiss_history_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(prompt_default_inner_size(
            &self.cfg.ui,
        )));
        self.app_state = super::AppState::Idle;
        self.set_window_visible(ctx, false);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::test_app;
    use super::super::{AppState, ClipApp};

    /// An app parked in `state`, with a glossary path that exists so
    /// nothing in the summon path has to touch a real store.
    fn app_in(state: AppState) -> (tempfile::TempDir, ClipApp) {
        let dir = tempfile::tempdir().unwrap();
        let glossary_path = dir.path().join("glossary.toml");
        std::fs::write(&glossary_path, "").unwrap();
        let mut app = test_app(glossary_path);
        app.app_state = state;
        (dir, app)
    }

    fn glossary_editor_with_unsaved_work(app: &ClipApp) -> AppState {
        let mut model = app.build_glossary_model();
        model.entries.push(crate::glossary::GlossaryEntry {
            source: "unsaved".into(),
            target: "work".into(),
            languages: vec!["*".into()],
            note: None,
        });
        AppState::ShowingGlossary { model }
    }

    #[test]
    fn summoning_history_does_not_clobber_the_open_glossary_editor() {
        let (_dir, mut app) = app_in(AppState::Idle);
        app.app_state = glossary_editor_with_unsaved_work(&app);

        app.summon_history(&egui::Context::default());

        match &app.app_state {
            AppState::ShowingGlossary { model } => assert!(
                model.dirty(),
                "the history viewer must not discard typed glossary entries"
            ),
            other => panic!("history replaced the glossary editor: {}", other.label()),
        }
    }

    #[test]
    fn summoning_history_does_not_clobber_the_open_settings_editor() {
        let (_dir, mut app) = app_in(AppState::Idle);
        let model = Box::new(app.build_settings_model());
        app.app_state = AppState::Settings { model };

        app.summon_history(&egui::Context::default());

        assert!(
            matches!(app.app_state, AppState::Settings { .. }),
            "the history viewer must not replace an open settings editor"
        );
    }

    #[test]
    fn summoning_history_from_idle_opens_the_viewer() {
        let (_dir, mut app) = app_in(AppState::Idle);

        app.summon_history(&egui::Context::default());

        assert!(matches!(app.app_state, AppState::ShowingHistory { .. }));
    }
}
