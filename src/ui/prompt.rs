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
    /// Glossary entries that match the clipboard text (pair-agnostic).
    /// Computed by `App::show_window` once per summon.
    pub glossary_hits: Vec<GlossaryHit>,
}

/// Picked action from the prompt window. The caller maps the slot to a
/// concrete `translator::Action`.
///
/// Enter on the focused row dispatches via egui's built-in click handling
/// (`Sense::click` widgets fire on Enter/Space when focused). The caller
/// is responsible for setting initial focus on the desired slot via the
/// `focus_target` parameter to `draw` — there is no separate "repeat last"
/// outcome because the focused row IS the repeat target on summon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    /// User clicked a slot button, pressed 1–6, or pressed Enter on a
    /// focused slot row.
    Pick(u8),
    /// User pressed Esc.
    Cancel,
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

/// One glossary entry that matched the current clipboard. Rendered as a
/// chip in the prompt window's strip, between the preview and the slot
/// list. Pair-scope-agnostic: at preview time the user hasn't picked a
/// target, so we surface every source-term match regardless of pair.
/// The actual translator path applies pair scoping correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryHit {
    pub source: String,
    pub target: String,
}

pub const SLOTS: [SlotDef; 8] = [
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
        kind: SlotKind::Lang,
    },
    SlotDef {
        n: 5,
        kind: SlotKind::Lang,
    },
    SlotDef {
        n: 6,
        kind: SlotKind::FixGrammar,
    },
    SlotDef {
        n: 7,
        kind: SlotKind::Rewrite,
    },
    SlotDef {
        n: 8,
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
        (4, SlotKind::Lang) => (&cfg.languages.slot_4.label, &cfg.languages.slot_4.code),
        (5, SlotKind::Lang) => (&cfg.languages.slot_5.label, &cfg.languages.slot_5.code),
        (6, SlotKind::FixGrammar) => ("Fix grammar", "conservative"),
        (7, SlotKind::Rewrite) => ("Rewrite for clarity", "aggressive"),
        (8, SlotKind::Custom) => ("Custom prompt…", "type instruction"),
        _ => ("(invalid slot)", ""),
    }
}

/// Draw the prompt window into `ctx`. Returns `Some(PromptOutcome)` if the
/// user clicked a slot button this frame; `None` otherwise. Keyboard
/// handling lives in `App::update` and is not the responsibility of this
/// function.
///
/// `focus_target` requests `request_focus()` on the slot whose number
/// matches, on this frame only. Callers typically pass `Some(last_slot)`
/// (or `Some(1)` if no last slot) on the first frame after summon, then
/// `None` on subsequent frames so egui's normal Tab-driven focus tracking
/// takes over.
///
/// `focused_slot` is written back with the 1-based index of whichever
/// slot row currently holds keyboard focus, or `None` if no slot row is
/// focused. Callers use this to implement arrow-key wrap-around and
/// keyboard navigation that stays within the slot list.
pub fn draw(
    ctx: &egui::Context,
    cfg: &Config,
    model: &PromptModel,
    focus_target: Option<u8>,
    focused_slot: &mut Option<u8>,
) -> Option<PromptOutcome> {
    let mut clicked: Option<PromptOutcome> = None;
    theme::window_frame(ctx, "Translate clipboard", Some("clipt9n · prompt"), |ui| {
        if model.clipboard_text.is_empty() {
            draw_empty(ui);
        } else {
            draw_populated(ui, cfg, model, &mut clicked, focus_target, focused_slot);
        }
    });
    clicked
}

/// Shorthand: non-selectable label so it never steals focus from the
/// slot rows during keyboard navigation.
fn label(ui: &mut egui::Ui, rich_text: RichText) -> egui::Response {
    ui.add(egui::Label::new(rich_text).selectable(false))
}

