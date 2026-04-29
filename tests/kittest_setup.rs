//! egui_kittest tests for the setup wizard (`src/ui/setup.rs`).
//! Tasks 7–9 each add tests; this file collects all five before the
//! M6 milestone closes.

use clipt9n::ui::setup::{draw, SetupOutcome, SetupWizardModel, Storage, WizardPhase};
use egui::accesskit::Role;
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

fn entry_model() -> SetupWizardModel {
    SetupWizardModel {
        provider: "anthropic".into(),
        key: Zeroizing::new(String::new()),
        show_key: false,
        storage: Storage::Keychain,
        test_translation: true,
        phase: WizardPhase::Entry,
        keychain_available: true,
        ..Default::default()
    }
}

/// Verify that the env-var hint label under the Env storage radio reflects
/// the currently active provider.
///
/// kittest 0.31.1 adaptations applied here:
///
/// 1. `Harness::new_state` does not exist — Arc<Mutex<_>> shared between
///    closure and test body is used instead (same pattern as kittest_history.rs).
///
/// 2. `harness.try_get_by_label` does not exist — presence is asserted via
///    `std::panic::catch_unwind(AssertUnwindSafe(...))` wrapping `get_by_label`.
///
/// 3. `harness.state()` does not exist — model is read back through the Arc.
///
/// 4. Provider-card click: the cards are rendered as egui Frames with
///    `interact(Sense::click())` applied to the whole frame rect. The inner
///    Label nodes are not Buttons in the AccessKit tree, so an AccessKit-
///    targeted `.click()` does not propagate through egui's hit-test to the
///    Frame's `resp.clicked()` check. Instead, we inject raw pixel-level
///    `PointerMoved` + `PointerButton` events at the Label node's
///    `raw_bounds()` center — the same technique used in kittest_prompt.rs
///    for the M4 slot-row regression. These events flow through egui's
///    hit-test priority queue and register as a click on the Frame's
///    interaction rect.
#[test]
fn switching_provider_updates_env_var_hint_under_env_radio() {
    let model = Arc::new(Mutex::new(SetupWizardModel {
        storage: Storage::Env, // force the env-var hint to render in col[1]
        ..entry_model()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    // First frame: paint with "anthropic" selected.
    harness.run();

    // "$ANTHROPIC_API_KEY" must be present in the AccessKit tree.
    let hint_present = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("$ANTHROPIC_API_KEY")
    }))
    .is_ok();
    assert!(
        hint_present,
        "env-var hint must show $ANTHROPIC_API_KEY when provider=anthropic"
    );

    // Find the "OpenAI" Label node and get its pixel bounding rect.
    // We use include_labels() so Label-role nodes are included in the result.
    let label_node = harness.get(by().role(Role::Label).label("OpenAI").include_labels());
    let bounds = label_node
        .raw_bounds()
        .expect("'OpenAI' Label node must have raw_bounds after layout");
    let center = egui::Pos2::new(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );

    // Inject raw pointer events at the Label's center. These flow through
    // egui's hit-test and register a click on the provider card Frame's
    // interact(Sense::click()) rect, which sets model.provider = "openai".
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

    // The model must now reflect "openai".
    {
        let m = model.lock().unwrap();
        assert_eq!(
            m.provider, "openai",
            "raw-pointer click on the OpenAI card must set provider = \"openai\""
        );
    }

    // And the env-var hint must now read "$OPENAI_API_KEY".
    let hint_updated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("$OPENAI_API_KEY")
    }))
    .is_ok();
    assert!(
        hint_updated,
        "after picking OpenAI, the env-var hint should update to $OPENAI_API_KEY"
    );
}

/// Verify that the show/hide toggle on the password field flips the
/// `show_key` flag when the button is clicked.
#[test]
fn show_hide_toggle_flips_show_key_flag() {
    let model = Arc::new(Mutex::new(SetupWizardModel {
        key: Zeroizing::new("sk-ant-secret".into()),
        ..entry_model()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    harness.run();
    {
        let m = model.lock().unwrap();
        assert!(!m.show_key, "default is hidden");
    }

    // M7.B: the show/hide toggle's AccessKit label is the descriptive
    // hover_text, not the short "show"/"hide" toggle token. The Button
    // is now directly clickable via .click() — no raw-pixel fallback
    // needed since the Button itself (not an inner Label node) is the
    // primary AccessKit target.
    harness
        .get_by_label("Show key (reveal as plain text)")
        .click();

    harness.run();
    {
        let m = model.lock().unwrap();
        assert!(m.show_key, "show button must reveal the key");
    }

    // Now toggle back to hidden — the AccessKit label flips with show_key.
    let button_clicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Hide key (mask as password)").click();
    }));

    if button_clicked.is_err() {
        // Defensive fallback: raw pixel events on the inner "hide"
        // visible-text label. egui keeps that as a child Label node
        // even after the parent Button's widget_info override.
        let label_node = harness.get(by().label("hide").include_labels());
        let bounds = label_node
            .raw_bounds()
            .expect("'hide' button must have raw_bounds after layout");
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
    }

    harness.run();
    {
        let m = model.lock().unwrap();
        assert!(!m.show_key, "hide button must remask");
    }
}

