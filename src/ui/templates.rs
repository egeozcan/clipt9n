//! Prompt-template editor — a text editor over the four `.j2` override
//! files in `<config_dir>/templates/`.
//!
//! Pure view plus pure helpers. Every effect — reading the files,
//! validating, writing, deleting, swapping the live `Templates` — lives
//! in `src/app/templates.rs`, mirroring the `ui/glossary.rs` ↔
//! `app/glossary.rs` split.
//!
//! Like the glossary editor, the model is a *working copy*: nothing
//! reaches disk until Save validates all four sources, and Cancel
//! discards the lot. That is what lets "Reset to default" skip its own
//! confirmation — it only rewrites a buffer, and the user can still
//! back out with Cancel.
//!
//! The one structural difference from the glossary: there is no add or
//! delete. The four kinds are fixed, and "customized" is not a stored
//! flag but a comparison — a source that equals its built-in *is* the
//! built-in, and Save removes the override file rather than writing a
//! copy of what the binary already ships.

use egui::{Color32, RichText, ScrollArea, Stroke, TextEdit, Vec2};

use crate::llm::templates::TemplateKind;
use crate::ui::theme;

/// Inner size of the editor window. Wider and taller than the glossary
/// (680×580) because the payload is prose: a template runs 10-15 lines
/// and reads badly in a narrow column.
pub const TEMPLATES_INNER_SIZE: Vec2 = Vec2::new(760.0, 640.0);

/// One kind's editing state.
#[derive(Debug, Clone)]
pub struct TemplateSlot {
    pub kind: TemplateKind,
    /// Working copy of the template source. Edited freely; only
    /// committed on Save.
    pub source: String,
    /// Snapshot taken when the window opened. `dirty()` diffs against
    /// it, so typing a change back out again clears the flag.
    pub original: String,
    /// Override path from `[templates]`, relative to the config dir.
    /// Empty means config explicitly disabled the override for this
    /// kind, which makes it read-only here — see `read_only`.
    pub rel_path: String,
    /// Set when the override file exists but could not be read. The
    /// buffer falls back to the built-in and the banner says why.
    pub load_error: Option<String>,
}

impl TemplateSlot {
    /// Whether this buffer differs from the built-in the binary ships.
    /// Drives the "customized" marker *and* the Save decision: a slot
    /// that is not customized has its override file removed.
    pub fn customized(&self) -> bool {
        self.source != self.kind.built_in_source()
    }

    /// A kind whose config path is empty cannot be edited here: writing
    /// `templates/translate.j2` while config says the override is off
    /// would produce a file the loader never reads, which is a worse
    /// outcome than refusing.
    ///
    /// Only reachable by hand-editing config.toml to `translate = ""`;
    /// an absent key falls back to `TemplatesConfig::default()`, which
    /// fills all four in.
    pub fn read_only(&self) -> bool {
        self.rel_path.is_empty()
    }
}

/// What the editor paints per frame.
#[derive(Debug, Clone)]
pub struct TemplatesModel {
    /// One slot per kind, in `TemplateKind::all()` order.
    pub slots: Vec<TemplateSlot>,
    /// Index into `slots` of the kind currently in the text area.
    pub selected: usize,
    /// Save rejection (validation or write failure), rendered in the
    /// footer. Empty means no error.
    pub err_msg: String,
    /// `<config_dir>/templates/`, shown under the title.
    pub dir_display: String,
    /// Whether the preview pane is showing.
    pub preview_open: bool,
    /// Whether the unsaved-changes confirmation is up.
    pub confirm_discard: bool,
}

impl Default for TemplatesModel {
    fn default() -> Self {
        Self {
            slots: TemplateKind::all()
                .into_iter()
                .map(|kind| TemplateSlot {
                    kind,
                    source: kind.built_in_source().to_string(),
                    original: kind.built_in_source().to_string(),
                    rel_path: kind.default_rel_path(),
                    load_error: None,
                })
                .collect(),
            selected: 0,
            err_msg: String::new(),
            dir_display: String::new(),
            preview_open: false,
            confirm_discard: false,
        }
    }
}

impl TemplatesModel {
    /// Whether any working copy differs from what was loaded.
    pub fn dirty(&self) -> bool {
        self.slots.iter().any(|s| s.source != s.original)
    }

