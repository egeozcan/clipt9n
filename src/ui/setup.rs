//! Setup wizard view — egui paint of the design's `setup-wizard.jsx`.
//! Pure view + small pure helpers. Connectivity check + sample-
//! translation orchestration live in `src/app.rs::update_setup_wizard`
//! (Task 9); this module emits intents (`SetupOutcome`) and the App
//! flips the `check1` / `check2` / `phase` / `err_msg` model fields in
//! response to channel results.

use egui::{Color32, RichText, Stroke, TextEdit, Vec2};
use zeroize::Zeroizing;

use crate::ui::theme;

/// What the wizard paints per frame. Mirrors `setup-wizard.jsx`'s
/// React state hooks: provider/key/show/storage/testRequested/phase/
/// check1/check2/errMsg.
#[derive(Debug, Clone, Default)]
pub struct SetupWizardModel {
    /// One of "anthropic" | "openai" | "gemini" | "ollama" | "deepseek".
    /// Default "anthropic" per design.
    pub provider: String,
    /// API key in flight. Wrapped in `Zeroizing` from the moment the
    /// user types it.
    pub key: Zeroizing<String>,
    /// Toggle between password and visible-text rendering.
    pub show_key: bool,
    /// "Keychain" (default) or "Env". When `keychain_available ==
    /// false`, this is forced to `Env` and the radio is hidden.
    pub storage: Storage,
    /// "Test with a real translation" checkbox. Default true per
    /// design. When false, only the connectivity check runs.
    pub test_translation: bool,
    /// State machine: Entry → Verifying → Done | Error → (Save) →
    /// Idle (the App-layer transition).
    pub phase: WizardPhase,
    /// Connectivity check status.
    pub check1: CheckStatus,
    /// Sample-translation check status.
    pub check2: CheckStatus,
    /// User-facing error string for the err-box. Empty unless
    /// `phase == Error`.
    pub err_msg: String,
    /// Cached at construction; the wizard hides the Keychain radio if
    /// false. The probe runs in `KeychainSecrets::keychain_available`.
    pub keychain_available: bool,
    /// Identifies the currently active verification run. Results from
    /// older runs (including a cancelled/reopened wizard) are ignored.
    pub verification_id: VerificationId,
    /// Key captured for the active verification run. Environment-backed
    /// verification resolves into this field without populating the typed
    /// key input, so Save can still enforce read-only env storage.
    #[doc(hidden)]
    pub verification_key: Zeroizing<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Storage {
    #[default]
    Keychain,
    Env,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WizardPhase {
    #[default]
    Entry,
    Verifying,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckStatus {
    #[default]
    Idle,
    Running,
    Ok,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupOutcome {
    /// "Cancel" button — abandon the wizard.
    Cancel,
    /// "Verify →" button — kick off check1 + (optional) check2.
    Verify,
    /// "Save and start ✓" button (only enabled when phase=Done).
    SaveAndStart,
    /// Error-recovery "Open config" button.
    OpenConfig,
    /// "Get your API key" link — open the provider's dashboard.
    OpenProviderKeyUrl(&'static str),
}

/// Provider-dashboard URL for the "Get your API key" link. Falls back
/// to Anthropic for unknown ids (defensive — should never trigger via
/// the wizard's own provider grid).
pub fn provider_key_url(provider_kind: &str) -> &'static str {
    match provider_kind {
        "anthropic" => "https://console.anthropic.com/settings/keys",
        "openai" => "https://platform.openai.com/api-keys",
        "gemini" => "https://aistudio.google.com/app/apikey",
        "ollama" => "https://ollama.com/download",
        "deepseek" => "https://platform.deepseek.com/api_keys",
        _ => "https://console.anthropic.com/settings/keys",
    }
}

/// Default setup-wizard viewport size. Matches design's 580×640.
pub const SETUP_WIZARD_INNER_SIZE: Vec2 = Vec2::new(580.0, 640.0);

/// All four providers per design. The label is what the wizard shows;
/// `default_env_var` is the hint string under the Env-storage radio
/// (e.g., `$ANTHROPIC_API_KEY`).
pub fn providers() -> Vec<(&'static str, &'static str, &'static str)> {
    const LABELS: [&str; 5] = [
        "Anthropic (Claude)",
        "OpenAI",
        "Google Gemini",
        "Ollama (local)",
        "DeepSeek",
    ];
    crate::llm::profiles::PROVIDER_PROFILES
        .iter()
        .zip(LABELS)
        .map(|(profile, label)| (profile.id, label, profile.env_var))
        .collect()
}

/// Look up the provider tuple by id. Returns `("anthropic", ..., ...)`
/// for unknown ids (defensive — should never happen in normal flow).
pub fn provider_meta(id: &str) -> (&'static str, &'static str, &'static str) {
    providers()
        .into_iter()
        .find(|(p, _, _)| *p == id)
        .unwrap_or(providers()[0])
}

/// Default base URL for each provider. Used by the wizard's
/// sample-translation spawn (Task 9) to construct a fresh provider
/// from the user's selection.
pub fn default_base_url(provider_kind: &str) -> &'static str {
    crate::llm::profiles::provider_profile(provider_kind)
        .or_else(|_| crate::llm::profiles::provider_profile("anthropic"))
        .expect("anthropic provider profile")
        .default_base_url
}

/// Default model for each provider. Used by `persist_setup_completion`
/// to auto-select a sensible model when the user switches provider
/// in the setup wizard.
pub fn default_model(provider_kind: &str) -> &'static str {
    crate::llm::profiles::provider_profile(provider_kind)
        .or_else(|_| crate::llm::profiles::provider_profile("anthropic"))
        .expect("anthropic provider profile")
        .default_model
}

/// Monotonic identifier scoped to one setup verification run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerificationId(pub u64);

/// Which check produced a result. The App receives this in a channel
/// and flips the corresponding `check1` / `check2` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupCheck {
    Connectivity,
    SampleTranslation,
}

/// Outcome of a single check. `Ok(())` flips the corresponding row to
/// `CheckStatus::Ok`; `Err(msg)` flips to `Fail` and stores the
/// message in `model.err_msg`.
pub type SetupCheckResult = (VerificationId, SetupCheck, Result<(), String>);

/// Construct the connectivity-check URL + auth header set for the
/// configured provider. Returns (url, auth_kind) where auth_kind is
/// either ("Authorization", "Bearer ...") for OpenAI-compat or
/// ("x-api-key", "...") + ("anthropic-version", "2023-06-01") for
/// Anthropic.
pub fn connectivity_request(
    provider: &str,
    base_url: &str,
    key: &str,
) -> (String, Vec<(String, String)>) {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let headers: Vec<(String, String)> = match provider {
        "anthropic" => vec![
            ("x-api-key".into(), key.to_string()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ],
        // openai / gemini / ollama all use Bearer auth on /v1/models
        _ => vec![("Authorization".into(), format!("Bearer {}", key))],
    };
    (url, headers)
}

/// Whether the Save-and-start button is enabled. Mirrors the jsx's
/// `phase === "done"` gate.
pub fn save_enabled(model: &SetupWizardModel) -> bool {
    matches!(model.phase, WizardPhase::Done)
}

/// Whether the Verify button is enabled. Mirrors `!key || phase ==
/// "verifying"` from jsx (negated).
pub fn verify_enabled(model: &SetupWizardModel) -> bool {
    (!model.key.is_empty() || model.storage == Storage::Env)
        && !matches!(model.phase, WizardPhase::Verifying)
}

/// Paint the wizard. Returns at most one outcome per frame.
pub fn draw(ctx: &egui::Context, model: &mut SetupWizardModel) -> Option<SetupOutcome> {
    let mut outcome: Option<SetupOutcome> = None;
    let frame = egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(20.0));

    frame.show(ctx, |ui| {
        ui.set_max_width(540.0); // 580px outer - 2 × 20px margin
                                 // Wrap entire wizard in a ScrollArea so long content (e.g.
                                 // provider error messages) doesn't get clipped by the fixed
                                 // viewport height.
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                // Header.
                ui.label(
                    RichText::new("Welcome to clipt9n")
                        .color(theme::INK)
                        .strong()
                        .size(15.0),
                );
                ui.label(
                    RichText::new("first-run · setup")
                        .color(theme::INK_3)
                        .monospace()
                        .size(11.0),
                );
                ui.add_space(14.0);

                // Step 1: provider grid.
                ui.label(
                    RichText::new("STEP 1 OF 3 · PROVIDER")
                        .color(theme::INK_3)
                        .monospace()
                        .size(10.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Pick your translation provider.")
                        .color(theme::INK)
                        .strong()
                        .size(13.5),
                );
                ui.add_space(8.0);

                let provs = providers();
                ui.columns(2, |cols| {
                    for (i, (id, label, _env_var)) in provs.iter().enumerate() {
                        let col = &mut cols[i % 2];
                        let active = model.provider == *id;
                        let bg = if active {
                            Color32::from_rgba_unmultiplied(200, 255, 94, 16)
                        } else {
                            theme::PANEL_2
                        };
                        let stroke = if active {
                            Stroke::new(1.0_f32, theme::ACCENT)
                        } else {
                            Stroke::new(1.0_f32, theme::LINE_SOFT)
                        };
                        let resp = egui::Frame::new()
                            .fill(bg)
                            .stroke(stroke)
                            .corner_radius(6.0)
                            .inner_margin(9.0)
                            .show(col, |ui| {
                                ui.horizontal(|ui| {
                                    let dot_color =
                                        if active { theme::ACCENT } else { theme::INK_3 };
                                    ui.label(RichText::new("●").color(dot_color).size(10.0));
                                    ui.add_space(8.0);
                                    ui.label(RichText::new(*label).color(theme::INK).size(12.5));
                                    if *id == "anthropic" {
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new("recommended")
                                                .color(theme::ACCENT)
                                                .monospace()
                                                .size(10.0),
                                        );
                                    }
                                    if *id == "ollama" {
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new("offline")
                                                .color(Color32::from_rgb(0x9a, 0xd6, 0xff))
                                                .monospace()
                                                .size(10.0),
                                        );
                                    }
                                });
                            })
                            .response
                            .interact(egui::Sense::click());
                        // AccessKit: expose the card as Role::Button with the provider
                        // name as its label so screen readers and kittest can find each
                        // card by name. The Frame+interact pattern does not auto-derive
                        // a button role, so we provide the WidgetInfo explicitly.
                        let card_label = label.to_string();
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &card_label)
                        });
                        if resp.clicked() {
                            model.provider = (*id).to_string();
                            if matches!(model.phase, WizardPhase::Error) {
                                // jsx: `if (phase !== "entry") setPhase("entry")`
                                model.phase = WizardPhase::Entry;
                                model.err_msg.clear();
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                if ui
                    .link("Get your API key from the provider dashboard")
                    .clicked()
                {
                    outcome = Some(SetupOutcome::OpenProviderKeyUrl(provider_key_url(
                        &model.provider,
                    )));
                }
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);

                // Step 2: key entry.
                ui.label(
                    RichText::new("STEP 2 · KEY")
                        .color(theme::INK_3)
                        .monospace()
                        .size(10.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    // The TextEdit needs to mutate the underlying String. We
                    // get a `&mut String` from `Deref<Target=String>` —
                    // egui's TextEdit accepts that.
                    let key_str: &mut String = &mut model.key;
                    let edit = TextEdit::singleline(key_str)
                        .password(!model.show_key)
                        .hint_text("sk-ant-…")
                        .desired_width(ui.available_width() - 80.0);
                    let resp = ui.add(edit);
                    if resp.changed() && matches!(model.phase, WizardPhase::Error) {
                        model.phase = WizardPhase::Entry;
                        model.err_msg.clear();
                    }
                    ui.add_space(4.0);
                    let toggle_label = if model.show_key { "hide" } else { "show" };
                    let hover_text = if model.show_key {
                        "Hide key (mask as password)"
                    } else {
                        "Show key (reveal as plain text)"
                    };
                    // AccessKit: use the descriptive hover_text as the widget label so
                    // screen readers announce the button's purpose ("Show key (reveal as
                    // plain text)" / "Hide key (mask as password)") rather than the
                    // short visible toggle token. The visible button text stays "show"/
                    // "hide" for sighted users; the AccessKit label and the tooltip
                    // both surface the descriptive form.
                    // `on_hover_text` takes self, so we check clicked() before calling it.
                    let toggle_resp = ui.button(RichText::new(toggle_label).monospace().size(11.0));
                    toggle_resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hover_text)
                    });
                    let key_toggled = toggle_resp.clicked();
                    toggle_resp.on_hover_text(hover_text);
                    if key_toggled {
                        model.show_key = !model.show_key;
                    }
                });

