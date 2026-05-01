//! Hide-icon confirmation modal. Rendered when
//! `AppState::ConfirmingTrayHide` is active. On Confirm, the App
//! persists `state.tray.visible = false` and drops the tray; on
//! Cancel, returns to Idle. Mirrors the shape of `ui/size_confirm.rs`
//! (M3).

use egui::{Align2, Color32, RichText, Vec2};

use crate::ui::theme;

/// Per-frame model. The hotkey display is the active configured prompt
/// hotkey (e.g. "Cmd+Option+T"), surfaced from `cfg.hotkey_display()` at the
/// transition into this state.
#[derive(Debug, Clone)]
pub struct TrayHideModel {
    pub hotkey_display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayHideOutcome {
    Confirm,
    Cancel,
}

/// Default modal size. Smaller than the prompt window — this is a
/// pure confirmation dialog.
pub const TRAY_HIDE_MODAL_SIZE: Vec2 = Vec2::new(440.0, 220.0);

/// Paint the modal. Returns at most one outcome per frame (the user
/// either pressed a button or did not).
pub fn draw(ctx: &egui::Context, model: &TrayHideModel) -> Option<TrayHideOutcome> {
    let mut outcome: Option<TrayHideOutcome> = None;

    egui::Window::new("hide-icon-confirm")
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        // Reduced-motion: egui::Window's default `fade_in: true` (via Area)
        // would alpha-fade the modal in over ctx.style().animation_time.
        // The fade adds nothing here (the dimmed backdrop signals modal
        // appearance instantly) and creates an inconsistency vs. the other
        // modals which have no animation. Always-suppress.
        .fade_in(false)
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(20.0)
                .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT))
                .corner_radius(10.0),
        )
        .show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.label(
                RichText::new("Hide tray icon?")
                    .color(theme::INK)
                    .strong()
                    .size(15.0),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "You can still summon clipt9n with {}.",
                    model.hotkey_display
                ))
                .color(theme::INK_2)
                .size(13.0),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("To show the icon again, run with --show-tray or edit state.toml.")
                    .color(theme::INK_3)
                    .size(11.5),
            );

            ui.add_space(20.0);
            ui.horizontal(|ui| {
                let cancel = ui.add(
                    egui::Button::new(RichText::new("Cancel").color(theme::INK).size(13.0))
                        .min_size(Vec2::new(110.0, 32.0))
                        .fill(theme::PANEL_2)
                        .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT)),
                );
                if cancel.clicked() {
                    outcome = Some(TrayHideOutcome::Cancel);
                }
                ui.add_space(8.0);
                let confirm = ui.add(
                    egui::Button::new(
                        RichText::new("Hide")
                            .color(Color32::from_rgb(0xFF, 0x76, 0x76))
                            .strong()
                            .size(13.0),
                    )
                    .min_size(Vec2::new(110.0, 32.0))
                    .fill(theme::PANEL_2)
                    .stroke(egui::Stroke::new(1.0, theme::LINE_SOFT)),
                );
                if confirm.clicked() {
                    outcome = Some(TrayHideOutcome::Confirm);
                }
            });
        });

    // Esc cancels, Enter confirms.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(TrayHideOutcome::Cancel);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(TrayHideOutcome::Confirm);
    }

    outcome
}