    pub fn selected_slot(&self) -> &TemplateSlot {
        &self.slots[self.selected]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplatesOutcome {
    /// Save button or Cmd/Ctrl+S — validate all four, write, reload.
    Save,
    /// Cancel, Esc on a clean form, or Discard in the confirmation —
    /// throw the working copies away.
    Close,
}

/// Summary line for the kind list: what this slot will do on Save.
pub fn slot_status(slot: &TemplateSlot) -> &'static str {
    if slot.read_only() {
        "off"
    } else if slot.customized() {
        "custom"
    } else {
        "default"
    }
}

/// Paint the editor. Returns an outcome iff the user asked for a
/// transition this frame; the App routes it (`app/templates.rs`).
pub fn draw(ctx: &egui::Context, model: &mut TemplatesModel) -> Option<TemplatesOutcome> {
    let mut outcome: Option<TemplatesOutcome> = None;

    // Keyboard first, so a shortcut wins over whatever widget has
    // focus. Esc on a dirty form opens the confirmation instead of
    // discarding — this window holds typed work, like Settings.
    //
    // Cmd+S only: the text area swallows plain typing, and a template
    // legitimately contains any character, so there is nothing else
    // safe to bind.
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
            outcome = Some(TemplatesOutcome::Close);
        }
    } else if save_combo {
        outcome = Some(TemplatesOutcome::Save);
    }

    egui::TopBottomPanel::bottom("templates_footer")
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(18, 12)),
        )
        .show_separator_line(true)
        .show(ctx, |ui| draw_footer(ui, model, &mut outcome));

    // Declared after the footer but before the panels below, so the kind
    // list spans the full height of the window body while the variables
    // strip and preview stay in the editor's column.
    egui::SidePanel::left("templates_kinds")
        .resizable(false)
        .exact_width(150.0)
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(12, 18)),
        )
        .show(ctx, |ui| draw_kind_list(ui, model));

    if model.preview_open {
        egui::TopBottomPanel::bottom("templates_preview")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 10)),
            )
            .show_separator_line(false)
            .show(ctx, |ui| draw_preview(ui, model));
    }

    // Pinned below the editor rather than drawn after it, so the text
    // area can claim every remaining pixel instead of leaving a dead
    // band between a short template and the hint.
    egui::TopBottomPanel::bottom("templates_variables")
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(18, 8)),
        )
        .show_separator_line(false)
        .show(ctx, |ui| draw_variables(ui, model));

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(18, 18)),
        )
        .show(ctx, |ui| {
            draw_header(ui, model);
            draw_banners(ui, model);
            draw_source_editor(ui, model);
        });

    if model.confirm_discard {
        draw_discard_modal(ctx, model, &mut outcome);
    }

    outcome
}

fn draw_kind_list(ui: &mut egui::Ui, model: &mut TemplatesModel) {
    ui.label(
        RichText::new("Templates")
            .color(theme::INK)
            .strong()
            .size(15.0),
    );
    ui.add_space(10.0);

    for index in 0..model.slots.len() {
        if draw_kind_row(ui, &model.slots[index], model.selected == index) {
            model.selected = index;
        }
    }
}

/// One row of the kind list. Returns true when it was clicked.
///
/// Painted by hand rather than with `SelectableLabel`, for two reasons.
/// `add_sized` justifies its widget, which centers the text in a column
/// where every other row of the window is left-aligned; and the built-in
/// selection fill is a flat wash that leaves accent-colored text sitting
/// on a near-matching background. An accent bar plus a tinted row gives
/// the selection a shape at full text contrast.
fn draw_kind_row(ui: &mut egui::Ui, slot: &TemplateSlot, active: bool) -> bool {
    let label = slot.kind.label();
    let status = slot_status(slot);
    // `status` describes the file; `changed` describes this session. A
    // row can be both ("custom" on disk, edited again since opening).
    let changed = slot.source != slot.original;

    let row_height = if changed || status != "default" {
        34.0
    } else {
        26.0
    };
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), row_height),
        egui::Sense::click(),
    );

    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(200, 255, 94, 26));
        // The bar is what makes the selection legible without washing
        // the text out: shape carries the state, contrast stays intact.
        painter.rect_filled(
            egui::Rect::from_min_size(rect.left_top(), Vec2::new(3.0, rect.height())),
            1.0,
            theme::ACCENT,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, 4.0, theme::PANEL_2);
    }

    let text_x = rect.left() + 12.0;
    let has_sub = changed || status != "default";
    let title_y = if has_sub {
        rect.top() + 11.0
    } else {
        rect.center().y
    };
    painter.text(
        egui::pos2(text_x, title_y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        // Full-strength ink either way. The row background says which is
        // selected; dimming the others just makes them hard to read.
        if active { theme::ACCENT } else { theme::INK },
    );

    // Only annotate what differs from the plain case. Stamping "default"
    // on every row is noise that buries the one row that is not.
    if has_sub {
        let (note, color) = if changed {
            ("unsaved", theme::WARN)
        } else if status == "custom" {
            ("custom", theme::ACCENT)
        } else {
            ("override off", theme::INK_3)
        };
        painter.text(
            egui::pos2(text_x, rect.top() + 24.0),
            egui::Align2::LEFT_CENTER,
            note,
            egui::FontId::monospace(10.0),
            color,
        );
    }

    let name = format!("{label} template ({status})");
    resp.widget_info(move || {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, &name)
    });
    resp.clicked()
}