                // Storage radio.
                ui.add_space(8.0);
                let (_, _, env_var) = provider_meta(&model.provider);
                ui.columns(2, |cols| {
                    // Keychain option (only shown if available).
                    if model.keychain_available {
                        let active = matches!(model.storage, Storage::Keychain);
                        let stroke = if active {
                            Stroke::new(1.0_f32, theme::ACCENT)
                        } else {
                            Stroke::new(1.0_f32, theme::LINE_SOFT)
                        };
                        let resp = egui::Frame::new()
                            .fill(theme::PANEL_2)
                            .stroke(stroke)
                            .corner_radius(6.0)
                            .inner_margin(8.0)
                            .show(&mut cols[0], |ui| {
                                ui.label(
                                    RichText::new("System Keychain")
                                        .color(theme::INK)
                                        .size(12.5)
                                        .strong(),
                                );
                                ui.label(
                                    RichText::new("Bound to clipt9n; other apps prompted on read.")
                                        .color(theme::INK_3)
                                        .size(11.0),
                                );
                            })
                            .response
                            .interact(egui::Sense::click());
                        if resp.clicked() {
                            model.storage = Storage::Keychain;
                        }
                    } else {
                        cols[0].label(
                            RichText::new("(Keychain unavailable on this system)")
                                .color(theme::INK_3)
                                .size(11.5),
                        );
                    }
                    // Env option (always shown).
                    let active = matches!(model.storage, Storage::Env);
                    let stroke = if active {
                        Stroke::new(1.0_f32, theme::ACCENT)
                    } else {
                        Stroke::new(1.0_f32, theme::LINE_SOFT)
                    };
                    let resp = egui::Frame::new()
                        .fill(theme::PANEL_2)
                        .stroke(stroke)
                        .corner_radius(6.0)
                        .inner_margin(8.0)
                        .show(&mut cols[1], |ui| {
                            ui.label(
                                RichText::new("Environment variable")
                                    .color(theme::INK)
                                    .size(12.5)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new(format!("${env_var}"))
                                    .color(theme::INK_3)
                                    .monospace()
                                    .size(11.0),
                            );
                        })
                        .response
                        .interact(egui::Sense::click());
                    if resp.clicked() {
                        model.storage = Storage::Env;
                    }
                });

                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);

                // Step 3: verify.
                ui.label(
                    RichText::new("STEP 3 · VERIFY")
                        .color(theme::INK_3)
                        .monospace()
                        .size(10.0)
                        .strong(),
                );
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    let mut t = model.test_translation;
                    ui.checkbox(&mut t, "");
                    model.test_translation = t;
                    ui.label(
                        RichText::new("Test with a real translation")
                            .color(theme::INK_2)
                            .size(12.5),
                    );
                    ui.label(
                        RichText::new(" (~$0.0001 in tokens, recommended)")
                            .color(theme::INK_3)
                            .size(11.5),
                    );
                });

                ui.add_space(6.0);
                // Check rows, painted in a panel.
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0_f32, theme::LINE_SOFT))
                    .corner_radius(6.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        draw_check_row(ui, "Connectivity (auth)", "GET /v1/models", model.check1);
                        if model.test_translation {
                            draw_check_row(
                                ui,
                                "Sample translation",
                                "\"Hello, world.\" → \"Hallo, Welt.\"",
                                model.check2,
                            );
                        }
                    });

                // Error box.
                if matches!(model.phase, WizardPhase::Error) && !model.err_msg.is_empty() {
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(255, 118, 118, 20))
                        .stroke(Stroke::new(
                            1.0_f32,
                            Color32::from_rgba_unmultiplied(255, 118, 118, 64),
                        ))
                        .corner_radius(6.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("!")
                                        .color(theme::BAD)
                                        .strong()
                                        .monospace()
                                        .size(13.0),
                                );
                                ui.add_space(6.0);
                                ui.vertical(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&model.err_msg)
                                                .color(theme::BAD)
                                                .strong()
                                                .monospace()
                                                .size(12.5),
                                        )
                                        .wrap(),
                                    );
                                    ui.label(
                                RichText::new(
                                    "Try a different key, or open config.toml to switch provider.",
                                )
                                .color(theme::INK_2)
                                .size(11.0),
                            );
                                    if ui
                                        .button(RichText::new("Open config").monospace().size(11.0))
                                        .clicked()
                                    {
                                        outcome = Some(SetupOutcome::OpenConfig);
                                    }
                                });
                            });
                        });
                }

                // Footer.
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        outcome = Some(SetupOutcome::Cancel);
                    }
                    ui.allocate_space(egui::Vec2::new(ui.available_width() - 180.0, 0.0));
                    if matches!(model.phase, WizardPhase::Done) {
                        let btn = egui::Button::new(
                            RichText::new("Save and start ✓")
                                .color(theme::ACCENT_INK)
                                .strong(),
                        )
                        .fill(theme::GOOD);
                        if ui.add(btn).clicked() {
                            outcome = Some(SetupOutcome::SaveAndStart);
                        }
                    } else {
                        let label = match model.phase {
                            WizardPhase::Verifying => "Verifying…",
                            _ => "Verify →",
                        };
                        let btn = egui::Button::new(
                            RichText::new(label).color(theme::ACCENT_INK).strong(),
                        )
                        .fill(if verify_enabled(model) {
                            theme::ACCENT
                        } else {
                            theme::PANEL_3
                        });
                        let resp = ui.add_enabled(verify_enabled(model), btn);
                        if resp.clicked() {
                            outcome = Some(SetupOutcome::Verify);
                        }
                    }
                });
            }); // ScrollArea
    });

    outcome
}

