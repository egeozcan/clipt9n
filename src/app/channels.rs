//! Channel draining, tray event dispatch, glossary reload, and
//! tray-menu action handlers. Extracted from `app/mod.rs` Step 6a
//! of the improvement plan.

use crate::platform::Platform;

use egui::ViewportCommand;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};

use super::pure;
use super::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HotkeyAction {
    Prompt,
    History,
    Selection,
    Replace,
    Unknown(u32),
}

#[derive(Debug, Clone, Copy)]
struct HotkeyIds {
    prompt: Option<u32>,
    history: Option<u32>,
    selection: Option<u32>,
    replace: Option<u32>,
}

fn drain_hotkey_actions(
    receiver: &crossbeam_channel::Receiver<GlobalHotKeyEvent>,
    ids: HotkeyIds,
) -> Vec<HotkeyAction> {
    let mut actions = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        if event.state != HotKeyState::Pressed {
            continue;
        }
        let action = if ids.prompt == Some(event.id) {
            HotkeyAction::Prompt
        } else if ids.history == Some(event.id) {
            HotkeyAction::History
        } else if ids.selection == Some(event.id) {
            HotkeyAction::Selection
        } else if ids.replace == Some(event.id) {
            HotkeyAction::Replace
        } else {
            HotkeyAction::Unknown(event.id)
        };
        actions.push(action);
    }
    actions
}

