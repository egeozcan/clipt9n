//! egui_kittest coverage for the prompt-template editor
//! (`src/ui/templates.rs`). These drive the real AccessKit tree produced
//! by `draw`, so they cover what unit tests on the model cannot: that
//! the kind list, the text area, and the footer buttons are reachable
//! and wired to the mutations they claim.
//!
//! The validate/write/reload side lives in `src/app/templates.rs` and is
//! covered by that module's unit tests.

use clipt9n::llm::templates::TemplateKind;
use clipt9n::ui::templates::{draw, TemplatesModel, TemplatesOutcome};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

/// A harness sharing `model` with the caller, plus a sink holding the
/// outcome any painted frame produced.
///
/// The sink is sticky for the same reason as the glossary harness:
/// `Harness::run` paints several frames per call, and a plain assignment
/// would let a later quiet frame erase the outcome the clicked frame
/// produced.
fn harness(
    model: Arc<Mutex<TemplatesModel>>,
) -> (Harness<'static>, Arc<Mutex<Option<TemplatesOutcome>>>) {
    let outcome = Arc::new(Mutex::new(None));
    let outcome_sink = Arc::clone(&outcome);
    let harness = Harness::new(move |ctx| {
        let mut m = model.lock().unwrap();
        if let Some(produced) = draw(ctx, &mut m) {
            *outcome_sink.lock().unwrap() = Some(produced);
        }
    });
    (harness, outcome)
}

fn seeded() -> Arc<Mutex<TemplatesModel>> {
    Arc::new(Mutex::new(TemplatesModel {
        dir_display: "/tmp/clipt9n/templates/".into(),
        ..Default::default()
    }))
}

fn index_of(model: &TemplatesModel, kind: TemplateKind) -> usize {
    model.slots.iter().position(|s| s.kind == kind).unwrap()
}

#[test]
fn every_kind_is_listed_with_its_status() {
    let model = seeded();
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    // Named per kind, so a screen-reader user (and these tests) can tell
    // which template a list entry selects and what it will do on Save.
    let _ = h.get_by_label("Translate template (default)");
    let _ = h.get_by_label("Fix grammar template (default)");
    let _ = h.get_by_label("Rewrite template (default)");
    let _ = h.get_by_label("Custom template (default)");
}

#[test]
fn selecting_a_kind_swaps_the_source_shown() {
    let model = seeded();
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();
    assert_eq!(model.lock().unwrap().selected, 0);

    h.get_by_label("Custom template (default)").click();
    h.run();

    let m = model.lock().unwrap();
    assert_eq!(m.selected, index_of(&m, TemplateKind::Custom));
    // The text area is now bound to that kind.
    drop(m);
    let _ = h.get_by_label("Custom template source");
}

#[test]
fn a_customized_slot_says_so_in_the_list() {
    let model = seeded();
    {
        let mut m = model.lock().unwrap();
        let i = index_of(&m, TemplateKind::Rewrite);
        m.slots[i].source = "Rewrite it {{ glossary_block }}".into();
        m.slots[i].original = m.slots[i].source.clone();
    }
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label("Rewrite template (custom)");
    let _ = h.get_by_label("Translate template (default)");
}

#[test]
fn reset_to_default_restores_the_built_in_source() {
    let model = seeded();
    {
        let mut m = model.lock().unwrap();
        m.slots[0].source = "Totally different".into();
        m.slots[0].original = m.slots[0].source.clone();
    }
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("Reset to default").click();
    h.run();

    let m = model.lock().unwrap();
    assert_eq!(m.slots[0].source, TemplateKind::Translate.built_in_source());
    assert!(!m.slots[0].customized());
    assert!(m.dirty(), "resetting an override is an unsaved change");
}

#[test]
fn reset_is_disabled_when_the_slot_already_matches_the_built_in() {
    let model = seeded();
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let reset = h.get_by_label("Reset to default");
    assert!(
        reset.is_disabled(),
        "nothing to reset on an untouched default"
    );
}

#[test]
fn the_save_button_reports_save() {
    let model = seeded();
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("Save").click();
    h.run();

    assert_eq!(*outcome.lock().unwrap(), Some(TemplatesOutcome::Save));
}

#[test]
fn escape_closes_immediately_when_nothing_was_edited() {
    let model = seeded();
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    h.press_key(egui::Key::Escape);
    h.run();

    assert_eq!(*outcome.lock().unwrap(), Some(TemplatesOutcome::Close));
    assert!(!model.lock().unwrap().confirm_discard);
}

