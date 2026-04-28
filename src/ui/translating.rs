//! In-flight translation overlay. Renders the design's `TranslatingWindow`
//! (animated lime sweep bar + elapsed-time counter + Cancel button), or a
//! static label when reduced motion is enabled. Pure render layer; key
//! handling (Esc → Cancel) lives in `App::update`.

use std::time::Duration;

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::ui::theme;

/// Number of cells in the animated sweep bar. Matches the design's
/// `cells = 16`.
pub const BAR_CELLS: usize = 16;

/// Animation tick interval. Matches the design's `setInterval(80ms)`.
pub const TICK_MS: u64 = 80;

/// Mutable state of the translating overlay. Owned by `App`; constructed
/// at dispatch time and dropped on outcome / cancel.
#[derive(Debug, Clone)]
pub struct TranslatingModel {
    /// Verb-form label per design ("Translating to Deutsch…", "Fixing
    /// grammar…", "Rewriting for clarity…", "Running custom prompt…").
    pub overlay_label: String,
    /// Provider model identifier, shown in the title-bar subtitle.
    pub provider_model: String,
    /// Elapsed wall-time since dispatch.
    pub elapsed: Duration,
    /// User has reduced motion enabled at OS level.
    pub reduced_motion: bool,
}

/// Click-dispatched outcomes. Esc is caller-handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslatingOutcome {
    Cancel,
}

/// Compute the per-cell opacities for the sweep bar at a given elapsed time.
/// The "head" of the sweep cycles through cells; cells within 4 of the head
/// are progressively dimmer. Returns one `f32 ∈ [0.15, 1.0]` per cell.
pub fn compute_bar_opacities(elapsed: Duration, cells: usize) -> Vec<f32> {
    let tick = (elapsed.as_millis() as u64 / TICK_MS) as usize;
    let head = tick % cells.max(1);
    (0..cells)
        .map(|i| {
            let dist = (i + cells - head) % cells;
            let intensity = if dist < 4 {
                (4 - dist) as f32 / 4.0
            } else {
                0.0
            };
            0.15 + intensity * 0.85
        })
        .collect()
}

/// Format elapsed time as the design's `(tick * 80 / 1000).toFixed(1)` —
/// e.g., "0.4s", "1.2s".
pub fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_millis() as f32 / 1000.0;
    format!("{secs:.1}s")
}

/// Render the overlay. Returns `Some(TranslatingOutcome::Cancel)` if the
/// user clicked Cancel; `None` otherwise.
pub fn draw(ctx: &egui::Context, model: &TranslatingModel) -> Option<TranslatingOutcome> {
    let mut clicked: Option<TranslatingOutcome> = None;
    theme::window_frame(
        ctx,
        &model.overlay_label,
        Some(&model.provider_model),
        |ui| {
            let body_padding = egui::Margin::symmetric(18, 16);
            egui::Frame::new()
                .inner_margin(body_padding)
                .show(ui, |ui| {
                    if model.reduced_motion {
                        // Static fallback per WCAG 2.3.3 (animation suppression).
                        // The widget_info call gives the label an AccessKit
                        // identity so screen readers announce "Translation in
                        // progress" — without it, reduced-motion users get
                        // zero feedback that an action is running (WCAG 4.1.3).
                        ui.add_space(8.0);
                        let resp =
                            ui.label(RichText::new("Translating…").color(theme::INK).size(13.5));
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                true,
                                "Translation in progress",
                            )
                        });
                        ui.add_space(8.0);
                    } else {
                        draw_animated_bar(ui, model);
                    }

                    // Meta row: endpoint + elapsed.
                    // TODO(M3 wiring): replace placeholder with the actual
                    // provider endpoint host (e.g., "api.anthropic.com")
                    // when the overlay is integrated in Task 10.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("request → api endpoint")
                                .color(theme::INK_3)
                                .monospace()
                                .size(11.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_elapsed(model.elapsed))
                                    .color(theme::INK_3)
                                    .monospace()
                                    .size(11.0),
                            );
                        });
                    });

                    ui.add_space(14.0);
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

                    // Footer: hint + Cancel button.
                    ui.horizontal(|ui| {
                        // Override window_frame's item_spacing.x=0 so the
                        // kbd cap doesn't visually touch "cancel".
                        ui.style_mut().spacing.item_spacing.x = 4.0;
                        theme::kbd(ui, "Esc");
                        ui.label(
                            RichText::new("cancel")
                                .color(theme::INK_3)
                                .monospace()
                                .size(11.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if cancel_button(ui).clicked() {
                                clicked = Some(TranslatingOutcome::Cancel);
                            }
                        });
                    });
                });
        },
    );
    clicked
}