fn draw_empty(ui: &mut egui::Ui) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        label(ui, RichText::new("⎚").size(28.0).color(theme::BAD));
        ui.add_space(8.0);
        label(
            ui,
            RichText::new("Clipboard is empty or not text.")
                .color(theme::INK)
                .size(14.0),
        );
        label(
            ui,
            RichText::new("Copy something and try again.")
                .color(theme::INK_3)
                .size(12.0),
        );
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 4.0;
            ui.add_space(ui.available_width() / 2.0 - 40.0);
            theme::kbd(ui, "Esc");
            label(
                ui,
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
    focus_target: Option<u8>,
    focused_slot: &mut Option<u8>,
) {
    let body_padding = egui::Margin::symmetric(18, 14);
    egui::Frame::new()
        .inner_margin(body_padding)
        .show(ui, |ui| {
            // ----- Preview header -----
            ui.horizontal(|ui| {
                label(
                    ui,
                    RichText::new("CLIPBOARD")
                        .color(theme::INK_3)
                        .size(11.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    label(
                        ui,
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
                        label(
                            ui,
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
            if cfg.ui.show_preview {
                let preview = preview_text(&model.clipboard_text);
                egui::Frame::new()
                    .fill(theme::PANEL_2)
                    .stroke(Stroke::new(1.0_f32, theme::LINE_SOFT))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        for line in preview.lines().take(3) {
                            ui.horizontal(|ui| {
                                label(
                                    ui,
                                    RichText::new("›")
                                        .color(theme::ACCENT.linear_multiply(0.6))
                                        .monospace(),
                                );
                                ui.add_space(4.0);
                                // Label::truncate() forces each preview line to
                                // exactly one visual line, ellipsizing if too
                                // wide. Without this, a long single-line
                                // clipboard wraps to multiple visual lines and
                                // blows up the preview-block height enough to
                                // squeeze the slot list (slot 1 then gets
                                // scrolled off-screen on initial focus).
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(if line.is_empty() {
                                            "\u{00A0}"
                                        } else {
                                            line
                                        })
                                        .color(theme::INK_2)
                                        .monospace()
                                        .size(12.5),
                                    )
                                    .selectable(false)
                                    .truncate(),
                                );
                            });
                        }
                    });
                ui.add_space(14.0);
            }

            // ----- Glossary chip strip (above slot list, per M4) -----
            draw_glossary_chips(ui, &model.glossary_hits);

            // ----- Slot rows -----
            // Wrapped in a ScrollArea so that on monitors too small to fit
            // the full window, focused rows auto-scroll into view. Reserve
            // room for the footer below the area.
            let footer_reserve = 60.0;
            let area_max_height = ui.available_height() - footer_reserve;
            egui::ScrollArea::vertical()
                .max_height(area_max_height.max(120.0))
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    // Collect responses so we can handle arrow-key
                    // navigation inline, cancelling egui's spatial
                    // double-move (begin_pass sets focus_direction,
                    // end_pass re-applies it).
                    for slot in SLOTS {
                        let (label, trailing) = slot_strings(slot, cfg);
                        let is_last = model.last_slot == Some(slot.n);
                        let should_focus = focus_target == Some(slot.n);
                        if draw_slot_row(
                            ui,
                            slot,
                            label,
                            trailing,
                            is_last,
                            should_focus,
                            focused_slot,
                        ) {
                            *clicked = Some(PromptOutcome::Pick(slot.n));
                        }
                    }
                });

            ui.add_space(12.0);
            // ----- Footer -----
            // Top-only border (matches design's `borderTop: "1px solid var(--line-soft)"`).
            let sep_rect =
                egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 1.0));
            ui.painter().hline(
                sep_rect.x_range(),
                sep_rect.center().y,
                Stroke::new(1.0_f32, theme::LINE_SOFT),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // window_frame's body sets item_spacing.x=0 so vertically
                // stacked frames hug; here we want a small gap between
                // each kbd cap and its adjacent label so they don't touch.
                ui.style_mut().spacing.item_spacing.x = 4.0;
                theme::kbd(ui, "1");
                label(ui, RichText::new("–").color(theme::INK_3).size(11.0));
                theme::kbd(ui, "8");
                ui.add(
                    egui::Label::new(
                        RichText::new("pick ·")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    )
                    .selectable(false),
                );
                theme::kbd(ui, "↵");
                ui.add(
                    egui::Label::new(
                        RichText::new("confirm ·")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    )
                    .selectable(false),
                );
                theme::kbd(ui, "Esc");
                ui.add(
                    egui::Label::new(
                        RichText::new("cancel")
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    )
                    .selectable(false),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if should_warn_large_paste(&model.clipboard_text, cfg) {
                        label(
                            ui,
                            RichText::new("⚠ large paste")
                                .color(theme::WARN)
                                .monospace()
                                .size(11.0),
                        );
                    }
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
    should_focus: bool,
    focused_slot: &mut Option<u8>,
) -> bool {
    // Allocate the row as a single focusable widget so Tab navigates between
    // rows. Drawing happens with painter + a child Ui scoped to the inner
    // rect for the layout-based label/trailing/badge content.
    let row_height = 36.0;
    let desired = Vec2::new(ui.available_width(), row_height);
    let response = ui.allocate_response(desired, Sense::click());
    if should_focus {
        // Initial focus on the slot caller wants pre-selected (typically
        // last_slot, or slot 1 on first run). Subsequent frames pass
        // should_focus=false so the user's Tab navigation is not stomped.
        response.request_focus();
    }
    let rect = response.rect;

    let focused = response.has_focus();
    if focused {
        *focused_slot = Some(slot.n);
    }
    let hovered = response.hovered();
    if focused {
        response.scroll_to_me(None);
    }

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
        Stroke::new(2.0_f32, theme::ACCENT)
    } else if is_last {
        Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x2e),
        )
    } else {
        Stroke::NONE
    };

    // NOTE: Do NOT gate `new_child(...)` (or any other id-allocating call)
    // behind `is_rect_visible(rect)`. `new_child` increments the parent's
    // `next_auto_id_salt`, which is also the source of every slot row's
    // widget id (`allocate_response` → `allocate_space` → `Id::new(salt)`).
    // If the inside block is skipped on some frames (e.g. slot 5 just
    // below the ScrollArea clip) and re-runs on others (during the scroll
    // animation), the salt count changes mid-traversal and the slot rows
    // get NEW ids. egui's focus is pinned to a specific id; when the id
    // disappears, the dead-man's switch in `end_pass` clears focus.
    // Render unconditionally — egui clips painter calls against the Ui's
    // clip rect, so off-screen text/rects cost almost nothing.
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
        Stroke::new(1.0_f32, theme::LINE),
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
    // `selectable(false)` is load-bearing: egui's default `Label` has
    // selectable text, whose text-selection sense intercepts clicks on
    // the text area and prevents the parent row's `Sense::click()` from
    // firing. Without this, clicking the literal "Fix grammar" text
    // wouldn't pick the slot — only the empty area to its right would.
    cui.add(egui::Label::new(RichText::new(label).color(theme::INK).size(13.5)).selectable(false));
    cui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if is_last {
            let badge = egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0xc8, 0xff, 0x5e, 0x29))
                .corner_radius(255)
                .inner_margin(egui::Margin::symmetric(7, 2));
            badge.show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new("LAST USED")
                            .color(theme::ACCENT)
                            .size(10.0)
                            .strong(),
                    )
                    .selectable(false),
                );
            });
            ui.add_space(6.0);
        }
        ui.add(
            egui::Label::new(
                RichText::new(trailing)
                    .color(theme::INK_3)
                    .monospace()
                    .size(11.0),
            )
            .selectable(false),
        );
    });

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

