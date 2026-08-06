//! Channel draining, tray event dispatch, glossary reload, and
//! tray-menu action handlers. Extracted from `app/mod.rs` Step 6a
//! of the improvement plan.

use crate::platform::Platform;

use egui::ViewportCommand;

use super::pure;
use super::AppState;

impl super::ClipApp {
    pub(super) fn drain_channels(&mut self, ctx: &egui::Context) {
        // Hotkey events
        while let Ok(event) = self.hotkey_rx.try_recv() {
            let is_prompt = event.id == self.prompt_hotkey_id;
            let is_history = self
                .history_hotkey_id
                .map(|id| event.id == id)
                .unwrap_or(false);
            let is_selection = self
                .selection_hotkey_id
                .map(|id| event.id == id)
                .unwrap_or(false);
            let is_replace = self
                .replace_hotkey_id
                .map(|id| event.id == id)
                .unwrap_or(false);
            if is_prompt {
                if matches!(self.app_state, AppState::Idle) {
                    self.show_window(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else if is_history {
                if matches!(self.app_state, AppState::Idle) {
                    self.summon_history(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else if is_selection {
                if matches!(self.app_state, AppState::Idle) {
                    self.show_window_from_selection(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            } else if is_replace {
                if matches!(self.app_state, AppState::Idle) {
                    self.replace_selection_inline(ctx);
                }
            } else {
                tracing::debug!(
                    event_id = event.id,
                    "ignoring hotkey event from unregistered ID"
                );
            }
        }
        // Translation results
        while let Ok(outcome) = self.result_rx.try_recv() {
            self.handle_translation_done(outcome, ctx);
        }
        // Glossary reload requests (SIGHUP, tray menu in M7)
        let mut reload_requested = false;
        while self.glossary_reload_rx.try_recv().is_ok() {
            reload_requested = true;
        }
        if reload_requested {
            self.reload_glossary();
        }
    }

    pub(super) fn reload_glossary(&mut self) {
        // Sync I/O on the egui update thread is acceptable here because
        // glossary files are small (typically <10KB) and the reload is
        // user-driven (SIGHUP / future tray menu), not periodic.
        match crate::glossary::Glossary::load(&self.glossary_path) {
            Ok(g) => {
                let entry_count = g.len();
                let is_non_empty = !g.is_empty();
                *crate::glossary::Glossary::write_shared(&self.glossary) = g;
                tracing::info!(
                    path = %self.glossary_path.display(),
                    entries = entry_count,
                    "glossary reloaded"
                );
                // Clear the malformed flag if the reload produced a
                // valid, non-empty glossary.
                if is_non_empty {
                    self.set_glossary_malformed(false);
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %self.glossary_path.display(),
                    "glossary reload failed; keeping previous entries"
                );
            }
        }
    }

    /// Drain pending tray menu events and dispatch. Called once per
    /// frame from `update()`. The static `MenuEvent` channel is drained
    /// unconditionally — events queued from a previous tray that's
    /// since been dropped would otherwise accumulate forever and fire
    /// stale on a future re-attach (Task 7's --show-tray recovery
    /// path). Dispatch is gated on `self.tray.is_some()`.
    pub(super) fn drain_tray_events(&mut self, ctx: &egui::Context) {
        // Drain the global static MenuEvent channel unconditionally —
        // events queued from a previous tray that's since been dropped
        // would otherwise accumulate forever and fire stale on a future
        // re-attach (Task 7's --show-tray recovery path).
        let Some(id) = crate::tray::TrayHandle::try_drain_menu_event() else {
            return;
        };
        if self.tray.is_none() {
            // Tray not active; silently discard.
            return;
        }
        match id.as_str() {
            crate::tray::ID_TRANSLATE => {
                // Same gate as the hotkey path: only show the prompt from
                // Idle. Otherwise, the user is mid-wizard / mid-history /
                // mid-translate; bring focus to whatever's open.
                if matches!(self.app_state, AppState::Idle) {
                    self.show_window(ctx);
                } else {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
            }
            crate::tray::ID_HISTORY => self.summon_history(ctx),
            crate::tray::ID_GLOSSARY_OPEN => self.dispatch_open_glossary(),
            crate::tray::ID_GLOSSARY_RELOAD => self.dispatch_reload_glossary(),
            crate::tray::ID_SETTINGS => self.dispatch_open_settings(ctx),
            crate::tray::ID_OPEN_CONFIG => self.dispatch_open_config(),
            crate::tray::ID_ACCESSIBILITY_SETTINGS => self.dispatch_open_accessibility_settings(),
            crate::tray::ID_RERUN_WIZARD => self.dispatch_rerun_wizard(ctx),
            crate::tray::ID_HIDE => self.dispatch_hide_tray_request(ctx),
            crate::tray::ID_QUIT => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            other => {
                tracing::debug!(id = %other, "tray menu event (handler not yet wired)");
            }
        }
    }

    fn dispatch_open_glossary(&self) {
        match crate::platform::current().open_path(&self.glossary_path) {
            Ok(()) => tracing::info!(path = %self.glossary_path.display(), "tray: opened glossary"),
            Err(e) => tracing::warn!(error = %e, "tray: open glossary failed"),
        }
    }

    fn dispatch_reload_glossary(&self) {
        let Some(tx) = self.glossary_reload_tx.as_ref() else {
            tracing::warn!("tray: reload glossary requested but no reload channel");
            return;
        };
        if let Err(e) = tx.send(()) {
            tracing::warn!(error = %e, "tray: reload glossary send failed");
        }
    }

    pub(super) fn dispatch_open_config(&self) {
        let cfg_path = self.config_path().to_path_buf();
        // Create an empty config file if it doesn't exist yet, so the
        // user can edit it immediately.
        if !cfg_path.exists() {
            if let Err(e) = std::fs::write(&cfg_path, "") {
                tracing::warn!(error = %e, path = %cfg_path.display(), "tray: failed to create config file");
                return;
            }
            tracing::info!(path = %cfg_path.display(), "tray: created empty config file");
        }
        match crate::platform::current().open_path(&cfg_path) {
            Ok(()) => tracing::info!(path = %cfg_path.display(), "tray: opened config"),
            Err(e) => tracing::warn!(error = %e, "tray: open config failed"),
        }
    }

    fn dispatch_open_accessibility_settings(&self) {
        match crate::platform::current().open_accessibility_settings() {
            Ok(()) => tracing::info!("tray: opened accessibility settings"),
            Err(e) => tracing::warn!(error = %e, "tray: open accessibility settings failed"),
        }
    }

    pub(super) fn dispatch_rerun_wizard(&mut self, ctx: &egui::Context) {
        tracing::info!(
            current_state = match &self.app_state {
                AppState::Idle => "idle",
                AppState::Showing => "showing",
                AppState::EnteringCustom { .. } => "entering_custom",
                AppState::ConfirmingSize { .. } => "confirming_size",
                AppState::Translating { .. } => "translating",
                AppState::TranslatingInline { .. } => "translating_inline",
                AppState::ShowingHistory { .. } => "showing_history",
                AppState::SetupWizard { .. } => "setup_wizard",
                AppState::Settings { .. } => "settings",
                AppState::ConfirmingTrayHide { .. } => "confirming_tray_hide",
                AppState::ShowingResult { .. } => "showing_result",
            },
            "dispatch_rerun_wizard"
        );
        // Already in the wizard? Don't reseed the model (would lose
        // the in-flight key); just bring the window back to the
        // foreground so the user can resume editing after they
        // tabbed away. With NSApplicationActivationPolicyAccessory
        // the window has no Dock icon to click, so the tray menu's
        // "Re-run setup wizard" item is the only re-focus path.
        if matches!(self.app_state, AppState::SetupWizard { .. }) {
            pure::reset_focus_loss_latch(&mut self.has_been_focused);
            self.set_window_visible(ctx, true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            crate::platform::current().activate_app();
            return;
        }
        // Probe the platform directly — same reason as main.rs's
        // first-launch path: an EnvSecrets-backed `self.secrets`
        // always reports `false` regardless of whether the OS
        // keychain is actually reachable.
        let keychain_available = crate::secrets::keychain_probe(&self.cfg.provider.api_key.service);
        let storage = if keychain_available {
            crate::ui::setup::Storage::Keychain
        } else {
            crate::ui::setup::Storage::Env
        };
        let model = crate::ui::setup::SetupWizardModel {
            provider: self.cfg.provider.kind.clone(),
            keychain_available,
            storage,
            test_translation: keychain_available, // env-only mode skips the live test
            ..Default::default()
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
            crate::ui::setup::SETUP_WIZARD_INNER_SIZE,
        ));
        pure::reset_focus_loss_latch(&mut self.has_been_focused);
        self.set_window_visible(ctx, true);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        crate::platform::current().activate_app();
        self.app_state = AppState::SetupWizard { model };
    }
}