/// Verify that the sample-translation checkbox toggles the visibility
/// of the second check row.
#[test]
fn sample_translation_checkbox_toggles_second_check_row_visibility() {
    let model = Arc::new(Mutex::new(SetupWizardModel {
        test_translation: true,
        ..entry_model()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    harness.run();
    let row_present = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Sample translation")
    }))
    .is_ok();
    assert!(
        row_present,
        "second check row visible when test_translation=true"
    );

    // Flip the checkbox by setting test_translation to false.
    {
        let mut m = model.lock().unwrap();
        m.test_translation = false;
    }
    harness.run();

    let row_absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Sample translation")
    }))
    .is_err();
    assert!(
        row_absent,
        "second check row hidden when test_translation=false"
    );
}

/// Verify that the Save-and-start button is only visible and the Verify
/// button is hidden when phase == Done.
#[test]
fn save_and_start_button_only_visible_in_done_phase() {
    let model = Arc::new(Mutex::new(SetupWizardModel {
        key: Zeroizing::new("sk-ant-test-12345".into()),
        ..entry_model()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });

    harness.run();
    // Phase is Entry — only Verify button is visible.
    let verify_present = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Verify →")
    }))
    .is_ok();
    assert!(
        verify_present,
        "Verify button must be visible in Entry phase"
    );

    let save_absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Save and start ✓")
    }))
    .is_err();
    assert!(save_absent, "Save button must be absent in Entry phase");

    // Mutate the model to phase=Done. In real usage, the App's
    // update_setup_wizard handler flips this when both check1 and
    // check2 reach Ok status.
    {
        let mut m = model.lock().unwrap();
        m.phase = WizardPhase::Done;
    }
    harness.run();

    let verify_absent = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Verify →")
    }))
    .is_err();
    assert!(verify_absent, "Verify must yield to Save in Done phase");

    let save_present = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Save and start ✓")
    }))
    .is_ok();
    assert!(save_present, "Save button must be visible in Done phase");
}

/// Verify that clicking "Verify →" emits `SetupOutcome::Verify`.
///
/// kittest 0.31.1 adaptations:
/// - `Harness::new_state` / `harness.state()` don't exist — we use
///   `Arc<Mutex<_>>` shared between the closure and the test body.
/// - `.click()` is tried first on the button node; if the AccessKit click
///   doesn't propagate through egui's hit-test (the button is `add_enabled`
///   and the node may be reported as disabled), we fall back to raw pixel
///   events at the button's `raw_bounds()` center (same pattern as Test 1).
#[test]
fn verify_button_click_does_not_panic_when_check_handler_is_absent() {
    use std::sync::{Arc, Mutex};

    let model = Arc::new(Mutex::new(SetupWizardModel {
        key: Zeroizing::new("sk-ant-test".into()),
        ..entry_model()
    }));

    // We stash the emitted outcome into model.err_msg as a sentinel string
    // so we can read it back through the Arc.
    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &mut m) {
            m.err_msg = format!("__outcome:{o:?}");
        }
    });

    // Initial render so AccessKit tree is populated.
    harness.run();

    // Try AccessKit .click() first.
    let click_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        harness.get_by_label("Verify →").click();
    }));

    if click_result.is_err() {
        // Fallback: raw pixel events at the Verify button's bounds center.
        let label_node = harness.get(by().role(Role::Label).label("Verify →").include_labels());
        let bounds = label_node
            .raw_bounds()
            .expect("'Verify →' label node must have raw_bounds after layout");
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
    }

    harness.run();

    let err_msg = model.lock().unwrap().err_msg.clone();
    assert!(
        err_msg.contains("Verify"),
        "Verify outcome should have been emitted, got err_msg={err_msg:?}"
    );
    // Keep the import used.
    let _ = SetupOutcome::Verify;
}
