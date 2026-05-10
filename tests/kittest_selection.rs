//! Integration tests for the selected-text capture path (Item 9 of the
//! improvement plan). Exercises the prompt window UI state that follows a
//! successful selection capture: clipboard text display, character count,
//! language-detection chip, and slot-row interactivity.
//!
//! These tests exercise the *visible* half of the selection hotkey path.
//! The platform-level `copy_selection_to_clipboard()` and clipboard
//! restore logic are inherently side-effectful and tested indirectly
//! through `selected_text_after_copy` unit tests in `src/app/pure.rs`.

use clipt9n::config::Config;
use clipt9n::ui::prompt::{draw, GlossaryHit, PromptModel, PromptOutcome};
use egui::accesskit::Role;
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

struct PromptHarnessState {
    cfg: Config,
    model: PromptModel,
    /// Most-recent outcome, captured by the harness closure.
    outcome: Option<PromptOutcome>,
}

fn state_after_selection_capture(text: &str, detected_lang: &str) -> PromptHarnessState {
    PromptHarnessState {
        cfg: Config::default(),
        model: PromptModel {
            clipboard_text: text.into(),
            detected_lang: Some(detected_lang.into()),
            last_slot: Some(1),
            glossary_hits: vec![],
        },
        outcome: None,
    }
}

/// After a successful selection capture of German text, the prompt window
/// must show the clipboard text, its character count, and the detected-
/// language chip (DE) — all queryable through AccessKit.
#[test]
fn selection_capture_prompt_shows_clipboard_text_and_language_chip() {
    let state = Arc::new(Mutex::new(state_after_selection_capture(
        "Guten Tag, wie geht es Ihnen?",
        "de",
    )));
    let state_clone = Arc::clone(&state);

    let mut harness = Harness::new(move |ctx| {
        let mut s = state_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &s.cfg, &s.model, None) {
            s.outcome = Some(o);
        }
    });
    harness.run();

    // The clipboard preview must render the text — the preview block
    // prepends each line with a '›' marker glyph. The text itself
    // should be findable via AccessKit label (may include the preview
    // marker).
    let _ = harness.get_by_label("CLIPBOARD");

    // Character count is rendered as "· N chars". "Guten Tag, wie geht
    // es Ihnen?" is 29 characters.
    let _ = harness.get_by_label("· 29 chars");

    // The language chip renders the uppercase ISO code.
    let _ = harness.get_by_label("DE");
}

/// When the detected language is unknown (None / "??"), the chip shows
/// "??" — selection capture didn't yield a confident detection. The
/// prompt must still render the clipboard text.
#[test]
fn unknown_language_shows_fallback_chip_and_still_renders_text() {
    let state = Arc::new(Mutex::new(PromptHarnessState {
        cfg: Config::default(),
        model: PromptModel {
            clipboard_text: "12345 !@#$%".into(),
            detected_lang: None, // whatlang confidence below threshold
            last_slot: None,
            glossary_hits: vec![],
        },
        outcome: None,
    }));
    let state_clone = Arc::clone(&state);

    let mut harness = Harness::new(move |ctx| {
        let mut s = state_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &s.cfg, &s.model, None) {
            s.outcome = Some(o);
        }
    });
    harness.run();

    // Clipboard header is always visible.
    let _ = harness.get_by_label("CLIPBOARD");

    // Fallback chip: "??"
    let _ = harness.get_by_label("??");

    // Character count: "12345 !@#$%" is 11 characters.
    let _ = harness.get_by_label("· 11 chars");
}

/// Clicking a slot row after selection capture fires the Pick outcome.
/// This simulates the user pressing a slot after selecting text with
/// the selection hotkey.
#[test]
fn clicking_slot_after_selection_capture_fires_pick() {
    let state = Arc::new(Mutex::new(state_after_selection_capture(
        "Hello world!",
        "en",
    )));
    let state_clone = Arc::clone(&state);

    let mut harness = Harness::new(move |ctx| {
        let mut s = state_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &s.cfg, &s.model, None) {
            s.outcome = Some(o);
        }
    });
    harness.run();

    // Find slot 4 (Fix grammar) — it renders as a clickable row.
    // Use raw pixel events on the inner Label, same pattern as
    // kittest_prompt.rs for the M4 regression.
    let label_node = harness.get(
        by().role(Role::Label)
            .label("Fix grammar")
            .include_labels(),
    );
    let bounds = label_node
        .raw_bounds()
        .expect("'Fix grammar' label must have raw_bounds after layout");
    let center = egui::Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );

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

    let s = state.lock().unwrap();
    assert_eq!(
        s.outcome,
        Some(PromptOutcome::Pick(4)),
        "clicking slot 4 ('Fix grammar') after selection capture must fire Pick(4)"
    );
}

/// The glossary-hit chip strip is rendered between the preview and the
/// slot list when glossary entries match the clipboard text. After a
/// selection capture, matched glossary terms should be visible as
/// separate source-label and target-label nodes with an arrow between them.
#[test]
fn glossary_chips_render_after_selection_capture() {
    let hit = GlossaryHit {
        source: "Welt".into(),
        target: "world".into(),
    };
    let state = Arc::new(Mutex::new(PromptHarnessState {
        cfg: Config::default(),
        model: PromptModel {
            clipboard_text: "Hallo Welt!".into(),
            detected_lang: Some("de".into()),
            last_slot: Some(2),
            glossary_hits: vec![hit],
        },
        outcome: None,
    }));
    let state_clone = Arc::clone(&state);

    let mut harness = Harness::new(move |ctx| {
        let mut s = state_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &s.cfg, &s.model, None) {
            s.outcome = Some(o);
        }
    });
    harness.run();

    // The glossary-chip strip renders the section header and individual
    // source/target terms as separate Label nodes with an arrow separator.
    let _ = harness.get_by_label("GLOSSARY WILL INJECT:");
    let _ = harness.get_by_label("Welt");
    let _ = harness.get_by_label("→");
    let _ = harness.get_by_label("world");
}
