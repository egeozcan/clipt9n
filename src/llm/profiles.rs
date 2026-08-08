//! Operational defaults for every supported LLM provider.

use crate::error::TranslateError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderImplementation {
    Anthropic,
    OpenAiCompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub implementation: ProviderImplementation,
    pub default_model: &'static str,
    pub default_base_url: &'static str,
    pub account: &'static str,
    pub env_var: &'static str,
    pub allow_loopback_http: bool,
}

pub static PROVIDER_PROFILES: &[ProviderProfile] = &[
    ProviderProfile {
        id: "anthropic",
        implementation: ProviderImplementation::Anthropic,
        default_model: "claude-haiku-4-5-20251001",
        default_base_url: "https://api.anthropic.com/v1",
        account: "anthropic",
        env_var: "ANTHROPIC_API_KEY",
        allow_loopback_http: false,
    },
    ProviderProfile {
        id: "openai",
        implementation: ProviderImplementation::OpenAiCompatible,
        default_model: "gpt-4o-mini",
        default_base_url: "https://api.openai.com/v1",
        account: "openai",
        env_var: "OPENAI_API_KEY",
        allow_loopback_http: false,
    },
    ProviderProfile {
        id: "gemini",
        implementation: ProviderImplementation::OpenAiCompatible,
        default_model: "gemini-2.0-flash",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        account: "gemini",
        env_var: "GEMINI_API_KEY",
        allow_loopback_http: false,
    },
    ProviderProfile {
        id: "ollama",
        implementation: ProviderImplementation::OpenAiCompatible,
        default_model: "llama3.2",
        default_base_url: "http://localhost:11434/v1",
        account: "ollama",
        env_var: "OLLAMA_API_KEY",
        allow_loopback_http: true,
    },
    ProviderProfile {
        id: "deepseek",
        implementation: ProviderImplementation::OpenAiCompatible,
        default_model: "deepseek-v4-flash",
        default_base_url: "https://api.deepseek.com/v1",
        account: "deepseek",
        env_var: "DEEPSEEK_API_KEY",
        allow_loopback_http: false,
    },
];

pub fn provider_profile(id: &str) -> Result<&'static ProviderProfile, TranslateError> {
    PROVIDER_PROFILES
        .iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| {
            TranslateError::Config(format!(
                "unknown provider type '{id}'; expected one of: {}",
                PROVIDER_PROFILES
                    .iter()
                    .map(|profile| profile.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_profile_contains_coherent_defaults() {
        let expected = [
            (
                "anthropic",
                ProviderImplementation::Anthropic,
                "claude-haiku-4-5-20251001",
                "https://api.anthropic.com/v1",
                "anthropic",
                "ANTHROPIC_API_KEY",
            ),
            (
                "openai",
                ProviderImplementation::OpenAiCompatible,
                "gpt-4o-mini",
                "https://api.openai.com/v1",
                "openai",
                "OPENAI_API_KEY",
            ),
            (
                "gemini",
                ProviderImplementation::OpenAiCompatible,
                "gemini-2.0-flash",
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "gemini",
                "GEMINI_API_KEY",
            ),
            (
                "ollama",
                ProviderImplementation::OpenAiCompatible,
                "llama3.2",
                "http://localhost:11434/v1",
                "ollama",
                "OLLAMA_API_KEY",
            ),
            (
                "deepseek",
                ProviderImplementation::OpenAiCompatible,
                "deepseek-v4-flash",
                "https://api.deepseek.com/v1",
                "deepseek",
                "DEEPSEEK_API_KEY",
            ),
        ];

        assert_eq!(PROVIDER_PROFILES.len(), expected.len());
        for (id, implementation, model, base_url, account, env_var) in expected {
            let profile = provider_profile(id).unwrap();
            assert_eq!(profile.id, id);
            assert_eq!(profile.implementation, implementation);
            assert_eq!(profile.default_model, model);
            assert_eq!(profile.default_base_url, base_url);
            assert_eq!(profile.account, account);
            assert_eq!(profile.env_var, env_var);
            assert_eq!(profile.allow_loopback_http, id == "ollama");
        }
    }

    #[test]
    fn unknown_provider_is_a_config_error() {
        let err = provider_profile("magic-llm").unwrap_err();
        assert!(err.to_string().contains("magic-llm"));
    }
}
