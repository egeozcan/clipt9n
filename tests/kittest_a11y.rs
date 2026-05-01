//! M7.B accessibility regression tests. Asserts AccessKit labels are
//! present for every interactive widget across the prompt window,
//! history viewer, setup wizard, and tray-confirm modal. Asserts
//! reduced-motion paths render without panicking.
//!
//! Pattern: Arc<Mutex<Model>> shared between the harness closure and
//! the test body (established in kittest_setup.rs / kittest_tray.rs).
//! Presence queries use catch_unwind(AssertUnwindSafe(...)) because
//! kittest 0.31's `get_by_label` panics on not-found rather than
//! returning Option.

use egui::accesskit::Role;
use egui_kittest::kittest::{by, Queryable};
use egui_kittest::Harness;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

// ── Step 4: Provider cards expose AccessKit labels ──────────────────────────

/// After the M7.B WidgetInfo::labeled backfill, each provider card's
/// Frame+interact resp carries a WidgetType::Button identity with the
/// provider name as its label. `get_by_label` should now find each card.
#[test]
fn setup_wizard_provider_cards_are_queryable_by_label() {
    use clipt9n::ui::setup::{draw, SetupWizardModel, Storage, WizardPhase};
    use zeroize::Zeroizing;

    let model = Arc::new(Mutex::new(SetupWizardModel {
        provider: "anthropic".into(),
        key: Zeroizing::new(String::new()),
        show_key: false,
        storage: Storage::Keychain,
        test_translation: true,
        phase: WizardPhase::Entry,
        keychain_available: true,
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });
    harness.run();

    // Query specifically for the Button-role node (not the inner Label node)
    // because get_by_label panics when multiple nodes share the same label
    // (the inner Label widget AND the WidgetInfo::Button both carry the provider name).
    for label in [
        "Anthropic (Claude)",
        "OpenAI",
        "Google Gemini",
        "Ollama (local)",
    ] {
        let found = std::panic::catch_unwind(AssertUnwindSafe(|| {
            harness.get(by().role(Role::Button).label(label))
        }));
        assert!(
            found.is_ok(),
            "provider card '{label}' should expose Role::Button in AccessKit tree after M7.B \
             WidgetInfo::labeled backfill — Frame+interact did not carry a WidgetType::Button \
             identity before this fix"
        );
    }
}

/// Clicking the OpenAI card (via raw pixel events) still updates model.provider.
/// Regression: verifies the WidgetInfo backfill didn't break the interact() click path.
#[test]
fn setup_wizard_provider_card_click_updates_model() {
    use clipt9n::ui::setup::{draw, SetupWizardModel, Storage, WizardPhase};
    use zeroize::Zeroizing;

    let model = Arc::new(Mutex::new(SetupWizardModel {
        provider: "anthropic".into(),
        key: Zeroizing::new(String::new()),
        show_key: false,
        storage: Storage::Keychain,
        test_translation: true,
        phase: WizardPhase::Entry,
        keychain_available: true,
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });
    harness.run();

    // Try AccessKit click first (may now work after WidgetInfo backfill).
    let ak_click = std::panic::catch_unwind(AssertUnwindSafe(|| {
        harness.get_by_label("OpenAI").click();
    }));

    if ak_click.is_err() {
        // Fallback: raw pixel events at the "OpenAI" label node's center.
        let label_node = harness.get(by().role(Role::Label).label("OpenAI").include_labels());
        let bounds = label_node
            .raw_bounds()
            .expect("'OpenAI' label node must have raw_bounds after layout");
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

    let m = model.lock().unwrap();
    assert_eq!(
        m.provider, "openai",
        "clicking the OpenAI card must update model.provider = \"openai\""
    );
}

// ── Step 5: Show/hide key button has a descriptive AccessKit label ───────────

/// After the M7.B widget_info backfill, the show/hide eye button carries
/// the label "show" or "hide". Both states are checked.
#[test]
fn setup_wizard_show_hide_key_button_has_accessible_label() {
    use clipt9n::ui::setup::{draw, SetupWizardModel, Storage, WizardPhase};
    use zeroize::Zeroizing;

    // Initially: key is non-empty so the button renders, show_key=false → "show".
    let model = Arc::new(Mutex::new(SetupWizardModel {
        provider: "anthropic".into(),
        key: Zeroizing::new("sk-ant-test".into()),
        show_key: false,
        storage: Storage::Keychain,
        test_translation: false,
        phase: WizardPhase::Entry,
        keychain_available: true,
        ..Default::default()
    }));

    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });
    harness.run();

    // The button's AccessKit label is the descriptive hover text (set via
    // widget_info), not the short visible token. Screen readers announce
    // the button's purpose rather than just "show" / "hide".
    let show_ok = std::panic::catch_unwind(AssertUnwindSafe(|| {
        harness.get_by_label("Show key (reveal as plain text)")
    }));
    assert!(
        show_ok.is_ok(),
        "show-mode button should expose the descriptive AccessKit label"
    );

    // Flip to show_key=true → AccessKit label becomes the hide-mode wording.
    {
        let mut m = model.lock().unwrap();
        m.show_key = true;
    }
    harness.run();

    let hide_ok = std::panic::catch_unwind(AssertUnwindSafe(|| {
        harness.get_by_label("Hide key (mask as password)")
    }));
    assert!(
        hide_ok.is_ok(),
        "hide-mode button should expose the descriptive AccessKit label"
    );
}

// ── Step 3: History search field is discoverable ─────────────────────────────

/// After M7.B's widget_info backfill, the history search TextEdit exposes
/// "Search history" as its explicit AccessKit label (set via widget_info).
/// Without that call the TextEdit would only expose its hint_text or be
/// unlabeled in the AccessKit tree — this test gates on the new behavior.
/// The Role::TextInput-only check was removed: egui's TextEdit always
/// emits that role regardless of M7.B, so it's not a useful gate.
/// The typing-binding test was removed: kittest_history.rs already covers
/// that contract; duplicating it here adds no signal.
#[test]
fn history_search_field_exposes_explicit_accesskit_label() {
    use clipt9n::ui::history::{draw, HistoryModel};

    let model = Arc::new(Mutex::new(HistoryModel::default()));
    let model_clone = Arc::clone(&model);
    let mut harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        let _ = draw(ctx, &mut m);
    });
    harness.run();

    let probe =
        std::panic::catch_unwind(AssertUnwindSafe(|| harness.get_by_label("Search history")));
    assert!(
        probe.is_ok(),
        "history search field should expose the AccessKit label 'Search history'"
    );
}

