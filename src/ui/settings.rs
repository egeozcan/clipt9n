//! Settings editor — a GUI over `config.toml`. Pure view plus pure
//! helpers; every side effect (writing the key to its backend,
//! persisting the file, rebuilding the provider) lives in
//! `src/app/settings.rs`.
//!
//! The model carries a *working copy* of `Config`. Nothing the user
//! types touches the live config until they press Save, and Save runs
//! the same `Config::validate` the file loader runs — the editor must
//! never be able to write a config that the next launch refuses to
//! load.
//!
//! Not editable here on purpose: `[templates]`. Overrides are compiled
//! once at startup and a bad Jinja template is a hard startup abort, so
//! they stay file-only; the Behavior tab points at the config folder.

use egui::{Color32, RichText, Stroke, TextEdit, Vec2};
use zeroize::Zeroizing;

use crate::config::{hotkey_combo_display, Config};
use crate::ui::theme;

/// Default settings-window size. Taller than the wizard (580×640)
/// because the Hotkeys tab stacks four hotkey groups.
pub const SETTINGS_INNER_SIZE: Vec2 = Vec2::new(640.0, 700.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Provider,
    Languages,
    Hotkeys,
    Behavior,
}

impl SettingsTab {
    pub fn all() -> [Self; 4] {
        [
            Self::Provider,
            Self::Languages,
            Self::Hotkeys,
            Self::Behavior,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::Languages => "Languages",
            Self::Hotkeys => "Hotkeys",
            Self::Behavior => "Behavior",
        }
    }
}

/// Where the API key lives. Mirrors the `provider.api_key.source`
/// values `secrets::resolve` actually honors. `"prompt"` is accepted by
/// the loader but resolves as env, so the editor shows it as `Env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyStorage {
    #[default]
    Keychain,
    Env,
    File,
}

impl KeyStorage {
    pub fn from_source(source: &str) -> Self {
        match source {
            "keychain" => Self::Keychain,
            "file" => Self::File,
            _ => Self::Env,
        }
    }

    pub fn as_source(self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::Env => "env",
            Self::File => "file",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Keychain => "System keychain",
            Self::Env => "Environment variable",
            Self::File => "Key file (0600)",
        }
    }
}

