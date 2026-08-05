//! History viewer window — egui paint of the design's
//! `history-window.jsx`. Pure view + small pure helpers for ago-
//! formatting and pair-label rendering. Keyboard event routing and the
//! actual store queries live in `src/app.rs`.

use egui::{Color32, RichText, ScrollArea, Sense, Stroke, TextEdit, Vec2};

use crate::history::store::HistoryEntry;
use crate::ui::theme;

/// What the viewer paints per frame.
#[derive(Debug, Default, Clone)]
pub struct HistoryModel {
    /// All entries returned by the most recent `History::query` call.
    /// The viewer applies the search filter on top of this in-memory
    /// (cheap; we have ≤max_entries items, default 100).
    pub entries: Vec<HistoryEntry>,
    /// Live search query. Refreshed every frame from the text edit.
    pub query: String,
    /// Index into the *filtered* list (recomputed each frame). 0 if
    /// the list is empty.
    pub selected: usize,
    /// Whether the Shift+Del confirmation modal is visible.
    pub confirm_clear: bool,
    /// Set true once on first paint after a corruption-state open;
    /// the viewer renders a one-shot warning banner and the App
    /// flips it false on the next frame so the toast doesn't loop.
    pub show_corruption_banner: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOutcome {
    /// Esc pressed (or Cancel button on the modal).
    Close,
    /// Enter on the focused row → copy result back to clipboard.
    CopyResult(i64),
    /// `s` pressed → copy original source.
    CopySource(i64),
    /// `d` pressed → delete the focused row.
    Delete(i64),
    /// Shift+Del confirmed → wipe all rows (key preserved).
    ClearAll,
}

/// Format `created_at` (unix epoch seconds) as a relative-time label
/// like "2 min ago", "1 h ago", "yesterday", or an ISO-ish "MMM DD"
/// for older entries. Uses `std::time::SystemTime::now()` so it's
/// pure-relative; in tests pass `Some(now)` to make it deterministic.
pub fn ago_label(created_at: i64, now_override: Option<i64>) -> String {
    let now = now_override.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    });
    let delta = now.saturating_sub(created_at);
    if delta < 60 {
        "just now".into()
    } else if delta < 60 * 60 {
        format!("{} min ago", delta / 60)
    } else if delta < 60 * 60 * 24 {
        format!("{} h ago", delta / 3600)
    } else if delta < 60 * 60 * 48 {
        "yesterday".into()
    } else {
        format!("{} d ago", delta / (60 * 60 * 24))
    }
}

/// Format the pair label per the design's render: "DE → EN" for
/// translate, the lowercase action name otherwise.
pub fn pair_label(entry: &HistoryEntry) -> String {
    match entry.action.as_str() {
        "translate" => {
            let s = entry
                .source_lang
                .as_deref()
                .map(|c| c.to_uppercase())
                .unwrap_or_else(|| "??".into());
            let t = entry
                .target_lang
                .as_deref()
                .map(|c| c.to_uppercase())
                .unwrap_or_else(|| "??".into());
            format!("{s} → {t}")
        }
        "fix_grammar" => "grammar".into(),
        other => other.into(), // "rewrite" / "custom"
    }
}

/// Pair label color per design (lime / blue / purple / orange).
pub fn pair_color(entry: &HistoryEntry) -> Color32 {
    match entry.action.as_str() {
        "fix_grammar" => Color32::from_rgb(0x9a, 0xd6, 0xff),
        "rewrite" => Color32::from_rgb(0xd4, 0xa8, 0xff),
        "custom" => Color32::from_rgb(0xff, 0xb8, 0x4d),
        _ => theme::ACCENT,
    }
}

/// Filter entries against the current `query`. Mirrors the SQL-side
/// `filter_matches` but operates on already-decrypted entries (cheap
/// at viewer scale).
pub fn filter_entries<'a>(entries: &'a [HistoryEntry], query: &str) -> Vec<&'a HistoryEntry> {
    let q = query.trim();
    if q.is_empty() {
        return entries.iter().collect();
    }
    let q_lc = q.to_lowercase();
    entries.iter().filter(|e| matches_lc(e, &q_lc)).collect()
}

