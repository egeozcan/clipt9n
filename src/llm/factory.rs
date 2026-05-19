//! Provider construction factory. Single source of truth for building
//! the configured `LlmProvider` from a `Config` + a freshly-resolved
//! API key.
//!
//! Used by:
//!   - `main.rs` at startup,
//!   - `lib.rs::run` for the CLI mode,
//!   - `app.rs::persist_setup_completion` for the live provider
//!     rebuild after the wizard's Save-and-start (M7 Task 10),
//!   - `app.rs::spawn_sample_translation_check` for the wizard's
//!     Verify step (passes per-provider default base URL via
//!     `base_url_override`).

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
    base_url_override: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, TranslateError> {
    let timeout = Duration::from_secs(cfg.provider.timeout_seconds);
    let base_url = base_url_override.unwrap_or(&cfg.provider.base_url);
    let provider: Arc<dyn LlmProvider> = match cfg.provider.kind.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(
            base_url,
            key,
            &cfg.provider.model,
            timeout,
        )?),
        "openai" | "gemini" | "ollama" | "deepseek" => Arc::new(OpenAiCompatibleProvider::new(
            base_url,
            key,
            &cfg.provider.model,
            timeout,
        )?),
        other => {
            return Err(TranslateError::Config(format!(
                "unknown provider type '{other}'; expected one of: anthropic, openai, gemini, ollama, deepseek"
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
        let p = build_provider(&cfg, key, None).expect("provider should build");
        // Type-level smoke: we got an Arc<dyn LlmProvider> back. No
        // network is touched here.
        assert_eq!(Arc::strong_count(&p), 1);
    }

    #[test]
    fn openai_compatible_kinds_route_to_openai_provider() {
        for kind in ["openai", "gemini", "ollama", "deepseek"] {
            let mut cfg = Config::default();
            cfg.provider.kind = kind.into();
            let key = Zeroizing::new("sk-test".to_string());
            let p = build_provider(&cfg, key, None).expect("should build for openai-compat kinds");
            assert_eq!(Arc::strong_count(&p), 1);
        }
    }

    #[test]
    fn unknown_provider_kind_errors() {
        let mut cfg = Config::default();
        cfg.provider.kind = "magic-llm".into();
        let key = Zeroizing::new("ignored".to_string());
        match build_provider(&cfg, key, None) {
            Ok(_) => panic!("expected Err, got Ok"),
            Err(TranslateError::Config(msg)) => assert!(msg.contains("magic-llm")),
            Err(other) => panic!("expected Config error, got {other:?}"),
        }
    }
}