/// What the settings window paints per frame.
#[derive(Debug, Clone, Default)]
pub struct SettingsModel {
    /// Working copy. Edited freely; only committed on Save.
    pub cfg: Config,
    /// Snapshot of the live config at open. Drives the
    /// "restart to apply" notice — see [`restart_required`].
    pub original: Config,
    pub tab: SettingsTab,
    /// New API key, if the user typed one. Empty means "keep the key
    /// that's already stored".
    pub api_key: Zeroizing<String>,
    pub show_key: bool,
    pub key_storage: KeyStorage,
    /// Probed once at open. When false the Keychain option is disabled.
    pub keychain_available: bool,
    /// Whether a key currently resolves through the *live* config.
    /// Drives the "leave blank to keep the stored key" hint versus the
    /// "no key stored yet" warning.
    pub has_stored_key: bool,
    /// Explicit acknowledgement required when the provider origin changes.
    pub provider_origin_change_confirmed: bool,
    /// Populated by the App when a save fails validation, key
    /// persistence, or provider construction.
    pub err_msg: String,
    /// Path shown next to the "Open config.toml" escape hatch.
    pub config_path_display: String,
    /// Modification time of the config file when this window opened.
    /// Save compares against it and refuses rather than silently
    /// reverting edits made in a text editor meanwhile. `None` when the
    /// file doesn't exist yet or its mtime is unreadable — then there is
    /// nothing to clobber.
    pub config_mtime: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsOutcome {
    /// Save button (or Cmd/Ctrl+S) — commit the working copy.
    Save,
    /// Cancel button or Esc — discard every edit.
    Cancel,
    /// Escape hatch to the raw file, for the sections the GUI omits.
    OpenConfigFile,
    /// "Get your API key" link.
    OpenProviderKeyUrl(&'static str),
}

/// Changes that the running process cannot pick up in place. Global
/// hotkeys are registered once at startup, and the history store is
/// opened (or not) at startup — flipping either needs a relaunch.
/// Everything else in this editor applies the moment Save succeeds.
pub fn restart_required(model: &SettingsModel) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if model.cfg.hotkey != model.original.hotkey {
        reasons.push("hotkey changes");
    }
    if model.cfg.history.enabled != model.original.history.enabled {
        reasons.push("enabling/disabling history");
    }
    reasons
}

/// Enabled hotkeys that resolve to the same physical combination.
/// Returned as human-readable pairs ("Prompt and History both use
/// Cmd+Option+T"); the OS would otherwise silently refuse the second
/// registration at next launch.
pub fn hotkey_conflicts(cfg: &Config) -> Vec<String> {
    let h = &cfg.hotkey;
    let combos: Vec<(&str, bool, String)> = vec![
        (
            "Prompt",
            h.enabled,
            hotkey_combo_display(&h.modifier, h.option, h.shift, &h.key),
        ),
        (
            "History",
            h.history.enabled,
            hotkey_combo_display(
                &h.history.modifier,
                h.history.option,
                h.history.shift,
                &h.history.key,
            ),
        ),
        (
            "Selection",
            h.selection.enabled,
            hotkey_combo_display(
                &h.selection.modifier,
                h.selection.option,
                h.selection.shift,
                &h.selection.key,
            ),
        ),
        (
            "Replace",
            h.replace.enabled,
            hotkey_combo_display(
                &h.replace.modifier,
                h.replace.option,
                h.replace.shift,
                &h.replace.key,
            ),
        ),
    ];
    let mut conflicts = Vec::new();
    for i in 0..combos.len() {
        for j in (i + 1)..combos.len() {
            let (a_name, a_on, a_combo) = &combos[i];
            let (b_name, b_on, b_combo) = &combos[j];
            if *a_on && *b_on && a_combo.eq_ignore_ascii_case(b_combo) {
                conflicts.push(format!("{a_name} and {b_name} both use {a_combo}"));
            }
        }
    }
    conflicts
}

/// Hotkeys whose key name the app cannot register.
///
/// This is a hard Save blocker, not a warning: an unregisterable *prompt*
/// key aborts the next launch outright (`main.rs` treats it as fatal
/// before it even checks `enabled`), which would leave the user with no
/// window and no tray icon to fix it from.
pub fn unsupported_hotkey_keys(cfg: &Config) -> Vec<String> {
    let h = &cfg.hotkey;
    [
        ("Prompt", &h.key),
        ("History", &h.history.key),
        ("Selection", &h.selection.key),
        ("Replace", &h.replace.key),
    ]
    .into_iter()
    .filter(|(_, key)| !crate::config::hotkey_key_is_supported(key))
    .map(|(name, key)| format!("{name} key \"{key}\" is not a single letter A–Z"))
    .collect()
}

/// Whether the edited provider base URL has a different origin from the
/// configuration snapshot opened by the settings window.
pub fn provider_origin_changed(model: &SettingsModel) -> bool {
    match (
        model.original.provider_endpoint(),
        model.cfg.provider_endpoint(),
    ) {
        (Ok(original), Ok(candidate)) => !original.same_origin(&candidate),
        _ => model.original.provider.base_url != model.cfg.provider.base_url,
    }
}

fn save_enabled_with_file_storage(
    model: &SettingsModel,
    secure_file_storage_supported: bool,
) -> bool {
    hotkey_conflicts(&model.cfg).is_empty()
        && unsupported_hotkey_keys(&model.cfg).is_empty()
        && (model.key_storage != KeyStorage::File || secure_file_storage_supported)
        && (!provider_origin_changed(model) || model.provider_origin_change_confirmed)
}

/// Whether Save should be clickable. Blocked only by things the editor
/// can detect locally; provider/key errors surface after the attempt.
pub fn save_enabled(model: &SettingsModel) -> bool {
    save_enabled_with_file_storage(model, crate::platform::secure_file_storage_supported())
}

/// Paint the settings window. Returns at most one outcome per frame.
pub fn draw(ctx: &egui::Context, model: &mut SettingsModel) -> Option<SettingsOutcome> {
    let mut outcome: Option<SettingsOutcome> = None;

    // Keyboard: Esc discards, Cmd/Ctrl+S saves. Checked before paint so
    // a shortcut wins over whatever widget holds focus.
    let (esc, save_combo) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::S) && (i.modifiers.command || i.modifiers.ctrl),
        )
    });
    // A dropdown swallows its own Esc. Without this check, dismissing a
    // combo box would throw away every edit in the form.
    let popup_open = ctx.memory(|m| m.any_popup_open());
    if esc && !popup_open {
        outcome = Some(SettingsOutcome::Cancel);
    } else if save_combo && save_enabled(model) {
        outcome = Some(SettingsOutcome::Save);
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(18.0))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("Settings")
                    .color(theme::INK)
                    .strong()
                    .size(15.0),
            );
            ui.label(
                RichText::new(&model.config_path_display)
                    .color(theme::INK_3)
                    .monospace()
                    .size(10.5),
            );
            ui.add_space(10.0);

            // Tab bar.
            ui.horizontal(|ui| {
                for tab in SettingsTab::all() {
                    let active = model.tab == tab;
                    let text = RichText::new(tab.label()).size(12.5).color(if active {
                        theme::ACCENT_INK
                    } else {
                        theme::INK_2
                    });
                    let btn = egui::Button::new(text).fill(if active {
                        theme::ACCENT
                    } else {
                        theme::PANEL_2
                    });
                    if ui.add(btn).clicked() {
                        model.tab = tab;
                    }
                }
            });
            ui.add_space(10.0);

            // Footer is laid out first (bottom-up) so the tab body gets
            // whatever height is left and scrolls inside it, instead of
            // pushing the buttons off-window on the Hotkeys tab.
            egui::TopBottomPanel::bottom("settings_footer")
                .frame(
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .inner_margin(egui::Margin::symmetric(18, 12)),
                )
                .show_inside(ui, |ui| {
                    draw_footer(ui, model, &mut outcome);
                });

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| match model.tab {
                    SettingsTab::Provider => draw_provider_tab(ui, model, &mut outcome),
                    SettingsTab::Languages => draw_languages_tab(ui, model),
                    SettingsTab::Hotkeys => draw_hotkeys_tab(ui, model),
                    SettingsTab::Behavior => draw_behavior_tab(ui, model),
                });
        });

    outcome
}

