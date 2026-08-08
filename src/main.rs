use clap::Parser;
use clipt9n::app::ClipApp;
use clipt9n::config::{Config, Modifier, NativeModifier};
use clipt9n::platform::{self, Platform};
use clipt9n::secrets::Secrets;
use clipt9n::Cli;
use directories::ProjectDirs;
use eframe::NativeOptions;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    if cli.action_or_none().is_some() {
        // CLI mode (M1 behavior): one-shot translation, then exit.
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(clipt9n::run())?;
        return Ok(());
    }

    // GUI mode.
    let (cfg_path, state_path) = gui_paths(&cli)?;
    let cfg = Config::load(&cfg_path)?;
    let config_dir = cfg_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("config path has no parent dir"))?;

    // Templates: strict load — a malformed override aborts startup.
    let templates = std::sync::Arc::new(clipt9n::llm::templates::Templates::load(
        &config_dir,
        &cfg.templates,
    )?);
    // Glossary: graceful load — fall back to empty on error.
    let glossary_path = config_dir.join(&cfg.glossary.file);
    let load_result = clipt9n::glossary::Glossary::load(&glossary_path);
    let glossary_malformed_at_startup = load_result.is_err();
    let glossary_inner = match load_result {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, "glossary load failed; continuing without glossary");
            clipt9n::glossary::Glossary::empty()
        }
    };
    let glossary = std::sync::Arc::new(std::sync::RwLock::new(glossary_inner));

    // Glossary reload channel — SIGHUP listener installed below once
    // the App (and its tokio runtime) exists.
    let (glossary_reload_tx, glossary_reload_rx) = crossbeam_channel::unbounded::<()>();

    // Encrypted history (M5). Graceful: open failure → log warn + run
    // with history disabled. Spec §8 corruption + missing-key rows.
    let history_path = config_dir.join("history.db");
    let keyfile_path = clipt9n::history::crypto::default_keyfile_path(&config_dir);
    let (history, history_disabled_initial): (
        Option<std::sync::Arc<clipt9n::history::store::History>>,
        bool,
    ) = if cfg.history.enabled {
        match clipt9n::secrets::provision_history_key(
            &keyfile_path,
            &cfg.provider.api_key.service,
            "history-key",
        ) {
            Ok(provisioned) => {
                match &provisioned.state {
                    clipt9n::secrets::HistoryKeyProvisionState::MigratedLegacy {
                        recovery_path,
                    } => tracing::warn!(
                        path = %recovery_path.display(),
                        "history key migrated and legacy key retained as owner-only recovery file"
                    ),
                    clipt9n::secrets::HistoryKeyProvisionState::KeychainPresentLegacyRecovered {
                        recovery_path,
                    } => tracing::warn!(
                        path = %recovery_path.display(),
                        "existing keychain key matched legacy key; legacy key retained as owner-only recovery file"
                    ),
                    clipt9n::secrets::HistoryKeyProvisionState::LegacyFallback { reason } => {
                        tracing::warn!(reason, "history keychain unavailable; using secure legacy key for this session")
                    }
                    clipt9n::secrets::HistoryKeyProvisionState::KeychainCreated => {
                        tracing::info!("new history key provisioned and verified in keychain")
                    }
                    clipt9n::secrets::HistoryKeyProvisionState::KeychainPresent => {}
                }
                match clipt9n::history::crypto::derive_key(&provisioned.secret)
                    .and_then(|key| clipt9n::history::store::History::open(&history_path, key))
                {
                    Ok(h) => (Some(std::sync::Arc::new(h)), false),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            path = %history_path.display(),
                            "history open failed; running with history disabled"
                        );
                        (None, true)
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "history key provisioning failed; running with history disabled"
                );
                (None, true)
            }
        }
    } else {
        tracing::info!("history disabled by config; skipping open");
        (None, false)
    };

    // Platform precondition (Accessibility on macOS, no-op elsewhere).
    let plat = platform::current();
    // Per spec §8: if Accessibility is missing, surface via tray
    // warning state rather than aborting startup. The hotkey will
    // simply fail to register below; the user can fix the permission
    // and the tray icon's tooltip explains the degraded state.
    let accessibility_revoked = match plat.ensure_hotkey_permissions() {
        Ok(()) => false,
        Err(e) => {
            tracing::warn!(error = %e, "accessibility permission missing; running with tray warning state");
            if let Err(open_err) = plat.open_accessibility_settings() {
                tracing::warn!(
                    error = %open_err,
                    "failed to open accessibility settings"
                );
            }
            true
        }
    };

    // Secrets resolution (M1 behavior: env-var only).
    let secrets: Box<dyn Secrets> = clipt9n::secrets::resolve(&cfg.provider.api_key);
    let api_key_opt = secrets.get_api_key().ok();
    // Capture for the tray status decision inside the eframe closure,
    // before api_key_opt is consumed by unwrap_or_else below.
    let has_api_key = api_key_opt.is_some();
    // If we have no key, construct a provider with a placeholder. The
    // setup wizard's Verify checks build their own client (with the
    // user's freshly-typed key) so the placeholder is unused until
    // the wizard completes and the app restarts (or Save-and-start
    // triggers a config rewrite that the user honors on next launch).
    let api_key =
        api_key_opt.unwrap_or_else(|| zeroize::Zeroizing::new("placeholder-no-key".into()));

    // Build the runtime provider via the factory. Same source of truth
    // as persist_setup_completion's live-rebuild path (M7 Task 10).
    let provider = clipt9n::llm::factory::build_provider(&cfg, api_key, None)?;

    // Hotkey registration. Three possible registrations: the prompt hotkey
    // (always constructed, optionally registered), selected-text hotkey, and
    // the history hotkey.
    let manager = GlobalHotKeyManager::new()?;

    // Prompt hotkey — same as M2.
    let prompt_modifier = Modifier::parse(&cfg.hotkey.modifier)
        .ok_or_else(|| anyhow::anyhow!("unknown hotkey modifier: {}", cfg.hotkey.modifier))?;
    let mut prompt_mods = match prompt_modifier.resolve_native() {
        NativeModifier::Ctrl => Modifiers::CONTROL,
        NativeModifier::Alt => Modifiers::ALT,
        NativeModifier::Meta => Modifiers::META,
    };
    if cfg.hotkey.shift {
        prompt_mods |= Modifiers::SHIFT;
    }
    if cfg.hotkey.option {
        prompt_mods |= Modifiers::ALT;
    }
    // An unregisterable key must not abort the launch. Aborting here
    // would be unrecoverable through the UI: no window and no tray icon
    // means no way to correct the key except hand-editing the TOML. Fall
    // back to the default so the app still comes up, and report it the
    // same way the other three hotkeys report a bad key.
    let (prompt_key_code, prompt_key_unsupported) = match letter_to_code(&cfg.hotkey.key) {
        Some(code) => (code, false),
        None => {
            tracing::warn!(
                key = %cfg.hotkey.key,
                "unsupported prompt hotkey key; prompt hotkey disabled — fix it in Settings"
            );
            (Code::KeyT, true)
        }
    };
    let prompt_hotkey = HotKey::new(Some(prompt_mods), prompt_key_code);
    let prompt_hotkey_id = prompt_hotkey.id();
    let hotkey_in_use = if prompt_key_unsupported {
        // Surface it on the tray pill — the hotkey genuinely won't work.
        true
    } else if cfg.hotkey.enabled {
        match manager.register(prompt_hotkey) {
            Ok(()) => false,
            Err(e) => {
                tracing::warn!(error = %e, "prompt hotkey registration failed; tray menu remains the entry point");
                true
            }
        }
    } else {
        false
    };

    // Selection hotkey — copies the current selection and opens the same
    // action prompt using that text. Failure is non-fatal; the clipboard
    // prompt hotkey and tray menu remain available.
    let (selection_hotkey_id, selection_hotkey_in_use) = if cfg.hotkey.selection.enabled {
        let mod_kind = match Modifier::parse(&cfg.hotkey.selection.modifier) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    modifier = %cfg.hotkey.selection.modifier,
                    "unknown selection hotkey modifier; selected-text hotkey disabled"
                );
                Modifier::Cmd
            }
        };
        let mut mods = match mod_kind.resolve_native() {
            NativeModifier::Ctrl => Modifiers::CONTROL,
            NativeModifier::Alt => Modifiers::ALT,
            NativeModifier::Meta => Modifiers::META,
        };
        if cfg.hotkey.selection.shift {
            mods |= Modifiers::SHIFT;
        }
        if cfg.hotkey.selection.option {
            mods |= Modifiers::ALT;
        }
        match letter_to_code(&cfg.hotkey.selection.key) {
            Some(code) => {
                let hk = HotKey::new(Some(mods), code);
                let id = hk.id();
                match manager.register(hk) {
                    Ok(()) => (Some(id), false),
                    Err(e) => {
                        tracing::warn!(error = %e, "selected-text hotkey registration failed");
                        (None, true)
                    }
                }
            }
            None => {
                tracing::warn!(
                    key = %cfg.hotkey.selection.key,
                    "unsupported selection hotkey key; selected-text hotkey disabled"
                );
                (None, false)
            }
        }
    } else {
        (None, false)
    };
    let hotkey_in_use = hotkey_in_use || selection_hotkey_in_use;

    // Replace hotkey — copies the current selection, translates it, and replaces inline.
    let (replace_hotkey_id, replace_hotkey_in_use) = if cfg.hotkey.replace.enabled {
        let mod_kind = match Modifier::parse(&cfg.hotkey.replace.modifier) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    modifier = %cfg.hotkey.replace.modifier,
                    "unknown replace hotkey modifier; replace hotkey disabled"
                );
                Modifier::Super
            }
        };
        let mut mods = match mod_kind.resolve_native() {
            NativeModifier::Ctrl => Modifiers::CONTROL,
            NativeModifier::Alt => Modifiers::ALT,
            NativeModifier::Meta => Modifiers::META,
        };
        if cfg.hotkey.replace.shift {
            mods |= Modifiers::SHIFT;
        }
        if cfg.hotkey.replace.option {
            mods |= Modifiers::ALT;
        }
        match letter_to_code(&cfg.hotkey.replace.key) {
            Some(code) => {
                let hk = HotKey::new(Some(mods), code);
                let id = hk.id();
                match manager.register(hk) {
                    Ok(()) => (Some(id), false),
                    Err(e) => {
                        tracing::warn!(error = %e, "replace hotkey registration failed");
                        (None, true)
                    }
                }
            }
            None => {
                tracing::warn!(
                    key = %cfg.hotkey.replace.key,
                    "unsupported replace hotkey key; replace hotkey disabled"
                );
                (None, false)
            }
        }
    } else {
        (None, false)
    };
    let hotkey_in_use = hotkey_in_use || replace_hotkey_in_use;

    // History hotkey — M5 addition. Failure to register (e.g., already
    // claimed by another app) is non-fatal; we log warn and the user
    // can still use the tray-menu "History" item once M7 lands.
    let history_hotkey_id = if cfg.hotkey.history.enabled {
        let mod_kind = match Modifier::parse(&cfg.hotkey.history.modifier) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    modifier = %cfg.hotkey.history.modifier,
                    "unknown history hotkey modifier; viewer hotkey disabled"
                );
                Modifier::Cmd
            }
        };
        let mut mods = match mod_kind.resolve_native() {
            NativeModifier::Ctrl => Modifiers::CONTROL,
            NativeModifier::Alt => Modifiers::ALT,
            NativeModifier::Meta => Modifiers::META,
        };
        if cfg.hotkey.history.shift {
            mods |= Modifiers::SHIFT;
        }
        if cfg.hotkey.history.option {
            mods |= Modifiers::ALT;
        }
        match letter_to_code(&cfg.hotkey.history.key) {
            Some(code) => {
                let hk = HotKey::new(Some(mods), code);
                let id = hk.id();
                match manager.register(hk) {
                    Ok(()) => Some(id),
                    Err(e) => {
                        tracing::warn!(error = %e, "history hotkey registration failed; viewer hotkey unavailable");
                        None
                    }
                }
            }
            None => {
                tracing::warn!(
                    key = %cfg.hotkey.history.key,
                    "unsupported history hotkey key; viewer hotkey disabled"
                );
                None
            }
        }
    } else {
        None
    };

    // Forward hotkey events into our own channel. The handler that fills
    // it is installed later, inside the eframe creator closure, because it
    // needs the egui context — see `install_hotkey_handler`.
    let (hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded::<GlobalHotKeyEvent>();

    // M6: first-launch detection. If we have no key in keychain AND
    // the keychain is reachable, start in the setup wizard. Otherwise
    // fall through to the normal Idle startup.
    let initial_setup_wizard: Option<clipt9n::ui::setup::SetupWizardModel> = {
        let probe = secrets.get_api_key();
        // Probe the platform keychain directly rather than asking the
        // active `Secrets` impl — when the config still has the default
        // `api_key.source = "env"`, `secrets` is an `EnvSecrets` whose
        // `keychain_available()` is hardcoded to false even on a working
        // macOS keychain. The wizard needs the underlying platform's
        // answer so it can offer keychain storage on first launch.
        let keychain_avail = clipt9n::secrets::keychain_probe(&cfg.provider.api_key.service);
        match probe {
            Err(clipt9n::error::TranslateError::MissingApiKey { .. }) if keychain_avail => {
                tracing::info!("setup wizard: no API key found; opening first-launch wizard");
                Some(clipt9n::ui::setup::SetupWizardModel {
                    provider: cfg.provider.kind.clone(),
                    keychain_available: true,
                    storage: clipt9n::ui::setup::Storage::Keychain,
                    test_translation: true,
                    ..Default::default()
                })
            }
            Err(clipt9n::error::TranslateError::MissingApiKey { .. }) => {
                tracing::warn!(
                    "no API key and keychain unavailable; opening wizard in \
                     env-only mode — user must set env var before next launch"
                );
                Some(clipt9n::ui::setup::SetupWizardModel {
                    provider: cfg.provider.kind.clone(),
                    keychain_available: false,
                    storage: clipt9n::ui::setup::Storage::Env,
                    test_translation: false,
                    ..Default::default()
                })
            }
            _ => None,
        }
    };

    // M7: Determine tray visibility before entering the eframe closure.
    // We need to load state here (before state_path moves into the closure).
    let tray_state_visible = clipt9n::state::State::load(&state_path).tray.visible;
    let tray_should_show = tray_state_visible || cli.show_tray;
    // --show-tray side effect: persist tray.visible=true so subsequent
    // launches continue showing the tray without the flag.
    if cli.show_tray && !tray_state_visible {
        let mut s = clipt9n::state::State::load(&state_path);
        s.tray.visible = true;
        if let Err(e) = s.save(&state_path) {
            tracing::warn!(error = %e, "failed to persist tray.visible=true after --show-tray");
        }
    }

    // The creator closure is `move`; hand it its own copy of the path
    // the config was actually loaded from so saves go back to that file.
    let cfg_path_for_app = cfg_path.clone();

    // eframe options: hidden, undecorated, always-on-top, centered window.
    let inner_size = clipt9n::ui::prompt_default_inner_size(&cfg.ui);
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([inner_size.x, inner_size.y])
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(false)
        .with_visible(false)
        .with_always_on_top()
        .with_active(true)
        // Hide from the Windows taskbar / Linux task list. macOS Dock
        // suppression is handled at runtime via `set_dock_visible`
        // below (LSUIElement only takes effect inside the .app bundle).
        .with_taskbar(false);
    let native_options = NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "clipt9n",
        native_options,
        Box::new(move |cc| {
            // Hide from the macOS Dock + Cmd+Tab. Runs on the main
            // thread inside the eframe creator closure, after winit
            // has initialized NSApplication. Complements the .app
            // bundle's LSUIElement=true (which only fires when
            // launched via Finder, not under `cargo run`).
            platform::current().set_dock_visible(false);

            install_hotkey_handler(&cc.egui_ctx, hotkey_tx);

            let mut app = ClipApp::new(
                cc,
                cfg,
                provider,
                templates,
                glossary,
                glossary_path,
                glossary_reload_rx,
                history,
                history_disabled_initial,
                secrets,
                cfg_path_for_app,
                state_path,
                hotkey_rx,
                prompt_hotkey_id,
                history_hotkey_id,
                selection_hotkey_id,
                replace_hotkey_id,
                accessibility_revoked,
                hotkey_in_use,
            );
            app.install_glossary_reload(glossary_reload_tx);
            if glossary_malformed_at_startup {
                app.set_glossary_malformed(true);
            }

            // M7: tray construction. Failure is non-fatal.
            if tray_should_show {
                let initial_status = if !has_api_key {
                    clipt9n::tray::TrayStatus::NoApiKey
                } else if accessibility_revoked {
                    clipt9n::tray::TrayStatus::Warn(
                        clipt9n::tray::WarnReason::AccessibilityPermissionRevoked,
                    )
                } else if hotkey_in_use {
                    clipt9n::tray::TrayStatus::Warn(
                        clipt9n::tray::WarnReason::HotkeyInUse,
                    )
                } else {
                    clipt9n::tray::TrayStatus::Ready
                };
                match clipt9n::tray::TrayHandle::build_with_panic_isolation(
                    initial_status,
                    &cc.egui_ctx,
                ) {
                    Ok(handle) => app.attach_tray(handle),
                    Err(e) => {
                        tracing::warn!(error = %e, "tray construction failed; running without tray icon");
                    }
                }
            }

            // The wizard wins over `--settings`: with no working key
            // there is nothing for the editor to rebuild a provider
            // from, so first-run setup has to come first.
            let app = match initial_setup_wizard {
                Some(model) => {
                    app.with_initial_state(clipt9n::app::InitialState::SetupWizard(model))
                }
                None if cli.settings => {
                    app.with_initial_state(clipt9n::app::InitialState::Settings)
                }
                None => app,
            };
            // Make the viewport visible if we're starting in a window
            // state. (Normal startup is hidden — only the hotkey shows
            // the prompt.)
            if let Some(size) = app.startup_window_size() {
                cc.egui_ctx
                    .send_viewport_cmd(eframe::egui::ViewportCommand::InnerSize(size));
                cc.egui_ctx
                    .send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
                cc.egui_ctx
                    .send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
                // Required for accessory-policy apps to surface the
                // window above the user's current foreground app.
                platform::current().activate_app();
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;

    drop(manager);
    Ok(())
}

/// Route global-hotkey events into `tx`, waking the egui context on each
/// one. Installed from the eframe creator closure, the only place with a
/// context to clone.
///
/// Why not `GlobalHotKeyEvent::receiver()`: that static channel delivers
/// the event but leaves the event loop asleep, so `ClipApp::update` would
/// not observe it until some unrelated pass ran. The app used to paper
/// over this by requesting a repaint every 150 ms — a poll that rendered
/// and presented a hidden window ~7x/second forever, costing ~2% CPU
/// around the clock for an app that is idle almost all the time.
///
/// Waking the context here is the same trick `tray.rs` uses for menu
/// events, and it lets the event loop sleep until something real happens.
/// Anything else that feeds a channel drained by `update()` must wake the
/// context too, or it will hang until the next unrelated frame.
fn install_hotkey_handler(
    ctx: &eframe::egui::Context,
    tx: crossbeam_channel::Sender<GlobalHotKeyEvent>,
) {
    let ctx = ctx.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
        if tx.send(ev).is_ok() {
            ctx.request_repaint();
        }
    }));
}

