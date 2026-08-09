//! Prompt-template editor — open, update loop, and the save
//! transaction. The view (`src/ui/templates.rs`) is pure; every effect
//! lands here.

use std::path::PathBuf;

use crate::error::TranslateError;
use crate::llm::templates::{TemplateKind, Templates};
use crate::platform::Platform;
use crate::ui::prompt_default_inner_size;
use crate::ui::templates::{TemplateSlot, TemplatesModel, TemplatesOutcome, TEMPLATES_INNER_SIZE};

impl super::ClipApp {
    /// Open the template editor seeded from the files on disk.
    /// Re-entrant: if the editor is already up, this refocuses rather
    /// than discarding whatever the user has typed.
    pub(super) fn dispatch_edit_templates(&mut self, ctx: &egui::Context) {
        if matches!(self.app_state, super::AppState::ShowingTemplates { .. }) {
            super::pure::reset_focus_loss_latch(&mut self.has_been_focused);
            self.set_window_visible(ctx, true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            crate::platform::current().activate_app();
            return;
        }

        // Same guard as `dispatch_edit_glossary`, for the same reason:
        // these states own work that replacing them would destroy — an
        // in-flight translation whose outcome is matched against the
        // current state, or an editor holding unsaved typing.
        let busy = match &self.app_state {
            super::AppState::Translating { .. } | super::AppState::TranslatingInline { .. } => {
                Some("a translation is in flight")
            }
            super::AppState::SetupWizard { .. } => Some("the setup wizard is open"),
            super::AppState::Settings { .. } => Some("the settings editor is open"),
            super::AppState::ShowingGlossary { .. } => Some("the glossary editor is open"),
            _ => None,
        };
        if let Some(reason) = busy {
            tracing::info!(reason, "template editor request ignored");
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }

        let model = self.build_templates_model();

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(TEMPLATES_INNER_SIZE));
        super::pure::reset_focus_loss_latch(&mut self.has_been_focused);
        self.set_window_visible(ctx, true);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        crate::platform::current().activate_app();
        self.app_state = super::AppState::ShowingTemplates {
            model: Box::new(model),
        };
    }

    /// The directory template overrides are resolved against — the
    /// config file's own parent, not a platform default, because
    /// `--config` can point anywhere and the loader used the same rule
    /// at startup.
    pub(super) fn config_dir(&self) -> Result<PathBuf, TranslateError> {
        self.cfg_path
            .parent()
            .map(|p| p.to_path_buf())
            .ok_or_else(|| {
                TranslateError::Template(format!(
                    "config path {} has no parent directory",
                    self.cfg_path.display()
                ))
            })
    }

    /// Seed the editor from the *files*, not from the live
    /// `Arc<Templates>`.
    ///
    /// Same reasoning as `build_glossary_model`: the files are what Save
    /// overwrites, so editing a compiled-at-startup copy would silently
    /// revert anything changed in an external editor since launch. It
    /// also means an override that fails to *read* still opens here —
    /// the built-in loads into the buffer and the banner explains, so
    /// the user can rebuild the file in-app.
    ///
    /// Note the asymmetry with `Templates::load`, which is deliberate: a
    /// file that fails to *parse* is a startup abort, but here it must
    /// still open, because fixing it is the whole point of the window.
    /// Save re-validates before writing, so nothing invalid escapes.
    pub(super) fn build_templates_model(&self) -> TemplatesModel {
        let config_dir = self.config_dir();
        let dir_display = match &config_dir {
            Ok(dir) => format!("{}/templates/", dir.display()),
            Err(_) => String::new(),
        };

        let slots = TemplateKind::all()
            .into_iter()
            .map(|kind| {
                let rel_path = self
                    .configured_template_path(kind)
                    .map(str::to_owned)
                    .unwrap_or_default();
                let (source, load_error) = self.read_template_source(&config_dir, kind, &rel_path);
                TemplateSlot {
                    kind,
                    original: source.clone(),
                    source,
                    rel_path,
                    load_error,
                }
            })
            .collect();

        TemplatesModel {
            slots,
            dir_display,
            ..Default::default()
        }
    }

    fn configured_template_path(&self, kind: TemplateKind) -> Option<&str> {
        let cfg = &self.cfg.templates;
        let raw = match kind {
            TemplateKind::Translate => cfg.translate.as_deref(),
            TemplateKind::FixGrammar => cfg.fix_grammar.as_deref(),
            TemplateKind::Rewrite => cfg.rewrite.as_deref(),
            TemplateKind::Custom => cfg.custom.as_deref(),
        };
        raw.filter(|s| !s.is_empty())
    }

