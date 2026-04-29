//! egui_kittest backfill for the M5 history viewer (`src/ui/history.rs`).
//! These tests assert against the AccessKit tree produced by the viewer's
//! `draw` function; the viewer's keyboard handling lives in `app.rs::handle_keys_history`
//! which these tests do NOT exercise (the App layer's keyboard routing is
//! tested via the same Harness in Task 4 once the slot-row regression
//! anchors the harness/app pattern).

use clipt9n::history::store::HistoryEntry;
use clipt9n::ui::history::{draw, HistoryModel, HistoryOutcome};
use egui_kittest::kittest::{Key, Queryable};
use egui_kittest::Harness;
use zeroize::Zeroizing;

fn fixture(id: i64, action: &str, source: &str, result: &str) -> HistoryEntry {
    HistoryEntry {
        id,
        created_at: 1_700_000_000,
        action: action.into(),
        source_lang: Some("en".into()),
        target_lang: Some("de".into()),
        char_count: source.chars().count() as i64,
        source: Some(Zeroizing::new(source.into())),
        result: Some(Zeroizing::new(result.into())),
    }
}

fn entries(n: usize) -> Vec<HistoryEntry> {
    (0..n)
        .map(|i| {
            fixture(
                i as i64 + 1,
                "translate",
                &format!("source-{i}"),
                &format!("result-{i}"),
            )
        })
        .collect()
}

#[test]
fn modal_opens_then_cancel_button_dismisses_without_clearing() {
    use std::sync::{Arc, Mutex};

    let model = Arc::new(Mutex::new(HistoryModel {
        entries: entries(1),
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    // First frame: paint normally. No modal.
    harness.run();
    // Try to check if modal is absent
    let modal_absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Clear all history?")
    })).is_err();
    assert!(modal_absent, "modal should be hidden initially");

    // Open the modal by simulating Shift+Del. The viewer's draw doesn't
    // handle keys directly (handle_keys_history does); for kittest we
    // toggle confirm_clear directly to mirror the App's flip.
    {
        let mut m = model.lock().unwrap();
        m.confirm_clear = true;
    }
    harness.run();
    // If get_by_label succeeds (doesn't panic), the element exists
    let _modal = harness.get_by_label("Clear all history?");

    // Dismiss via the Cancel button. (The Esc keyboard path lives in
    // app.rs::handle_keys_history and is out of scope for this draw-only test.)
    harness.get_by_label("Cancel").click();
    harness.run();
    {
        let m = model.lock().unwrap();
        assert!(!m.confirm_clear);
        assert_eq!(m.entries.len(), 1, "rows must be untouched");
    }
}

#[test]
fn modal_clear_button_emits_clear_all_outcome() {
    use std::sync::{Arc, Mutex};

    let model = Arc::new(Mutex::new(HistoryModel {
        entries: entries(3),
        confirm_clear: true,
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &mut m) {
            // Stash the outcome on the model via a side-channel.
            if let HistoryOutcome::ClearAll = o {
                m.query = "__clear_all__".into();
            }
        }
    });

    harness.run();
    harness.get_by_label("Clear all").click();
    harness.run();

    {
        let m = model.lock().unwrap();
        assert_eq!(
            m.query,
            "__clear_all__",
            "ClearAll outcome should have been emitted"
        );
        assert!(
            !m.confirm_clear,
            "modal should auto-dismiss after click"
        );
    }
}

