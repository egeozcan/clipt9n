//! `config.toml` loader. Reads the spec §6 schema. M1–M3 only consumed
//! `[provider]`, `[provider.api_key]`, `[languages]`, `[hotkey]`, `[ui]`.
//! M4 added `[glossary]` and `[templates]`. M5 adds `[history]` and the
//! nested `[hotkey.history]` sub-table. `[tray]` and `[logging]` are
//! still loaded but unused pending later milestones. Defaults applied
//! when fields are absent.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TranslateError;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderConfig,
    pub languages: LanguagesConfig,
    pub hotkey: HotkeyConfig,
    pub ui: UiConfig,
    pub glossary: GlossaryConfig,
    pub templates: TemplatesConfig,
    pub history: HistoryConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// One of: "anthropic", "openai", "gemini", "ollama", "deepseek".
    /// gemini, ollama, and deepseek route through the OpenAI-compatible provider.
    #[serde(rename = "type")]
    pub kind: String,
    pub model: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub api_key: ApiKeyConfig,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        let profile = crate::llm::profiles::provider_profile("anthropic")
            .expect("anthropic provider profile");
        Self {
            kind: profile.id.into(),
            model: profile.default_model.into(),
            base_url: profile.default_base_url.into(),
            timeout_seconds: 30,
            api_key: ApiKeyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ApiKeyConfig {
    /// "keychain" | "env" | "file" | "prompt". M1 only honored "env"; M6
    /// added "keychain"; M8 added "file" as the macOS dev fallback when
    /// the OS keychain silently fails to persist writes.
    pub source: String,
    pub service: String,
    pub account: String,
    pub env_var: String,
    /// Path to the keyfile when `source = "file"`. Empty by default;
    /// the setup wizard fills this in with `<config_dir>/api-key` when
    /// the keychain readback fails and the file fallback engages.
    #[serde(default)]
    pub path: String,
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        let profile = crate::llm::profiles::provider_profile("anthropic")
            .expect("anthropic provider profile");
        Self {
            source: "env".into(),
            service: "clipboard-translator".into(),
            account: profile.account.into(),
            env_var: profile.env_var.into(),
            path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LanguagesConfig {
    pub slot_1: LanguageSlot,
    pub slot_2: LanguageSlot,
    pub slot_3: LanguageSlot,
    pub slot_4: LanguageSlot,
    pub slot_5: LanguageSlot,
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
                label: "Deutsch (formell)".into(),
                code: "de".into(),
            },
            slot_4: LanguageSlot {
                label: "Türkçe".into(),
                code: "tr".into(),
            },
            slot_5: LanguageSlot {
                label: "Türkçe (resmî)".into(),
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// "cmd" → Cmd on macOS, Ctrl on Linux/Windows. "ctrl" → Ctrl on every OS.
    /// "alt" / "super" allowed but unmapped (passthrough).
    pub modifier: String,
    /// Adds Option/Alt to the configured base modifier.
    pub option: bool,
    pub shift: bool,
    /// Key name accepted by `global-hotkey::hotkey::Code`. Single uppercase letter
    /// like "T" maps to `Code::KeyT`.
    pub key: String,
    pub enabled: bool,
    /// Second hotkey for the history viewer (M5). Independent of the
    /// prompt hotkey above. Set `enabled = false` to skip registration.
    pub history: HistoryHotkeyConfig,
    /// Dedicated hotkey for translating the current selected text. Unlike
    /// the prompt hotkey, this copies the selection first and does not fall
    /// back to existing clipboard contents.
    pub selection: SelectionHotkeyConfig,
    /// Dedicated hotkey for translating the current selected text and replacing it inline.
    pub replace: ReplaceHotkeyConfig,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "cmd".into(),
            option: true,
            shift: false,
            key: "T".into(),
            enabled: true,
            history: HistoryHotkeyConfig::default(),
            selection: SelectionHotkeyConfig::default(),
            replace: ReplaceHotkeyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct HistoryHotkeyConfig {
    pub modifier: String,
    pub option: bool,
    pub shift: bool,
    pub key: String,
    pub enabled: bool,
}

impl Default for HistoryHotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "cmd".into(),
            option: true,
            shift: false,
            key: "H".into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SelectionHotkeyConfig {
    pub modifier: String,
    pub option: bool,
    pub shift: bool,
    pub key: String,
    pub enabled: bool,
    pub copy_delay_ms: u64,
}

impl Default for SelectionHotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "cmd".into(),
            option: true,
            shift: false,
            key: "Y".into(),
            enabled: true,
            copy_delay_ms: 80,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ReplaceHotkeyConfig {
    pub modifier: String,
    pub option: bool,
    pub shift: bool,
    pub key: String,
    pub enabled: bool,
    pub copy_delay_ms: u64,
    pub default_slot: u8,
}

impl Default for ReplaceHotkeyConfig {
    fn default() -> Self {
        Self {
            modifier: "super".into(),
            option: true,
            shift: false,
            key: "U".into(),
            enabled: true,
            copy_delay_ms: 80,
            default_slot: 1,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// "normal" or "compact". Drives prompt window width (520 vs 460).
    pub density: String,
    pub show_preview: bool,
    /// Above this character count, dispatch shows a confirm modal before
    /// sending the clipboard to the API. Spec §6 default is 2000.
    pub confirm_size_threshold: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            density: "normal".into(),
            show_preview: true,
            confirm_size_threshold: 2000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GlossaryConfig {
    /// When false, the glossary loader is bypassed entirely and
    /// `{{ glossary_block }}` always renders empty.
    pub enabled: bool,
    /// Path to the glossary TOML file, relative to the config dir.
    pub file: String,
    /// Whether term matching against source text is case-sensitive.
    /// Default false (case-insensitive); spec §6 default.
    pub case_sensitive: bool,
    /// One of "auto", "word_boundary", "substring". Spec §5.4. The
    /// glossary parser validates this value at load; arbitrary strings
    /// fall back to "auto" with a warn log.
    pub matching: String,
}

impl Default for GlossaryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            file: "glossary.toml".into(),
            case_sensitive: false,
            matching: "auto".into(),
        }
    }
}

/// Path (relative to config dir) for an override file. `None` or
/// `Some("")` means "use built-in for this action". Default values
/// point at the conventional `templates/<action>.j2` paths; the
/// override loader treats those as opt-in (file must exist).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplatesConfig {
    pub translate: Option<String>,
    pub fix_grammar: Option<String>,
    pub rewrite: Option<String>,
    pub custom: Option<String>,
}

impl Default for TemplatesConfig {
    fn default() -> Self {
        Self {
            translate: Some("templates/translate.j2".into()),
            fix_grammar: Some("templates/fix_grammar.j2".into()),
            rewrite: Some("templates/rewrite.j2".into()),
            custom: Some("templates/custom.j2".into()),
        }
    }
}

/// `[history]` block per spec §6 + §7. M5 wires this into the
/// `History` opener and the per-translation insert path.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// When false, history is fully disabled — the SQLite file is
    /// neither opened at startup nor written to. The viewer hotkey
    /// still registers but the viewer shows an empty list.
    pub enabled: bool,
    /// Maximum entries retained. Older rows are pruned at insert time.
    pub max_entries: usize,
    /// When false, source/result columns are NULL (metadata-only row).
    /// Useful for high-sensitivity environments per spec §9.
    pub store_text: bool,
    /// Whether the "Clear all" action requires a confirmation modal.
    /// Default true; setting false makes Shift+Del immediately destructive.
    pub confirm_clear: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 100,
            store_text: true,
            confirm_clear: true,
        }
    }
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
        let mut cfg: Self = toml::from_str(&contents)
            .map_err(|e| TranslateError::Config(format!("parsing {}: {e}", path.display())))?;
        cfg.validate()?;
        cfg.normalize();
        Ok(cfg)
    }

    /// Reject configs the rest of the app can't honor. Called by `load`
    /// after parsing, and by the settings editor before it commits an
    /// edited config to disk — the GUI must fail the same way the file
    /// loader would, or a save could produce a config that won't load.
    pub fn validate(&self) -> Result<(), TranslateError> {
        match self.provider.api_key.source.as_str() {
            "keychain" | "env" | "prompt" | "file" => {}
            other => {
                return Err(TranslateError::Config(format!(
                    "provider.api_key.source must be keychain, env, file, or prompt; got {other}"
                )));
            }
        }

        match self.glossary.matching.as_str() {
            "auto" | "word_boundary" | "substring" => {}
            other => {
                return Err(TranslateError::Config(format!(
                    "glossary.matching must be auto, word_boundary, or substring; got {other}"
                )));
            }
        }

        if self.history.max_entries == 0 {
            return Err(TranslateError::Config(
                "history.max_entries must be greater than zero".into(),
            ));
        }

        Ok(())
    }

    /// Auto-correct stale provider defaults when the user changes `type`
    /// but leaves the previous provider's model / base_url / account
    /// untouched. Only fires when the value matches another provider's
    /// hardcoded default exactly — custom values are never overwritten.
    fn normalize(&mut self) {
        let kind = self.provider.kind.as_str();
        let Ok(profile) = crate::llm::profiles::provider_profile(kind) else {
            return;
        };
        let default_model = profile.default_model;
        let default_url = profile.default_base_url;
        let default_account = profile.account;

        // Detect if the model matches the default for a DIFFERENT provider.
        let model_is_stale = self.provider.model != default_model
            && crate::llm::profiles::PROVIDER_PROFILES
                .iter()
                .any(|candidate| {
                    candidate.id != kind && self.provider.model == candidate.default_model
                });
        if model_is_stale {
            self.provider.model = default_model.to_string();
        }

        // Same heuristic for base_url.
        let url_is_stale = self.provider.base_url != default_url
            && crate::llm::profiles::PROVIDER_PROFILES
                .iter()
                .any(|candidate| {
                    candidate.id != kind && self.provider.base_url == candidate.default_base_url
                });
        if url_is_stale {
            self.provider.base_url = default_url.to_string();
        }

        // Same for the API key account name.
        if self.provider.api_key.account != default_account
            && crate::llm::profiles::PROVIDER_PROFILES
                .iter()
                .any(|candidate| candidate.account == self.provider.api_key.account)
        {
            self.provider.api_key.account = default_account.to_string();
        }

        self.normalize_languages();
    }

    /// Auto-correct stale language slot defaults. When a slot's saved value
    /// matches a previously-shipped default that has since been replaced,
    /// reset it to the current default. Custom user values are preserved.
    fn normalize_languages(&mut self) {
        // (slot index, prior default label, prior default code)
        // Pre-3c08cfb, slot_3 shipped as Türkçe/tr. Now it defaults to
        // Deutsch (formell)/de; reset only when the saved value still
        // matches the prior default verbatim.
        let stale_slot_3 =
            self.languages.slot_3.label == "Türkçe" && self.languages.slot_3.code == "tr";
        if stale_slot_3 {
            let default = LanguagesConfig::default();
            self.languages.slot_3 = default.slot_3;
        }
    }

    /// Look up a target-language label by ISO code from configured slots.
    /// Returns `UnsupportedLanguage(code)` if no slot matches.
    pub fn label_for_code(&self, code: &str) -> Result<&str, TranslateError> {
        for slot in [
            &self.languages.slot_1,
            &self.languages.slot_2,
            &self.languages.slot_3,
            &self.languages.slot_4,
            &self.languages.slot_5,
        ] {
            if slot.code == code {
                return Ok(&slot.label);
            }
        }
        Err(TranslateError::UnsupportedLanguage(code.to_string()))
    }

    /// Persist the `[provider]` and `[provider.api_key]` sections
    /// back to disk. Used by the setup wizard's Save-and-start path.
    /// Conservatively rewrites the entire file — the existing toml
    /// crate doesn't support in-place section replacement, and
    /// config.toml is small. Other sections are preserved (we
    /// re-serialize the full Config).
    pub fn persist(&self, path: &Path) -> Result<(), TranslateError> {
        use crate::config_commit::AtomicConfigStore;
        crate::config_commit::DiskAtomicConfig::new(path).replace(self)
    }
}