    /// Buffer contents for one kind: the override file when it reads,
    /// the built-in otherwise. A missing file is the normal case (no
    /// override yet) and is not an error.
    fn read_template_source(
        &self,
        config_dir: &Result<PathBuf, TranslateError>,
        kind: TemplateKind,
        rel_path: &str,
    ) -> (String, Option<String>) {
        let built_in = || kind.built_in_source().to_string();
        if rel_path.is_empty() {
            return (built_in(), None);
        }
        let dir = match config_dir {
            Ok(dir) => dir,
            Err(e) => return (built_in(), Some(e.to_string())),
        };
        let abs = match crate::config::resolve_confined_path(dir, rel_path) {
            Ok(abs) => abs,
            Err(e) => return (built_in(), Some(e.to_string())),
        };
        match std::fs::read_to_string(&abs) {
            Ok(text) => (text, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (built_in(), None),
            Err(e) => (built_in(), Some(format!("reading {}: {e}", abs.display()))),
        }
    }

    pub(super) fn update_showing_templates(
        &mut self,
        ctx: &egui::Context,
        mut model: Box<TemplatesModel>,
    ) {
        match crate::ui::templates::draw(ctx, &mut model) {
            Some(TemplatesOutcome::Close) => {
                tracing::info!("template editor closed — no changes written");
                self.dismiss_templates_to_idle(ctx);
            }
            Some(TemplatesOutcome::Save) => match self.apply_templates(&model) {
                Ok(customized) => {
                    tracing::info!(customized, "templates saved and reloaded");
                    self.dismiss_templates_to_idle(ctx);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "template save rejected");
                    model.err_msg = e.to_string();
                    self.app_state = super::AppState::ShowingTemplates { model };
                }
            },
            None => {
                self.app_state = super::AppState::ShowingTemplates { model };
            }
        }
    }

    /// Commit the working copies: validate all, then write, then
    /// rebuild the live `Templates`. Returns the number of kinds left
    /// with an override file.
    ///
    /// Ordering matters exactly as it does in `apply_glossary` and
    /// `apply_settings` — every source that can be rejected is checked
    /// before a byte is written, so a rejected save leaves both the
    /// files and the running templates untouched.
    ///
    /// A slot whose buffer equals the built-in has its override file
    /// *removed* rather than rewritten. That keeps "Reset to default" a
    /// pure buffer edit (no separate delete path, no confirmation) and
    /// stops the templates folder filling with copies of what the binary
    /// already ships.
    fn apply_templates(&mut self, model: &TemplatesModel) -> Result<usize, TranslateError> {
        let config_dir = self.config_dir()?;

        // Phase 1 — resolve and validate. Nothing has been written yet,
        // so any error here aborts with the disk untouched.
        struct Planned {
            abs: PathBuf,
            /// `None` means "remove the override".
            contents: Option<String>,
        }
        let mut plan = Vec::new();
        for slot in &model.slots {
            if slot.read_only() {
                continue;
            }
            let abs = crate::config::resolve_confined_path(&config_dir, &slot.rel_path)?;
            if slot.customized() {
                crate::llm::templates::validate_template_source(&slot.source, &abs, slot.kind)?;
                plan.push(Planned {
                    abs,
                    contents: Some(slot.source.clone()),
                });
            } else {
                plan.push(Planned {
                    abs,
                    contents: None,
                });
            }
        }

        // Phase 2 — apply. Not a single atomic transaction across four
        // files, and it cannot be: each write is individually atomic,
        // but a failure partway leaves the earlier ones applied. That
        // degrades safely because every source in the plan already
        // validated, so whatever landed still loads — and phase 3 reads
        // the directory back, so the live templates match disk either
        // way rather than a half-applied model.
        let mut customized = 0usize;
        for step in &plan {
            match &step.contents {
                Some(contents) => {
                    super::atomic::write_text_atomically(
                        &step.abs,
                        contents,
                        TranslateError::Template,
                    )?;
                    customized += 1;
                }
                None => match std::fs::remove_file(&step.abs) {
                    Ok(()) => {
                        tracing::info!(path = %step.abs.display(), "template override removed")
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(TranslateError::Template(format!(
                            "removing {}: {e}",
                            step.abs.display()
                        )))
                    }
                },
            }
        }

        // Phase 3 — re-read rather than installing the buffers directly.
        // This is the one path that proves the bytes just written parse
        // back through the *startup* loader, which is the invariant that
        // matters: anything this editor accepts must be something the
        // next launch loads.
        let fresh = Templates::load(&config_dir, &self.cfg.templates)?;
        self.templates = std::sync::Arc::new(fresh);
        // In-flight translations keep the Arc they cloned at dispatch
        // (`start_translation`), so a request already running finishes
        // against the templates it started with. Same snapshot
        // semantics as the glossary.
        Ok(customized)
    }