#[test]
fn enter_inside_modal_emits_clear_all_via_keyboard_path() {
    use std::sync::{Arc, Mutex};

    // Keyboard activation path (distinct from Test 2's mouse-click path).
    // We focus the "Clear all" button via AccessKit and dispatch a
    // Key::Enter press through `Node::key_press`, which internally calls
    // `focus()` then injects KeyDown+KeyUp events targeted at the focused
    // node. This proves the AccessKit tree exposes the danger button as
    // both focus-acquirable AND keyboard-activatable, which is a real
    // assistive-tech (AT) concern beyond mouse activation.
    //
    // Note: in production, `app.rs::handle_keys_history` catches a
    // global Key::Enter and emits ClearAll directly without going
    // through the button at all. This test exercises the button's
    // keyboard contract, which is what the AccessKit tree promises to
    // screen-reader / keyboard-only users.

    let model = Arc::new(Mutex::new(HistoryModel {
        entries: entries(3),
        confirm_clear: true,
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        if let Some(HistoryOutcome::ClearAll) = draw(ctx, &mut m) {
            m.query = "__clear_all__".into();
        }
    });

    harness.run();
    // `Node::key_press` focuses the node first, then sends Enter
    // (KeyDown + KeyUp) — no mouse click involved.
    harness.get_by_label("Clear all").key_press(Key::Enter);
    harness.run();

    let m = model.lock().unwrap();
    assert_eq!(m.query, "__clear_all__");
    assert!(
        !m.confirm_clear,
        "modal should auto-dismiss after keyboard activation"
    );
}

#[test]
fn typing_into_search_field_propagates_to_model_query() {
    use std::sync::{Arc, Mutex};

    // Verifies the TextEdit binding contract: typing into the focused search
    // field propagates the typed characters into model.query. This is the
    // draw-layer prerequisite for the App layer's defense in
    // handle_keys_history (app.rs:989-1001), which inspects egui::Event::Text
    // events to suppress the global 's'/'d' shortcuts when the user is
    // actively typing into search.
    //
    // ADAPTATION (kittest 0.31.1): The Role enum lives at `egui::accesskit::Role`
    // (re-exported from accesskit via egui; egui_kittest does NOT re-export it).
    // We focus the TextInput by accesskit role, then push a Text event into
    // the input queue and assert model.query reflects the typed string.

    let model = Arc::new(Mutex::new(HistoryModel {
        entries: entries(2),
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    harness.run();

    // Focus the TextEdit via its accesskit role.
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.run();

    // Type "smart" into the focused field by pushing a Text event.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("smart".into()));
    harness.run();

    // The TextEdit's binding to `model.query` should have captured the text.
    {
        let m = model.lock().unwrap();
        assert_eq!(
            m.query, "smart",
            "TextEdit binding should propagate typed text into model.query"
        );
        assert_eq!(m.entries.len(), 2, "entries should be unmodified");
    }
}

#[test]
fn typing_a_query_filters_the_list_and_clamps_selected() {
    use std::sync::{Arc, Mutex};

    let mut initial_entries = entries(5);
    // Make one of them stand out so a filter actually narrows.
    initial_entries[2] = fixture(3, "rewrite", "rewriting source", "the rewritten output");
    initial_entries[4] = fixture(5, "rewrite", "another rewrite case", "..");

    let model = Arc::new(Mutex::new(HistoryModel {
        entries: initial_entries,
        selected: 4, // out of range after filter
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    harness.run();

    // Focus the TextEdit and type "rewr" into it. The viewer's draw applies
    // the filter and clamps model.selected if it would exceed the new list length.
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.run();
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("rewr".into()));
    harness.run();

    // After filter, only 2 entries match. The viewer's draw clamps
    // model.selected to 0 if it would otherwise exceed the filtered
    // list length (lines 197-199 in history.rs).
    {
        let m = model.lock().unwrap();
        assert_eq!(m.query, "rewr", "TextEdit should have captured typed text");
        assert_eq!(
            m.selected, 0,
            "viewer should clamp out-of-range selected to 0 (not max), got {}",
            m.selected
        );
    }
}

#[test]
fn active_row_marker_renders_when_selected_changes() {
    use std::sync::{Arc, Mutex};

    // The viewer's draw does not react to arrow keys (the App layer's
    // handle_keys_history does). This test pins the rendering contract:
    // when the App mutates `selected`, the active-row marker (▸) appears
    // somewhere in the tree, and the viewer renders without panic at
    // both interior (1) and boundary (2) indices.

    let model = Arc::new(Mutex::new(HistoryModel {
        entries: entries(3),
        selected: 0,
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    harness.run();

    // Mutate selected to the boundary index (last row) — mirrors what
    // the App layer would do on ArrowDown × 2.
    {
        let mut m = model.lock().unwrap();
        m.selected = 2;
    }
    harness.run();

    // Verify the active-row arrow marker ("▸") is visible when selected=2.
    let has_marker = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("▸")
    })).is_ok();
    assert!(has_marker, "the active-row arrow marker should be visible");

    // When selected=1 (interior index), verify rendering works without panic.
    {
        let mut m = model.lock().unwrap();
        m.selected = 1;
    }
    harness.run();
    {
        let m = model.lock().unwrap();
        assert_eq!(m.selected, 1);
    }
}
