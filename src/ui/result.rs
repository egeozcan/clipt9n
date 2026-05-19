//! Translation result preview window. Shown when the user picks a
//! translation slot while holding Shift — the result is displayed
//! here instead of being written directly to the clipboard.
//!
//! The window shows the translated text in a scrollable preview block
//! with two actions: Copy (write to clipboard + dismiss) and Dismiss
//! (discard without writing). Keyboard: Enter → Copy, Esc → Dismiss.

use egui::{RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// View model for the result preview window.
#[derive(Debug, Clone)]
pub struct ResultModel {
    /// The translated text to preview.
    pub result_text: String,
    /// Verb-form label of the action that produced the result
    /// ("Translate to Deutsch", "Fix grammar", etc.).
    pub action_label: String,
    /// Character count of the source text (for the stats line).
    pub source_char_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultOutcome {
    /// User wants to copy the result to clipboard and dismiss.
    Copy,
    /// User wants to dismiss without copying.
    Dismiss,
}

/// Maximum height allocated to the scrollable preview area.
const PREVIEW_MAX_HEIGHT: f32 = 280.0;

pub fn draw(ctx: &egui::Context, model: &ResultModel) -> Option<ResultOutcome> {
    let mut clicked: Option<ResultOutcome> = None;
    theme::window_frame(ctx, "Translation result", Some("clipt9n · preview"), |ui| {
        let body_padding = egui::Margin::symmetric(18, 14);
        egui::Frame::new()
            .inner_margin(body_padding)
            .show(ui, |ui| {
                // ----- Header: action label + stats -----
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&model.action_label)
                            .color(theme::INK)
                            .strong()
                            .size(13.5),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} chars → {} chars",
                                model.source_char_count,
                                model.result_text.chars().count()
                            ))
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                        );
                    });
                });
                ui.add_space(10.0);

                // ----- Result preview block -----
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        let available = ui.available_width();
                        let galley = ui.painter().layout_no_wrap(
                            model.result_text.clone(),
                            egui::FontId::monospace(12.5),
                            theme::INK,
                        );
                        let line_height = galley.size().y.max(18.0);
                        // Estimate rows from galley width vs available width.
                        // For simplicity, we use a ScrollArea so the text
                        // flows naturally; multi-line is handled by wrapping
                        // inside the scroll area.
                        let text_height = compute_text_height(
                            &model.result_text,
                            available - 24.0, // inner padding
                            line_height,
                            ui,
                        );
                        let area_height = text_height.min(PREVIEW_MAX_HEIGHT);
                        egui::ScrollArea::vertical()
                            .max_height(area_height.max(60.0))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.set_min_width(available - 24.0);
                                ui.label(
                                    RichText::new(&model.result_text)
                                        .color(theme::INK)
                                        .monospace()
                                        .size(12.5),
                                );
                            });
                    });

                ui.add_space(14.0);
                // ----- Footer -----
                let sep_rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    Vec2::new(ui.available_width(), 1.0),
                );
                ui.painter().hline(
                    sep_rect.x_range(),
                    sep_rect.center().y,
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.style_mut().spacing.item_spacing.x = 4.0;
                    theme::kbd(ui, "↵");
                    ui.label(
                        RichText::new("copy ·")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    theme::kbd(ui, "Esc");
                    ui.label(
                        RichText::new("dismiss")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if copy_button(ui).clicked() {
                            clicked = Some(ResultOutcome::Copy);
                        }
                        ui.add_space(8.0);
                        if dismiss_button(ui).clicked() {
                            clicked = Some(ResultOutcome::Dismiss);
                        }
                    });
                });
            });
    });
    clicked
}

/// Estimate the height needed to render `text` in a monospace layout
/// with wrapping at `max_width`. Returns the total pixel height.
fn compute_text_height(text: &str, max_width: f32, _line_height: f32, ui: &mut egui::Ui) -> f32 {
    let galley = ui.painter().layout(
        text.to_string(),
        egui::FontId::monospace(12.5),
        theme::INK,
        max_width,
    );
    galley.size().y + 24.0 // padding fudge
}

fn copy_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Copy to clipboard";
    let padding = Vec2::new(14.0, 7.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(
            label.into(),
            egui::FontId::proportional(12.5),
            theme::ACCENT_INK,
        )
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
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Copy to clipboard")
    });
    response
}

fn dismiss_button(ui: &mut egui::Ui) -> egui::Response {
    let label = "Dismiss";
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
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Dismiss"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_model_constructs_with_fields() {
        let m = ResultModel {
            result_text: "Bonjour".into(),
            action_label: "Translate to Français".into(),
            source_char_count: 5,
        };
        assert_eq!(m.result_text, "Bonjour");
        assert_eq!(m.action_label, "Translate to Français");
        assert_eq!(m.source_char_count, 5);
    }
}