    /// Re-read the override files and swap the live `Templates`, for
    /// edits made outside the app.
    ///
    /// The editor's Save already does this as its last step, so this
    /// exists for the "Open config folder, edit `translate.j2` in vim"
    /// workflow — which before this had no path short of a restart,
    /// because templates are not on the SIGHUP reload that the glossary
    /// gets.
    ///
    /// Graceful where startup is strict: a file that fails to parse
    /// keeps the previous templates and logs, rather than aborting.
    /// Startup can afford to refuse because nothing is running yet; here
    /// there is a working set to fall back to, and dropping the user's
    /// session over a typo in a file they can fix would be the worse
    /// trade.
    pub(super) fn dispatch_reload_templates(&mut self) {
        let config_dir = match self.config_dir() {
            Ok(dir) => dir,
            Err(e) => {
                tracing::warn!(error = %e, "template reload failed; keeping previous templates");
                return;
            }
        };
        match Templates::load(&config_dir, &self.cfg.templates) {
            Ok(fresh) => {
                self.templates = std::sync::Arc::new(fresh);
                tracing::info!(dir = %config_dir.display(), "templates reloaded");
            }
            Err(e) => {
                tracing::warn!(error = %e, "template reload failed; keeping previous templates");
            }
        }
    }

    fn dismiss_templates_to_idle(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(prompt_default_inner_size(
            &self.cfg.ui,
        )));
        self.app_state = super::AppState::Idle;
        self.set_window_visible(ctx, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::test_app;

    /// A `ClipApp` rooted in a fresh config dir, plus that dir.
    fn app_in_dir() -> (tempfile::TempDir, crate::app::ClipApp) {
        let dir = tempfile::tempdir().unwrap();
        let app = test_app(dir.path().join("glossary.toml"));
        (dir, app)
    }

    fn write_override(dir: &tempfile::TempDir, kind: TemplateKind, body: &str) {
        let path = dir.path().join("templates");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(format!("{}.j2", kind.name())), body).unwrap();
    }

    fn slot(model: &TemplatesModel, kind: TemplateKind) -> &TemplateSlot {
        model.slots.iter().find(|s| s.kind == kind).unwrap()
    }

    #[test]
    fn a_config_dir_without_overrides_seeds_every_slot_from_the_built_ins() {
        let (_dir, app) = app_in_dir();
        let model = app.build_templates_model();
        assert_eq!(model.slots.len(), 4);
        for s in &model.slots {
            assert_eq!(s.source, s.kind.built_in_source());
            assert!(!s.customized(), "{} should be default", s.kind.name());
            assert!(s.load_error.is_none());
            assert!(!s.read_only());
        }
        assert!(!model.dirty());
    }

    #[test]
    fn an_existing_override_seeds_that_slot_from_the_file() {
        let (dir, app) = app_in_dir();
        write_override(
            &dir,
            TemplateKind::Rewrite,
            "Rewritten {{ glossary_block }}",
        );
        let model = app.build_templates_model();
        assert_eq!(
            slot(&model, TemplateKind::Rewrite).source,
            "Rewritten {{ glossary_block }}"
        );
        assert!(slot(&model, TemplateKind::Rewrite).customized());
        // Untouched kinds still come from the binary.
        assert!(!slot(&model, TemplateKind::Custom).customized());
        assert!(!model.dirty());
    }

    /// The counterpart to `Templates::load` aborting startup: a file the
    /// loader would reject must still open here, or the editor cannot be
    /// the way out of a broken template.
    #[test]
    fn a_malformed_override_still_opens_in_the_editor() {
        let (dir, app) = app_in_dir();
        write_override(&dir, TemplateKind::Translate, "{{ unclosed ");
        let model = app.build_templates_model();
        let s = slot(&model, TemplateKind::Translate);
        assert_eq!(s.source, "{{ unclosed ");
        assert!(s.load_error.is_none(), "unreadable != unparseable");
    }

    #[test]
    fn an_empty_configured_path_makes_the_slot_read_only() {
        let (_dir, mut app) = app_in_dir();
        app.cfg.templates.translate = Some(String::new());
        let model = app.build_templates_model();
        assert!(slot(&model, TemplateKind::Translate).read_only());
        assert!(!slot(&model, TemplateKind::Rewrite).read_only());
    }

