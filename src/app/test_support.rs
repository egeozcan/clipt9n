//! Shared test fixtures for the `app` sub-modules. A `ClipApp` has ~40
//! fields, most of them irrelevant to any single test; constructing one
//! per test module meant maintaining the same 60-line literal in
//! several places.

use std::path::PathBuf;
use std::sync::{mpsc, Arc, RwLock};

use async_trait::async_trait;

use super::AppState;

struct NoopProvider;

#[async_trait]
impl crate::llm::LlmProvider for NoopProvider {
    async fn complete(
        &self,
        _system: &str,
        _user: &str,
    ) -> Result<String, crate::error::TranslateError> {
        Ok(String::new())
    }
}

/// A `ClipApp` wired to inert doubles, with its glossary rooted at
/// `glossary_path`. Sibling paths (config.toml, state.toml) are derived
/// from it, so a single `tempdir` gives a test a complete config dir.
pub(super) fn test_app(glossary_path: PathBuf) -> super::ClipApp {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (_hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();
    let (result_tx, result_rx) = mpsc::channel();
    let (_reload_tx, glossary_reload_rx) = crossbeam_channel::unbounded();
    let (setup_check_tx, setup_check_rx) = mpsc::channel();
    let state_path = glossary_path.with_file_name("state.toml");

    super::ClipApp {
        cfg: crate::config::Config::default(),
        cfg_path: glossary_path.with_file_name("config.toml"),
        state_path,
        state: crate::state::State::default(),
        provider: Some(Arc::new(NoopProvider)),
        templates: Arc::new(crate::llm::templates::Templates::built_in()),
        glossary: Arc::new(RwLock::new(crate::glossary::Glossary::empty())),
        glossary_path,
        glossary_malformed: std::sync::atomic::AtomicBool::new(true),
        runtime,
        hotkey_rx,
        result_tx,
        result_rx,
        desktop_io: Box::new(crate::desktop_io::SystemDesktopIo),
        glossary_reload_rx,
        history: None,
        history_disabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        history_warned: std::sync::atomic::AtomicBool::new(false),
        secrets: Box::new(crate::secrets::EnvSecrets::new("CLIPT9N_TEST_KEY")),
        tray: None,
        tray_status_observer: None,
        setup_check_tx,
        setup_check_rx,
        setup_verification_gen: 0,
        accessibility_revoked: false,
        hotkey_in_use: false,
        prompt_hotkey_id: None,
        history_hotkey_id: None,
        selection_hotkey_id: None,
        replace_hotkey_id: None,
        app_state: AppState::Idle,
        prompt_model: crate::ui::prompt::PromptModel {
            clipboard_text: String::new(),
            detected_lang: None,
            last_slot: None,
            glossary_hits: Vec::new(),
        },
        has_been_focused: false,
        initial_focus_pending: false,
        dispatch_gen: 0,
        last_translation_at: None,
        reduced_motion: false,
        pending_preview: false,
        previous_app_pid: None,
        repaint_ctx: egui::Context::default(),
        last_sent_visible: None,
    }
}