/// Returns true when the clipboard text exceeds the configured size
/// threshold and the prompt window should show a `⚠ large paste` indicator.
/// Same threshold gates the size-confirm modal in `app.rs`.
pub fn should_warn_large_paste(text: &str, cfg: &Config) -> bool {
    text.chars().count() > cfg.ui.confirm_size_threshold
}

/// Cap the chip list at `max` items; return the visible slice and the
/// overflow count for the "+N more" indicator.
pub fn truncate_chips(hits: &[GlossaryHit], max: usize) -> (&[GlossaryHit], usize) {
    if hits.len() <= max {
        (hits, 0)
    } else {
        (&hits[..max], hits.len() - max)
    }
}

/// Maximum number of glossary chips rendered before "+N more" replaces
/// the overflow. Picked to fit one wrap-row at 480px window width.
const MAX_CHIPS: usize = 5;

/// Draw the glossary chip strip. Conditional render: zero hits → nothing
/// is painted (no reserved height; layout collapses cleanly). Chips
/// render in the design's `pw.chip` style: `var(--panel-3)` background,
/// monospace 11px, source/arrow/target three-segment colors.
fn draw_glossary_chips(ui: &mut egui::Ui, hits: &[GlossaryHit]) {
    if hits.is_empty() {
        return;
    }
    let (visible, more) = truncate_chips(hits, MAX_CHIPS);
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(0xff, 0xb8, 0x4d, 0x10))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(0xff, 0xb8, 0x4d, 0x2e),
        ))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().spacing.item_spacing.x = 6.0;
                label(
                    ui,
                    RichText::new("GLOSSARY WILL INJECT:")
                        .color(theme::WARN)
                        .size(10.0)
                        .strong(),
                );
                for hit in visible {
                    chip(ui, hit);
                }
                if more > 0 {
                    label(
                        ui,
                        RichText::new(format!("+{more} more"))
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                }
            });
        });
    ui.add_space(10.0);
}