/// Logical hotkey modifier as authored by the user. Mapped to the
/// OS-appropriate physical modifier via `resolve_native()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// "cmd" → Meta on macOS, Ctrl on Linux/Windows.
    Cmd,
    /// "ctrl" → Ctrl on every OS.
    Ctrl,
    /// "alt" → Alt on every OS.
    Alt,
    /// "super" → Meta on every OS.
    Super,
}

/// Native modifier flag returned by `resolve_native()`. Mirrors
/// `global_hotkey::hotkey::Modifiers` shape so the main-loop conversion
/// is a one-liner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeModifier {
    Ctrl,
    Alt,
    Meta,
}

impl Modifier {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cmd" => Some(Self::Cmd),
            "ctrl" | "control" => Some(Self::Ctrl),
            "alt" | "option" => Some(Self::Alt),
            "super" | "meta" | "win" => Some(Self::Super),
            _ => None,
        }
    }

    pub fn resolve_native(self) -> NativeModifier {
        match self {
            Self::Cmd => crate::platform::cmd_modifier(),
            Self::Ctrl => NativeModifier::Ctrl,
            Self::Alt => NativeModifier::Alt,
            Self::Super => NativeModifier::Meta,
        }
    }

    /// Human-readable form for UI strings ("Cmd", "Ctrl", "Alt", "Super").
    pub fn display(self) -> &'static str {
        match self {
            Self::Cmd => "Cmd",
            Self::Ctrl => "Ctrl",
            Self::Alt => "Alt",
            Self::Super => "Super",
        }
    }
}

