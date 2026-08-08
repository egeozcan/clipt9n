//! Glossary editor — open, update loop, and the save transaction.
//! The view (`src/ui/glossary.rs`) is pure; every effect lands here.

use crate::error::TranslateError;
use crate::glossary::Glossary;
use crate::platform::Platform;
use crate::ui::glossary::{GlossaryModel, GlossaryOutcome, GLOSSARY_INNER_SIZE};
use crate::ui::prompt_default_inner_size;

impl super::ClipApp {
    /// Open the structured glossary editor seeded from the file on
    /// disk. Re-entrant: if the editor is already up, this refocuses
    /// rather than discarding whatever the user has typed.
    pub(super) fn dispatch_edit_glossary(&mut self, ctx: &egui::Context) {
        if matches!(self.app_state, super::AppState::ShowingGlossary { .. }) {
            super::pure::reset_focus_loss_latch(&mut self.has_been_focused);
            self.set_window_visible(ctx, true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            crate::platform::current().activate_app();
            return;
        }

        // Same guard as `dispatch_open_settings`, for the same reason:
        // these states own work that replacing them would destroy — an
        // in-flight translation whose outcome is matched against the
        // current state, or wizard verification checks with no working
        // provider behind them.
        let busy = match &self.app_state {
            super::AppState::Translating { .. } | super::AppState::TranslatingInline { .. } => {
                Some("a translation is in flight")
            }
            super::AppState::SetupWizard { .. } => Some("the setup wizard is open"),
            super::AppState::Settings { .. } => Some("the settings editor is open"),
            _ => None,
        };
        if let Some(reason) = busy {
            tracing::info!(reason, "glossary editor request ignored");
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }

        let model = self.build_glossary_model();

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(GLOSSARY_INNER_SIZE));
        super::pure::reset_focus_loss_latch(&mut self.has_been_focused);
        self.set_window_visible(ctx, true);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        crate::platform::current().activate_app();
        self.app_state = super::AppState::ShowingGlossary { model };
    }

    /// Seed the editor from the *file*, not from the live in-memory
    /// `Glossary`.
    ///
    /// Two reasons. The file is the thing Save overwrites, so editing a
    /// stale in-memory copy would silently revert edits made in an
    /// external editor since startup. And reading the raw text is what
    /// lets a malformed file still open here — the user can rebuild it
    /// in-app instead of being locked out by their own typo, which is
    /// most of the value of this window given the `GlossaryMalformed`
    /// tray warning points at exactly that state.
    pub(super) fn build_glossary_model(&self) -> GlossaryModel {
        let path_display = self.glossary_path.display().to_string();
        let raw = match std::fs::read_to_string(&self.glossary_path) {
            Ok(text) => text,
            // A missing file is the normal first-run case: the editor
            // opens empty and Save creates it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                return GlossaryModel {
                    path_display,
                    load_error: Some(format!("reading {}: {e}", self.glossary_path.display())),
                    ..Default::default()
                }
            }
        };

        let comments_will_be_dropped = crate::glossary::contains_comments(&raw);
        match Glossary::load_str(&raw) {
            Ok(glossary) => {
                let entries = glossary.entries().to_vec();
                GlossaryModel {
                    original: entries.clone(),
                    entries,
                    path_display,
                    comments_will_be_dropped,
                    ..Default::default()
                }
            }
            Err(e) => GlossaryModel {
                path_display,
                // Not `comments_will_be_dropped`: the file is being
                // replaced wholesale either way, and the load error
                // banner already says so. Two banners would bury it.
                load_error: Some(e.to_string()),
                ..Default::default()
            },
        }
    }

    pub(super) fn update_showing_glossary(
        &mut self,
        ctx: &egui::Context,
        mut model: GlossaryModel,
    ) {
        match crate::ui::glossary::draw(ctx, &mut model) {
            Some(GlossaryOutcome::Close) => {
                tracing::info!("glossary editor closed — no changes written");
                self.dismiss_glossary_to_idle(ctx);
            }
            Some(GlossaryOutcome::Save) => match self.apply_glossary(&model) {
                Ok(count) => {
                    tracing::info!(entries = count, "glossary saved");
                    self.dismiss_glossary_to_idle(ctx);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "glossary save rejected");
                    model.err_msg = e.to_string();
                    self.app_state = super::AppState::ShowingGlossary { model };
                }
            },
            None => {
                self.app_state = super::AppState::ShowingGlossary { model };
            }
        }
    }

    /// Commit the working copy: validate, then write, then reload.
    ///
    /// Ordering matters exactly as it does in `apply_settings` —
    /// everything that can fail runs against a *candidate* glossary
    /// before a byte is written, so a rejected save leaves both the
    /// file and the running glossary untouched. Returns the entry count
    /// on success.
    fn apply_glossary(&mut self, model: &GlossaryModel) -> Result<usize, TranslateError> {
        // Validates by the same path the loader uses, so anything this
        // accepts is something `Glossary::load` will accept on the next
        // startup.
        let candidate = Glossary::from_entries(model.entries.clone())?;
        let contents = candidate.to_toml()?;

        write_text_atomically(&self.glossary_path, &contents)?;

        // Re-read rather than installing `candidate` directly: this is
        // the one path that proves the bytes we just wrote parse back,
        // and it already clears `glossary_malformed` and logs. The tray
        // pill refreshes later in this same frame via `update()`.
        self.reload_glossary();
        Ok(candidate.len())
    }

    fn dismiss_glossary_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(prompt_default_inner_size(
            &self.cfg.ui,
        )));
        self.app_state = super::AppState::Idle;
        self.set_window_visible(ctx, false);
    }
}

