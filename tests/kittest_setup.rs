//! egui_kittest tests for the setup wizard (`src/ui/setup.rs`).
//! Tasks 7–9 each add tests; this file collects all five before the
//! M6 milestone closes.

use clipt9n::ui::setup::{draw, SetupWizardModel, Storage, WizardPhase};
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
