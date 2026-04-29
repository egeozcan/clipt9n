//! Setup wizard view. Full implementation lands in Task 7. This stub
//! exists so the `pub mod setup` declaration in `ui/mod.rs` resolves
//! during Task 6's incremental commits.

use egui::Vec2;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Default)]
pub struct SetupWizardModel {
    pub provider: String,
    pub key: Zeroizing<String>,
    pub show_key: bool,
    pub storage: Storage,
    pub test_translation: bool,
    pub phase: WizardPhase,
    pub check1: CheckStatus,
    pub check2: CheckStatus,
    pub err_msg: String,
    pub keychain_available: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Storage {
    #[default]
    Keychain,
    Env,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WizardPhase {
    #[default]
    Entry,
    Verifying,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckStatus {
    #[default]
    Idle,
    Running,
    Ok,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupOutcome {
    Cancel,
    Verify,
    SaveAndStart,
    OpenConfig,
}

/// Default setup-wizard viewport size (matches design's 580×640).
pub const SETUP_WIZARD_INNER_SIZE: Vec2 = Vec2::new(580.0, 640.0);

/// Painted in Task 7. The stub returns `None` so the App's match arm
/// compiles cleanly during the incremental Task 6 commit.
pub fn draw(_ctx: &egui::Context, _model: &mut SetupWizardModel) -> Option<SetupOutcome> {
    None
}