    #[test]
    fn saving_a_customized_slot_writes_the_file_and_swaps_the_live_templates() {
        let (dir, mut app) = app_in_dir();
        let mut model = app.build_templates_model();
        let index = model
            .slots
            .iter()
            .position(|s| s.kind == TemplateKind::Custom)
            .unwrap();
        model.slots[index].source = "Do this: {{ user_instruction }}".into();

        let count = app.apply_templates(&model).unwrap();
        assert_eq!(count, 1, "only the custom slot should have a file");

        let written = std::fs::read_to_string(dir.path().join("templates/custom.j2")).unwrap();
        assert_eq!(written, "Do this: {{ user_instruction }}");

        // The live templates reflect the new source, not the built-in.
        let rendered = crate::llm::templates::render(
            &app.templates,
            TemplateKind::Custom,
            &crate::llm::templates::TemplateContext::for_custom("shout", ""),
        )
        .unwrap();
        assert_eq!(rendered, "Do this: shout");
    }

    #[test]
    fn an_invalid_template_is_rejected_before_anything_is_written() {
        let (dir, mut app) = app_in_dir();
        let mut model = app.build_templates_model();
        model.slots[0].source = "Hello {{ nonexistent_variable }}".into();

        let err = app.apply_templates(&model).unwrap_err();
        assert!(
            matches!(err, TranslateError::Template(_)),
            "expected a template error, got {err:?}"
        );
        assert!(
            !dir.path().join("templates/translate.j2").exists(),
            "a rejected save must not write"
        );
    }

    /// The rejection must be all-or-nothing across the four kinds, not
    /// just for the offending one.
    #[test]
    fn one_invalid_template_blocks_the_other_slots_writes() {
        let (dir, mut app) = app_in_dir();
        let mut model = app.build_templates_model();
        // A valid edit ordered before the invalid one.
        model.slots[0].source = "Translate to {{ target_language }} please".into();
        model.slots[1].source = "{% for x in %}".into();

        assert!(app.apply_templates(&model).is_err());
        assert!(!dir.path().join("templates/translate.j2").exists());
        assert!(!dir.path().join("templates/fix_grammar.j2").exists());
    }

    #[test]
    fn resetting_to_the_built_in_removes_the_override_file() {
        let (dir, mut app) = app_in_dir();
        write_override(&dir, TemplateKind::Rewrite, "Custom rewrite");
        let mut model = app.build_templates_model();
        let index = model
            .slots
            .iter()
            .position(|s| s.kind == TemplateKind::Rewrite)
            .unwrap();
        // What the "Reset to default" button does.
        model.slots[index].source = TemplateKind::Rewrite.built_in_source().to_string();

        let count = app.apply_templates(&model).unwrap();
        assert_eq!(count, 0);
        assert!(
            !dir.path().join("templates/rewrite.j2").exists(),
            "override should be deleted, not rewritten"
        );
        // And the live templates fall back to the built-in.
        let rendered = crate::llm::templates::render(
            &app.templates,
            TemplateKind::Rewrite,
            &crate::llm::templates::TemplateContext::for_rewrite(""),
        )
        .unwrap();
        assert!(rendered.contains("MAY restructure sentences"));
    }

    #[test]
    fn saving_an_untouched_model_is_a_no_op_that_leaves_no_files() {
        let (dir, mut app) = app_in_dir();
        let model = app.build_templates_model();
        assert_eq!(app.apply_templates(&model).unwrap(), 0);
        assert!(!dir.path().join("templates").join("translate.j2").exists());
    }

    #[test]
    fn a_read_only_slot_is_never_written_even_when_its_buffer_differs() {
        let (dir, mut app) = app_in_dir();
        app.cfg.templates.translate = Some(String::new());
        let mut model = app.build_templates_model();
        model.slots[0].source = "should not reach disk".into();

        app.apply_templates(&model).unwrap();
        assert!(!dir.path().join("templates/translate.j2").exists());
    }

    #[test]
    fn a_saved_override_survives_a_reload_through_the_startup_loader() {
        let (dir, mut app) = app_in_dir();
        let mut model = app.build_templates_model();
        model.slots[0].source = "To {{ target_language }}!".into();
        app.apply_templates(&model).unwrap();

        // Exactly what main.rs does at launch.
        let reloaded = Templates::load(dir.path(), &app.cfg.templates).unwrap();
        let rendered = crate::llm::templates::render(
            &reloaded,
            TemplateKind::Translate,
            &crate::llm::templates::TemplateContext::for_translate("Turkish", ""),
        )
        .unwrap();
        assert_eq!(rendered, "To Turkish!");
    }