fn matches_lc(entry: &HistoryEntry, q_lc: &str) -> bool {
    if entry.action.to_lowercase().contains(q_lc) {
        return true;
    }
    if let Some(s) = entry.source_lang.as_deref() {
        if s.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    if let Some(t) = entry.target_lang.as_deref() {
        if t.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    if let Some(s) = entry.source.as_ref() {
        if s.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    if let Some(r) = entry.result.as_ref() {
        if r.to_lowercase().contains(q_lc) {
            return true;
        }
    }
    false
}

/// Paint the history viewer. Returns an outcome iff the user
/// triggered a transition this frame (Esc → Close, Enter → CopyResult,
/// etc.). The caller (App) is responsible for routing the outcome,
/// updating `model.selected` based on arrow-key navigation, and
/// re-querying the store after any deletion / clear.
pub fn draw(ctx: &egui::Context, model: &mut HistoryModel) -> Option<HistoryOutcome> {
    let mut outcome: Option<HistoryOutcome> = None;
    let frame = egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(20.0));

    frame.show(ctx, |ui| {
        ui.set_max_width(640.0); // 680px outer - 2 * 20px margin
        // Title row.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("History")
                    .color(theme::INK)
                    .strong()
                    .size(15.0),
            );
            ui.add_space(8.0);
            let count = model.entries.len();
            ui.label(
                RichText::new(format!("{count} entries · encrypted"))
                    .color(theme::INK_3)
                    .size(11.5),
            );
            ui.allocate_space(ui.available_size_before_wrap());
        });
        ui.add_space(10.0);

        // Optional corruption banner (one-shot).
        if model.show_corruption_banner {
            ui.label(
                RichText::new(
                    "History database unreadable. New history will not be saved.",
                )
                .color(theme::WARN)
                .size(11.5),
            );
            ui.add_space(8.0);
        }

        // Search row.
        ui.horizontal(|ui| {
            ui.label(RichText::new("⌕").color(theme::INK_3).monospace());
            ui.add_space(8.0);
            let edit = TextEdit::singleline(&mut model.query)
                .hint_text("Search history")
                .desired_width(ui.available_width() - 100.0);
            let edit_resp = ui.add(edit);
            // AccessKit label: explicit identity for screen readers (hint_text
            // already sets the AccessKit value, but a named label makes the
            // input discoverable by its role in the a11y tree).
            edit_resp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::TextEdit,
                    true,
                    "Search history",
                )
            });
        });
        ui.separator();

        // Filtered list.
        let filtered = filter_entries(&model.entries, &model.query);
        if model.selected >= filtered.len() && !filtered.is_empty() {
            model.selected = 0;
        }

        ScrollArea::vertical()
            .max_height(250.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if filtered.is_empty() {
                    ui.add_space(20.0);
                    ui.label(
                        RichText::new("No matches.")
                            .color(theme::INK_3)
                            .size(12.0),
                    );
                } else {
                    for (i, e) in filtered.iter().enumerate() {
                        let active = i == model.selected;
                        let bg = if active {
                            Color32::from_rgba_unmultiplied(200, 255, 94, 16)
                        } else {
                            Color32::TRANSPARENT
                        };
                        let resp = egui::Frame::new()
                            .fill(bg)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(if active { "▸" } else { " " })
                                            .color(theme::ACCENT)
                                            .monospace(),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(ago_label(e.created_at, None))
                                            .color(theme::INK_3)
                                            .monospace()
                                            .size(11.0),
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(pair_label(e))
                                            .color(pair_color(e))
                                            .monospace()
                                            .size(11.0)
                                            .strong(),
                                    );
                                    ui.add_space(8.0);
                                    let truncated = e
                                        .source
                                        .as_ref()
                                        .map(|s| truncate_for_row(s.as_str(), 64))
                                        .unwrap_or_else(|| "(text not stored)".into());
                                    ui.label(
                                        RichText::new(truncated)
                                            .color(if active { theme::INK } else { theme::INK_2 })
                                            .size(12.0),
                                    );
                                });
                            })
                            .response
                            .interact(Sense::click());
                        if resp.clicked() {
                            model.selected = i;
                        }
                        if resp.double_clicked() {
                            outcome = Some(HistoryOutcome::CopyResult(e.id));
                        }
                    }
                }
            });

        // Detail block for selected row.
        if let Some(sel) = filtered.get(model.selected) {
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(theme::PANEL_2)
                .stroke(Stroke::new(1.0_f32, theme::LINE_SOFT))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.columns(2, |cols| {
                        cols[0].label(
                            RichText::new("SOURCE")
                                .color(theme::INK_3)
                                .monospace()
                                .size(10.0)
                                .strong(),
                        );
                        cols[0].add_space(4.0);
                        let src = sel
                            .source
                            .as_ref()
                            .map(|s| s.as_str().to_owned())
                            .unwrap_or_else(|| "(text not stored)".into());
                        cols[0].label(
                            RichText::new(src)
                                .color(theme::INK)
                                .monospace()
                                .size(11.5),
                        );
                        cols[1].label(
                            RichText::new("RESULT")
                                .color(theme::INK_3)
                                .monospace()
                                .size(10.0)
                                .strong(),
                        );
                        cols[1].add_space(4.0);
                        let res = sel
                            .result
                            .as_ref()
                            .map(|s| s.as_str().to_owned())
                            .unwrap_or_else(|| "(text not stored)".into());
                        cols[1].label(
                            RichText::new(res)
                                .color(theme::INK)
                                .monospace()
                                .size(11.5),
                        );
                    });
                });
        }

        // Footer keymap.
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            theme::kbd(ui, "↵");
            ui.label(RichText::new("copy result").color(theme::INK_3).size(11.0));
            ui.add_space(12.0);
            theme::kbd(ui, "s");
            ui.label(RichText::new("copy source").color(theme::INK_3).size(11.0));
            ui.add_space(12.0);
            theme::kbd(ui, "d");
            ui.label(RichText::new("delete").color(theme::INK_3).size(11.0));
            ui.add_space(12.0);
            theme::kbd(ui, "⇧+Del");
            ui.label(RichText::new("clear all").color(theme::INK_3).size(11.0));
            ui.allocate_space(ui.available_size_before_wrap());
            theme::kbd(ui, "Esc");
            ui.label(RichText::new("close").color(theme::INK_3).size(11.0));
        });

        // Confirmation modal (rendered on top via egui::Window).
        if model.confirm_clear {
            egui::Window::new("clear_all_confirm")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .frame(
                    egui::Frame::new()
                        .fill(theme::PANEL)
                        .stroke(Stroke::new(1.0_f32, theme::LINE))
                        .inner_margin(18.0),
                )
                .show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.label(
                        RichText::new("Clear all history?")
                            .color(theme::INK)
                            .strong()
                            .size(14.0),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{} entries will be permanently removed. The encryption key stays in place.",
                            model.entries.len()
                        ))
                        .color(theme::INK_2)
                        .size(12.5),
                    );
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.allocate_space(Vec2::new(ui.available_width() - 200.0, 0.0));
                        if ui.button("Cancel").clicked() {
                            model.confirm_clear = false;
                        }
                        ui.add_space(8.0);
                        let danger = egui::Button::new(
                            RichText::new("Clear all")
                                .color(theme::ACCENT_INK)
                                .strong(),
                        )
                        .fill(theme::BAD);
                        if ui.add(danger).clicked() {
                            outcome = Some(HistoryOutcome::ClearAll);
                            model.confirm_clear = false;
                        }
                    });
                });
        }
    });

    outcome
}

