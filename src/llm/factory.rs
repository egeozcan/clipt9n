//! Provider construction factory. Single source of truth for building
//! the configured `LlmProvider` from a `Config` + a freshly-resolved
//! API key. Used by:
//!   - `main.rs` at startup,
//!   - `app.rs::persist_setup_completion` for live provider rebuild
//!     after the Save-and-start path (M7 Task 10),
//!   - `lib.rs::run` for the CLI mode,
//!   - the wizard's sample-translation check (`app.rs::spawn_sample_translation_check`)
//!     when we want a *real* provider rather than just the connectivity probe.

use std::sync::Arc;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::openai::OpenAiCompatibleProvider;
use crate::llm::LlmProvider;

/// Construct the configured `LlmProvider` from `cfg.provider.kind` and
/// the supplied `key`. Returns `Arc` so the caller can clone cheaply
/// across tokio spawn boundaries (M3's translator pattern).
pub fn build_provider(
    cfg: &Config,
    key: Zeroizing<String>,
) -> Result<Arc<dyn LlmProvider>, TranslateError> {
    let timeout = Duration::from_secs(cfg.provider.timeout_seconds);
    let provider: Arc<dyn LlmProvider> = match cfg.provider.kind.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(
            &cfg.provider.base_url,
            key,
            &cfg.provider.model,
            timeout,
        )?),
        "openai" | "gemini" | "ollama" => Arc::new(OpenAiCompatibleProvider::new(
            &cfg.provider.base_url,
            key,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn anthropic_provider_constructs() {
        let cfg = Config::default(); // default provider.kind = "anthropic"
        let key = Zeroizing::new("sk-test-12345".to_string());
        let p = build_provider(&cfg, key).expect("provider should build");
        // Type-level smoke: we got an Arc<dyn LlmProvider> back. No
        // network is touched here.
        assert_eq!(Arc::strong_count(&p), 1);
    }

    #[test]
    fn unknown_provider_kind_errors() {
        let mut cfg = Config::default();
        cfg.provider.kind = "magic-llm".into();
        let key = Zeroizing::new("ignored".to_string());
        match build_provider(&cfg, key) {
            Ok(_) => panic!("expected Err, got Ok"),
            Err(TranslateError::Config(msg)) => assert!(msg.contains("magic-llm")),
            Err(other) => panic!("expected Config error, got {other:?}"),
        }
    }
}
