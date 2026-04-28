//! Pre-dispatch confirmation modal for oversized clipboards. Renders inside
//! the existing viewport (no new viewport spawned). Pure render layer; key
//! handling (Esc → Cancel, Enter → Confirm) lives in `App::update`.

use egui::{RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// Maximum source-text length rendered in the modal preview before
/// truncation with ellipsis.
const PREVIEW_MAX_CHARS: usize = 300;

/// Mutable view state.
#[derive(Debug, Clone)]
pub struct SizeConfirmModel {
    /// Character count of the pending clipboard text.
    pub char_count: usize,
    /// Truncated preview of the source — caller passes the result of
    /// `format_preview`, not the raw clipboard.
    pub preview: String,
    /// Verb-form label of the action that will run on confirm
    /// ("Translate to Deutsch", "Run custom prompt", etc).
    pub action_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeConfirmOutcome {
    Confirm,
    Cancel,
}

/// Truncate the preview to ≤`PREVIEW_MAX_CHARS` chars with an ellipsis. The
/// modal body shows up to two lines; longer source is implied via the count.
pub fn format_preview(text: &str) -> String {
    if text.chars().count() <= PREVIEW_MAX_CHARS {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(PREVIEW_MAX_CHARS).collect();
        out.push('…');
        out
    }
}

pub fn draw(ctx: &egui::Context, model: &SizeConfirmModel) -> Option<SizeConfirmOutcome> {
    let mut clicked: Option<SizeConfirmOutcome> = None;
    theme::window_frame(ctx, "Confirm send", Some("clipt9n · large clipboard"), |ui| {
        let body_padding = egui::Margin::symmetric(18, 14);
        egui::Frame::new()
            .inner_margin(body_padding)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("{} characters", model.char_count))
                        .color(theme::WARN)
                        .strong()
                        .size(14.0),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Sending this clipboard to the API for: {}.",
                        model.action_label
                    ))
                    .color(theme::INK)
                    .size(12.5),
                );
                ui.add_space(12.0);

                ui.label(
                    RichText::new("PREVIEW")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if model.preview.is_empty() {
                                "\u{00A0}"
                            } else {
                                &model.preview
                            })
                            .color(theme::INK_2)
                            .monospace()
                            .size(12.0),
                        );
                    });

                ui.add_space(14.0);
                let sep_rect =
                    egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 1.0));
                ui.painter().hline(
                    sep_rect.x_range(),
                    sep_rect.center().y,
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    theme::kbd(ui, "↵");
                    ui.label(
                        RichText::new("send ·")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    theme::kbd(ui, "Esc");
                    ui.label(
                        RichText::new("cancel")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if confirm_button(ui).clicked() {
                            clicked = Some(SizeConfirmOutcome::Confirm);
                        }
                        ui.add_space(8.0);
                        if cancel_button(ui).clicked() {
                            clicked = Some(SizeConfirmOutcome::Cancel);
                        }
                    });
                });
            });
    });
    clicked
}

fn confirm_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Send →";
    let padding = Vec2::new(14.0, 7.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.5), theme::ACCENT_INK)
        .size();
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::ACCENT.gamma_multiply(0.92)
        } else {
            theme::ACCENT
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        if response.has_focus() {
            // Focus stroke uses ACCENT_INK (very dark) on the ACCENT
            // background — high contrast for keyboard users. ACCENT-on-ACCENT
            // would be nearly invisible.
            ui.painter().rect_stroke(
                rect,
                6.0,
                Stroke::new(2.0, theme::ACCENT_INK),
                egui::StrokeKind::Outside,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            theme::ACCENT_INK,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Send"));
    response
}

fn cancel_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Cancel";
    let padding = Vec2::new(12.0, 5.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.0), theme::INK_2)
        .size();
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::PANEL_3
        } else {
            theme::PANEL_2
        };
        ui.painter().rect_filled(rect, 6.0, bg);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, theme::LINE),
            egui::StrokeKind::Inside,
        );
        if response.has_focus() {
            // ACCENT stroke on the dark PANEL_2/3 background is high-contrast
            // and keyboard-visible. Without this, Tab landing on Cancel gives
            // no visual feedback.
            ui.painter().rect_stroke(
                rect,
                6.0,
                Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Outside,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            theme::INK_2,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_under_limit_returns_unchanged() {
        let s = "short input";
        assert_eq!(format_preview(s), s);
    }

    #[test]
    fn preview_over_limit_truncates_with_ellipsis() {
        let s = "x".repeat(PREVIEW_MAX_CHARS + 100);
        let p = format_preview(&s);
        assert_eq!(p.chars().count(), PREVIEW_MAX_CHARS + 1); // truncated + ellipsis
        assert!(p.ends_with('…'));
    }
}