// -----------------------------------------------------------------------
// Tabs
// -----------------------------------------------------------------------

fn draw_provider_tab(
    ui: &mut egui::Ui,
    model: &mut SettingsModel,
    outcome: &mut Option<SettingsOutcome>,
) {
    section_label(ui, "TRANSLATION PROVIDER");

    let providers = crate::ui::setup::providers();
    let current_label = providers
        .iter()
        .find(|(id, _, _)| *id == model.cfg.provider.kind)
        .map(|(_, label, _)| *label)
        .unwrap_or("(custom)");
    field_row(ui, "Type", |ui| {
        egui::ComboBox::from_id_salt("provider_kind")
            .selected_text(current_label)
            .width(260.0)
            .show_ui(ui, |ui| {
                for (id, label, _env_var) in &providers {
                    let mut selected = model.cfg.provider.kind.clone();
                    if ui
                        .selectable_value(&mut selected, (*id).to_string(), *label)
                        .clicked()
                        && selected != model.cfg.provider.kind
                    {
                        // Switching provider carries its defaults with
                        // it — same courtesy the setup wizard extends,
                        // and it keeps `Config::normalize` from having
                        // to guess on the next load.
                        let profile = crate::llm::profiles::provider_profile(id)
                            .expect("settings providers come from provider profiles");
                        model.cfg.provider.kind = selected;
                        model.cfg.provider.model = profile.default_model.to_string();
                        model.cfg.provider.base_url = profile.default_base_url.to_string();
                        model.cfg.provider.api_key.account = profile.account.to_string();
                        model.cfg.provider.api_key.env_var = profile.env_var.to_string();
                        model.provider_origin_change_confirmed = false;
                    }
                }
            });
    });

    field_row(ui, "Model", |ui| {
        ui.add(
            TextEdit::singleline(&mut model.cfg.provider.model)
                .desired_width(300.0)
                .hint_text("model id"),
        );
    });
    field_row(ui, "Base URL", |ui| {
        if ui
            .add(
                TextEdit::singleline(&mut model.cfg.provider.base_url)
                    .desired_width(340.0)
                    .hint_text("https://…"),
            )
            .changed()
        {
            model.provider_origin_change_confirmed = false;
        }
    });
    if provider_origin_changed(model) {
        ui.checkbox(
            &mut model.provider_origin_change_confirmed,
            "I confirm sending API credentials and text to this new provider origin",
        );
    }
    field_row(ui, "Timeout", |ui| {
        ui.add(
            egui::DragValue::new(&mut model.cfg.provider.timeout_seconds)
                .range(1..=600)
                .suffix(" s"),
        );
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
    section_label(ui, "API KEY");

    field_row(ui, "Stored in", |ui| {
        egui::ComboBox::from_id_salt("key_storage")
            .selected_text(model.key_storage.label())
            .width(260.0)
            .show_ui(ui, |ui| {
                ui.add_enabled_ui(model.keychain_available, |ui| {
                    ui.selectable_value(
                        &mut model.key_storage,
                        KeyStorage::Keychain,
                        KeyStorage::Keychain.label(),
                    );
                });
                ui.selectable_value(
                    &mut model.key_storage,
                    KeyStorage::Env,
                    KeyStorage::Env.label(),
                );
                ui.add_enabled_ui(crate::platform::secure_file_storage_supported(), |ui| {
                    ui.selectable_value(
                        &mut model.key_storage,
                        KeyStorage::File,
                        KeyStorage::File.label(),
                    );
                });
            });
    });
    if !model.keychain_available && model.key_storage == KeyStorage::Keychain {
        hint(
            ui,
            "Keychain is unreachable on this system — pick another store.",
        );
    }
    if !crate::platform::secure_file_storage_supported() && model.key_storage == KeyStorage::File {
        warn_line(
            ui,
            "Key-file storage is unavailable on this platform — choose the keychain or an environment variable before saving.",
        );
    }
    match model.key_storage {
        KeyStorage::Env => {
            field_row(ui, "Variable", |ui| {
                ui.add(
                    TextEdit::singleline(&mut model.cfg.provider.api_key.env_var)
                        .desired_width(260.0),
                );
            });
            hint(
                ui,
                "clipt9n reads this variable at launch; export it in your shell profile.",
            );
        }
        KeyStorage::File => {
            field_row(ui, "Path", |ui| {
                ui.add(
                    TextEdit::singleline(&mut model.cfg.provider.api_key.path)
                        .desired_width(340.0)
                        .hint_text("<config dir>/api-key"),
                );
            });
            hint(
                ui,
                "Plain text at 0600. Leave blank for <config dir>/api-key.",
            );
        }
        KeyStorage::Keychain => {
            field_row(ui, "Account", |ui| {
                ui.add(
                    TextEdit::singleline(&mut model.cfg.provider.api_key.account)
                        .desired_width(260.0),
                );
            });
        }
    }

    // The key field is write-only: we never render an existing secret,
    // only accept a replacement.
    field_row(ui, "New key", |ui| {
        let key_str: &mut String = &mut model.api_key;
        ui.add(
            TextEdit::singleline(key_str)
                .password(!model.show_key)
                .desired_width(300.0)
                .hint_text(if model.has_stored_key {
                    "leave blank to keep current key"
                } else {
                    "sk-…"
                }),
        );
        let toggle_label = if model.show_key { "hide" } else { "show" };
        let hover = if model.show_key {
            "Hide key (mask as password)"
        } else {
            "Show key (reveal as plain text)"
        };
        let resp = ui.button(RichText::new(toggle_label).monospace().size(11.0));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hover));
        let clicked = resp.clicked();
        resp.on_hover_text(hover);
        if clicked {
            model.show_key = !model.show_key;
        }
    });

    if model.has_stored_key {
        hint(ui, "A key is already stored for the current settings.");
    } else {
        warn_line(
            ui,
            "No key resolves for the current settings — enter one before saving.",
        );
    }
    if ui
        .add(egui::Link::new(
            RichText::new("Get your API key from the provider dashboard")
                .color(theme::ACCENT)
                .size(11.5),
        ))
        .clicked()
    {
        *outcome = Some(SettingsOutcome::OpenProviderKeyUrl(
            crate::ui::setup::provider_key_url(&model.cfg.provider.kind),
        ));
    }
    hint(
        ui,
        "Saving re-checks the key by rebuilding the provider — it does not call the API.",
    );
}