fn letter_to_code(key: &str) -> Option<Code> {
    match key.to_ascii_uppercase().as_str() {
        "A" => Some(Code::KeyA),
        "B" => Some(Code::KeyB),
        "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD),
        "E" => Some(Code::KeyE),
        "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG),
        "H" => Some(Code::KeyH),
        "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ),
        "K" => Some(Code::KeyK),
        "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM),
        "N" => Some(Code::KeyN),
        "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP),
        "Q" => Some(Code::KeyQ),
        "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS),
        "T" => Some(Code::KeyT),
        "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV),
        "W" => Some(Code::KeyW),
        "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY),
        "Z" => Some(Code::KeyZ),
        _ => None,
    }
}

fn gui_paths(cli: &Cli) -> anyhow::Result<(std::path::PathBuf, std::path::PathBuf)> {
    if let Some(cfg_path) = cli.config_path.clone() {
        let state_path = cfg_path
            .parent()
            .map(|p| p.join("state.toml"))
            .ok_or_else(|| anyhow::anyhow!("config path has no parent dir"))?;
        return Ok((cfg_path, state_path));
    }

    let cfg_path = ProjectDirs::from("", "", "clipboard-translator")
        .map(|d| d.config_dir().join("config.toml"))
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let state_path = ProjectDirs::from("", "", "clipboard-translator")
        .map(|d| d.config_dir().join("state.toml"))
        .ok_or_else(|| anyhow::anyhow!("could not determine state path"))?;
    Ok((cfg_path, state_path))
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_paths_use_explicit_config_parent_for_state() {
        let cfg_path = std::path::PathBuf::from("/tmp/clipt9n-profile/config.toml");
        let cli = Cli {
            translate_to: None,
            fix_grammar: false,
            rewrite: false,
            custom: None,
            show_tray: false,
            config_path: Some(cfg_path.clone()),
            settings: false,
        };

        let (resolved_cfg, state_path) = gui_paths(&cli).unwrap();

        assert_eq!(resolved_cfg, cfg_path);
        assert_eq!(
            state_path,
            std::path::PathBuf::from("/tmp/clipt9n-profile/state.toml")
        );
    }
}
