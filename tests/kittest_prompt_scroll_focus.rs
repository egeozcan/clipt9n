//! Regression test for the "arrow keys lose focus when ScrollArea scrolls" bug.
//!
//! Root cause was that `draw_slot_row` gated its inner content allocation
//! (`ui.new_child(...)`) behind `if ui.is_rect_visible(rect)`. `new_child`
//! increments the parent UI's `next_auto_id_salt`, which is also the source
//! of every slot row's widget id. When a slot's `is_rect_visible` flipped
//! mid scroll-animation, the salt count changed for subsequent slots → all
//! ids shifted → egui's focus (pinned to a specific id) was discarded by
//! the dead-man's switch in `end_pass`.
//!
//! This test constrains the harness so the 8-slot list overflows the
//! ScrollArea and presses ArrowDown enough times to walk 1 → 8, asserting
//! focus is preserved at every step.

use clipt9n::config::Config;
use clipt9n::ui::prompt::{draw, PromptModel};
use egui::Key;
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

struct PromptHarnessState {
    cfg: Config,
    model: PromptModel,
    focused_slot: Option<u8>,
    first_frame: bool,
}

fn fresh_state() -> PromptHarnessState {
    PromptHarnessState {
        cfg: Config::default(),
        model: PromptModel {
            clipboard_text: "Hallo Welt! Wie geht es dir heute?".into(),
            detected_lang: Some("de".into()),
            last_slot: Some(1),
            glossary_hits: vec![],
        },
        focused_slot: None,
        first_frame: true,
    }
}

#[test]
fn arrow_down_keeps_focus_through_scroll_to_last_slot() {
    let state = Arc::new(Mutex::new(fresh_state()));
    let state_clone = Arc::clone(&state);

    // Constrain to a small height so the 8-slot list overflows the
    // ScrollArea and scrolling is required to reveal slots 5–8.
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(480.0, 380.0))
        .build(move |ctx| {
            let mut s = state_clone.lock().unwrap();
            let focus_target = if s.first_frame {
                s.first_frame = false;
                Some(1)
            } else {
                None
            };
            let mut focused = None;
            let _ = draw(ctx, &s.cfg, &s.model, focus_target, &mut focused);
            s.focused_slot = focused;
        });

    // Let initial focus settle on slot 1.
    harness.run();
    harness.run();
    assert_eq!(
        state.lock().unwrap().focused_slot,
        Some(1),
        "slot 1 should have focus after initial frames"
    );

    // Walk down through every slot. The scroll-induced focus loss bug
    // would clear focus once a slot below the visible region was reached
    // (typically around slot 5 with this harness size).
    for expected in 2..=8u8 {
        harness.press_key(Key::ArrowDown);
        // Several steps so the scroll animation completes before we
        // sample focus state.
        for _ in 0..6 {
            harness.run();
        }
        let focused = state.lock().unwrap().focused_slot;
        assert_eq!(
            focused,
            Some(expected),
            "after ArrowDown to slot {expected}, focus should sit on \
             that slot — got {focused:?}. Focus was lost because the \
             slot's widget id shifted mid scroll-animation."
        );
    }
}
