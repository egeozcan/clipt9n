//! egui_kittest tests for the settings editor (`src/ui/settings.rs`).
//! Same shape as `kittest_setup.rs` / `kittest_tray.rs`: an
//! `Arc<Mutex<_>>` shared between the harness closure and the test body
//! so state can be inspected between frames, and `catch_unwind` around
//! `get_by_label` for presence probes (it panics rather than returning
//! `Option` in kittest 0.31).

use clipt9n::config::Config;
use clipt9n::ui::settings::{draw, KeyStorage, SettingsModel, SettingsOutcome, SettingsTab};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::{Arc, Mutex};

fn model_on(tab: SettingsTab) -> SettingsModel {
    let cfg = Config::default();
    SettingsModel {
        original: cfg.clone(),
        cfg,
        tab,
        key_storage: KeyStorage::Keychain,
        keychain_available: true,
        has_stored_key: true,
        config_path_display: "/tmp/clipt9n/config.toml".into(),
        ..Default::default()
    }
}

/// A harness plus handles on the state it paints and the outcomes it
/// has emitted.
type Rig = (
    Harness<'static>,
    Arc<Mutex<SettingsModel>>,
    Arc<Mutex<Option<SettingsOutcome>>>,
);

/// Build a harness that paints `model` and records the outcome of every
/// frame that produced one.
fn harness_for(model: SettingsModel) -> Rig {
    let model = Arc::new(Mutex::new(model));
    let outcome: Arc<Mutex<Option<SettingsOutcome>>> = Arc::new(Mutex::new(None));
    let model_clone = Arc::clone(&model);
    let outcome_clone = Arc::clone(&outcome);
    let harness = Harness::new(move |ctx| {
        let mut m = model_clone.lock().unwrap();
        if let Some(o) = draw(ctx, &mut m) {
            *outcome_clone.lock().unwrap() = Some(o);
        }
    });
    (harness, model, outcome)
}

fn label_exists(harness: &Harness<'static>, label: &str) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| harness.get_by_label(label))).is_ok()
}

#[test]
fn cancel_button_dispatches_cancel_outcome() {
    let (mut harness, _model, outcome) = harness_for(model_on(SettingsTab::Provider));
    harness.run();
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(*outcome.lock().unwrap(), Some(SettingsOutcome::Cancel));
}

#[test]
fn save_button_dispatches_save_outcome() {
    let (mut harness, _model, outcome) = harness_for(model_on(SettingsTab::Provider));
    harness.run();
    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(*outcome.lock().unwrap(), Some(SettingsOutcome::Save));
}

#[test]
fn open_config_button_dispatches_open_config_outcome() {
    let (mut harness, _model, outcome) = harness_for(model_on(SettingsTab::Provider));
    harness.run();
    harness.get_by_label("Open config.toml").click();
    harness.run();
    assert_eq!(
        *outcome.lock().unwrap(),
        Some(SettingsOutcome::OpenConfigFile)
    );
}

#[test]
fn clicking_a_tab_switches_the_visible_body() {
    let (mut harness, model, _outcome) = harness_for(model_on(SettingsTab::Provider));
    harness.run();
    // The Provider tab owns the base-URL row; Languages owns the slot rows.
    assert!(label_exists(&harness, "Base URL"));
    assert!(!label_exists(&harness, "Slot 1"));

    harness.get_by_label("Languages").click();
    harness.run();

    assert_eq!(model.lock().unwrap().tab, SettingsTab::Languages);
    assert!(label_exists(&harness, "Slot 1"));
    assert!(!label_exists(&harness, "Base URL"));
}

#[test]
fn hotkey_conflict_blocks_save_and_shows_the_reason() {
    let mut m = model_on(SettingsTab::Hotkeys);
    // Point the history hotkey at the prompt hotkey's combination.
    m.cfg.hotkey.history.modifier = m.cfg.hotkey.modifier.clone();
    m.cfg.hotkey.history.option = m.cfg.hotkey.option;
    m.cfg.hotkey.history.shift = m.cfg.hotkey.shift;
    m.cfg.hotkey.history.key = m.cfg.hotkey.key.clone();

    let (mut harness, _model, outcome) = harness_for(m);
    harness.run();

    assert!(
        label_exists(
            &harness,
            "Conflict: Prompt and History both use Cmd+Option+T"
        ),
        "the conflict should be named, not just silently disable Save"
    );

    // A disabled Save must not emit an outcome even when clicked.
    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(*outcome.lock().unwrap(), None);
}

#[test]
fn unregisterable_key_is_named_and_blocks_save() {
    let mut m = model_on(SettingsTab::Hotkeys);
    // Saving this used to be a one-click way to make the next launch
    // abort before the window or the tray icon existed.
    m.cfg.hotkey.key = "F5".into();

    let (mut harness, _model, outcome) = harness_for(m);
    harness.run();
    assert!(label_exists(
        &harness,
        "Prompt key \"F5\" is not a single letter A–Z"
    ));

    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(*outcome.lock().unwrap(), None);
}

#[test]
fn changed_hotkey_surfaces_the_restart_notice() {
    let (mut harness, model, _outcome) = harness_for(model_on(SettingsTab::Hotkeys));
    harness.run();
    assert!(
        !label_exists(&harness, "Takes effect on next launch: hotkey changes."),
        "an untouched config should not claim a restart is needed"
    );

    model.lock().unwrap().cfg.hotkey.key = "J".into();
    harness.run();

    assert!(label_exists(
        &harness,
        "Takes effect on next launch: hotkey changes."
    ));
}

#[test]
fn escape_dispatches_cancel() {
    let (mut harness, _model, outcome) = harness_for(model_on(SettingsTab::Provider));
    harness.run();
    harness.press_key(egui::Key::Escape);
    harness.run();
    assert_eq!(*outcome.lock().unwrap(), Some(SettingsOutcome::Cancel));
}

#[test]
fn missing_key_is_called_out_on_the_provider_tab() {
    let mut m = model_on(SettingsTab::Provider);
    m.has_stored_key = false;
    let (mut harness, _model, _outcome) = harness_for(m);
    harness.run();
    assert!(label_exists(
        &harness,
        "No key resolves for the current settings — enter one before saving."
    ));
}
