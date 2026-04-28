//! The hotkey-summoned prompt window. Renders the design's
//! `prompt-window.jsx` (M2 wires slots 1–3 end-to-end via the caller; slots
//! 4–6 render but are no-ops in M2). The view is a pure function of
//! `PromptModel`; event handling lives in `update()` (Task 11).

use egui::{Color32, RichText, Sense, Stroke, Vec2};

use crate::config::Config;
use crate::ui::theme;

/// What the prompt window currently knows.
#[derive(Debug, Clone)]
pub struct PromptModel {
    /// Current clipboard text (already filtered to text-only). Empty string
    /// → render the empty state.
    pub clipboard_text: String,
    /// Auto-detected language code for the clipboard, if any (M4 sets this;
    /// M2 always passes `None`).
    pub detected_lang: Option<String>,
    /// 1-based slot index of the most recently used action ("last used" badge
    /// + Enter-to-repeat affordance). `None` on first run.
    pub last_slot: Option<u8>,
}

/// Picked action from the prompt window. The caller maps the slot to a
/// concrete `translator::Action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    /// User clicked a slot button or pressed 1–6.
    Pick(u8),
    /// User pressed Esc.
    Cancel,
    /// User pressed Enter while `last_slot` was set.
    RepeatLast,
}

/// Static slot definitions (matches `data.jsx` SLOTS).
#[derive(Debug, Clone, Copy)]
pub struct SlotDef {
    pub n: u8,
    pub kind: SlotKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Slot 1, 2, 3 — language slots; label/code come from `Config::languages`.
    Lang,
    /// Slot 4 — fix grammar.
    FixGrammar,
    /// Slot 5 — rewrite.
    Rewrite,
    /// Slot 6 — custom (M3 wires; M2 is no-op).
    Custom,
}

pub const SLOTS: [SlotDef; 6] = [
    SlotDef {
        n: 1,
        kind: SlotKind::Lang,
    },
    SlotDef {
        n: 2,
        kind: SlotKind::Lang,
    },
    SlotDef {
        n: 3,
        kind: SlotKind::Lang,
    },
    SlotDef {
        n: 4,
        kind: SlotKind::FixGrammar,
    },
    SlotDef {
        n: 5,
        kind: SlotKind::Rewrite,
    },
    SlotDef {
        n: 6,
        kind: SlotKind::Custom,
    },
];

/// Resolve a slot's display label and trailing tag/code. Returns
/// (label, trailing) where `trailing` is the right-aligned hint text
/// (lang code or descriptive tag).
pub fn slot_strings(slot: SlotDef, cfg: &Config) -> (&str, &str) {
    match (slot.n, slot.kind) {
        (1, SlotKind::Lang) => (&cfg.languages.slot_1.label, &cfg.languages.slot_1.code),
        (2, SlotKind::Lang) => (&cfg.languages.slot_2.label, &cfg.languages.slot_2.code),
        (3, SlotKind::Lang) => (&cfg.languages.slot_3.label, &cfg.languages.slot_3.code),
        (4, SlotKind::FixGrammar) => ("Fix grammar", "conservative"),
        (5, SlotKind::Rewrite) => ("Rewrite for clarity", "aggressive"),
        (6, SlotKind::Custom) => ("Custom prompt…", "type instruction"),
        _ => ("(invalid slot)", ""),
    }
}

/// Draw the prompt window into `ctx`. Returns `Some(PromptOutcome)` if the
/// user clicked a slot button this frame; `None` otherwise. Keyboard
/// handling lives in `App::update` and is not the responsibility of this
/// function.
pub fn draw(ctx: &egui::Context, cfg: &Config, model: &PromptModel) -> Option<PromptOutcome> {
    let mut clicked: Option<PromptOutcome> = None;
    theme::window_frame(ctx, "Translate clipboard", Some("clipt9n · prompt"), |ui| {
        if model.clipboard_text.is_empty() {
            draw_empty(ui);
        } else {
            draw_populated(ui, cfg, model, &mut clicked);
        }
    });
    clicked
}

fn draw_empty(ui: &mut egui::Ui) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("⎚").size(28.0).color(theme::BAD));
        ui.add_space(8.0);
        ui.label(
            RichText::new("Clipboard is empty or not text.")
                .color(theme::INK)
                .size(14.0),
        );
        ui.label(
            RichText::new("Copy something and try again.")
                .color(theme::INK_3)
                .size(12.0),
        );
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 40.0);
            theme::kbd(ui, "Esc");
            ui.label(
                RichText::new("to dismiss")
                    .color(theme::INK_3)
                    .size(11.0)
                    .monospace(),
            );
        });
    });
    ui.add_space(20.0);
}