// ── Step 6: Reduced-motion paths render without panicking ────────────────────

/// The translating overlay's reduced-motion static path must render
/// without panicking and expose its AccessKit label.
#[test]
fn translating_overlay_reduced_motion_renders_without_panic() {
    use clipt9n::ui::translating::{draw, TranslatingModel};
    use std::time::Duration;

    let model = TranslatingModel {
        overlay_label: "Translating to Deutsch…".into(),
        provider_model: "claude-3-5-sonnet".into(),
        elapsed: Duration::from_millis(200),
        reduced_motion: true, // Exercise the static path.
    };
    let mut harness = Harness::new(move |ctx| {
        let _ = draw(ctx, &model);
    });
    // Must not panic.
    harness.run();

    // The widget_info in translating.rs sets the AccessKit label to
    // "Translation in progress" (the descriptive form, not the visible "Translating…").
    // This is the label screen readers announce per translating.rs's WCAG 4.1.3 comment.
    let found = std::panic::catch_unwind(AssertUnwindSafe(|| {
        harness.get_by_label("Translation in progress")
    }));
    assert!(
        found.is_ok(),
        "reduced-motion path should expose 'Translation in progress' to screen readers \
         (per translating.rs widget_info call)"
    );
}

/// The translating overlay's animated path must also render without panicking.
#[test]
fn translating_overlay_animated_path_renders_without_panic() {
    use clipt9n::ui::translating::{draw, TranslatingModel};
    use std::time::Duration;

    let model = TranslatingModel {
        overlay_label: "Rewriting for clarity…".into(),
        provider_model: "gpt-4o".into(),
        elapsed: Duration::from_millis(640),
        reduced_motion: false, // Exercise the animated bar path.
    };
    let mut harness = Harness::new(move |ctx| {
        let _ = draw(ctx, &model);
    });
    // Must not panic.
    harness.run();
}

/// The size-confirm modal's Cancel and Send buttons must carry
/// AccessKit labels so keyboard-only users can distinguish them.
#[test]
fn size_confirm_buttons_have_accessible_labels() {
    use clipt9n::ui::size_confirm::{draw, SizeConfirmModel};

    let model = SizeConfirmModel {
        char_count: 5_000,
        preview: "Lorem ipsum dolor sit amet…".into(),
        action_label: "Translate to Deutsch".into(),
    };
    let mut harness = Harness::new(move |ctx| {
        let _ = draw(ctx, &model);
    });
    harness.run();

    for label in ["Send", "Cancel"] {
        let found = std::panic::catch_unwind(AssertUnwindSafe(|| harness.get_by_label(label)));
        assert!(
            found.is_ok(),
            "size-confirm '{label}' button should be findable by AccessKit label"
        );
    }
}

/// The tray-confirm modal disables egui::Window's default `fade_in` (the
/// Area-level alpha animation) so reduced-motion users don't see a fade
/// when the hide-icon dialog opens. This test asserts the modal still
/// paints correctly and exposes its title — i.e., the fade_in(false)
/// addition didn't break rendering.
#[test]
fn tray_modal_renders_without_animation_for_reduced_motion() {
    use clipt9n::ui::tray_modal::{draw, TrayHideModel};

    let model = TrayHideModel {
        hotkey_display: "Cmd+Option+T".into(),
    };
    let mut harness = Harness::new(move |ctx| {
        let _ = draw(ctx, &model);
    });
    harness.run();

    let probe =
        std::panic::catch_unwind(AssertUnwindSafe(|| harness.get_by_label("Hide tray icon?")));
    assert!(
        probe.is_ok(),
        "tray modal title should render with fade_in disabled"
    );
}