/// Write `contents` to `path` via a same-directory temp file and an
/// atomic replace, so an interrupted save cannot leave a truncated
/// glossary behind. Mirrors `DiskAtomicConfig::replace` minus its
/// previous-contents rollback: that exists to protect a config the app
/// is mid-way through adopting, whereas here the bytes are already
/// final by the time the rename runs.
fn write_text_atomically(path: &std::path::Path, contents: &str) -> Result<(), TranslateError> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| TranslateError::Glossary("glossary path has no parent".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| TranslateError::Glossary(format!("creating {}: {e}", parent.display())))?;

    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("glossary.toml"),
        std::process::id()
    ));
    let staged = (|| {
        let mut temp = std::fs::File::create(&temp_path).map_err(|e| {
            TranslateError::Glossary(format!("creating {}: {e}", temp_path.display()))
        })?;
        temp.write_all(contents.as_bytes()).map_err(|e| {
            TranslateError::Glossary(format!("writing {}: {e}", temp_path.display()))
        })?;
        temp.flush().map_err(|e| {
            TranslateError::Glossary(format!("flushing {}: {e}", temp_path.display()))
        })?;
        temp.sync_all().map_err(|e| {
            TranslateError::Glossary(format!("syncing {}: {e}", temp_path.display()))
        })?;
        crate::platform::current()
            .replace_file(&temp_path, path)
            .map_err(|e| {
                TranslateError::Glossary(format!(
                    "replacing {} with {}: {e}",
                    path.display(),
                    temp_path.display()
                ))
            })
    })();
    if staged.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    staged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::test_app;
    use crate::glossary::GlossaryEntry;

    fn entry(source: &str, target: &str) -> GlossaryEntry {
        GlossaryEntry {
            source: source.into(),
            target: target.into(),
            languages: vec!["*".into()],
            note: None,
        }
    }

    #[test]
    fn atomic_write_creates_the_file_and_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glossary.toml");
        write_text_atomically(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file should be renamed away");
    }

    #[test]
    fn atomic_write_replaces_existing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glossary.toml");
        std::fs::write(&path, "old").unwrap();
        write_text_atomically(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn serialized_entries_load_back_identically() {
        let mut noted = entry("SLA", "SLA");
        noted.note = Some("keep the acronym".into());
        let mut scoped = entry("Vorgang", "case");
        scoped.languages = vec!["de->en".into()];
        let entries = vec![entry("Smart Table", "Smart Table"), noted, scoped];

        let toml = Glossary::from_entries(entries.clone())
            .unwrap()
            .to_toml()
            .unwrap();
        let reloaded = Glossary::load_str(&toml).unwrap();
        assert_eq!(reloaded.entries(), entries.as_slice());
    }

    #[test]
    fn an_entry_without_a_note_serializes_without_the_key() {
        let toml = Glossary::from_entries(vec![entry("a", "b")])
            .unwrap()
            .to_toml()
            .unwrap();
        assert!(!toml.contains("note"), "unexpected note key in:\n{toml}");
        assert!(
            toml.contains("[[entry]]"),
            "missing entry table in:\n{toml}"
        );
    }

    #[test]
    fn an_invalid_entry_is_rejected_before_serialization() {
        let mut bad = entry("Vorgang", "case");
        bad.languages = vec!["german->english".into()];
        let err = Glossary::from_entries(vec![bad]).unwrap_err();
        assert!(
            matches!(err, TranslateError::Glossary(_)),
            "expected a glossary error, got {err:?}"
        );
    }

    #[test]
    fn an_empty_target_is_rejected() {
        assert!(Glossary::from_entries(vec![entry("Vorgang", "  ")]).is_err());
    }

    /// A `ClipApp` whose glossary file holds `contents`, or no file at
    /// all when `contents` is `None`.
    fn app_with_glossary(
        contents: Option<&str>,
    ) -> (tempfile::TempDir, crate::app::ClipApp, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glossary.toml");
        if let Some(text) = contents {
            std::fs::write(&path, text).unwrap();
        }
        let app = test_app(path.clone());
        (dir, app, path)
    }

    #[test]
    fn the_editor_seeds_from_the_file_on_disk() {
        let (_dir, app, _path) = app_with_glossary(Some(
            r#"
[[entry]]
source = "Vorgang"
target = "case"
languages = ["de->en"]
"#,
        ));
        let model = app.build_glossary_model();
        assert!(model.load_error.is_none());
        assert_eq!(model.entries.len(), 1);
        assert_eq!(model.entries[0].source, "Vorgang");
        assert_eq!(model.original, model.entries, "opens clean, not dirty");
        assert!(!model.comments_will_be_dropped);
    }

    #[test]
    fn a_missing_file_opens_an_empty_editor_rather_than_an_error() {
        let (_dir, app, _path) = app_with_glossary(None);
        let model = app.build_glossary_model();
        assert!(model.entries.is_empty());
        assert!(
            model.load_error.is_none(),
            "first run is not an error state"
        );
    }

    #[test]
    fn a_malformed_file_still_opens_so_it_can_be_rebuilt_in_app() {
        let (_dir, app, _path) =
            app_with_glossary(Some("[[entry]]\nsource = \"missing target\"\n"));
        let model = app.build_glossary_model();
        assert!(
            model.load_error.is_some(),
            "the parse failure must be surfaced"
        );
        assert!(model.entries.is_empty());
    }

    #[test]
    fn a_commented_file_warns_before_the_first_save() {
        let (_dir, app, _path) = app_with_glossary(Some(
            "# product names\n[[entry]]\nsource = \"a\"\ntarget = \"b\"\n",
        ));
        let model = app.build_glossary_model();
        assert!(model.comments_will_be_dropped);
    }

    #[test]
    fn saving_writes_the_file_and_swaps_the_live_glossary() {
        let (_dir, mut app, path) = app_with_glossary(Some(""));
        let model = crate::ui::glossary::GlossaryModel {
            entries: vec![entry("Smart Table", "Smart Table")],
            ..Default::default()
        };

        let count = app.apply_glossary(&model).unwrap();
        assert_eq!(count, 1);

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("Smart Table"), "wrote:\n{on_disk}");

        let live = crate::glossary::Glossary::read_shared(&app.glossary);
        assert_eq!(live.len(), 1, "the running glossary picked up the save");
        assert_eq!(live.entries()[0].source, "Smart Table");
    }

    #[test]
    fn saving_a_valid_glossary_clears_the_malformed_tray_warning() {
        let (_dir, mut app, _path) = app_with_glossary(Some("this is not toml"));
        assert!(
            app.glossary_malformed
                .load(std::sync::atomic::Ordering::Relaxed),
            "fixture starts in the malformed state"
        );
        let model = crate::ui::glossary::GlossaryModel {
            entries: vec![entry("a", "b")],
            ..Default::default()
        };
        app.apply_glossary(&model).unwrap();
        assert!(!app
            .glossary_malformed
            .load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn a_rejected_save_leaves_the_file_and_the_live_glossary_untouched() {
        let original = "[[entry]]\nsource = \"keep\"\ntarget = \"me\"\n";
        let (_dir, mut app, path) = app_with_glossary(Some(original));
        let mut bad = entry("Vorgang", "case");
        bad.languages = vec!["german->english".into()];
        let model = crate::ui::glossary::GlossaryModel {
            entries: vec![bad],
            ..Default::default()
        };

        assert!(app.apply_glossary(&model).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "a rejected save must not touch the file"
        );
        assert!(
            crate::glossary::Glossary::read_shared(&app.glossary).is_empty(),
            "a rejected save must not swap the running glossary"
        );
    }

    /// An editor session with one unsaved edit in it.
    fn app_with_open_editor() -> (tempfile::TempDir, crate::app::ClipApp) {
        let (dir, mut app, _path) = app_with_glossary(Some(""));
        let mut model = app.build_glossary_model();
        model.entries.push(entry("unsaved", "work"));
        assert!(model.dirty());
        app.app_state = crate::app::AppState::ShowingGlossary { model };
        (dir, app)
    }

    fn still_editing(app: &crate::app::ClipApp) -> bool {
        match &app.app_state {
            crate::app::AppState::ShowingGlossary { model } => model.dirty(),
            _ => false,
        }
    }

    #[test]
    fn opening_settings_does_not_clobber_the_open_glossary_editor() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_open_settings(&egui::Context::default());
        assert!(
            still_editing(&app),
            "the settings editor must not replace unsaved glossary entries"
        );
    }

    #[test]
    fn rerunning_the_wizard_does_not_clobber_the_open_glossary_editor() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_rerun_wizard(&egui::Context::default());
        assert!(
            still_editing(&app),
            "the wizard has no stake in the glossary; it must not discard these edits"
        );
    }

    #[test]
    fn reopening_the_editor_keeps_the_session_it_already_has() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_edit_glossary(&egui::Context::default());
        assert!(
            still_editing(&app),
            "a second Edit glossary… should refocus, not reseed from disk"
        );
    }

    #[test]
    fn saving_an_empty_table_writes_an_empty_glossary() {
        let (_dir, mut app, path) =
            app_with_glossary(Some("[[entry]]\nsource = \"gone\"\ntarget = \"gone\"\n"));
        let model = crate::ui::glossary::GlossaryModel::default();

        assert_eq!(app.apply_glossary(&model).unwrap(), 0);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("gone"));
        assert!(crate::glossary::Glossary::read_shared(&app.glossary).is_empty());
    }
}
