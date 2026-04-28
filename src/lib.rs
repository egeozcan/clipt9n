//! Public library surface.

pub mod clipboard;
pub mod config;
pub mod error;
pub mod llm;
pub mod notify;
pub mod platform;
pub mod secrets;
pub mod state;
pub mod translator;
pub mod ui;

use std::time::Duration;

use clap::{ArgGroup, Parser};
use directories::ProjectDirs;

use crate::clipboard::{ArboardClipboard, Clipboard};
use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::openai::OpenAiCompatibleProvider;
use crate::llm::LlmProvider;
use crate::secrets::{EnvSecrets, Secrets};
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

/// Build the configured `LlmProvider` for the current `[provider]` block.
fn build_provider(
    cfg: &Config,
    secrets: &dyn Secrets,
) -> Result<Box<dyn LlmProvider>, TranslateError> {
    let api_key = secrets.get_api_key()?;
    let timeout = Duration::from_secs(cfg.provider.timeout_seconds);
    let provider: Box<dyn LlmProvider> = match cfg.provider.kind.as_str() {
        "anthropic" => Box::new(AnthropicProvider::new(
            &cfg.provider.base_url,
            api_key,
            &cfg.provider.model,
            timeout,
        )?),
        "openai" | "gemini" | "ollama" => Box::new(OpenAiCompatibleProvider::new(
            &cfg.provider.base_url,
            api_key,
            &cfg.provider.model,
            timeout,
        )?),
        other => {
            return Err(TranslateError::Config(format!(
                "unknown provider type '{other}'; expected one of: anthropic, openai, gemini, ollama"
            )));
        }
    };
    Ok(provider)
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

    let secrets: Box<dyn Secrets> = Box::new(EnvSecrets::new(cfg.provider.api_key.env_var.clone()));

    let provider = build_provider(&cfg, secrets.as_ref())?;

    // Test-only path: when CLIPT9N_TEST_INPUT is set, skip the real clipboard
    // and use it as source text. When CLIPT9N_TEST_PRINT_RESULT is set, print
    // the translated result to stdout instead of writing to the clipboard.
    // This makes `tests/cli_smoke.rs` runnable in CI without a desktop session.
    if let Ok(input) = std::env::var("CLIPT9N_TEST_INPUT") {
        let print_result = std::env::var("CLIPT9N_TEST_PRINT_RESULT").is_ok();
        let translator = Translator::new(&cfg, provider.as_ref());
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

    let translator = Translator::new(&cfg, provider.as_ref());
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