fn draw_languages_tab(ui: &mut egui::Ui, model: &mut SettingsModel) {
    section_label(ui, "LANGUAGE SLOTS");
    hint(
        ui,
        "Slots 1–5 are the prompt window's translate targets. Slot 6 is always the custom instruction.",
    );
    ui.add_space(6.0);

    let slots: [(&str, &mut crate::config::LanguageSlot); 5] = [
        ("Slot 1", &mut model.cfg.languages.slot_1),
        ("Slot 2", &mut model.cfg.languages.slot_2),
        ("Slot 3", &mut model.cfg.languages.slot_3),
        ("Slot 4", &mut model.cfg.languages.slot_4),
        ("Slot 5", &mut model.cfg.languages.slot_5),
    ];
    for (name, slot) in slots {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(name)
                    .color(theme::INK_3)
                    .monospace()
                    .size(11.5),
            );
            ui.add_space(6.0);
            ui.add(
                TextEdit::singleline(&mut slot.label)
                    .desired_width(240.0)
                    .hint_text("label shown in the prompt"),
            );
            ui.add_space(6.0);
            ui.add(
                TextEdit::singleline(&mut slot.code)
                    .desired_width(70.0)
                    .hint_text("code"),
            );
        });
        ui.add_space(4.0);
    }
    ui.add_space(6.0);
    hint(
        ui,
        "The code is the ISO language passed to the model. Two slots may share a code (e.g. a formal variant); the label is what steers the tone.",
    );
}

