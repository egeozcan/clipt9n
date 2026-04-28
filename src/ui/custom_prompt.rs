//! Slot-6 custom prompt window. Renders the design's `custom-prompt.jsx`.
//! Pure render layer: no event handling — `app.rs` owns Cmd+Enter / Esc /
//! preset clicks. The view is a function of `CustomPromptModel`.

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// Hardcoded list per design `custom-prompt.jsx`. Order is meaningful
/// (rendered left-to-right, wrapped). Edit only with design approval.
pub const PRESETS: &[&str] = &[
    "translate to formal Spanish",
    "make this sound more diplomatic",
    "explain like I'm five",
    "summarize in one sentence",
    "convert to bullet points",
];

/// Mutable state of the custom prompt window. Owned by `App`; reset to
/// default on every entry to `EnteringCustom` so a previously-entered
/// instruction is never recalled (spec privacy rule).
#[derive(Debug, Clone, Default)]
pub struct CustomPromptModel {
    /// Source clipboard text (read-only; rendered in the preview block).
    pub clipboard_text: String,
    /// Live editor contents.
    pub instruction: String,
    /// Set to `true` on entry; the renderer clears it after calling
    /// `request_focus` once. Without this, the textarea would refocus on
    /// every frame and steal focus from preset buttons.
    pub focus_textarea_next_frame: bool,
}

/// Click-dispatched outcomes. Enter / Esc are caller-handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomPromptOutcome {
    /// User clicked `Run →` or pressed Cmd+Enter (caller emits the latter).
    Submit,
    /// User clicked outside / pressed a preset chip (selects it as text).
    PresetPicked(usize),
}

/// Truncate clipboard text to ≤200 chars with an ellipsis, matching the
/// design's `slice(0, 200) + "…"` rule.
pub fn preview_truncate(text: &str) -> String {
    if text.chars().count() <= 200 {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(200).collect();
        out.push('…');
        out
    }
}

/// Returns true when the instruction (after trim) is non-empty — `Run →`
/// is enabled iff this returns true.
pub fn submit_enabled(instruction: &str) -> bool {
    !instruction.trim().is_empty()
}

/// Render the custom prompt window. Returns `Some(CustomPromptOutcome)` on
/// click events; keyboard handling (Cmd+Enter, Esc) lives in `App::update`.
/// Mutates `model.instruction` to reflect TextEdit contents and may set
/// `model.focus_textarea_next_frame = false` after first focus call.
pub fn draw(
    ctx: &egui::Context,
    model: &mut CustomPromptModel,
) -> Option<CustomPromptOutcome> {
    let mut clicked: Option<CustomPromptOutcome> = None;
    theme::window_frame(ctx, "Custom prompt", Some("clipt9n · slot 6"), |ui| {
        let body_padding = egui::Margin::symmetric(18, 14);
        egui::Frame::new()
            .inner_margin(body_padding)
            .show(ui, |ui| {
                // ----- "Instruction" label -----
                ui.label(
                    RichText::new("INSTRUCTION")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.add_space(4.0);

                // ----- Multi-line text editor -----
                let editor = egui::TextEdit::multiline(&mut model.instruction)
                    .hint_text("e.g. \"translate to formal Spanish\"")
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY);
                let response = ui.add(editor);
                if model.focus_textarea_next_frame {
                    response.request_focus();
                    model.focus_textarea_next_frame = false;
                }
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        true,
                        "Custom prompt instruction",
                    )
                });

                ui.add_space(8.0);

                // ----- Preset chips -----
                ui.horizontal_wrapped(|ui| {
                    for (i, preset) in PRESETS.iter().enumerate() {
                        let chip_resp = chip(ui, preset);
                        if chip_resp.clicked() {
                            clicked = Some(CustomPromptOutcome::PresetPicked(i));
                        }
                        chip_resp.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                preset,
                            )
                        });
                    }
                });

                ui.add_space(14.0);

                // ----- "Will be applied to" preview -----
                ui.label(
                    RichText::new("WILL BE APPLIED TO")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.add_space(4.0);
                let preview = preview_truncate(&model.clipboard_text);
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(if preview.is_empty() { " " } else { &preview })
                                .color(theme::INK_2)
                                .monospace()
                                .size(12.0),
                        );
                    });

                ui.add_space(12.0);

                // ----- Footer: hint + Run button -----
                let sep_rect =
                    egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 1.0));
                ui.painter().hline(
                    sep_rect.x_range(),
                    sep_rect.center().y,
                    Stroke::new(1.0, theme::LINE_SOFT),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::kbd(ui, "⌘");
                    ui.label(RichText::new("+").color(theme::INK_3).size(11.0));
                    theme::kbd(ui, "↵");
                    ui.label(
                        RichText::new("run ·")
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
                        let enabled = submit_enabled(&model.instruction);
                        if run_button(ui, enabled).clicked() && enabled {
                            clicked = Some(CustomPromptOutcome::Submit);
                        }
                    });
                });
            });
    });
    clicked
}

