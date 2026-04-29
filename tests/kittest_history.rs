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