fn draw_header(ui: &mut egui::Ui, model: &TemplatesModel) {
    let slot = model.selected_slot();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(slot.kind.label())
                .color(theme::INK)
                .strong()
                .size(15.0),
        );
        ui.add_space(8.0);
        if model.dirty() {
            ui.label(
                RichText::new("unsaved changes")
                    .color(theme::WARN)
                    .monospace()
                    .size(11.0),
            );
        }
    });
    let path = if slot.read_only() {
        format!("{} — override disabled in config.toml", model.dir_display)
    } else {
        format!("{}{}", model.dir_display, file_name(&slot.rel_path))
    };
    ui.label(
        RichText::new(path)
            .color(theme::INK_3)
            .monospace()
            .size(10.5),
    );
    ui.add_space(10.0);
}

/// The file name portion of a configured override path, for display
/// next to the directory. Paths are relative and confined, so this is
/// presentation only.
fn file_name(rel_path: &str) -> String {
    std::path::Path::new(rel_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel_path.to_owned())
}

fn draw_banners(ui: &mut egui::Ui, model: &TemplatesModel) {
    let slot = model.selected_slot();
    if let Some(err) = slot.load_error.as_deref() {
        banner(
            ui,
            theme::BAD,
            &format!(
                "{err}\n\nThe editor loaded the built-in default instead. \
                 Saving replaces the file with what you see below."
            ),
        );
    }
    if slot.read_only() {
        banner(
            ui,
            theme::WARN,
            "config.toml sets this template's override path to an empty string, so the \
             built-in below is what runs and this editor has nowhere to write. Set \
             [templates] to a path under the config folder to edit it here.",
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

fn draw_source_editor(ui: &mut egui::Ui, model: &mut TemplatesModel) {
    let height = ui.available_height().max(120.0);
    // `TextEdit` sizes itself in rows, so an explicit count is the only
    // way to make it fill the panel; without it a short template leaves
    // a band of empty panel below the box.
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let rows = ((height / row_height).floor() as usize).max(6);
    let read_only = model.selected_slot().read_only();
    let label = format!("{} template source", model.selected_slot().kind.label());
    let slot = &mut model.slots[model.selected];

    ScrollArea::vertical()
        .id_salt("templates_source")
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let edit = TextEdit::multiline(&mut slot.source)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .desired_rows(rows)
                .interactive(!read_only)
                .hint_text("Template source (minijinja)");
            let resp = ui.add(edit);
            resp.widget_info(move || {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, !read_only, &label)
            });
        });
}

fn draw_variables(ui: &mut egui::Ui, model: &TemplatesModel) {
    let slot = model.selected_slot();
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("variables:").color(theme::INK_3).size(10.5));
        for name in slot.kind.variables() {
            ui.label(
                RichText::new(format!("{{{{ {name} }}}}"))
                    .color(theme::ACCENT)
                    .monospace()
                    .size(10.5),
            );
        }
    });
    ui.label(
        RichText::new(
            "Any other name is rejected on Save. Names outside this list render empty here.",
        )
        .color(theme::INK_3)
        .size(10.0),
    );
}

fn draw_preview(ui: &mut egui::Ui, model: &TemplatesModel) {
    let slot = model.selected_slot();
    ui.label(
        RichText::new("Preview — rendered with sample values")
            .color(theme::INK_3)
            .size(10.5),
    );
    ui.add_space(4.0);
    let (text, color) = match crate::llm::templates::render_preview(&slot.source, slot.kind) {
        Ok(rendered) => (rendered, theme::INK_2),
        Err(e) => (e.to_string(), theme::BAD),
    };
    egui::Frame::new()
        .fill(theme::PANEL_2)
        .stroke(Stroke::new(1.0_f32, theme::LINE_SOFT))
        .corner_radius(6.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("templates_preview_body")
                .max_height(150.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).color(color).monospace().size(11.0))
                            .wrap(),
                    );
                });
        });
}