/// Whether `key` names a hotkey the app can actually register.
///
/// `main.rs::letter_to_code` is the registration-side table and accepts
/// exactly `A`–`Z`. Keep the two in sync: the settings editor refuses to
/// save anything this rejects, so widening one without the other either
/// blocks a key that would work or — worse — lets a key through that
/// fails to bind at launch.
pub fn hotkey_key_is_supported(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase()
}

/// Render an arbitrary modifier/option/shift/key combination for UI
/// display (e.g., "Cmd+Option+T"). Logical, not OS-mapped — it echoes
/// what the user authored. An unparseable modifier renders as "?" so a
/// typo is visible rather than silently normalized.
pub fn hotkey_combo_display(modifier: &str, option: bool, shift: bool, key: &str) -> String {
    let modifier = Modifier::parse(modifier)
        .map(Modifier::display)
        .unwrap_or("?");
    let mut parts = vec![modifier.to_string()];
    if option {
        parts.push("Option".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    parts.push(key.to_string());
    parts.join("+")
}

impl Config {
    /// Render the configured hotkey for UI display (e.g., "Cmd+Option+T").
    /// Returns "(disabled)" if `[hotkey].enabled = false`.
    pub fn hotkey_display(&self) -> String {
        if !self.hotkey.enabled {
            return "(disabled)".to_string();
        }
        hotkey_combo_display(
            &self.hotkey.modifier,
            self.hotkey.option,
            self.hotkey.shift,
            &self.hotkey.key,
        )
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
    fn default_hotkey_is_cmd_option_t() {
        let cfg = Config::default();
        assert_eq!(cfg.hotkey.modifier, "cmd");
        assert!(!cfg.hotkey.shift);
        assert!(cfg.hotkey.option);
        assert_eq!(cfg.hotkey.key, "T");
        assert!(cfg.hotkey.enabled);
    }

    #[test]
    fn default_ui_density_is_normal() {
        let cfg = Config::default();
        assert_eq!(cfg.ui.density, "normal");
        assert!(cfg.ui.show_preview);
    }

    #[test]
    fn default_confirm_size_threshold_is_2000() {
        let cfg = Config::default();
        assert_eq!(cfg.ui.confirm_size_threshold, 2000);
    }

    #[test]
    fn loads_confirm_size_threshold_override() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[ui]
confirm_size_threshold = 5000
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.ui.confirm_size_threshold, 5000);
    }

    #[test]
    fn loads_hotkey_override() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey]
modifier = "ctrl"
shift = false
key = "Y"
enabled = true
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.hotkey.modifier, "ctrl");
        assert!(!cfg.hotkey.shift);
        assert_eq!(cfg.hotkey.key, "Y");
    }

    #[test]
    fn supported_hotkey_keys_are_single_uppercase_letters() {
        assert!(hotkey_key_is_supported("T"));
        assert!(hotkey_key_is_supported("Z"));
        // Everything `letter_to_code` has no arm for. "F5" and "Space"
        // are the tempting ones — a config carrying either aborts the
        // launch, so the editor must never write them.
        for bad in ["", "t", "F5", "Space", "TT", "1", "Ü"] {
            assert!(!hotkey_key_is_supported(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn hotkey_display_uses_logical_name() {
        let cfg = Config::default();
        // The displayed string is logical, not OS-mapped (it's a UI affordance, not a key event).
        // On every OS, a default config shows "Cmd+Option+T" because that's how the user wrote it.
        assert_eq!(cfg.hotkey_display(), "Cmd+Option+T");
    }

    #[test]
    fn hotkey_display_no_shift() {
        let mut cfg = Config::default();
        cfg.hotkey.shift = false;
        cfg.hotkey.option = false;
        cfg.hotkey.modifier = "ctrl".into();
        cfg.hotkey.key = "Y".into();
        assert_eq!(cfg.hotkey_display(), "Ctrl+Y");
    }

    #[test]
    fn resolve_modifier_returns_native_for_cmd() {
        use crate::config::Modifier;
        let resolved = Modifier::Cmd.resolve_native();
        assert_eq!(resolved, crate::platform::cmd_modifier());
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

    #[test]
    fn default_glossary_is_enabled_with_default_path() {
        let cfg = Config::default();
        assert!(cfg.glossary.enabled);
        assert_eq!(cfg.glossary.file, "glossary.toml");
        assert!(!cfg.glossary.case_sensitive);
        assert_eq!(cfg.glossary.matching, "auto");
    }

    #[test]
    fn loads_glossary_overrides() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[glossary]
enabled = false
file = "my-glossary.toml"
case_sensitive = true
matching = "word_boundary"
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert!(!cfg.glossary.enabled);
        assert_eq!(cfg.glossary.file, "my-glossary.toml");
        assert!(cfg.glossary.case_sensitive);
        assert_eq!(cfg.glossary.matching, "word_boundary");
    }

    #[test]
    fn default_template_paths_point_at_templates_dir() {
        let cfg = Config::default();
        assert_eq!(
            cfg.templates.translate.as_deref(),
            Some("templates/translate.j2")
        );
        assert_eq!(
            cfg.templates.fix_grammar.as_deref(),
            Some("templates/fix_grammar.j2")
        );
        assert_eq!(
            cfg.templates.rewrite.as_deref(),
            Some("templates/rewrite.j2")
        );
        assert_eq!(cfg.templates.custom.as_deref(), Some("templates/custom.j2"));
    }

    #[test]
    fn loads_template_overrides() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[templates]
translate = "alt/translate.j2"
custom = ""
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.templates.translate.as_deref(), Some("alt/translate.j2"));
        // Empty string is preserved as Some("") — Task 6 treats it as "use built-in".
        assert_eq!(cfg.templates.custom.as_deref(), Some(""));
        // Other templates default to their conventional paths.
        assert_eq!(
            cfg.templates.fix_grammar.as_deref(),
            Some("templates/fix_grammar.j2")
        );
    }

    #[test]
    fn default_history_section() {
        let cfg = Config::default();
        assert!(cfg.history.enabled);
        assert_eq!(cfg.history.max_entries, 100);
        assert!(cfg.history.store_text);
        assert!(cfg.history.confirm_clear);
    }

    #[test]
    fn loads_history_overrides() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[history]
enabled = false
max_entries = 25
store_text = false
confirm_clear = false
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert!(!cfg.history.enabled);
        assert_eq!(cfg.history.max_entries, 25);
        assert!(!cfg.history.store_text);
        assert!(!cfg.history.confirm_clear);
    }

    #[test]
    fn default_history_hotkey_is_cmd_option_h() {
        let cfg = Config::default();
        assert_eq!(cfg.hotkey.history.modifier, "cmd");
        assert!(!cfg.hotkey.history.shift);
        assert!(cfg.hotkey.history.option);
        assert_eq!(cfg.hotkey.history.key, "H");
        assert!(cfg.hotkey.history.enabled);
    }

    #[test]
    fn loads_history_hotkey_disabled() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey.history]