fn draw_check_row(ui: &mut egui::Ui, label: &str, detail: &str, status: CheckStatus) {
    let (dot, color) = match status {
        CheckStatus::Idle => ("○", theme::INK_3),
        CheckStatus::Running => ("◐", theme::WARN),
        CheckStatus::Ok => ("✓", theme::GOOD),
        CheckStatus::Fail => ("✕", theme::BAD),
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(dot)
                .color(color)
                .monospace()
                .size(13.0)
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(label).color(theme::INK).size(12.5));
        ui.allocate_space(Vec2::new(ui.available_width() - 220.0, 0.0));
        ui.label(
            RichText::new(detail)
                .color(theme::INK_3)
                .monospace()
                .size(11.0),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_returns_five_entries_in_design_order() {
        let p = providers();
        assert_eq!(p.len(), 5);
        assert_eq!(p[0].0, "anthropic");
        assert_eq!(p[1].0, "openai");
        assert_eq!(p[2].0, "gemini");
        assert_eq!(p[3].0, "ollama");
        assert_eq!(p[4].0, "deepseek");
    }

    #[test]
    fn provider_meta_falls_back_to_anthropic_for_unknown() {
        let (id, _, _) = provider_meta("nonexistent");
        assert_eq!(id, "anthropic");
    }

    #[test]
    fn provider_meta_resolves_known_id() {
        let (_, label, env_var) = provider_meta("openai");
        assert_eq!(label, "OpenAI");
        assert_eq!(env_var, "OPENAI_API_KEY");
    }

    #[test]
    fn provider_key_url_routes_to_dashboard_per_provider() {
        assert_eq!(
            provider_key_url("anthropic"),
            "https://console.anthropic.com/settings/keys"
        );
        assert_eq!(
            provider_key_url("openai"),
            "https://platform.openai.com/api-keys"
        );
        assert_eq!(
            provider_key_url("gemini"),
            "https://aistudio.google.com/app/apikey"
        );
        assert_eq!(provider_key_url("ollama"), "https://ollama.com/download");
        assert_eq!(
            provider_key_url("deepseek"),
            "https://platform.deepseek.com/api_keys"
        );
    }

    #[test]
    fn provider_key_url_falls_back_to_anthropic_for_unknown() {
        assert_eq!(
            provider_key_url("nonexistent"),
            "https://console.anthropic.com/settings/keys"
        );
    }

    #[test]
    fn save_enabled_only_when_phase_is_done() {
        let mut m = SetupWizardModel::default();
        assert!(!save_enabled(&m));
        m.phase = WizardPhase::Verifying;
        assert!(!save_enabled(&m));
        m.phase = WizardPhase::Error;
        assert!(!save_enabled(&m));
        m.phase = WizardPhase::Done;
        assert!(save_enabled(&m));
    }

    #[test]
    fn verify_enabled_requires_key_and_not_verifying() {
        let mut m = SetupWizardModel::default();
        assert!(!verify_enabled(&m), "empty key disables verify");
        m.key = Zeroizing::new("sk-test-12345".into());
        assert!(verify_enabled(&m));
        m.phase = WizardPhase::Verifying;
        assert!(!verify_enabled(&m), "verifying disables re-click");
    }

    #[test]
    fn environment_storage_can_verify_without_a_typed_key() {
        let model = SetupWizardModel {
            provider: "openai".into(),
            storage: Storage::Env,
            ..Default::default()
        };
        assert!(verify_enabled(&model));
    }

    #[test]
    fn default_provider_is_anthropic_after_explicit_set() {
        let m = SetupWizardModel {
            provider: "anthropic".into(),
            ..Default::default()
        };
        assert_eq!(m.provider, "anthropic");
    }

    #[test]
    fn connectivity_request_anthropic_uses_x_api_key_and_version_header() {
        let (url, headers) =
            connectivity_request("anthropic", "https://api.anthropic.com/v1", "sk-ant-...");
        assert_eq!(url, "https://api.anthropic.com/v1/models");
        assert!(headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-ant-..."));
        assert!(headers.iter().any(|(k, _)| k == "anthropic-version"));
    }

    #[test]
    fn connectivity_request_openai_uses_bearer_auth() {
        let (url, headers) = connectivity_request("openai", "https://api.openai.com/v1", "sk-test");
        assert_eq!(url, "https://api.openai.com/v1/models");
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-test"));
    }

    #[test]
    fn connectivity_request_deepseek_uses_bearer_auth() {
        let (url, headers) = connectivity_request(
            "deepseek",
            "https://api.deepseek.com/v1",
            "sk-deepseek-test",
        );
        assert_eq!(url, "https://api.deepseek.com/v1/models");
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-deepseek-test"));
    }

    #[test]
    fn default_model_returns_provider_specific_models() {
        assert_eq!(default_model("anthropic"), "claude-haiku-4-5-20251001");
        assert_eq!(default_model("openai"), "gpt-4o-mini");
        assert_eq!(default_model("gemini"), "gemini-2.0-flash");
        assert_eq!(default_model("ollama"), "llama3.2");
        assert_eq!(default_model("deepseek"), "deepseek-v4-flash");
    }

    #[test]
    fn default_model_falls_back_to_anthropic_for_unknown() {
        assert_eq!(default_model("nonexistent"), "claude-haiku-4-5-20251001");
    }

    #[test]
    fn connectivity_request_strips_trailing_slash_from_base_url() {
        let (url, _) = connectivity_request("openai", "https://api.openai.com/v1/", "sk");
        assert_eq!(url, "https://api.openai.com/v1/models");
    }
}
