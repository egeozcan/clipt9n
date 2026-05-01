//! egui_kittest tests for the tray hide-confirm modal
//! (`src/ui/tray_modal.rs`). Mirrors the M6 kittest_setup.rs shape:
//! Arc<Mutex<Model>> shared across the harness closure and the test
//! body for state inspection between frames.

use clipt9n::ui::tray_modal::{draw, TrayHideModel, TrayHideOutcome};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

fn entry_model() -> TrayHideModel {
    TrayHideModel {
        hotkey_display: "Cmd+Option+T".into(),
    }
}

#[test]
fn cancel_dispatches_cancel_outcome() {
    let outcome: Arc<Mutex<Option<TrayHideOutcome>>> = Arc::new(Mutex::new(None));
    let model = entry_model();

    let outcome_clone = Arc::clone(&outcome);
    let mut harness = Harness::new(move |ctx| {
        let result = draw(ctx, &model);
        if let Some(o) = result {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });

    harness.run();
    let cancel_btn = harness.get_by_label("Cancel");
    cancel_btn.click();
    harness.run();

    let recorded = outcome.lock().unwrap();
    assert_eq!(*recorded, Some(TrayHideOutcome::Cancel));
}

#[test]
fn hide_button_dispatches_confirm_outcome() {
    let outcome: Arc<Mutex<Option<TrayHideOutcome>>> = Arc::new(Mutex::new(None));
    let model = entry_model();

    let outcome_clone = Arc::clone(&outcome);
    let mut harness = Harness::new(move |ctx| {
        let result = draw(ctx, &model);
        if let Some(o) = result {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });

    harness.run();
    let hide_btn = harness.get_by_label("Hide");
    hide_btn.click();
    harness.run();

    let recorded = outcome.lock().unwrap();
    assert_eq!(*recorded, Some(TrayHideOutcome::Confirm));
}

#[test]
fn modal_displays_configured_hotkey() {
    let model = TrayHideModel {
        hotkey_display: "Ctrl+Shift+Z".into(),
    };
    let mut harness = Harness::new(move |ctx| {
        let _ = draw(ctx, &model);
    });
    harness.run();

    // The modal renders a label containing the configured hotkey as part of
    // "You can still summon clipt9n with Ctrl+Shift+Z."
    // We use catch_unwind (established kittest pattern from kittest_setup.rs)
    // since get_by_label panics on not-found rather than returning Option.
    let probe = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("You can still summon clipt9n with Ctrl+Shift+Z.")
    }));
    assert!(
        probe.is_ok(),
        "the modal should render the configured hotkey"
    );
}

#[test]
fn esc_key_dispatches_cancel() {
    let outcome: Arc<Mutex<Option<TrayHideOutcome>>> = Arc::new(Mutex::new(None));
    let model = entry_model();

    let outcome_clone = Arc::clone(&outcome);
    let mut harness = Harness::new(move |ctx| {
        let result = draw(ctx, &model);
        if let Some(o) = result {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });

    harness.run();
    // Inject a raw Escape key event using the established kittest pattern
    // (input_mut().events.push) since egui_kittest 0.31 doesn't have a
    // top-level key_press convenience method.
    {
        let input = harness.input_mut();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run();

    let recorded = outcome.lock().unwrap();
    assert_eq!(*recorded, Some(TrayHideOutcome::Cancel));
}
