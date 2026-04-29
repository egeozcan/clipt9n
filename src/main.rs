use std::sync::Arc;

use clap::Parser;
use clipt9n::app::ClipApp;
use clipt9n::config::{Config, Modifier, NativeModifier};
use clipt9n::error::TranslateError;
use clipt9n::llm::anthropic::AnthropicProvider;
use clipt9n::llm::openai::OpenAiCompatibleProvider;
use clipt9n::llm::LlmProvider;
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
    let cfg_path = ProjectDirs::from("", "", "clipboard-translator")
        .map(|d| d.config_dir().join("config.toml"))
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
    let cfg = Config::load(&cfg_path)?;
    let state_path = ProjectDirs::from("", "", "clipboard-translator")
        .map(|d| d.config_dir().join("state.toml"))
        .ok_or_else(|| anyhow::anyhow!("could not determine state path"))?;
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
    let glossary_inner = match clipt9n::glossary::Glossary::load(&glossary_path) {
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
        match clipt9n::history::crypto::load_and_derive(&keyfile_path) {
            Ok(key) => match clipt9n::history::store::History::open(&history_path, key) {
                Ok(h) => (Some(std::sync::Arc::new(h)), false),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %history_path.display(),
                        "history open failed; running with history disabled"
                    );
                    (None, true)
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %keyfile_path.display(),
                    "history keyfile load failed; running with history disabled"
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
    if let Err(e) = plat.ensure_hotkey_permissions() {
        tracing::error!(error = %e, "hotkey permission check failed");
        // ensure_hotkey_permissions has already opened System Settings;
        // exit non-zero so the user grants permission and relaunches.
        return Err(anyhow::anyhow!(e));
    }

    // Secrets resolution (M1 behavior: env-var only).
    let secrets: Box<dyn Secrets> = clipt9n::secrets::resolve(&cfg.provider.api_key);
    let api_key = secrets.get_api_key()?;
    let timeout = std::time::Duration::from_secs(cfg.provider.timeout_seconds);

    let provider: Arc<dyn LlmProvider> = match cfg.provider.kind.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(
            &cfg.provider.base_url,
            api_key,
            &cfg.provider.model,
            timeout,
        )?),
        "openai" | "gemini" | "ollama" => Arc::new(OpenAiCompatibleProvider::new(
            &cfg.provider.base_url,
            api_key,
            &cfg.provider.model,
            timeout,
        )?),
        other => {
            return Err(anyhow::anyhow!(TranslateError::Config(format!(
                "unknown provider type '{other}'"
            ))))
        }
    };

    // Hotkey registration. Two registered: the prompt hotkey (always)
    // and the history hotkey (M5; suppressible via [hotkey.history]
    // enabled = false).
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
    let prompt_key_code = letter_to_code(&cfg.hotkey.key)
        .ok_or_else(|| anyhow::anyhow!("unsupported hotkey key: {}", cfg.hotkey.key))?;
    let prompt_hotkey = HotKey::new(Some(prompt_mods), prompt_key_code);
    let prompt_hotkey_id = prompt_hotkey.id();
    if cfg.hotkey.enabled {
        manager.register(prompt_hotkey)?;
    }

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

    // Forward hotkey events from the global-hotkey channel into ours.
    let hotkey_rx = GlobalHotKeyEvent::receiver().clone();

    // eframe options: hidden, undecorated, always-on-top, centered window.
    let inner_w = if cfg.ui.density == "compact" {
        460.0
    } else {
        520.0
    };
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([inner_w, 470.0])
        .with_decorations(false)
        .with_resizable(false)
        .with_transparent(false)
        .with_visible(false)
        .with_always_on_top()
        .with_active(true);
    let native_options = NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "clipt9n",
        native_options,
        Box::new(move |cc| {
            let app = ClipApp::new(
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
                state_path,
                hotkey_rx,
                prompt_hotkey_id,
                history_hotkey_id,
            );
            app.install_glossary_reload(glossary_reload_tx);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;

    drop(manager);
    Ok(())
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

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();
}