fn draw_hotkeys_tab(ui: &mut egui::Ui, model: &mut SettingsModel) {
    for problem in unsupported_hotkey_keys(&model.cfg) {
        warn_line(ui, &problem);
    }
    for conflict in hotkey_conflicts(&model.cfg) {
        warn_line(ui, &format!("Conflict: {conflict}"));
    }

    hotkey_group(
        ui,
        "PROMPT WINDOW",
        "Opens the action picker for the current clipboard.",
        &mut model.cfg.hotkey.enabled,
        &mut model.cfg.hotkey.modifier,
        &mut model.cfg.hotkey.option,
        &mut model.cfg.hotkey.shift,
        &mut model.cfg.hotkey.key,
        "prompt",
    );
    hotkey_group(
        ui,
        "HISTORY VIEWER",
        "Opens the encrypted translation history.",
        &mut model.cfg.hotkey.history.enabled,
        &mut model.cfg.hotkey.history.modifier,
        &mut model.cfg.hotkey.history.option,
        &mut model.cfg.hotkey.history.shift,
        &mut model.cfg.hotkey.history.key,
        "history",
    );
    hotkey_group(
        ui,
        "TRANSLATE SELECTION",
        "Copies the current selection first, then opens the picker.",
        &mut model.cfg.hotkey.selection.enabled,
        &mut model.cfg.hotkey.selection.modifier,
        &mut model.cfg.hotkey.selection.option,
        &mut model.cfg.hotkey.selection.shift,
        &mut model.cfg.hotkey.selection.key,
        "selection",
    );
    field_row(ui, "Copy delay", |ui| {
        ui.add(
            egui::DragValue::new(&mut model.cfg.hotkey.selection.copy_delay_ms)
                .range(0..=2000)
                .suffix(" ms"),
        );
    });

    hotkey_group(
        ui,
        "REPLACE INLINE",
        "Translates the selection in the background and pastes it back.",
        &mut model.cfg.hotkey.replace.enabled,
        &mut model.cfg.hotkey.replace.modifier,
        &mut model.cfg.hotkey.replace.option,
        &mut model.cfg.hotkey.replace.shift,
        &mut model.cfg.hotkey.replace.key,
        "replace",
    );
    field_row(ui, "Copy delay", |ui| {
        ui.add(
            egui::DragValue::new(&mut model.cfg.hotkey.replace.copy_delay_ms)
                .range(0..=2000)
                .suffix(" ms"),
        );
    });
    field_row(ui, "Target slot", |ui| {
        ui.add(egui::DragValue::new(&mut model.cfg.hotkey.replace.default_slot).range(1..=5));
        let label = slot_label(&model.cfg, model.cfg.hotkey.replace.default_slot);
        ui.label(RichText::new(label).color(theme::INK_3).size(11.5));
    });

    ui.add_space(8.0);
    hint(
        ui,
        "The key must be a single uppercase letter, A–Z. Function and named keys are not supported.",
    );
}