fn truncate_for_row(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(max_chars + 1);
    for (count, ch) in s.chars().enumerate() {
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    fn entry(
        action: &str,
        source: &str,
        result: &str,
        src_lang: Option<&str>,
        tgt_lang: Option<&str>,
    ) -> HistoryEntry {
        HistoryEntry {
            id: 1,
            created_at: 1_700_000_000,
            action: action.into(),
            source_lang: src_lang.map(String::from),
            target_lang: tgt_lang.map(String::from),
            char_count: source.chars().count() as i64,
            source: Some(Zeroizing::new(source.into())),
            result: Some(Zeroizing::new(result.into())),
        }
    }

    #[test]
    fn ago_label_under_60s_is_just_now() {
        assert_eq!(ago_label(1000, Some(1059)), "just now");
    }

    #[test]
    fn ago_label_minutes() {
        assert_eq!(ago_label(1000, Some(1000 + 5 * 60)), "5 min ago");
    }

    #[test]
    fn ago_label_hours() {
        assert_eq!(ago_label(1000, Some(1000 + 3 * 3600)), "3 h ago");
    }

    #[test]
    fn ago_label_yesterday() {
        let yesterday = 1000 + 30 * 3600;
        assert_eq!(ago_label(1000, Some(yesterday)), "yesterday");
    }

    #[test]
    fn ago_label_days() {
        let three_days = 1000 + 3 * 24 * 3600;
        assert_eq!(ago_label(1000, Some(three_days)), "3 d ago");
    }

    #[test]
    fn pair_label_translate_uses_uppercase_codes() {
        let e = entry("translate", "x", "y", Some("de"), Some("en"));
        assert_eq!(pair_label(&e), "DE → EN");
    }

    #[test]
    fn pair_label_fix_grammar_renders_as_grammar() {
        let e = entry("fix_grammar", "x", "y", Some("de"), None);
        assert_eq!(pair_label(&e), "grammar");
    }

    #[test]
    fn pair_label_rewrite_and_custom_pass_through() {
        let r = entry("rewrite", "x", "y", Some("en"), None);
        assert_eq!(pair_label(&r), "rewrite");
        let c = entry("custom", "x", "y", Some("en"), None);
        assert_eq!(pair_label(&c), "custom");
    }

    #[test]
    fn filter_returns_all_when_query_empty() {
        let entries = vec![
            entry("translate", "Hello", "Hallo", Some("en"), Some("de")),
            entry("rewrite", "x", "y", Some("en"), None),
        ];
        assert_eq!(filter_entries(&entries, "").len(), 2);
        assert_eq!(filter_entries(&entries, "   ").len(), 2);
    }

    #[test]
    fn filter_matches_decrypted_text_case_insensitive() {
        let entries = vec![
            entry(
                "translate",
                "Smart Table demo",
                "Smart Table Demo",
                Some("en"),
                Some("de"),
            ),
            entry("rewrite", "noise", "more noise", Some("en"), None),
        ];
        let hits = filter_entries(&entries, "smart");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn filter_matches_action() {
        let entries = vec![
            entry("translate", "x", "y", Some("en"), Some("de")),
            entry("rewrite", "z", "w", Some("en"), None),
        ];
        let hits = filter_entries(&entries, "rewr");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, "rewrite");
    }
}