impl super::ClipApp {
    pub(super) fn drain_channels(&mut self, ctx: &egui::Context) {
        // Hotkey events. Released events are discarded before ID routing,
        // so one physical key press can dispatch at most one app action.
        let ids = HotkeyIds {
            prompt: self.prompt_hotkey_id,
            history: self.history_hotkey_id,
            selection: self.selection_hotkey_id,
            replace: self.replace_hotkey_id,
        };
        for action in drain_hotkey_actions(&self.hotkey_rx, ids) {
            match action {
                HotkeyAction::Prompt => {
                    if matches!(self.app_state, AppState::Idle) {
                        self.show_window(ctx);
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Focus);
                    }
                }
                // No gate here: `summon_history` owns it, so the tray
                // menu and this path can't drift apart.
                HotkeyAction::History => self.summon_history(ctx),
                HotkeyAction::Selection => {
                    if matches!(self.app_state, AppState::Idle) {
                        self.show_window_from_selection(ctx);
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Focus);
                    }
                }
                HotkeyAction::Replace => {
                    if matches!(self.app_state, AppState::Idle) {
                        self.replace_selection_inline(ctx);
                    }
                }
                HotkeyAction::Unknown(event_id) => {
                    tracing::debug!(event_id, "ignoring hotkey event from unregistered ID");
                }
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
                *crate::glossary::Glossary::write_shared(&self.glossary) = g;
                tracing::info!(
                    path = %self.glossary_path.display(),
                    entries = entry_count,
                    "glossary reloaded"
                );
                // Any successfully parsed glossary is healthy, including an
                // intentionally empty file.
                self.set_glossary_malformed(false);
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
            crate::tray::ID_GLOSSARY_EDIT => self.dispatch_edit_glossary(ctx),
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

    fn dispatch_reload_glossary(&mut self) {
        // Tray actions already run on the update thread. Reload directly so
        // an idle app does not need a second repaint to drain its own signal.
        self.reload_glossary();
        self.refresh_tray_status();
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
            current_state = self.app_state.label(),
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
        // Refuse the states that own work this would destroy.
        //
        // A translation in flight has a worker whose outcome is matched
        // against the current state; replacing it here means a normal
        // translation never reaches the clipboard or history and an
        // inline replacement silently never pastes. The glossary editor
        // holds unsaved entries, and unlike the settings editor it has
        // no stake in the provider the wizard exists to fix — so there
        // is no reason for the wizard to win there, and every reason
        // not to destroy the user's typing.
        //
        // The settings editor is deliberately *not* on this list: it
        // edits the same provider the wizard is here to repair, so the
        // wizard taking over is the point.
        let busy = match &self.app_state {
            AppState::Translating { .. } | AppState::TranslatingInline { .. } => {
                Some("a translation is in flight")
            }
            AppState::ShowingGlossary { .. } => Some("the glossary editor is open"),
            _ => None,
        };
        if let Some(reason) = busy {
            tracing::info!(reason, "setup wizard request ignored");
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
        let mut model = crate::ui::setup::SetupWizardModel {
            provider: self.cfg.provider.kind.clone(),
            keychain_available,
            storage,
            test_translation: keychain_available, // env-only mode skips the live test
            ..Default::default()
        };
        super::setup::seed_setup_verification(&mut self.setup_verification_gen, &mut model);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use super::super::test_support::test_app;

    /// An app parked mid-translation, with the on-disk glossary the
    /// wizard's keychain probe path expects to exist.
    fn app_translating(state: AppState) -> (tempfile::TempDir, super::super::ClipApp) {
        let dir = tempfile::tempdir().unwrap();
        let glossary_path = dir.path().join("glossary.toml");
        std::fs::write(&glossary_path, "").unwrap();
        let mut app = test_app(glossary_path);
        app.app_state = state;
        (dir, app)
    }

    #[test]
    fn rerunning_the_wizard_does_not_clobber_an_in_flight_translation() {
        let (_dir, mut app) = app_translating(AppState::Translating {
            gen: 7,
            action_label: "Translate".into(),
            overlay_label: "Translating…".into(),
            started_at: std::time::Instant::now(),
            preview_mode: false,
        });

        app.dispatch_rerun_wizard(&egui::Context::default());

        match &app.app_state {
            AppState::Translating { gen, .. } => assert_eq!(
                *gen, 7,
                "the wizard must not replace the state the worker's outcome is matched against"
            ),
            other => panic!(
                "the wizard discarded a running translation: {}",
                other.label()
            ),
        }
    }

    #[test]
    fn rerunning_the_wizard_does_not_clobber_an_in_flight_inline_replacement() {
        let (_dir, mut app) = app_translating(AppState::TranslatingInline {
            gen: 3,
            action_label: "Replace".into(),
            started_at: std::time::Instant::now(),
            target: crate::desktop_io::DesktopTarget::for_test(42, 1),
        });

        app.dispatch_rerun_wizard(&egui::Context::default());

        assert!(
            matches!(app.app_state, AppState::TranslatingInline { gen: 3, .. }),
            "clobbering this state means the replacement silently never pastes"
        );
    }

    #[test]
    fn pressed_then_released_dispatches_one_action() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(GlobalHotKeyEvent {
            id: 10,
            state: HotKeyState::Pressed,
        })
        .unwrap();
        tx.send(GlobalHotKeyEvent {
            id: 10,
            state: HotKeyState::Released,
        })
        .unwrap();

        let actions = drain_hotkey_actions(
            &rx,
            HotkeyIds {
                prompt: Some(10),
                history: Some(11),
                selection: Some(12),
                replace: Some(13),
            },
        );

        assert_eq!(actions, vec![HotkeyAction::Prompt]);
    }

    #[test]
    fn valid_empty_glossary_clears_a_previous_malformed_status() {
        let dir = tempfile::tempdir().unwrap();
        let glossary_path = dir.path().join("glossary.toml");
        std::fs::write(&glossary_path, "").unwrap();
        let mut app = test_app(glossary_path);
        assert!(app
            .glossary_malformed
            .load(std::sync::atomic::Ordering::Relaxed));

        app.reload_glossary();

        assert!(!app
            .glossary_malformed
            .load(std::sync::atomic::Ordering::Relaxed));
        assert!(crate::glossary::Glossary::read_shared(&app.glossary).is_empty());
    }

    #[test]
    fn tray_reload_from_idle_loads_and_refreshes_status_in_same_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let glossary_path = dir.path().join("glossary.toml");
        std::fs::write(
            &glossary_path,
            "[[entry]]\nsource = \"hello\"\ntarget = \"hola\"\n",
        )
        .unwrap();
        let mut app = test_app(glossary_path);
        let observed_statuses = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed_statuses_for_sink = Arc::clone(&observed_statuses);
        app.set_tray_status_observer_for_test(move |status| {
            observed_statuses_for_sink.lock().unwrap().push(status);
        });

        app.dispatch_reload_glossary();

        assert!(matches!(app.app_state, AppState::Idle));
        assert_eq!(
            crate::glossary::Glossary::read_shared(&app.glossary).len(),
            1
        );
        assert_eq!(
            *observed_statuses.lock().unwrap(),
            vec![crate::tray::TrayStatus::Ready]
        );
    }
}