fn draw_behavior_tab(ui: &mut egui::Ui, model: &mut SettingsModel) {
    section_label(ui, "PROMPT WINDOW");
    field_row(ui, "Density", |ui| {
        let current = if model.cfg.ui.density == "compact" {
            "Compact"
        } else {
            "Normal"
        };
        egui::ComboBox::from_id_salt("ui_density")
            .selected_text(current)
            .width(180.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut model.cfg.ui.density, "normal".into(), "Normal");
                ui.selectable_value(&mut model.cfg.ui.density, "compact".into(), "Compact");
            });
    });
    ui.checkbox(&mut model.cfg.ui.show_preview, "Show clipboard preview");
    field_row(ui, "Confirm above", |ui| {
        ui.add(
            egui::DragValue::new(&mut model.cfg.ui.confirm_size_threshold)
                .range(0..=100_000)
                .suffix(" chars"),
        );
    });
    hint(
        ui,
        "Longer clipboards get a confirmation step before anything is sent to the provider.",
    );

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
    section_label(ui, "GLOSSARY");
    ui.checkbox(&mut model.cfg.glossary.enabled, "Apply glossary terms");
    field_row(ui, "File", |ui| {
        ui.add(
            TextEdit::singleline(&mut model.cfg.glossary.file)
                .desired_width(260.0)
                .hint_text("glossary.toml"),
        );
    });
    hint(ui, "Relative to the config folder.");
    ui.checkbox(
        &mut model.cfg.glossary.case_sensitive,
        "Case-sensitive matching",
    );
    field_row(ui, "Matching", |ui| {
        egui::ComboBox::from_id_salt("glossary_matching")
            .selected_text(model.cfg.glossary.matching.clone())
            .width(180.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut model.cfg.glossary.matching, "auto".into(), "auto");
                ui.selectable_value(
                    &mut model.cfg.glossary.matching,
                    "word_boundary".into(),
                    "word_boundary",
                );
                ui.selectable_value(
                    &mut model.cfg.glossary.matching,
                    "substring".into(),
                    "substring",
                );
            });
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
    section_label(ui, "HISTORY");
    ui.checkbox(&mut model.cfg.history.enabled, "Keep encrypted history");
    field_row(ui, "Keep", |ui| {
        ui.add(
            egui::DragValue::new(&mut model.cfg.history.max_entries)
                .range(1..=10_000)
                .suffix(" entries"),
        );
    });
    ui.checkbox(
        &mut model.cfg.history.store_text,
        "Store the source and result text",
    );
    hint(
        ui,
        "Off keeps metadata only — timestamps, languages, and action, with no text.",
    );
    ui.checkbox(
        &mut model.cfg.history.confirm_clear,
        "Confirm before clearing all history",
    );

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);
    section_label(ui, "PROMPT TEMPLATES");
    hint(
        ui,
        "Template overrides are files under the config folder and are compiled at startup. Use \"Open config.toml\" below to reach them.",
    );
}

// -----------------------------------------------------------------------
// Shared widgets
// -----------------------------------------------------------------------

fn draw_footer(ui: &mut egui::Ui, model: &SettingsModel, outcome: &mut Option<SettingsOutcome>) {
    if !model.err_msg.is_empty() {
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 118, 118, 20))
            .stroke(Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(255, 118, 118, 64),
            ))
            .corner_radius(6.0)
            .inner_margin(9.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&model.err_msg)
                            .color(theme::BAD)
                            .monospace()
                            .size(11.5),
                    )
                    .wrap(),
                );
            });
        ui.add_space(6.0);
    }

    let restart = restart_required(model);
    if !restart.is_empty() {
        ui.label(
            RichText::new(format!(
                "Takes effect on next launch: {}.",
                restart.join(", ")
            ))
            .color(theme::WARN)
            .size(11.5),
        );
        ui.add_space(4.0);
    }

    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            *outcome = Some(SettingsOutcome::Cancel);
        }
        if ui
            .button(RichText::new("Open config.toml").size(12.0))
            .clicked()
        {
            *outcome = Some(SettingsOutcome::OpenConfigFile);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled = save_enabled(model);
            let btn = egui::Button::new(RichText::new("Save").color(theme::ACCENT_INK).strong())
                .fill(if enabled {
                    theme::ACCENT
                } else {
                    theme::PANEL_3
                });
            if ui.add_enabled(enabled, btn).clicked() {
                *outcome = Some(SettingsOutcome::Save);
            }
            ui.label(RichText::new("Esc discards").color(theme::INK_3).size(11.0));
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn hotkey_group(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    enabled: &mut bool,
    modifier: &mut String,
    option: &mut bool,
    shift: &mut bool,
    key: &mut String,
    id_salt: &str,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .color(theme::INK_3)
                .monospace()
                .size(10.0)
                .strong(),
        );
        ui.add_space(8.0);
        let preview = if *enabled {
            hotkey_combo_display(modifier, *option, *shift, key)
        } else {
            "(disabled)".to_string()
        };
        ui.label(
            RichText::new(preview)
                .color(if *enabled {
                    theme::ACCENT
                } else {
                    theme::INK_3
                })
                .monospace()
                .size(11.5),
        );
    });
    ui.label(RichText::new(description).color(theme::INK_3).size(11.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(enabled, "Enabled");
        ui.add_space(8.0);
        ui.add_enabled_ui(*enabled, |ui| {
            egui::ComboBox::from_id_salt(format!("{id_salt}_modifier"))
                .selected_text(modifier.clone())
                .width(90.0)
                .show_ui(ui, |ui| {
                    for m in ["cmd", "ctrl", "alt", "super"] {
                        ui.selectable_value(modifier, m.to_string(), m);
                    }
                });
            ui.checkbox(option, "Option");
            ui.checkbox(shift, "Shift");
            ui.add(
                TextEdit::singleline(key)
                    .desired_width(52.0)
                    .hint_text("key"),
            );
        });
    });
    ui.add_space(2.0);
}

