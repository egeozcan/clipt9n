//! Glossary editor window — a structured table over `glossary.toml`.
//!
//! Pure view plus pure helpers (filtering, language-list parsing). Every
//! effect — reading the file, validating, writing, swapping the live
//! glossary — lives in `src/app/glossary.rs`, mirroring the
//! `ui/settings.rs` ↔ `app/settings.rs` split.
//!
//! The model is a *working copy*: nothing reaches disk until Save
//! succeeds, and Cancel discards the lot. That is what lets row deletes
//! and adds skip their own confirmation modals — they are undoable
//! right up until the user commits.

use egui::{Color32, RichText, ScrollArea, Stroke, TextEdit, Vec2};

use crate::glossary::GlossaryEntry;
use crate::ui::theme;

/// Inner size of the editor window. Wider than history (680) is not
/// needed; taller, because the edit panel sits below the table.
pub const GLOSSARY_INNER_SIZE: Vec2 = Vec2::new(680.0, 580.0);

/// What the editor paints per frame.
#[derive(Debug, Default, Clone)]
pub struct GlossaryModel {
    /// Working copy. Edited freely; only committed on Save.
    pub entries: Vec<GlossaryEntry>,
    /// Snapshot taken when the window opened. `dirty()` diffs against
    /// it, so re-typing a value back to its original clears the flag.
    pub original: Vec<GlossaryEntry>,
    /// Live search query, filtering the table in memory.
    pub query: String,
    /// Index into `entries` (never into the filtered view) whose fields
    /// the edit panel is bound to.
    pub editing: Option<usize>,
    /// Text buffer for the edited row's `languages`, parsed back into
    /// the entry every frame. Kept separate because the entry stores a
    /// `Vec<String>` and the field is one comma-separated line.
    pub lang_buf: String,
    /// Text buffer for the edited row's `note`, for the same reason —
    /// the entry stores `Option<String>` and empty means `None`.
    pub note_buf: String,
    /// Save rejection (validation or write failure), rendered in the
    /// footer. Empty means no error.
    pub err_msg: String,
    /// Absolute path shown under the title.
    pub path_display: String,
    /// True when the file on disk has comments that Save will drop.
    /// A structured editor cannot round-trip them, so we say so up
    /// front rather than deleting them silently.
    pub comments_will_be_dropped: bool,
    /// Set when the file exists but failed to parse. The editor opens
    /// with whatever it could not read shown as a banner and an empty
    /// table — the user can rebuild the file in-app rather than being
    /// locked out by their own typo.
    pub load_error: Option<String>,
    /// Whether the unsaved-changes confirmation is up.
    pub confirm_discard: bool,
}

impl GlossaryModel {
    /// Whether the working copy differs from what was loaded.
    pub fn dirty(&self) -> bool {
        self.entries != self.original
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlossaryOutcome {
    /// Save button or Cmd/Ctrl+S — validate, write, reload.
    Save,
    /// Cancel, Esc on a clean form, or Discard in the confirmation —
    /// throw the working copy away.
    Close,
}

/// Split a comma-separated language-scope field into the entry's list.
/// An empty field means "all pairs", matching the loader's own
/// normalization of an omitted `languages` key.
pub fn parse_languages(input: &str) -> Vec<String> {
    let parsed: Vec<String> = input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if parsed.is_empty() {
        vec!["*".into()]
    } else {
        parsed
    }
}

/// Render an entry's language list back into the single-line field.
pub fn format_languages(languages: &[String]) -> String {
    languages.join(", ")
}

/// Indices (into `entries`) surviving the search query. Indices rather
/// than references so the row buttons can act on the real entry even
/// while a filter is applied.
pub fn filter_indices(entries: &[GlossaryEntry], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..entries.len()).collect();
    }
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| entry_matches(e, &q))
        .map(|(i, _)| i)
        .collect()
}

fn entry_matches(entry: &GlossaryEntry, q_lc: &str) -> bool {
    if entry.source.to_lowercase().contains(q_lc) || entry.target.to_lowercase().contains(q_lc) {
        return true;
    }
    if entry
        .languages
        .iter()
        .any(|l| l.to_lowercase().contains(q_lc))
    {
        return true;
    }
    entry
        .note
        .as_deref()
        .is_some_and(|n| n.to_lowercase().contains(q_lc))
}