#[test]
fn escape_on_an_edited_template_asks_before_discarding() {
    let model = seeded();
    {
        model.lock().unwrap().slots[0]
            .source
            .push_str("\nextra rule");
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    h.press_key(egui::Key::Escape);
    h.run();

    assert_eq!(
        *outcome.lock().unwrap(),
        None,
        "unsaved edits must not vanish on a single Esc"
    );
    assert!(model.lock().unwrap().confirm_discard);
    let _ = h.get_by_label("Discard template changes?");
}

/// A change in an unselected kind still has to block Esc — the
/// confirmation reads the whole model, not just what is on screen.
#[test]
fn escape_asks_even_when_the_edit_is_in_a_kind_that_is_not_showing() {
    let model = seeded();
    {
        let mut m = model.lock().unwrap();
        let i = index_of(&m, TemplateKind::Custom);
        m.slots[i].source.push_str("\nextra");
        m.selected = index_of(&m, TemplateKind::Translate);
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    h.press_key(egui::Key::Escape);
    h.run();

    assert_eq!(*outcome.lock().unwrap(), None);
    assert!(model.lock().unwrap().confirm_discard);
}

#[test]
fn the_confirmation_can_discard_keep_editing_or_save() {
    // Discard → Close.
    let model = seeded();
    {
        let mut m = model.lock().unwrap();
        m.slots[0].source.push_str("changed");
        m.confirm_discard = true;
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();
    h.get_by_label("Discard").click();
    h.run();
    assert_eq!(*outcome.lock().unwrap(), Some(TemplatesOutcome::Close));
    assert!(!model.lock().unwrap().confirm_discard);

    // Keep editing → stays open, edits intact.
    let model = seeded();
    {
        let mut m = model.lock().unwrap();
        m.slots[0].source.push_str("changed");
        m.confirm_discard = true;
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();
    h.get_by_label("Keep editing").click();
    h.run();
    assert_eq!(*outcome.lock().unwrap(), None);
    let m = model.lock().unwrap();
    assert!(!m.confirm_discard);
    assert!(m.slots[0].source.ends_with("changed"));
}

#[test]
fn a_read_only_kind_says_why_and_offers_no_editable_field() {
    let model = seeded();
    {
        model.lock().unwrap().slots[0].rel_path = String::new();
    }
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label_contains("empty string");
    let source = h.get_by_label("Translate template source");
    assert!(
        source.is_disabled(),
        "an override that config disabled must not be typed into"
    );
    let reset = h.get_by_label("Reset to default");
    assert!(reset.is_disabled());
}

#[test]
fn an_unreadable_override_shows_its_error_and_still_opens() {
    let model = seeded();
    {
        model.lock().unwrap().slots[0].load_error =
            Some("reading translate.j2: permission denied".into());
    }
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label_contains("permission denied");
    // Still usable: the user can rebuild the file from here.
    let source = h.get_by_label("Translate template source");
    assert!(!source.is_disabled());
}

#[test]
fn a_save_rejection_is_shown_without_closing_the_window() {
    let model = seeded();
    {
        model.lock().unwrap().err_msg =
            "translate.j2 line 3: undefined variable or render error: nope".into();
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label_contains("undefined variable");
    assert_eq!(*outcome.lock().unwrap(), None);
}

#[test]
fn the_preview_pane_renders_the_current_source_on_demand() {
    let model = seeded();
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.get_by_label_contains("rendered with sample values")
    }))
    .is_err();
    assert!(absent, "the preview stays closed until asked for");

    h.get_by_label("Preview").click();
    h.run();

    assert!(model.lock().unwrap().preview_open);
    // The translate built-in substitutes the sample target language.
    let _ = h.get_by_label_contains("German");
}

#[test]
fn the_preview_reports_a_broken_template_instead_of_rendering() {
    let model = seeded();
    {
        let mut m = model.lock().unwrap();
        m.slots[0].source = "Hello {{ not_a_real_variable }}".into();
        m.preview_open = true;
    }
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label_contains("not_a_real_variable");
}

#[test]
fn the_variable_palette_is_scoped_to_the_selected_kind() {
    let model = seeded();
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();
    let _ = h.get_by_label("{{ target_language }}");

    h.get_by_label("Custom template (default)").click();
    h.run();

    // `user_instruction` is only meaningful for the custom action, and
    // `target_language` only for translate.
    let _ = h.get_by_label("{{ user_instruction }}");
    let absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.get_by_label("{{ target_language }}")
    }))
    .is_err();
    assert!(
        absent,
        "translate-only variables must not be advertised here"
    );
}