fn draw_footer(
    ui: &mut egui::Ui,
    model: &mut TemplatesModel,
    outcome: &mut Option<TemplatesOutcome>,
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
            // click cannot silently discard the working copies.
            if model.dirty() {
                model.confirm_discard = true;
            } else {
                *outcome = Some(TemplatesOutcome::Close);
            }
        }
        ui.add_space(8.0);

        let slot = model.selected_slot();
        // No confirmation: this only rewrites a buffer, and Cancel
        // still backs the whole session out.
        let resettable = slot.customized() && !slot.read_only();
        if ui
            .add_enabled(resettable, egui::Button::new("Reset to default"))
            .clicked()
        {
            let kind = model.slots[model.selected].kind;
            model.slots[model.selected].source = kind.built_in_source().to_string();
        }
        ui.add_space(8.0);
        ui.checkbox(&mut model.preview_open, "Preview");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn = egui::Button::new(RichText::new("Save").color(theme::ACCENT_INK).strong())
                .fill(theme::ACCENT);
            if ui.add(btn).clicked() {
                *outcome = Some(TemplatesOutcome::Save);
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
    model: &mut TemplatesModel,
    outcome: &mut Option<TemplatesOutcome>,
) {
    egui::Window::new("templates_discard_confirm")
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
                RichText::new("Discard template changes?")
                    .color(theme::INK)
                    .strong()
                    .size(14.0),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Your edits have not been written to the templates folder.")
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
                    *outcome = Some(TemplatesOutcome::Close);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save =
                        egui::Button::new(RichText::new("Save").color(theme::ACCENT_INK).strong())
                            .fill(theme::ACCENT);
                    if ui.add(save).clicked() {
                        model.confirm_discard = false;
                        *outcome = Some(TemplatesOutcome::Save);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> TemplatesModel {
        TemplatesModel {
            dir_display: "/cfg/templates/".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_fresh_model_is_clean_and_uncustomized() {
        let m = model();
        assert!(!m.dirty());
        assert_eq!(m.slots.len(), 4);
        assert!(m.slots.iter().all(|s| !s.customized()));
        assert!(m.slots.iter().all(|s| slot_status(s) == "default"));
    }

    #[test]
    fn editing_a_slot_marks_the_model_dirty_and_customized() {
        let mut m = model();
        m.slots[0].source.push_str("\nextra");
        assert!(m.dirty());
        assert!(m.slots[0].customized());
        assert_eq!(slot_status(&m.slots[0]), "custom");
        // Other slots are unaffected.
        assert!(!m.slots[1].customized());
    }

    #[test]
    fn typing_a_change_back_out_clears_dirty() {
        let mut m = model();
        let before = m.slots[2].source.clone();
        m.slots[2].source.push('x');
        assert!(m.dirty());
        m.slots[2].source = before;
        assert!(!m.dirty());
    }

    /// A slot loaded from a customized file is dirty-free but still
    /// reports "custom" — the two flags answer different questions.
    #[test]
    fn a_loaded_override_is_customized_but_not_dirty() {
        let mut m = model();
        m.slots[1].source = "Custom text {{ glossary_block }}".into();
        m.slots[1].original = m.slots[1].source.clone();
        assert!(!m.dirty());
        assert!(m.slots[1].customized());
    }

    #[test]
    fn an_empty_configured_path_is_read_only() {
        let mut m = model();
        m.slots[0].rel_path = String::new();
        assert!(m.slots[0].read_only());
        assert_eq!(slot_status(&m.slots[0]), "off");
        assert!(!m.slots[1].read_only());
    }

    #[test]
    fn read_only_wins_over_customized_in_the_status_line() {
        let mut m = model();
        m.slots[0].rel_path = String::new();
        m.slots[0].source = "edited".into();
        assert_eq!(slot_status(&m.slots[0]), "off");
    }

    #[test]
    fn file_name_shows_the_leaf_of_a_nested_override_path() {
        assert_eq!(file_name("templates/translate.j2"), "translate.j2");
        assert_eq!(file_name("translate.j2"), "translate.j2");
    }

    #[test]
    fn every_kind_lists_its_own_variables() {
        let m = model();
        let translate = m.slots[0].kind.variables();
        assert!(translate.contains(&"target_language"));
        let custom = m.slots[3].kind.variables();
        assert!(custom.contains(&"user_instruction"));
        assert!(!custom.contains(&"target_language"));
        // glossary_block is available everywhere.
        assert!(m
            .slots
            .iter()
            .all(|s| s.kind.variables().contains(&"glossary_block")));
    }
}