enabled = false
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert!(!cfg.hotkey.history.enabled);
        // Defaults preserved for the rest:
        assert_eq!(cfg.hotkey.history.key, "H");
    }

    #[test]
    fn loads_history_hotkey_custom_key() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey.history]
modifier = "ctrl"
shift = false
key = "L"
enabled = true
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.hotkey.history.modifier, "ctrl");
        assert!(!cfg.hotkey.history.shift);
        assert_eq!(cfg.hotkey.history.key, "L");
        assert!(cfg.hotkey.history.enabled);
    }

    #[test]
    fn default_selection_hotkey_is_cmd_option_y() {
        let cfg = Config::default();
        assert_eq!(cfg.hotkey.selection.modifier, "cmd");
        assert!(!cfg.hotkey.selection.shift);
        assert!(cfg.hotkey.selection.option);
        assert_eq!(cfg.hotkey.selection.key, "Y");
        assert!(cfg.hotkey.selection.enabled);
        assert_eq!(cfg.hotkey.selection.copy_delay_ms, 80);
    }

    #[test]
    fn loads_selection_hotkey_override() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey.selection]
modifier = "ctrl"
shift = false
key = "K"
enabled = false
copy_delay_ms = 150
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.hotkey.selection.modifier, "ctrl");
        assert!(!cfg.hotkey.selection.shift);
        assert_eq!(cfg.hotkey.selection.key, "K");
        assert!(!cfg.hotkey.selection.enabled);
        assert_eq!(cfg.hotkey.selection.copy_delay_ms, 150);
    }

    #[test]
    fn default_replace_hotkey_is_super_option_u() {
        let cfg = Config::default();
        assert_eq!(cfg.hotkey.replace.modifier, "super");
        assert!(!cfg.hotkey.replace.shift);
        assert!(cfg.hotkey.replace.option);
        assert_eq!(cfg.hotkey.replace.key, "U");
        assert!(cfg.hotkey.replace.enabled);
        assert_eq!(cfg.hotkey.replace.copy_delay_ms, 80);
        assert_eq!(cfg.hotkey.replace.default_slot, 1);
    }

    #[test]
    fn loads_replace_hotkey_override() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[hotkey.replace]