/// A blank entry for the Add button. Scoped to all pairs, which is the
/// overwhelmingly common case (product names, acronyms).
pub fn blank_entry() -> GlossaryEntry {
    GlossaryEntry {
        source: String::new(),
        target: String::new(),
        languages: vec!["*".into()],
        note: None,
    }
}

/// Point the edit panel at `index`, refilling the text buffers from
/// that entry. Call this on every selection change — the buffers are
/// per-row state, and reusing a stale one would copy one row's
/// languages onto another.
pub fn begin_editing(model: &mut GlossaryModel, index: usize) {
    let Some(entry) = model.entries.get(index) else {
        return;
    };
    model.lang_buf = format_languages(&entry.languages);
    model.note_buf = entry.note.clone().unwrap_or_default();
    model.editing = Some(index);
}

/// Drop the edited row, keeping the edit panel pointed at something
/// sensible (the row that slid into its place, or nothing).
pub fn delete_entry(model: &mut GlossaryModel, index: usize) {
    if index >= model.entries.len() {
        return;
    }
    model.entries.remove(index);
    match model.editing {
        Some(editing) if editing == index => {
            if index < model.entries.len() {
                begin_editing(model, index);
            } else if model.entries.is_empty() {
                model.editing = None;
            } else {
                begin_editing(model, model.entries.len() - 1);
            }
        }
        // Rows after the removed one shifted down by one.
        Some(editing) if editing > index => model.editing = Some(editing - 1),
        _ => {}
    }
}

/// Paint the editor. Returns an outcome iff the user asked for a
/// transition this frame; the App routes it (`app/glossary.rs`).
pub fn draw(ctx: &egui::Context, model: &mut GlossaryModel) -> Option<GlossaryOutcome> {
    let mut outcome: Option<GlossaryOutcome> = None;

    // Keyboard first, so a shortcut wins over whatever widget has
    // focus. Esc on a dirty form opens the confirmation instead of
    // discarding — this window holds typed work, like Settings.
    let (esc, save_combo) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::S) && (i.modifiers.command || i.modifiers.ctrl),
        )
    });
    if esc {
        if model.confirm_discard {
            model.confirm_discard = false;
        } else if model.dirty() {
            model.confirm_discard = true;
        } else {
            outcome = Some(GlossaryOutcome::Close);
        }
    } else if save_combo {
        outcome = Some(GlossaryOutcome::Save);
    }

    // Footer and edit panel are pinned to the bottom, so the table can
    // claim whatever height is left instead of leaving a dead band
    // between a short list and the buttons.
    egui::TopBottomPanel::bottom("glossary_footer")
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(18, 12)),
        )
        .show_separator_line(true)
        .show(ctx, |ui| draw_footer(ui, model, &mut outcome));

    if model.editing.is_some() {
        egui::TopBottomPanel::bottom("glossary_edit_panel")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 10)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| draw_edit_panel(ui, model));
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(18.0))
        .show(ctx, |ui| {
            draw_header(ui, model);
            draw_banners(ui, model);
            draw_toolbar(ui, model);
            ui.separator();
            draw_table(ui, model);
        });

    if model.confirm_discard {
        draw_discard_modal(ctx, model, &mut outcome);
    }

    outcome
}

fn draw_header(ui: &mut egui::Ui, model: &GlossaryModel) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Glossary")
                .color(theme::INK)
                .strong()
                .size(15.0),
        );
        ui.add_space(8.0);
        let count = model.entries.len();
        let plural = if count == 1 { "entry" } else { "entries" };
        ui.label(
            RichText::new(format!("{count} {plural}"))
                .color(theme::INK_3)
                .size(11.5),
        );
        if model.dirty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("● unsaved")
                    .color(theme::WARN)
                    .monospace()
                    .size(11.0),
            );
        }
    });
    ui.label(
        RichText::new(&model.path_display)
            .color(theme::INK_3)
            .monospace()
            .size(10.5),
    );
    ui.add_space(10.0);
}

fn draw_banners(ui: &mut egui::Ui, model: &GlossaryModel) {
    if let Some(err) = model.load_error.as_deref() {
        banner(
            ui,
            theme::BAD,
            &format!(
                "{err}\n\nThe file could not be parsed, so the table below starts empty. \
                 Saving replaces the file with the entries you build here."
            ),
        );
    }
    if model.comments_will_be_dropped {
        banner(
            ui,
            theme::WARN,
            "This file has comments. Saving from this editor rewrites it from the \
             entries below, which drops them. Use \"Open glossary\" from the tray to \
             edit the file directly instead.",
        );
    }
}

