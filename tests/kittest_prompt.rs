//! Regression test for the M4 slot-row click-eating bug. Clicking on
//! the LITERAL slot text (e.g., the word "English") must fire the
//! slot's `Pick(n)` outcome — even though egui `Label`s default to
//! `Sense::click_and_drag()` for text selection. The fix lives in
//! `src/ui/prompt.rs::draw_slot_row` where the slot frame is wrapped in
//! `Sense::click()` (the row's response) AND the inner labels use
//! `selectable(false)`.
//!
//! IMPORTANT: This test injects RAW pixel-level pointer events
//! (`PointerMoved` + `PointerButton`) at the inner Label's bounding-box
//! center, NOT an AccessKit-targeted Click action. AccessKit-targeted
//! clicks bypass egui's hit-test priority queue and thus would NOT
//! reproduce the M4 hit-test race (where a `Sense::click_and_drag`
//! Label inside the button wins the hit-test against its parent).
//! Pixel-level events DO go through egui's hit-test, so removing
//! `selectable(false)` from the Label correctly causes this test to fail.

use clipt9n::config::Config;
use clipt9n::ui::prompt::{draw, PromptModel, PromptOutcome};
use egui::accesskit::Role;
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

struct PromptHarnessState {
    cfg: Config,
    model: PromptModel,
    /// Most-recent `Pick(n)` outcome, captured by the harness closure.
    picked: Option<u8>,
}

fn fresh_state(text: &str) -> PromptHarnessState {
    PromptHarnessState {
        cfg: Config::default(), // 3 language slots: en, de, tr
        model: PromptModel {
            clipboard_text: text.into(),
            detected_lang: Some("de".into()),
            last_slot: Some(1),
            glossary_hits: vec![],
        },
        picked: None,
    }
}

#[test]
fn clicking_the_literal_slot_text_fires_pick_outcome() {
    let state = Arc::new(Mutex::new(fresh_state("Guten Tag.")));
    let state_clone = Arc::clone(&state);

    let mut harness = Harness::new(move |ctx| {
        let mut s = state_clone.lock().unwrap();
        if let Some(PromptOutcome::Pick(n)) = draw(ctx, &s.cfg, &s.model, None, &mut None) {
            s.picked = Some(n);
        }
    });

    // Lay out one frame so the AccessKit tree (with bounding rects) is built.
    harness.run();

    // The slot-1 row contains the language label "English" (default
    // config slot_1.label = "English"). The M4 regression target:
    // clicking the literal text PIXELS must fire Pick(1). A default
    // `Label` in egui has `Sense::click_and_drag()` for text selection,
    // and would win egui's hit-test against the parent row's
    // `Sense::click()` — eating the click. The fix wraps the inner
    // Label in `selectable(false)` (no click sense) AND the row frame
    // in `Sense::click()`. Both halves must hold for this test to pass.
    //
    // To exercise BOTH halves, we inject RAW pointer events at the
    // inner Label's bounding-box center. (An AccessKit-targeted Click
    // would bypass egui's hit-test and only catch the row-frame half.)
    //
    // Step 1: Find the inner Label whose value is "English". By default
    // kittest filters out label-provider nodes when querying by label,
    // so we use `By::new().role(Label).label("English").include_labels()`
    // to keep them in the result.
    let label_node = harness.get(by().role(Role::Label).label("English").include_labels());

    // Step 2: Get the Label's pixel rect from AccessKit (egui populates
    // this when the widget is laid out).
    let bounds = label_node
        .raw_bounds()
        .expect("inner 'English' Label node must have raw_bounds after layout");
    let center = egui::Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );

    // Step 3: Inject raw pointer events at the Label's center. These
    // flow through egui's hit-test priority queue, which is the path
    // the M4 bug walked through.
    let modifiers = egui::Modifiers::default();
    {
        let input = harness.input_mut();
        input.events.push(egui::Event::PointerMoved(center));
        input.events.push(egui::Event::PointerButton {
            pos: center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        });
        input.events.push(egui::Event::PointerButton {
            pos: center,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        });
    }
    harness.run();

    {
        let s = state.lock().unwrap();
        assert_eq!(
            s.picked,
            Some(1),
            "clicking the literal 'English' label PIXELS must fire \
             Pick(1) — this is the M4 regression check. If this fails, \
             EITHER the inner Label lost `selectable(false)` (and its \
             default Sense::click_and_drag is now eating the click), \
             OR the row frame lost `Sense::click()` (and the response \
             can no longer fire)."
        );
    }
}
