//! Public library surface.

pub mod app;
pub mod clipboard;
pub mod config;
pub mod error;
pub mod glossary;
pub mod history;
pub mod llm;
pub mod notify;
pub mod platform;
pub mod secrets;
pub mod state;
pub mod translator;
pub mod tray;
pub mod ui;

use clap::{ArgGroup, Parser};
use directories::ProjectDirs;

use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::config::Config;
use crate::error::TranslateError;
use crate::secrets::Secrets;
use crate::translator::{Action, Translator};

/// CLI arguments. Exactly one of `--translate-to`, `--fix-grammar`,
/// `--rewrite`, `--custom` must be specified.
#[derive(Parser, Debug)]
#[command(
    name = "clipt9n",
    version,
    about = "Clipboard translator (M1: CLI walking skeleton)"
)]
#[command(group(ArgGroup::new("action").required(false).args(["translate_to", "fix_grammar", "rewrite", "custom"])))]
pub struct Cli {
    /// Translate to the given ISO language code (must match a slot in config).
    #[arg(long = "translate-to", value_name = "CODE")]
    pub translate_to: Option<String>,

    /// Fix grammar/spelling errors in the source language.
    #[arg(long = "fix-grammar")]
    pub fix_grammar: bool,

    /// Rewrite for clarity in the source language.
    #[arg(long = "rewrite")]
    pub rewrite: bool,

    /// Apply a custom instruction.
    #[arg(long = "custom", value_name = "INSTRUCTION")]
    pub custom: Option<String>,

    /// Force the tray icon to appear even if `state.toml` has
    /// `[tray] visible = false`. Side effect: also flips the persisted
    /// state back to `visible = true` so subsequent launches without
    /// the flag continue showing the tray. Recovery path documented in
    /// the hide-icon confirm modal.
    #[arg(long = "show-tray")]
    pub show_tray: bool,

    /// Optional path to config.toml. Defaults to platform config dir.
    #[arg(long = "config", value_name = "PATH")]
    pub config_path: Option<std::path::PathBuf>,
}

impl Cli {
    /// Return the explicit CLI action, or `None` if no action flag was given
    /// (which means: launch the GUI).
    pub fn action_or_none(&self) -> Option<Action> {
        if let Some(code) = &self.translate_to {
            Some(Action::Translate { code: code.clone() })
        } else if self.fix_grammar {
            Some(Action::FixGrammar)
        } else if self.rewrite {
            Some(Action::Rewrite)
        } else {
            self.custom.as_ref().map(|instruction| Action::Custom {
                instruction: instruction.clone(),
            })
        }
    }
}

/// Default config path: `<config_dir>/clipboard-translator/config.toml`.
fn default_config_path() -> Option<std::path::PathBuf> {
    ProjectDirs::from("", "", "clipboard-translator").map(|d| d.config_dir().join("config.toml"))
}

