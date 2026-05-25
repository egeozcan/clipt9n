//! Design tokens (a11y-corrected) and reusable visual primitives.
//!
//! Source palette: `handoff/clipt9n/project/Clipboard Translator.html`.
//! a11y corrections per `docs/superpowers/specs/2026-04-28-clipt9n-implementation-design.md`
//! ("A11y baseline" section): `--ink-3` lifted from `#80869294` (alpha 58%,
//! ~3.5:1 contrast) to solid `#9ca3b1` (~5.1:1, AA pass). Disabled-state
//! foreground bumped to `#7a818d`.

use egui::{Color32, Stroke, Visuals};

// ----- Palette -----

pub const BG: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x17, 0x1c);
pub const PANEL_2: Color32 = Color32::from_rgb(0x1c, 0x20, 0x27);
pub const PANEL_3: Color32 = Color32::from_rgb(0x23, 0x27, 0x2f);
pub const LINE: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x39);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x20, 0x24, 0x2c);

pub const INK: Color32 = Color32::from_rgb(0xe9, 0xec, 0xf1);
pub const INK_2: Color32 = Color32::from_rgb(0xb6, 0xbc, 0xc7);
/// a11y-corrected from #80869294 (alpha 58%) to solid #9ca3b1.
pub const INK_3: Color32 = Color32::from_rgb(0x9c, 0xa3, 0xb1);
/// Decorative gutter only (line numbers in glossary view). Documents the
/// 3.6:1 ratio is acceptable for non-text UI chrome.
pub const MUTED: Color32 = Color32::from_rgb(0x6c, 0x72, 0x7d);
/// Disabled foreground on PANEL_3, ~3.2:1.
pub const DISABLED_FG: Color32 = Color32::from_rgb(0x7a, 0x81, 0x8d);

pub const ACCENT: Color32 = Color32::from_rgb(0xc8, 0xff, 0x5e);
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
pub const WARN: Color32 = Color32::from_rgb(0xff, 0xb8, 0x4d);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x76, 0x76);
pub const GOOD: Color32 = Color32::from_rgb(0x8f, 0xe3, 0xa7);

// ----- Visuals -----

/// Build the dark-mode `Visuals` for the entire app. Sets backgrounds,
/// strokes, and selection/focus colors so every interactive widget gets
/// the lime accent for selection AND a 2px ACCENT focus ring (a11y).
pub fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.window_fill = PANEL;
    v.panel_fill = PANEL;
    v.faint_bg_color = PANEL_2;
    v.extreme_bg_color = BG;
    v.override_text_color = Some(INK);
    v.hyperlink_color = ACCENT;

    // Selection (highlighted rows, selected text)
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x18); // ~9% accent
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    // Focus ring: 2px ACCENT on every focusable widget.
    v.widgets.inactive.bg_fill = PANEL_2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, INK_2);

    v.widgets.hovered.bg_fill = PANEL_3;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, INK);

    v.widgets.active.bg_fill = PANEL_3;
    v.widgets.active.bg_stroke = Stroke::new(2.0, ACCENT); // ← focus ring
    v.widgets.active.fg_stroke = Stroke::new(1.0, INK);

    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE_SOFT);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK_3);

    v
}

// ----- Reusable widgets -----

/// Render a kbd-style key cap. Use for footer keymap hints.
/// Inner labels use `selectable(false)` so decorative kbd caps
/// never steal keyboard focus from the slot list.
pub fn kbd(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let frame = egui::Frame::new()
        .fill(PANEL_3)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(5, 1));
    frame
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .monospace()
                        .size(10.5)
                        .color(INK_2),
                )
                .selectable(false),
            );
        })
        .response
}

/// `WindowFrame` analog: title bar + body. Rendered as content inside an
/// already-borderless egui viewport (the viewport has decorations off, so
/// we paint our own title bar). Returns the inner-body return value of
/// `body`.
pub fn window_frame<R>(
    ctx: &egui::Context,
    title: &str,
    subtitle: Option<&str>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(PANEL))
        .show(ctx, |ui| {
            // Title bar
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0x14, 0x16, 0x1c, 0x99))
                .inner_margin(egui::Margin::symmetric(12, 9))
                .stroke(Stroke::new(1.0, LINE_SOFT))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(2.0);
                        // Accent dot
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 3.0, ACCENT);
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(title).color(INK).size(13.0).strong());
                        if let Some(sub) = subtitle {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(sub).color(INK_3).monospace().size(11.0));
                        }
                    });
                });
            ui.add_space(4.0);
            ui.scope(|ui| {
                ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 6.0);
                body(ui)
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_3_is_a11y_corrected() {
        assert_eq!(INK_3, Color32::from_rgb(0x9c, 0xa3, 0xb1));
    }

    #[test]
    fn visuals_use_accent_for_focus_ring() {
        let v = visuals();
        assert_eq!(v.widgets.active.bg_stroke.color, ACCENT);
        assert_eq!(v.widgets.active.bg_stroke.width, 2.0);
    }

    #[test]
    fn visuals_text_color_is_ink() {
        let v = visuals();
        assert_eq!(v.override_text_color, Some(INK));
    }
}