fn banner(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            20,
        ))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 64),
        ))
        .corner_radius(6.0)
        .inner_margin(9.0)
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).color(color).size(11.5)).wrap());
        });
    ui.add_space(8.0);
}

fn draw_toolbar(ui: &mut egui::Ui, model: &mut GlossaryModel) {
    ui.horizontal(|ui| {
        let edit = TextEdit::singleline(&mut model.query)
            .hint_text("Search entries")
            .desired_width(ui.available_width() - 120.0);
        let resp = ui.add(edit);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search entries")
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add = egui::Button::new(
                RichText::new("+ Add entry")
                    .color(theme::ACCENT_INK)
                    .strong()
                    .size(12.0),
            )
            .fill(theme::ACCENT);
            if ui.add(add).clicked() {
                // A blank row matches no non-empty query, so clear the
                // filter — otherwise Add appears to do nothing.
                model.query.clear();
                model.entries.push(blank_entry());
                begin_editing(model, model.entries.len() - 1);
            }
        });
    });
    ui.add_space(6.0);
}

fn draw_table(ui: &mut egui::Ui, model: &mut GlossaryModel) {
    let visible = filter_indices(&model.entries, &model.query);
    // Deferred so the row loop can borrow `model.entries` immutably.
    let mut edit_request: Option<usize> = None;
    let mut delete_request: Option<usize> = None;

    ScrollArea::vertical()
        .id_salt("glossary_rows")
        .max_height(240.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if model.entries.is_empty() {
                ui.add_space(20.0);
                ui.label(
                    RichText::new("No entries yet. \"+ Add entry\" creates one.")
                        .color(theme::INK_3)
                        .size(12.0),
                );
                return;
            }
            if visible.is_empty() {
                ui.add_space(20.0);
                ui.label(RichText::new("No matches.").color(theme::INK_3).size(12.0));
                return;
            }
            for &index in &visible {
                let entry = &model.entries[index];
                let active = model.editing == Some(index);
                let bg = if active {
                    Color32::from_rgba_unmultiplied(200, 255, 94, 16)
                } else {
                    Color32::TRANSPARENT
                };
                egui::Frame::new()
                    .fill(bg)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if active { "▸" } else { " " })
                                    .color(theme::ACCENT)
                                    .monospace(),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(row_term(&entry.source))
                                    .color(theme::INK)
                                    .monospace()
                                    .size(11.5),
                            );
                            ui.label(
                                RichText::new("→")
                                    .color(theme::INK_3)
                                    .monospace()
                                    .size(11.5),
                            );
                            ui.label(
                                RichText::new(row_term(&entry.target))
                                    .color(theme::INK_2)
                                    .monospace()
                                    .size(11.5),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format_languages(&entry.languages))
                                    .color(theme::ACCENT)
                                    .monospace()
                                    .size(11.0),
                            );

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Word buttons rather than ✎/✕
                                    // glyphs: the bundled font has no
                                    // coverage for those, so they paint
                                    // as empty boxes.
                                    if row_button(ui, "delete", &entry.source).clicked() {
                                        delete_request = Some(index);
                                    }
                                    if row_button(ui, "edit", &entry.source).clicked() {
                                        edit_request = Some(index);
                                    }
                                },
                            );
                        });
                    });
            }
        });

    // Delete first: an index captured this frame refers to the
    // pre-delete vector, so applying an edit afterwards would point at
    // the wrong row. Only one can fire per frame in practice.
    if let Some(index) = delete_request {
        delete_entry(model, index);
    } else if let Some(index) = edit_request {
        begin_editing(model, index);
    }
}