fn draw_populated(
    ui: &mut egui::Ui,
    cfg: &Config,
    model: &PromptModel,
    clicked: &mut Option<PromptOutcome>,
) {
    let body_padding = egui::Margin::symmetric(18, 14);
    egui::Frame::new()
        .inner_margin(body_padding)
        .show(ui, |ui| {
            // ----- Preview header -----
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("CLIPBOARD")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("· {} chars", model.clipboard_text.chars().count()))
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                    let lang = model
                        .detected_lang
                        .as_deref()
                        .unwrap_or("??")
                        .to_uppercase();
                    let lang_frame = egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x1f))
                        .corner_radius(3)
                        .inner_margin(egui::Margin::symmetric(6, 1));
                    lang_frame.show(ui, |ui| {
                        ui.label(
                            RichText::new(lang)
                                .color(theme::ACCENT)
                                .monospace()
                                .size(10.0)
                                .strong(),
                        );
                    });
                });
            });
            ui.add_space(6.0);

            // ----- Preview block -----
            let preview = preview_text(&model.clipboard_text);
            egui::Frame::new()
                .fill(theme::PANEL_2)
                .stroke(Stroke::new(1.0, theme::LINE_SOFT))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    for line in preview.lines().take(3) {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("›")
                                    .color(theme::ACCENT.linear_multiply(0.6))
                                    .monospace(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(if line.is_empty() { "\u{00A0}" } else { line })
                                    .color(theme::INK_2)
                                    .monospace()
                                    .size(12.5),
                            );
                        });
                    }
                });
            ui.add_space(14.0);

            // ----- Slot rows -----
            for slot in SLOTS {
                let (label, trailing) = slot_strings(slot, cfg);
                let is_last = model.last_slot == Some(slot.n);
                if draw_slot_row(ui, slot, label, trailing, is_last) {
                    *clicked = Some(PromptOutcome::Pick(slot.n));
                }
            }

            // ----- Glossary chip area (M2 always empty; M4 fills it) -----
            // Empty placeholder reserved so the layout doesn't shift when M4
            // adds chips. Render nothing; the gap above the footer is enough.

            ui.add_space(12.0);
            // ----- Footer -----
            egui::Frame::new()
                .stroke(Stroke {
                    width: 1.0,
                    color: theme::LINE_SOFT,
                })
                .show(ui, |ui| {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        theme::kbd(ui, "1");
                        ui.label(RichText::new("–").color(theme::INK_3).size(11.0));
                        theme::kbd(ui, "6");
                        ui.label(
                            RichText::new("pick ·")
                                .color(theme::INK_3)
                                .monospace()
                                .size(11.0),
                        );
                        theme::kbd(ui, "↵");
                        let enter_label = if model.last_slot.is_some() {
                            "repeat last ·"
                        } else {
                            "— ·"
                        };
                        ui.label(
                            RichText::new(enter_label)
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
                            if model.clipboard_text.chars().count() > 2000 {
                                ui.label(
                                    RichText::new("⚠ large paste")
                                        .color(theme::WARN)
                                        .monospace()
                                        .size(11.0),
                                );
                            }
                        });
                    });
                });
        });
}

fn draw_slot_row(
    ui: &mut egui::Ui,
    slot: SlotDef,
    label: &str,
    trailing: &str,
    is_last: bool,
) -> bool {
    // Allocate the row as a single focusable widget so Tab navigates between
    // rows. Drawing happens with painter + a child Ui scoped to the inner
    // rect for the layout-based label/trailing/badge content.
    let row_height = 36.0;
    let desired = Vec2::new(ui.available_width(), row_height);
    let response = ui.allocate_response(desired, Sense::click());
    let rect = response.rect;

    let focused = response.has_focus();
    let hovered = response.hovered();

    let bg = if focused {
        Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x18)
    } else if hovered {
        theme::PANEL_3
    } else if is_last {
        Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x10)
    } else {
        Color32::TRANSPARENT
    };
    let border = if focused {
        Stroke::new(2.0, theme::ACCENT)
    } else if is_last {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x2e))
    } else {
        Stroke::NONE
    };

    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 6.0, bg);
        if border.width > 0.0 {
            ui.painter()
                .rect_stroke(rect, 6.0, border, egui::StrokeKind::Inside);
        }

        // Inner content: horizontal layout with badge, label, trailing.
        let inner_rect = rect.shrink2(Vec2::new(10.0, 7.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let cui = &mut child;

        // Number badge
        let (badge_rect, _) = cui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::hover());
        cui.painter().rect_filled(badge_rect, 4.0, theme::PANEL_3);
        cui.painter().rect_stroke(
            badge_rect,
            4.0,
            Stroke::new(1.0, theme::LINE),
            egui::StrokeKind::Inside,
        );
        cui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{}", slot.n),
            egui::FontId::monospace(11.5),
            theme::INK_2,
        );
        cui.add_space(8.0);
        cui.label(RichText::new(label).color(theme::INK).size(13.5));
        cui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if is_last {
                let badge = egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x29))
                    .corner_radius(255)
                    .inner_margin(egui::Margin::symmetric(7, 2));
                badge.show(ui, |ui| {
                    ui.label(
                        RichText::new("LAST USED")
                            .color(theme::ACCENT)
                            .size(10.0)
                            .strong(),
                    );
                });
                ui.add_space(6.0);
            }
            ui.label(
                RichText::new(trailing)
                    .color(theme::INK_3)
                    .monospace()
                    .size(11.0),
            );
        });
    }

    // AccessKit / VoiceOver hook.
    let label_for_a11y = label.to_string();
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label_for_a11y));

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// Truncate clipboard text to ~110 chars with an ellipsis (matches
/// `prompt-window.jsx` preview rules). Splits to lines for the caller.
pub fn preview_text(text: &str) -> String {
    if text.chars().count() <= 110 {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(110).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_under_limit_returns_unchanged() {
        let s = "hello world";
        assert_eq!(preview_text(s), s);
    }

    #[test]
    fn preview_over_limit_truncates_with_ellipsis() {
        let s = "x".repeat(200);
        let p = preview_text(&s);
        assert_eq!(p.chars().count(), 111); // 110 + ellipsis
        assert!(p.ends_with('…'));
    }

    #[test]
    fn slot_strings_for_slot_1_uses_config_label() {
        let cfg = Config::default();
        let (label, code) = slot_strings(SLOTS[0], &cfg);
        assert_eq!(label, "English");
        assert_eq!(code, "en");
    }

    #[test]
    fn slot_strings_for_slot_4_returns_fix_grammar_tag() {
        let cfg = Config::default();
        let (label, tag) = slot_strings(SLOTS[3], &cfg);
        assert_eq!(label, "Fix grammar");
        assert_eq!(tag, "conservative");
    }
}