fn draw_animated_bar(ui: &mut egui::Ui, model: &TranslatingModel) {
    let opacities = compute_bar_opacities(model.elapsed, BAR_CELLS);
    let bar_height = 16.0;
    let total = ui.available_width();
    let gap = 3.0;
    let cell_w = (total - gap * (BAR_CELLS as f32 - 1.0)) / BAR_CELLS as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, bar_height), Sense::hover());
    if ui.is_rect_visible(rect) {
        for (i, op) in opacities.iter().enumerate() {
            let x = rect.left() + (cell_w + gap) * i as f32;
            let cell_rect =
                egui::Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(cell_w, bar_height));
            let alpha = (op * 255.0).clamp(0.0, 255.0) as u8;
            let color = Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, alpha);
            ui.painter().rect_filled(cell_rect, 2.0, color);
        }
    }
    ui.add_space(12.0);
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
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Cancel translation")
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacities_have_correct_length() {
        let v = compute_bar_opacities(Duration::from_millis(0), BAR_CELLS);
        assert_eq!(v.len(), BAR_CELLS);
    }

    #[test]
    fn opacities_are_clamped_in_range() {
        for ms in [0, 80, 240, 1280, 5000] {
            let v = compute_bar_opacities(Duration::from_millis(ms), BAR_CELLS);
            for o in v {
                assert!((0.15..=1.0).contains(&o), "opacity {o} out of range");
            }
        }
    }

    #[test]
    fn opacities_have_a_bright_head_at_t0() {
        let v = compute_bar_opacities(Duration::from_millis(0), BAR_CELLS);
        // At tick=0 the head is at index 0; cell 0 should be the brightest.
        assert!(
            (v[0] - 1.0).abs() < 0.01,
            "expected head at index 0 to be 1.0, got {}",
            v[0]
        );
        // Cells far from head should be at minimum (0.15).
        assert!((v[8] - 0.15).abs() < 0.01);
    }

    #[test]
    fn head_advances_with_time() {
        let t0 = compute_bar_opacities(Duration::from_millis(0), BAR_CELLS);
        let t1 = compute_bar_opacities(Duration::from_millis(TICK_MS), BAR_CELLS);
        // The head moved one cell, so the brightest cell index shifted.
        let bright = |v: &[f32]| {
            v.iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap()
        };
        assert_eq!(bright(&t0), 0);
        assert_eq!(bright(&t1), 1);
    }

    #[test]
    fn head_wraps_at_cell_boundary() {
        // The modular `(i + cells - head) % cells` distance formula must
        // wrap cleanly: head=15 at t=15·TICK_MS, then head=0 at t=16·TICK_MS.
        let v_15 = compute_bar_opacities(Duration::from_millis(TICK_MS * 15), BAR_CELLS);
        let v_16 = compute_bar_opacities(Duration::from_millis(TICK_MS * 16), BAR_CELLS);
        let bright = |v: &[f32]| {
            v.iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i)
                .unwrap()
        };
        assert_eq!(bright(&v_15), 15);
        assert_eq!(
            bright(&v_16),
            0,
            "head should wrap from cell 15 back to cell 0"
        );
    }

    #[test]
    fn format_elapsed_renders_one_decimal() {
        assert_eq!(format_elapsed(Duration::from_millis(0)), "0.0s");
        // 450ms → 0.45_f32. f32 cannot represent 0.45 exactly; the nearest
        // value is ≈0.4499998807, so {:.1} rounds DOWN to "0.4s" rather than
        // up to "0.5s". The display behavior is correct (matches design's
        // toFixed(1)) — the precision quirk is just a property of f32.
        assert_eq!(format_elapsed(Duration::from_millis(450)), "0.4s");
        assert_eq!(format_elapsed(Duration::from_secs(2)), "2.0s");
        assert_eq!(format_elapsed(Duration::from_millis(12345)), "12.3s");
    }
}