/// A per-row action button. The accessible name carries the row's term
/// ("Edit Vorgang"), so a screen reader — and the kittest suite — can
/// tell one row's buttons from another's.
fn row_button(ui: &mut egui::Ui, action: &str, source: &str) -> egui::Response {
    let label = format!(
        "{}{} {}",
        action[..1].to_uppercase(),
        &action[1..],
        row_term(source)
    );
    let resp = ui.button(
        RichText::new(action)
            .color(theme::INK_3)
            .monospace()
            .size(10.5),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp
}

/// Row display for a term, with a placeholder for the blank row Add
/// creates — an empty cell reads as a rendering bug.
fn row_term(term: &str) -> String {
    if term.trim().is_empty() {
        "(empty)".into()
    } else {
        term.to_owned()
    }
}

fn draw_edit_panel(ui: &mut egui::Ui, model: &mut GlossaryModel) {
    ui.add_space(10.0);
    let Some(index) = model.editing else {
        ui.label(
            RichText::new("Select a row's ✎ to edit it.")
                .color(theme::INK_3)
                .size(11.5),
        );
        return;
    };
    if index >= model.entries.len() {
        model.editing = None;
        return;
    }

    // Split the buffers off `model` so the entry can be borrowed
    // mutably alongside them.
    let GlossaryModel {
        entries,
        lang_buf,
        note_buf,
        ..
    } = model;
    let entry = &mut entries[index];

    egui::Frame::new()
        .fill(theme::PANEL_2)
        .stroke(Stroke::new(1.0_f32, theme::LINE_SOFT))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            field_row(
                ui,
                "source",
                &mut entry.source,
                "Term as it appears in text",
            );
            field_row(ui, "target", &mut entry.target, "Mandated translation");
            field_row(ui, "languages", lang_buf, "* or de->en, comma separated");
            field_row(ui, "note", note_buf, "Optional hint for the model");

            ui.add_space(2.0);
            ui.label(
                RichText::new(
                    "languages: \"*\" applies to every pair; \"de->en\" scopes to one direction.",
                )
                .color(theme::INK_3)
                .size(10.5),
            );
        });

    // Push the buffers back into the entry every frame — the fields are
    // the source of truth while the panel is open.
    entry.languages = parse_languages(lang_buf);
    entry.note = if note_buf.trim().is_empty() {
        None
    } else {
        Some(note_buf.clone())
    };
}

/// One labelled field. The label sits in a fixed-width column so the
/// four inputs line up regardless of label length.
fn field_row(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(76.0, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(
                    RichText::new(label)
                        .color(theme::INK_3)
                        .monospace()
                        .size(11.5),
                );
            },
        );
        let resp = ui.add(
            TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(ui.available_width()),
        );
        let name = label.to_owned();
        resp.widget_info(move || {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &name)
        });
    });
}

fn draw_footer(
    ui: &mut egui::Ui,
    model: &mut GlossaryModel,
    outcome: &mut Option<GlossaryOutcome>,
) {
    if !model.err_msg.is_empty() {
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 118, 118, 20))
            .stroke(Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(255, 118, 118, 64),
            ))
            .corner_radius(6.0)
            .inner_margin(9.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&model.err_msg)
                            .color(theme::BAD)
                            .monospace()
                            .size(11.5),
                    )
                    .wrap(),
                );
            });
        ui.add_space(6.0);
    }

    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            // Route through the same dirty check as Esc so a stray
            // click cannot silently discard the working copy.
            if model.dirty() {
                model.confirm_discard = true;
            } else {
                *outcome = Some(GlossaryOutcome::Close);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn = egui::Button::new(RichText::new("Save").color(theme::ACCENT_INK).strong())
                .fill(theme::ACCENT);
            if ui.add(btn).clicked() {
                *outcome = Some(GlossaryOutcome::Save);
            }
            // Not "Esc discards": with unsaved edits, Esc asks first.
            let hint = if model.dirty() {
                "Esc asks before discarding"
            } else {
                "Esc closes"
            };
            ui.label(RichText::new(hint).color(theme::INK_3).size(11.0));
        });
    });
}