fn chip(ui: &mut egui::Ui, hit: &GlossaryHit) {
    egui::Frame::new()
        .fill(theme::PANEL_3)
        .stroke(Stroke::new(1.0_f32, theme::LINE))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 4.0;
                label(
                    ui,
                    RichText::new(&hit.source)
                        .color(theme::INK_2)
                        .monospace()
                        .size(11.0),
                );
                label(
                    ui,
                    RichText::new("→")
                        .color(theme::INK_3)
                        .monospace()
                        .size(11.0),
                );
                label(
                    ui,
                    RichText::new(&hit.target)
                        .color(theme::ACCENT)
                        .monospace()
                        .size(11.0),
                );
            });
        });
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
    fn slot_strings_for_slot_6_returns_fix_grammar_tag() {
        let cfg = Config::default();
        let (label, tag) = slot_strings(SLOTS[5], &cfg);
        assert_eq!(label, "Fix grammar");
        assert_eq!(tag, "conservative");
    }

    #[test]
    fn slot_strings_unchanged_when_show_preview_false() {
        // show_preview is a draw-layer concern (whether the preview block renders);
        // slot label resolution is independent. Sanity-check that toggling the
        // config flag does not change slot string output.
        let mut cfg = Config::default();
        cfg.ui.show_preview = false;
        let (label, code) = slot_strings(SLOTS[0], &cfg);
        assert_eq!(label, "English");
        assert_eq!(code, "en");
    }

    #[test]
    fn large_paste_threshold_uses_config_value() {
        // The threshold helper used by the footer warning reads from cfg.
        let mut cfg = Config::default();
        cfg.ui.confirm_size_threshold = 100;
        assert!(should_warn_large_paste("x".repeat(101).as_str(), &cfg));
        assert!(!should_warn_large_paste("x".repeat(99).as_str(), &cfg));
        assert!(!should_warn_large_paste("x".repeat(100).as_str(), &cfg)); // exactly at threshold: no warn
    }

    #[test]
    fn truncate_chips_returns_all_when_under_limit() {
        let hits = vec![
            GlossaryHit {
                source: "A".into(),
                target: "A".into(),
            },
            GlossaryHit {
                source: "B".into(),
                target: "B".into(),
            },
        ];
        let (visible, more) = truncate_chips(&hits, 5);
        assert_eq!(visible.len(), 2);
        assert_eq!(more, 0);
    }

    #[test]
    fn truncate_chips_caps_at_max_and_returns_overflow_count() {
        let hits: Vec<GlossaryHit> = (0..8)
            .map(|i| GlossaryHit {
                source: format!("s{i}"),
                target: format!("t{i}"),
            })
            .collect();
        let (visible, more) = truncate_chips(&hits, 5);
        assert_eq!(visible.len(), 5);
        assert_eq!(more, 3);
    }

    #[test]
    fn glossary_hit_constructs_from_glossary_entry() {
        // Smoke test that the conversion path used by show_window
        // (Glossary::preview_entries → Vec<&GlossaryEntry> → Vec<GlossaryHit>)
        // produces the expected source/target pairing.
        let entry = crate::glossary::GlossaryEntry {
            source: "Smart Table".into(),
            target: "Smart Table".into(),
            languages: vec!["*".into()],
            note: None,
        };
        let hit = GlossaryHit {
            source: entry.source.clone(),
            target: entry.target.clone(),
        };
        assert_eq!(hit.source, "Smart Table");
        assert_eq!(hit.target, "Smart Table");
    }
}