/// End-to-end run: parse args → load config → resolve secrets → read clipboard
/// → call translator → write clipboard. Public for the cli_smoke integration
/// test.
pub async fn run() -> Result<(), TranslateError> {
    init_tracing();

    let cli = Cli::parse();
    let cfg_path = cli
        .config_path
        .clone()
        .or_else(default_config_path)
        .ok_or_else(|| TranslateError::Config("could not determine config directory".into()))?;
    let cfg = Config::load(&cfg_path)?;

    let secrets: Box<dyn Secrets> = crate::secrets::resolve(&cfg.provider.api_key);

    let key = secrets.get_api_key()?;
    let provider = crate::llm::factory::build_provider(&cfg, key, None)?;

    // Build templates (strict — abort on malformed override) and glossary
    // (graceful — log warn + fall back to empty on malformed).
    let config_dir = cfg_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| TranslateError::Config("config path has no parent dir".into()))?;
    let templates = std::sync::Arc::new(crate::llm::templates::Templates::load(
        &config_dir,
        &cfg.templates,
    )?);
    let glossary_path = config_dir.join(&cfg.glossary.file);
    let glossary = match crate::glossary::Glossary::load(&glossary_path) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, "glossary load failed; continuing without glossary");
            crate::glossary::Glossary::empty()
        }
    };

    // History (M5): open if enabled. CLI mode never inserts; we open so
    // a corrupt DB is surfaced at first user contact rather than
    // silently failing on the first GUI translation.
    if cfg.history.enabled {
        let history_path = config_dir.join("history.db");
        let keyfile_path = crate::history::crypto::default_keyfile_path(&config_dir);
        if let Err(e) = crate::history::crypto::load_and_derive_with_keychain_fallback(
            &keyfile_path,
            &cfg.provider.api_key.service,
            "history-key",
        )
        .and_then(|key| crate::history::store::History::open(&history_path, key))
        .map(|_| ())
        {
            tracing::warn!(error = %e, "CLI history open failed (non-fatal)");
        }
    }

    // Test-only path: when CLIPT9N_TEST_INPUT is set, skip the real clipboard
    // and use it as source text. When CLIPT9N_TEST_PRINT_RESULT is set, print
    // the translated result to stdout instead of writing to the clipboard.
    // This makes `tests/cli_smoke.rs` runnable in CI without a desktop session.

    if let Ok(input) = std::env::var("CLIPT9N_TEST_INPUT") {
        let print_result = std::env::var("CLIPT9N_TEST_PRINT_RESULT").is_ok();
        let translator = Translator::new(&cfg, provider.as_ref(), &templates, &glossary);
        let action = cli.action_or_none().ok_or_else(|| {
            TranslateError::Config("no CLI action; GUI mode is not yet wired in run()".into())
        })?;
        let result = translator.execute(&action, &input).await?;
        if print_result {
            println!("{result}");
        }
        return Ok(());
    }

    let mut clipboard: Box<dyn Clipboard> = Box::new(ArboardClipboard::new()?);
    let source_text = clipboard.read_text()?;
    if source_text.is_empty() {
        return Err(TranslateError::EmptyOrNonTextClipboard);
    }

    let translator = Translator::new(&cfg, provider.as_ref(), &templates, &glossary);
    let action = cli.action_or_none().ok_or_else(|| {
        TranslateError::Config("no CLI action; GUI mode is not yet wired in run()".into())
    })?;
    let result = translator.execute(&action, &source_text).await?;

    clipboard.write_text(&result)?;

    tracing::info!(
        action = ?action_kind(&action),
        chars_in = source_text.chars().count(),
        chars_out = result.chars().count(),
        "translation complete"
    );
    Ok(())
}

fn action_kind(a: &Action) -> &'static str {
    match a {
        Action::Translate { .. } => "translate",
        Action::FixGrammar => "fix_grammar",
        Action::Rewrite => "rewrite",
        Action::Custom { .. } => "custom",
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

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn translate_to_parses() {
        let cli = Cli::try_parse_from(["clipt9n", "--translate-to=de"]).unwrap();
        assert_eq!(cli.translate_to.as_deref(), Some("de"));
        assert!(matches!(cli.action_or_none(), Some(Action::Translate { code }) if code == "de"));
    }

    #[test]
    fn fix_grammar_parses() {
        let cli = Cli::try_parse_from(["clipt9n", "--fix-grammar"]).unwrap();
        assert!(cli.fix_grammar);
        assert!(matches!(cli.action_or_none(), Some(Action::FixGrammar)));
    }

    #[test]
    fn rewrite_parses() {
        let cli = Cli::try_parse_from(["clipt9n", "--rewrite"]).unwrap();
        assert!(matches!(cli.action_or_none(), Some(Action::Rewrite)));
    }

    #[test]
    fn custom_parses() {
        let cli = Cli::try_parse_from(["clipt9n", "--custom=translate to formal Spanish"]).unwrap();
        assert!(matches!(
            cli.action_or_none(),
            Some(Action::Custom { instruction }) if instruction == "translate to formal Spanish"
        ));
    }

    #[test]
    fn multiple_actions_are_rejected() {
        let res = Cli::try_parse_from(["clipt9n", "--fix-grammar", "--rewrite"]);
        assert!(res.is_err(), "clap should reject multiple actions");
    }

    #[test]
    fn no_action_is_now_accepted_for_gui_mode() {
        let cli = Cli::try_parse_from(["clipt9n"]).unwrap();
        assert!(cli.translate_to.is_none());
        assert!(!cli.fix_grammar);
        assert!(!cli.rewrite);
        assert!(cli.custom.is_none());
        assert!(cli.action_or_none().is_none());
    }

    #[test]
    fn translate_to_still_resolves_action() {
        let cli = Cli::try_parse_from(["clipt9n", "--translate-to=de"]).unwrap();
        assert!(matches!(cli.action_or_none(), Some(Action::Translate { code }) if code == "de"));
    }

    #[test]
    fn cli_command_renders() {
        // Smoke test that clap's metadata is well-formed.
        Cli::command().debug_assert();
    }
}
