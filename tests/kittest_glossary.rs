//! egui_kittest coverage for the glossary editor (`src/ui/glossary.rs`).
//! These drive the real AccessKit tree produced by `draw`, so they cover
//! the parts unit tests on the pure helpers cannot: that the buttons are
//! reachable and wired to the model mutations they claim.
//!
//! The save/write side lives in `src/app/glossary.rs` and is covered by
//! that module's unit tests.

use clipt9n::glossary::GlossaryEntry;
use clipt9n::ui::glossary::{draw, GlossaryModel, GlossaryOutcome};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

fn entry(source: &str, target: &str) -> GlossaryEntry {
    GlossaryEntry {
        source: source.into(),
        target: target.into(),
        languages: vec!["*".into()],
        note: None,
    }
}

/// A harness sharing `model` with the caller, plus a sink holding the
/// outcome any painted frame produced.
///
/// The sink is sticky — set once, never cleared — because `Harness::run`
/// paints several frames per call and a plain assignment would let a
/// later quiet frame erase the outcome the clicked frame produced. The
/// app has no such problem: `update_showing_glossary` routes each
/// frame's return value the moment it gets it.
fn harness(
    model: Arc<Mutex<GlossaryModel>>,
) -> (Harness<'static>, Arc<Mutex<Option<GlossaryOutcome>>>) {
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

fn seeded(entries: Vec<GlossaryEntry>) -> Arc<Mutex<GlossaryModel>> {
    Arc::new(Mutex::new(GlossaryModel {
        original: entries.clone(),
        entries,
        path_display: "/tmp/clipt9n/glossary.toml".into(),
        ..Default::default()
    }))
}

#[test]
fn each_row_exposes_named_edit_and_delete_controls() {
    let model = seeded(vec![
        entry("Smart Table", "Smart Table"),
        entry("SLA", "SLA"),
    ]);
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    // Named per row, so screen-reader users (and these tests) can tell
    // which entry a button acts on.
    let _ = h.get_by_label("Edit Smart Table");
    let _ = h.get_by_label("Delete Smart Table");
    let _ = h.get_by_label("Edit SLA");
    let _ = h.get_by_label("Delete SLA");
}

#[test]
fn add_entry_appends_a_row_and_opens_it_for_editing() {
    let model = seeded(vec![entry("SLA", "SLA")]);
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("+ Add entry").click();
    h.run();

    let m = model.lock().unwrap();
    assert_eq!(m.entries.len(), 2);
    assert_eq!(m.editing, Some(1), "the new row opens in the edit panel");
    assert!(m.dirty(), "an added row is an unsaved change");
}

#[test]
fn add_entry_clears_a_filter_that_would_hide_the_new_row() {
    let model = seeded(vec![entry("SLA", "SLA")]);
    {
        model.lock().unwrap().query = "SLA".into();
    }
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("+ Add entry").click();
    h.run();

    let m = model.lock().unwrap();
    assert!(
        m.query.is_empty(),
        "a blank row matches no query; leaving the filter up makes Add look broken"
    );
    let _ = h.get_by_label("Edit (empty)");
}

#[test]
fn the_delete_button_removes_that_row() {
    let model = seeded(vec![
        entry("Smart Table", "Smart Table"),
        entry("SLA", "SLA"),
    ]);
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("Delete Smart Table").click();
    h.run();

    let m = model.lock().unwrap();
    assert_eq!(m.entries.len(), 1);
    assert_eq!(m.entries[0].source, "SLA");
}

#[test]
fn the_edit_button_binds_the_panel_to_that_row() {
    let mut scoped = entry("Vorgang", "case");
    scoped.languages = vec!["de->en".into()];
    scoped.note = Some("legal term".into());
    let model = seeded(vec![entry("SLA", "SLA"), scoped]);
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("Edit Vorgang").click();
    h.run();

    let m = model.lock().unwrap();
    assert_eq!(m.editing, Some(1));
    assert_eq!(m.lang_buf, "de->en");
    assert_eq!(m.note_buf, "legal term");
}

#[test]
fn the_save_button_reports_save() {
    let model = seeded(vec![entry("SLA", "SLA")]);
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    h.get_by_label("Save").click();
    h.run();

    assert_eq!(*outcome.lock().unwrap(), Some(GlossaryOutcome::Save));
}

#[test]
fn escape_closes_immediately_when_nothing_was_edited() {
    let model = seeded(vec![entry("SLA", "SLA")]);
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    h.press_key(egui::Key::Escape);
    h.run();

    assert_eq!(*outcome.lock().unwrap(), Some(GlossaryOutcome::Close));
    assert!(!model.lock().unwrap().confirm_discard);
}

#[test]
fn escape_on_an_edited_table_asks_before_discarding() {
    let model = seeded(vec![entry("SLA", "SLA")]);
    {
        model.lock().unwrap().entries[0].target = "service level agreement".into();
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
    let _ = h.get_by_label("Discard glossary changes?");
}

#[test]
fn the_confirmation_can_discard_keep_editing_or_save() {
    // Discard → Close.
    let model = seeded(vec![entry("SLA", "SLA")]);
    {
        let mut m = model.lock().unwrap();
        m.entries[0].target = "changed".into();
        m.confirm_discard = true;
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();
    h.get_by_label("Discard").click();
    h.run();
    assert_eq!(*outcome.lock().unwrap(), Some(GlossaryOutcome::Close));
    assert!(!model.lock().unwrap().confirm_discard);

    // Keep editing → stays open, edits intact.
    let model = seeded(vec![entry("SLA", "SLA")]);
    {
        let mut m = model.lock().unwrap();
        m.entries[0].target = "changed".into();
        m.confirm_discard = true;
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();
    h.get_by_label("Keep editing").click();
    h.run();
    assert_eq!(*outcome.lock().unwrap(), None);
    let m = model.lock().unwrap();
    assert!(!m.confirm_discard);
    assert_eq!(m.entries[0].target, "changed");
}

#[test]
fn the_comment_warning_renders_only_when_the_file_has_comments() {
    let model = seeded(vec![entry("SLA", "SLA")]);
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();
    let absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.get_by_label_contains("This file has comments")
    }))
    .is_err();
    assert!(absent, "no comments in the file, no warning");

    {
        model.lock().unwrap().comments_will_be_dropped = true;
    }
    h.run();
    let _ = h.get_by_label_contains("This file has comments");
}

#[test]
fn a_malformed_file_opens_with_its_parse_error_shown() {
    let model = Arc::new(Mutex::new(GlossaryModel {
        load_error: Some("parsing glossary.toml: expected `=`".into()),
        path_display: "/tmp/clipt9n/glossary.toml".into(),
        ..Default::default()
    }));
    let (mut h, _) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label_contains("expected `=`");
    // Still usable: the user can rebuild the file from here.
    let _ = h.get_by_label("+ Add entry");
}

#[test]
fn a_save_rejection_is_shown_without_closing_the_window() {
    let model = seeded(vec![entry("Vorgang", "case")]);
    {
        model.lock().unwrap().err_msg = "entry 0 has an invalid language scope".into();
    }
    let (mut h, outcome) = harness(Arc::clone(&model));
    h.run();

    let _ = h.get_by_label_contains("invalid language scope");
    assert_eq!(*outcome.lock().unwrap(), None);
}
