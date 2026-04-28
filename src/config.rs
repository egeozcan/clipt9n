//! `config.toml` loader. M1 only reads the subset of the spec §6 schema that
//! M1 actually uses: `[provider]`, `[provider.api_key]`, `[languages]`. Other
//! sections (`[hotkey]`, `[ui]`, `[history]`, `[tray]`, `[templates]`,
//! `[glossary]`, `[logging]`) are loaded into the struct but not consumed by
//! M1 — later milestones add behavior. Defaults applied when fields are absent.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TranslateError;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderConfig,
    pub languages: LanguagesConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: ProviderConfig::default(),
            languages: LanguagesConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// One of: "anthropic", "openai", "gemini", "ollama".
    /// gemini and ollama route through the OpenAI-compatible provider.
    #[serde(rename = "type")]
    pub kind: String,
    pub model: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub api_key: ApiKeyConfig,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: "anthropic".into(),
            model: "claude-haiku-4-5-20251001".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            timeout_seconds: 30,
            api_key: ApiKeyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ApiKeyConfig {
    /// "keychain" | "env" | "prompt". M1 only honors "env" — keychain is M6.
    pub source: String,
    pub service: String,
    pub account: String,
    pub env_var: String,
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        Self {
            source: "env".into(),
            service: "clipboard-translator".into(),
            account: "anthropic".into(),
            env_var: "ANTHROPIC_API_KEY".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LanguagesConfig {
    pub slot_1: LanguageSlot,
    pub slot_2: LanguageSlot,
    pub slot_3: LanguageSlot,
}

impl Default for LanguagesConfig {
    fn default() -> Self {
        Self {
            slot_1: LanguageSlot {
                label: "English".into(),
                code: "en".into(),
            },
            slot_2: LanguageSlot {
                label: "Deutsch".into(),
                code: "de".into(),
            },
            slot_3: LanguageSlot {
                label: "Türkçe".into(),
                code: "tr".into(),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LanguageSlot {
    pub label: String,
    pub code: String,
}

impl Config {
    /// Load config from `path`. If `path` doesn't exist, return defaults.
    /// Returns an error only on read errors or malformed TOML.
    pub fn load(path: &Path) -> Result<Self, TranslateError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|e| TranslateError::Config(format!("reading {}: {e}", path.display())))?;
        toml::from_str(&contents)
            .map_err(|e| TranslateError::Config(format!("parsing {}: {e}", path.display())))
    }

    /// Look up a target-language label by ISO code from configured slots.
    /// Returns `UnsupportedLanguage(code)` if no slot matches.
    pub fn label_for_code(&self, code: &str) -> Result<&str, TranslateError> {
        for slot in [
            &self.languages.slot_1,
            &self.languages.slot_2,
            &self.languages.slot_3,
        ] {
            if slot.code == code {
                return Ok(&slot.label);
            }
        }
        Err(TranslateError::UnsupportedLanguage(code.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn missing_file_returns_defaults() {
        let path = std::path::PathBuf::from("/tmp/clipt9n-nonexistent-config-12345.toml");
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.provider.kind, "anthropic");
        assert_eq!(cfg.provider.model, "claude-haiku-4-5-20251001");
        assert_eq!(cfg.languages.slot_1.code, "en");
        assert_eq!(cfg.languages.slot_2.label, "Deutsch");
    }

    #[test]
    fn loads_full_config() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[provider]
type = "openai"
model = "gpt-5"
base_url = "https://api.openai.com/v1"
timeout_seconds = 45

[provider.api_key]
source = "env"
env_var = "OPENAI_API_KEY"

[languages.slot_1]
label = "Français"
code = "fr"
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.provider.kind, "openai");
        assert_eq!(cfg.provider.model, "gpt-5");
        assert_eq!(cfg.provider.timeout_seconds, 45);
        assert_eq!(cfg.provider.api_key.env_var, "OPENAI_API_KEY");
        assert_eq!(cfg.languages.slot_1.label, "Français");
        // Other slots default
        assert_eq!(cfg.languages.slot_2.code, "de");
    }

    #[test]
    fn malformed_toml_returns_config_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "this is not valid TOML [[[").unwrap();
        let err = Config::load(f.path()).unwrap_err();
        match err {
            TranslateError::Config(msg) => assert!(msg.contains("parsing")),
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn label_for_code_resolves_default_slots() {
        let cfg = Config::default();
        assert_eq!(cfg.label_for_code("en").unwrap(), "English");
        assert_eq!(cfg.label_for_code("de").unwrap(), "Deutsch");
        assert_eq!(cfg.label_for_code("tr").unwrap(), "Türkçe");
    }

    #[test]
    fn label_for_unknown_code_returns_unsupported_error() {
        let cfg = Config::default();
        let err = cfg.label_for_code("fr").unwrap_err();
        match err {
            TranslateError::UnsupportedLanguage(code) => assert_eq!(code, "fr"),
            other => panic!("expected UnsupportedLanguage, got {other:?}"),
        }
    }
}