    // ---- reload ----

    #[test]
    fn reload_picks_up_a_file_edited_outside_the_app() {
        let (dir, mut app) = app_in_dir();
        write_override(&dir, TemplateKind::Rewrite, "Externally edited");

        app.dispatch_reload_templates();

        let rendered = crate::llm::templates::render(
            &app.templates,
            TemplateKind::Rewrite,
            &crate::llm::templates::TemplateContext::for_rewrite(""),
        )
        .unwrap();
        assert_eq!(rendered, "Externally edited");
    }

    /// Startup aborts on a malformed override; a reload must not, or a
    /// typo in an external editor would take the running session with
    /// it.
    #[test]
    fn a_malformed_file_leaves_the_previous_templates_running() {
        let (dir, mut app) = app_in_dir();
        write_override(&dir, TemplateKind::Rewrite, "Good {{ glossary_block }}");
        app.dispatch_reload_templates();

        write_override(&dir, TemplateKind::Rewrite, "{{ broken ");
        app.dispatch_reload_templates();

        let rendered = crate::llm::templates::render(
            &app.templates,
            TemplateKind::Rewrite,
            &crate::llm::templates::TemplateContext::for_rewrite(""),
        )
        .unwrap();
        assert_eq!(rendered.trim(), "Good", "the good template must survive");
    }

    // ---- summon guards ----

    /// An editor session with one unsaved edit in it.
    fn app_with_open_editor() -> (tempfile::TempDir, crate::app::ClipApp) {
        let (dir, mut app) = app_in_dir();
        let mut model = app.build_templates_model();
        model.slots[0].source.push_str("\nunsaved work");
        assert!(model.dirty());
        app.app_state = crate::app::AppState::ShowingTemplates {
            model: Box::new(model),
        };
        (dir, app)
    }

    fn still_editing(app: &crate::app::ClipApp) -> bool {
        match &app.app_state {
            crate::app::AppState::ShowingTemplates { model } => model.dirty(),
            _ => false,
        }
    }

    #[test]
    fn reopening_the_editor_keeps_the_session_it_already_has() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_edit_templates(&egui::Context::default());
        assert!(
            still_editing(&app),
            "a second Edit prompt templates… should refocus, not reseed from disk"
        );
    }

    #[test]
    fn opening_settings_does_not_clobber_the_open_template_editor() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_open_settings(&egui::Context::default());
        assert!(
            still_editing(&app),
            "the settings editor must not replace unsaved template edits"
        );
    }

    #[test]
    fn rerunning_the_wizard_does_not_clobber_the_open_template_editor() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_rerun_wizard(&egui::Context::default());
        assert!(
            still_editing(&app),
            "the wizard has no stake in the templates; it must not discard these edits"
        );
    }

    #[test]
    fn the_glossary_editor_does_not_clobber_the_open_template_editor() {
        let (_dir, mut app) = app_with_open_editor();
        app.dispatch_edit_glossary(&egui::Context::default());
        assert!(still_editing(&app));
    }

    /// The mirror of the above: an open glossary editor holds typed work
    /// too, so the template editor must refuse from there.
    #[test]
    fn the_template_editor_does_not_clobber_an_open_glossary_editor() {
        let (_dir, mut app) = app_in_dir();
        let mut glossary = app.build_glossary_model();
        glossary.entries.push(crate::glossary::GlossaryEntry {
            source: "unsaved".into(),
            target: "work".into(),
            languages: vec!["*".into()],
            note: None,
        });
        app.app_state = crate::app::AppState::ShowingGlossary { model: glossary };

        app.dispatch_edit_templates(&egui::Context::default());

        assert!(
            matches!(
                &app.app_state,
                crate::app::AppState::ShowingGlossary { model } if model.dirty()
            ),
            "unsaved glossary entries must survive a template-editor request"
        );
    }

    #[test]
    fn a_path_escaping_the_config_dir_is_rejected() {
        let (_dir, mut app) = app_in_dir();
        app.cfg.templates.custom = Some("../escape.j2".into());
        let mut model = app.build_templates_model();
        let index = model
            .slots
            .iter()
            .position(|s| s.kind == TemplateKind::Custom)
            .unwrap();
        model.slots[index].source = "escaped {{ user_instruction }}".into();

        let err = app.apply_templates(&model).unwrap_err();
        assert!(
            err.to_string().contains("outside"),
            "expected a confinement error, got {err}"
        );
    }
}