fn draw_discard_modal(
    ctx: &egui::Context,
    model: &mut GlossaryModel,
    outcome: &mut Option<GlossaryOutcome>,
) {
    egui::Window::new("glossary_discard_confirm")
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
                RichText::new("Discard glossary changes?")
                    .color(theme::INK)
                    .strong()
                    .size(14.0),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Your edits have not been written to glossary.toml.")
                    .color(theme::INK_2)
                    .size(12.5),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Keep editing").clicked() {
                    model.confirm_discard = false;
                }
                ui.add_space(8.0);
                let danger =
                    egui::Button::new(RichText::new("Discard").color(theme::ACCENT_INK).strong())
                        .fill(theme::BAD);
                if ui.add(danger).clicked() {
                    model.confirm_discard = false;
                    *outcome = Some(GlossaryOutcome::Close);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save =
                        egui::Button::new(RichText::new("Save").color(theme::ACCENT_INK).strong())
                            .fill(theme::ACCENT);
                    if ui.add(save).clicked() {
                        model.confirm_discard = false;
                        *outcome = Some(GlossaryOutcome::Save);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, target: &str) -> GlossaryEntry {
        GlossaryEntry {
            source: source.into(),
            target: target.into(),
            languages: vec!["*".into()],
            note: None,
        }
    }

    #[test]
    fn empty_language_field_means_all_pairs() {
        assert_eq!(parse_languages(""), vec!["*".to_string()]);
        assert_eq!(parse_languages("  , ,"), vec!["*".to_string()]);
    }

    #[test]
    fn language_field_splits_and_trims() {
        assert_eq!(
            parse_languages(" de->en , tr->en "),
            vec!["de->en".to_string(), "tr->en".to_string()]
        );
    }

    #[test]
    fn language_field_round_trips() {
        let langs = vec!["de->en".to_string(), "*".to_string()];
        assert_eq!(parse_languages(&format_languages(&langs)), langs);
    }

    #[test]
    fn filter_matches_source_target_languages_and_note() {
        let mut noted = entry("SLA", "SLA");
        noted.note = Some("service level agreement".into());
        let mut scoped = entry("Vorgang", "case");
        scoped.languages = vec!["de->en".into()];
        let entries = vec![entry("Smart Table", "Smart Table"), noted, scoped];

        assert_eq!(filter_indices(&entries, ""), vec![0, 1, 2]);
        assert_eq!(filter_indices(&entries, "smart"), vec![0]);
        assert_eq!(filter_indices(&entries, "service"), vec![1]);
        assert_eq!(filter_indices(&entries, "de->en"), vec![2]);
        assert!(filter_indices(&entries, "nothing").is_empty());
    }

    #[test]
    fn dirty_tracks_the_working_copy_against_the_snapshot() {
        let entries = vec![entry("a", "b")];
        let mut model = GlossaryModel {
            entries: entries.clone(),
            original: entries,
            ..Default::default()
        };
        assert!(!model.dirty());
        model.entries[0].target = "c".into();
        assert!(model.dirty());
        model.entries[0].target = "b".into();
        assert!(!model.dirty(), "reverting an edit clears dirty");
    }

    #[test]
    fn begin_editing_refills_buffers_from_the_selected_row() {
        let mut scoped = entry("Vorgang", "case");
        scoped.languages = vec!["de->en".into()];
        scoped.note = Some("legal term".into());
        let mut model = GlossaryModel {
            entries: vec![entry("a", "b"), scoped],
            lang_buf: "stale".into(),
            note_buf: "stale".into(),
            ..Default::default()
        };
        begin_editing(&mut model, 1);
        assert_eq!(model.editing, Some(1));
        assert_eq!(model.lang_buf, "de->en");
        assert_eq!(model.note_buf, "legal term");
    }

    #[test]
    fn deleting_the_edited_row_moves_the_panel_to_the_next_row() {
        let mut model = GlossaryModel {
            entries: vec![entry("a", "a"), entry("b", "b"), entry("c", "c")],
            ..Default::default()
        };
        begin_editing(&mut model, 1);
        delete_entry(&mut model, 1);
        assert_eq!(model.entries.len(), 2);
        assert_eq!(model.editing, Some(1));
        assert_eq!(
            model.entries[1].source, "c",
            "panel follows the row that slid up"
        );
    }

    #[test]
    fn deleting_the_last_row_clears_the_edit_panel() {
        let mut model = GlossaryModel {
            entries: vec![entry("a", "a")],
            ..Default::default()
        };
        begin_editing(&mut model, 0);
        delete_entry(&mut model, 0);
        assert!(model.entries.is_empty());
        assert_eq!(model.editing, None);
    }

    #[test]
    fn deleting_a_row_above_the_edited_one_shifts_the_index() {
        let mut model = GlossaryModel {
            entries: vec![entry("a", "a"), entry("b", "b"), entry("c", "c")],
            ..Default::default()
        };
        begin_editing(&mut model, 2);
        delete_entry(&mut model, 0);
        assert_eq!(model.editing, Some(1));
        assert_eq!(model.entries[1].source, "c");
    }
}
