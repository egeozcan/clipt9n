pub mod custom_prompt;
pub mod history;
pub mod prompt;
pub mod setup;
pub mod size_confirm;
pub mod theme;
pub mod translating;

use egui::Vec2;

use crate::config::UiConfig;

/// Inner-size of the prompt window for the configured UI density.
/// Centralized so M5's `dismiss_history_to_idle` and M6's
/// `dismiss_setup_to_idle` can both call this rather than duplicating
/// the magic numbers (520×470 normal / 460×470 compact).
pub fn prompt_default_inner_size(ui: &UiConfig) -> Vec2 {
    let w = if ui.density == "compact" {
        460.0
    } else {
        520.0
    };
    Vec2::new(w, 470.0)
}