/// A labeled row: fixed-width caption on the left, widget on the right.
fn field_row(ui: &mut egui::Ui, label: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized(
            Vec2::new(96.0, 18.0),
            egui::Label::new(RichText::new(label).color(theme::INK_2).size(12.0)),
        );
        add_contents(ui);
    });
    ui.add_space(4.0);
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .color(theme::INK_3)
            .monospace()
            .size(10.0)
            .strong(),
    );
    ui.add_space(6.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(theme::INK_3).size(11.0)).wrap());
    ui.add_space(4.0);
}

fn warn_line(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(theme::WARN).size(11.5)).wrap());
    ui.add_space(4.0);
}

/// Human-readable name of a language slot, for the replace-hotkey's
/// target-slot spinner. Out-of-range indices can't come from the
/// spinner (it clamps 1..=5) but the config file can hold anything.
fn slot_label(cfg: &Config, slot: u8) -> String {
    match slot {
        1 => cfg.languages.slot_1.label.clone(),
        2 => cfg.languages.slot_2.label.clone(),
        3 => cfg.languages.slot_3.label.clone(),
        4 => cfg.languages.slot_4.label.clone(),
        5 => cfg.languages.slot_5.label.clone(),
        other => format!("(no slot {other})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with(cfg: Config) -> SettingsModel {
        SettingsModel {
            original: cfg.clone(),
            cfg,
            ..Default::default()
        }
    }

    #[test]
    fn key_storage_round_trips_through_source_strings() {
        for storage in [KeyStorage::Keychain, KeyStorage::Env, KeyStorage::File] {
            assert_eq!(KeyStorage::from_source(storage.as_source()), storage);
        }
    }

    #[test]
    fn key_storage_maps_prompt_source_to_env() {
        assert_eq!(KeyStorage::from_source("prompt"), KeyStorage::Env);
    }

    #[test]
    fn restart_required_is_empty_for_untouched_config() {
        let model = model_with(Config::default());
        assert!(restart_required(&model).is_empty());
    }

    #[test]
    fn restart_required_flags_hotkey_edits() {
        let mut model = model_with(Config::default());
        model.cfg.hotkey.key = "J".into();
        assert_eq!(restart_required(&model), vec!["hotkey changes"]);
    }

    #[test]
    fn restart_required_flags_history_toggle_but_not_its_tuning() {
        let mut model = model_with(Config::default());
        model.cfg.history.max_entries = 500;
        assert!(
            restart_required(&model).is_empty(),
            "max_entries is read per-use, so it applies live"
        );
        model.cfg.history.enabled = false;
        assert_eq!(restart_required(&model), vec!["enabling/disabling history"]);
    }

    #[test]
    fn default_hotkeys_do_not_conflict() {
        assert!(hotkey_conflicts(&Config::default()).is_empty());
    }

    #[test]
    fn identical_enabled_hotkeys_conflict() {
        let mut cfg = Config::default();
        cfg.hotkey.history.modifier = cfg.hotkey.modifier.clone();
        cfg.hotkey.history.option = cfg.hotkey.option;
        cfg.hotkey.history.shift = cfg.hotkey.shift;
        cfg.hotkey.history.key = cfg.hotkey.key.clone();
        let conflicts = hotkey_conflicts(&cfg);
        assert_eq!(conflicts.len(), 1, "conflicts: {conflicts:?}");
        assert!(conflicts[0].contains("Prompt and History"));
    }

    #[test]
    fn disabled_hotkeys_never_conflict() {
        let mut cfg = Config::default();
        cfg.hotkey.history.modifier = cfg.hotkey.modifier.clone();
        cfg.hotkey.history.option = cfg.hotkey.option;
        cfg.hotkey.history.key = cfg.hotkey.key.clone();
        cfg.hotkey.history.enabled = false;
        assert!(hotkey_conflicts(&cfg).is_empty());
    }

    #[test]
    fn unsupported_file_storage_blocks_save_until_user_selects_a_supported_store() {
        let mut model = model_with(Config::default());
        model.key_storage = KeyStorage::File;
        assert!(!save_enabled_with_file_storage(&model, false));

        model.key_storage = KeyStorage::Env;
        assert!(save_enabled_with_file_storage(&model, false));
    }

    #[test]
    fn supported_file_storage_can_be_saved() {
        let mut model = model_with(Config::default());
        model.key_storage = KeyStorage::File;
        assert!(save_enabled_with_file_storage(&model, true));
    }

    #[test]
    fn provider_origin_change_requires_dedicated_confirmation() {
        let mut model = model_with(Config::default());
        model.cfg.provider.base_url = "https://proxy.example.com/v1".into();

        assert!(provider_origin_changed(&model));
        assert!(!save_enabled(&model));

        model.provider_origin_change_confirmed = true;
        assert!(save_enabled(&model));
    }

    #[test]
    fn provider_path_change_on_same_origin_needs_no_confirmation() {
        let mut model = model_with(Config::default());
        model.cfg.provider.base_url = "https://api.anthropic.com/alternate".into();

        assert!(!provider_origin_changed(&model));
        assert!(save_enabled(&model));
    }

    #[test]
    fn save_is_blocked_while_hotkeys_conflict() {
        let mut model = model_with(Config::default());
        assert!(save_enabled(&model));
        model.cfg.hotkey.history.modifier = model.cfg.hotkey.modifier.clone();
        model.cfg.hotkey.history.option = model.cfg.hotkey.option;
        model.cfg.hotkey.history.shift = model.cfg.hotkey.shift;
        model.cfg.hotkey.history.key = model.cfg.hotkey.key.clone();
        assert!(!save_enabled(&model));
    }

    #[test]
    fn default_hotkey_keys_are_all_registerable() {
        assert!(unsupported_hotkey_keys(&Config::default()).is_empty());
    }

    #[test]
    fn unregisterable_key_names_are_reported_and_block_save() {
        let mut model = model_with(Config::default());
        // The two names the old hint text advertised. Saving either
        // meant the next launch came up with no window and no tray.
        model.cfg.hotkey.key = "F5".into();
        model.cfg.hotkey.history.key = "Space".into();
        let problems = unsupported_hotkey_keys(&model.cfg);
        assert_eq!(problems.len(), 2, "problems: {problems:?}");
        assert!(problems[0].contains("Prompt key \"F5\""));
        assert!(problems[1].contains("History key \"Space\""));
        assert!(!save_enabled(&model));
    }

    #[test]
    fn lowercase_key_is_rejected() {
        let mut model = model_with(Config::default());
        model.cfg.hotkey.key = "t".into();
        assert!(!save_enabled(&model));
    }

    #[test]
    fn slot_label_resolves_configured_slots() {
        let cfg = Config::default();
        assert_eq!(slot_label(&cfg, 1), "English");
        assert_eq!(slot_label(&cfg, 5), "Türkçe (resmî)");
        assert_eq!(slot_label(&cfg, 9), "(no slot 9)");
    }

    #[test]
    fn tabs_cover_every_variant_once() {
        let tabs = SettingsTab::all();
        let mut labels: Vec<&str> = tabs.iter().map(|t| t.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), tabs.len());
    }
}