/// Render a single preset chip. Click selects it as the instruction text.
fn chip(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::monospace(11.0), theme::INK_2)
        .size();
    let padding = Vec2::new(9.0, 3.0);
    let desired = galley_size + padding * 2.0;
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if response.hovered() {
            theme::PANEL_3
        } else {
            Color32::from_rgba_unmultiplied(0x23, 0x27, 0x2f, 0xcc)
        };
        ui.painter().rect_filled(rect, 999.0, bg);
        ui.painter().rect_stroke(
            rect,
            999.0,
            Stroke::new(1.0, theme::LINE),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(11.0),
            theme::INK_2,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Render the primary `Run →` action button. When `enabled` is false, paints
/// a disabled style and a non-click response.
fn run_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let label = "Run →";
    let padding = Vec2::new(14.0, 7.0);
    let galley_size = ui
        .painter()
        .layout_no_wrap(label.into(), egui::FontId::proportional(12.5), theme::ACCENT_INK)
        .size();
    let desired = galley_size + padding * 2.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(desired, sense);
    if ui.is_rect_visible(rect) {
        let (bg, fg) = if !enabled {
            (theme::PANEL_3, theme::INK_3)
        } else if response.hovered() {
            (theme::ACCENT.gamma_multiply(0.92), theme::ACCENT_INK)
        } else {
            (theme::ACCENT, theme::ACCENT_INK)
        };
        ui.painter().rect_filled(rect, 6.0, bg);
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
            egui::FontId::proportional(12.5),
            fg,
        );
    }
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Run custom prompt")
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_under_limit_returns_unchanged() {
        let s = "short input";
        assert_eq!(preview_truncate(s), s);
    }

    #[test]
    fn preview_over_limit_truncates_with_ellipsis() {
        let s = "x".repeat(300);
        let p = preview_truncate(&s);
        assert_eq!(p.chars().count(), 201); // 200 + ellipsis
        assert!(p.ends_with('…'));
    }

    #[test]
    fn submit_disabled_for_empty_instruction() {
        assert!(!submit_enabled(""));
        assert!(!submit_enabled("   "));
        assert!(!submit_enabled("\n\t"));
    }

    #[test]
    fn submit_enabled_for_nontrivial_instruction() {
        assert!(submit_enabled("translate to formal Spanish"));
        assert!(submit_enabled("  hello  "));
    }

    #[test]
    fn presets_are_five_items() {
        // Keep the count locked so that any future addition is a deliberate
        // design change, not an accidental one (renderer wraps; layout is
        // calibrated for ~5 chips).
        assert_eq!(PRESETS.len(), 5);
    }

    #[test]
    fn custom_prompt_model_default_is_empty() {
        let m = CustomPromptModel::default();
        assert!(m.clipboard_text.is_empty());
        assert!(m.instruction.is_empty());
        assert!(!m.focus_textarea_next_frame);
    }
}