modifier = "ctrl"
shift = true
key = "N"
enabled = false
copy_delay_ms = 120
default_slot = 3
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.hotkey.replace.modifier, "ctrl");
        assert!(cfg.hotkey.replace.shift);
        assert_eq!(cfg.hotkey.replace.key, "N");
        assert!(!cfg.hotkey.replace.enabled);
        assert_eq!(cfg.hotkey.replace.copy_delay_ms, 120);
        assert_eq!(cfg.hotkey.replace.default_slot, 3);
    }

    #[test]
    fn invalid_glossary_matching_is_config_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "[glossary]\nmatching = \"regex\"\n").unwrap();
        let err = Config::load(f.path()).unwrap_err();
        match err {
            TranslateError::Config(msg) => {
                assert!(msg.contains("glossary.matching"), "msg: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn invalid_api_key_source_is_config_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "[provider.api_key]\nsource = \"plaintext\"\n").unwrap();
        let err = Config::load(f.path()).unwrap_err();
        match err {
            TranslateError::Config(msg) => {
                assert!(msg.contains("provider.api_key.source"), "msg: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn file_api_key_source_is_accepted() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            "[provider.api_key]\nsource = \"file\"\npath = \"/tmp/x\"\n"
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.provider.api_key.source, "file");
        assert_eq!(cfg.provider.api_key.path, "/tmp/x");
    }

    #[test]
    fn zero_history_max_entries_is_config_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "[history]\nmax_entries = 0\n").unwrap();
        let err = Config::load(f.path()).unwrap_err();
        match err {
            TranslateError::Config(msg) => {
                assert!(msg.contains("history.max_entries"), "msg: {msg}");
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    #[test]
    fn full_config_round_trips_every_section() {
        let mut cfg = Config::default();
        cfg.provider.kind = "gemini".into();
        cfg.provider.api_key.source = "keychain".into();
        cfg.hotkey.history.enabled = false;
        cfg.hotkey.selection.key = "K".into();
        cfg.glossary.matching = "substring".into();
        cfg.history.store_text = false;
        let f = NamedTempFile::new().unwrap();
        cfg.persist(f.path()).unwrap();
        let loaded = Config::load(f.path()).unwrap();
        assert_eq!(loaded.provider.kind, "gemini");
        assert_eq!(loaded.provider.api_key.source, "keychain");
        assert!(!loaded.hotkey.history.enabled);
        assert_eq!(loaded.hotkey.selection.key, "K");
        assert_eq!(loaded.glossary.matching, "substring");
        assert!(!loaded.history.store_text);
    }

    #[test]
    fn normalize_fixes_stale_model_when_type_changes() {
        // Simulate: user had Anthropic, then changed type to "deepseek"
        // without updating model or base_url.
        let toml = r#"
[provider]
type = "deepseek"
model = "claude-haiku-4-5-20251001"
base_url = "https://api.anthropic.com/v1"
timeout_seconds = 30

[provider.api_key]
source = "file"
account = "anthropic"
"#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.normalize();
        assert_eq!(cfg.provider.kind, "deepseek");
        assert_eq!(cfg.provider.model, "deepseek-v4-flash");
        assert_eq!(cfg.provider.base_url, "https://api.deepseek.com/v1");
        assert_eq!(cfg.provider.api_key.account, "deepseek");
    }

    #[test]
    fn normalize_preserves_custom_model() {
        // User set a custom model — should never be overwritten.
        let toml = r#"
[provider]
type = "deepseek"
model = "deepseek-v4-pro"
base_url = "https://api.deepseek.com/v1"
timeout_seconds = 30

[provider.api_key]
source = "file"
account = "deepseek"
"#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.normalize();
        assert_eq!(cfg.provider.model, "deepseek-v4-pro");
        assert_eq!(cfg.provider.base_url, "https://api.deepseek.com/v1");
        assert_eq!(cfg.provider.api_key.account, "deepseek");
    }

    #[test]
    fn normalize_noop_when_already_correct() {
        let toml = r#"
[provider]
type = "anthropic"
model = "claude-haiku-4-5-20251001"
base_url = "https://api.anthropic.com/v1"
timeout_seconds = 30

[provider.api_key]
source = "file"
account = "anthropic"
"#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.normalize();
        assert_eq!(cfg.provider.model, "claude-haiku-4-5-20251001");
        assert_eq!(cfg.provider.base_url, "https://api.anthropic.com/v1");
        assert_eq!(cfg.provider.api_key.account, "anthropic");
    }

    #[test]
    fn normalize_resets_stale_slot_3_to_deutsch_formell() {
        // Pre-expansion configs had slot_3 = Türkçe/tr; new default is
        // Deutsch (formell)/de.
        let toml = r#"
[provider]
type = "anthropic"

[provider.api_key]
source = "file"
account = "anthropic"

[languages.slot_1]
label = "English"
code = "en"

[languages.slot_2]
label = "Deutsch"
code = "de"

[languages.slot_3]
label = "Türkçe"
code = "tr"
"#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.normalize();
        assert_eq!(cfg.languages.slot_3.label, "Deutsch (formell)");
        assert_eq!(cfg.languages.slot_3.code, "de");
        // Slots 4 and 5 fall back to defaults (Türkçe / Türkçe (resmî)).
        assert_eq!(cfg.languages.slot_4.label, "Türkçe");
        assert_eq!(cfg.languages.slot_5.label, "Türkçe (resmî)");
    }

    #[test]
    fn normalize_preserves_customized_slot_3() {
        let toml = r#"
[provider]
type = "anthropic"

[provider.api_key]
source = "file"
account = "anthropic"

[languages.slot_3]
label = "Français"
code = "fr"
"#;
        let mut cfg: Config = toml::from_str(toml).unwrap();
        cfg.normalize();
        assert_eq!(cfg.languages.slot_3.label, "Français");
        assert_eq!(cfg.languages.slot_3.code, "fr");
    }

    #[test]
    fn persist_round_trips_through_load() {
        let mut cfg = Config::default();
        cfg.provider.kind = "openai".into();
        cfg.provider.api_key.source = "keychain".into();
        cfg.provider.api_key.account = "openai".into();
        let f = NamedTempFile::new().unwrap();
        cfg.persist(f.path()).unwrap();
        let reloaded = Config::load(f.path()).unwrap();
        assert_eq!(reloaded.provider.kind, "openai");
        assert_eq!(reloaded.provider.api_key.source, "keychain");
        assert_eq!(reloaded.provider.api_key.account, "openai");
    }
}
