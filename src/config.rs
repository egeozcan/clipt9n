//! `config.toml` loader. Reads the spec §6 schema. M1–M3 only consumed
//! `[provider]`, `[provider.api_key]`, `[languages]`, `[hotkey]`, `[ui]`.
//! M4 added `[glossary]` and `[templates]`. M5 adds `[history]` and the
//! nested `[hotkey.history]` sub-table. Defaults apply when fields are
//! absent; unknown user-authored fields are rejected so typos fail closed.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::TranslateError;

/// Resolve a user-authored path beneath the configuration directory.
///
/// Absolute paths and parent traversal are rejected lexically. Existing
/// targets (or the nearest existing ancestor of a missing target) are then
/// canonicalized so symlinks cannot escape the configuration directory.
pub fn resolve_confined_path(
    config_dir: &Path,
    configured_path: &str,
) -> Result<PathBuf, TranslateError> {
    let authored = Path::new(configured_path);
    if authored.is_absolute() {
        return Err(TranslateError::Config(format!(
            "configured path must be relative to the configuration directory: {configured_path}"
        )));
    }

    let mut relative = PathBuf::new();
    for component in authored.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(TranslateError::Config(format!(
                    "configured path resolves outside the configuration directory: {configured_path}"
                )));
            }
        }
    }

    let root = match std::fs::symlink_metadata(config_dir) {
        Ok(_) => std::fs::canonicalize(config_dir).map_err(|e| {
            TranslateError::Config(format!(
                "resolving configuration directory {}: {e}",
                config_dir.display()
            ))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if config_dir
                .components()
                .any(|component| component == Component::ParentDir)
            {
                return Err(TranslateError::Config(format!(
                    "missing configuration directory must not contain parent traversal: {}",
                    config_dir.display()
                )));
            }
            let absolute = if config_dir.is_absolute() {
                config_dir.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|e| {
                        TranslateError::Config(format!("resolving current directory: {e}"))
                    })?
                    .join(config_dir)
            };
            let mut existing = absolute.as_path();
            let mut missing_components = Vec::new();
            loop {
                match std::fs::symlink_metadata(existing) {
                    Ok(_) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let component = existing.file_name().ok_or_else(|| {
                            TranslateError::Config(format!(
                                "could not resolve missing configuration directory {}",
                                config_dir.display()
                            ))
                        })?;
                        missing_components.push(component.to_os_string());
                        existing = existing.parent().ok_or_else(|| {
                            TranslateError::Config(format!(
                                "could not resolve missing configuration directory {}",
                                config_dir.display()
                            ))
                        })?;
                    }
                    Err(e) => {
                        return Err(TranslateError::Config(format!(
                            "checking configuration directory {}: {e}",
                            config_dir.display()
                        )));
                    }
                }
            }
            let mut missing_root = std::fs::canonicalize(existing).map_err(|e| {
                TranslateError::Config(format!(
                    "resolving configuration directory ancestor {}: {e}",
                    existing.display()
                ))
            })?;
            for component in missing_components.into_iter().rev() {
                missing_root.push(component);
            }
            return Ok(missing_root.join(relative));
        }
        Err(e) => {
            return Err(TranslateError::Config(format!(
                "checking configuration directory {}: {e}",
                config_dir.display()
            )));
        }
    };
    let candidate = root.join(relative);

    let mut existing = candidate.as_path();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    TranslateError::Config(format!(
                        "configured path resolves outside the configuration directory: {configured_path}"
                    ))
                })?;
            }
            Err(e) => {
                return Err(TranslateError::Config(format!(
                    "checking configured path {}: {e}",
                    candidate.display()
                )));
            }
        }
    }

    let target_exists = existing == candidate;
    let unresolved_suffix = candidate
        .strip_prefix(existing)
        .expect("existing path is an ancestor of the configured candidate")
        .to_path_buf();
    let resolved_existing = std::fs::canonicalize(existing).map_err(|e| {
        TranslateError::Config(format!(
            "resolving configured path {}: {e}",
            existing.display()
        ))
    })?;
    if !resolved_existing.starts_with(&root) {
        return Err(TranslateError::Config(format!(
            "configured path resolves outside the configuration directory: {configured_path}"
        )));
    }

    if target_exists {
        Ok(resolved_existing)
    } else {
        Ok(resolved_existing.join(unresolved_suffix))
    }
}

/// Parsed and validated provider base URL.
#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    url: reqwest::Url,
}

impl ProviderEndpoint {
    /// Parse a provider endpoint. HTTPS is always required for remote hosts;
    /// HTTP is accepted only for loopback hosts when the provider profile
    /// explicitly opts into local HTTP.
    pub fn parse(value: &str, allow_loopback_http: bool) -> Result<Self, TranslateError> {
        let mut url = reqwest::Url::parse(value).map_err(|e| {
            TranslateError::Config(format!("provider.base_url is not a valid URL: {e}"))
        })?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(TranslateError::Config(
                "provider.base_url must not contain embedded credentials".into(),
            ));
        }
        let host = url.host_str().ok_or_else(|| {
            TranslateError::Config("provider.base_url must include a host".into())
        })?;
        let host_without_brackets = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host_without_brackets
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        match url.scheme() {
            "https" => {}
            "http" if allow_loopback_http && loopback => {}
            "http" if loopback => {
                return Err(TranslateError::Config(
                    "provider.base_url permits loopback HTTP only for local provider profiles"
                        .into(),
                ));
            }
            "http" => {
                return Err(TranslateError::Config(
                    "provider.base_url must use HTTPS for remote hosts".into(),
                ));
            }
            scheme => {
                return Err(TranslateError::Config(format!(
                    "provider.base_url must use HTTP or HTTPS; got {scheme}"
                )));
            }
        }

        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Self { url })
    }

    pub fn request_url(&self, relative_path: &str) -> reqwest::Url {
        self.url
            .join(relative_path.trim_start_matches('/'))
            .expect("provider request paths are valid URL path segments")
    }

    pub fn same_origin(&self, other: &Self) -> bool {
        self.url.scheme() == other.url.scheme()
            && self.url.host_str().map(str::to_ascii_lowercase)
                == other.url.host_str().map(str::to_ascii_lowercase)
            && self.url.port_or_known_default() == other.url.port_or_known_default()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct LanguageSlot {
    pub label: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
        cfg.normalize();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject configs the rest of the app can't honor. Called by `load`
    /// after parsing, and by the settings editor before it commits an
    /// edited config to disk — the GUI must fail the same way the file
    /// loader would, or a save could produce a config that won't load.
    pub fn validate(&self) -> Result<(), TranslateError> {
        self.provider_endpoint()?;

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

    pub fn provider_endpoint(&self) -> Result<ProviderEndpoint, TranslateError> {
        let profile = crate::llm::profiles::provider_profile(&self.provider.kind)?;
        ProviderEndpoint::parse(&self.provider.base_url, profile.allow_loopback_http)
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
    fn load_normalizes_stale_ollama_url_after_switch_to_openai() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[provider]
type = "openai"
model = "llama3.2"
base_url = "http://localhost:11434/v1"

[provider.api_key]
account = "ollama"
"#
        )
        .unwrap();

        let cfg = Config::load(file.path()).unwrap();

        assert_eq!(cfg.provider.kind, "openai");
        assert_eq!(cfg.provider.model, "gpt-4o-mini");
        assert_eq!(cfg.provider.base_url, "https://api.openai.com/v1");
        assert_eq!(cfg.provider.api_key.account, "openai");
    }

    #[test]
    fn load_rejects_custom_loopback_http_for_openai() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[provider]
type = "openai"
base_url = "http://localhost:9999/custom"
"#
        )
        .unwrap();

        let err = Config::load(file.path()).unwrap_err();
        assert!(
            err.to_string().contains("local provider profiles"),
            "error: {err}"
        );
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
    fn rejects_unknown_fields_in_user_authored_config_sections() {
        for contents in [
            "[history]\nstore_txt = false\n",
            "[provider]\nmodle = \"typo\"\n",
            "[hotkey.history]\nshfit = true\n",
            "[templates]\ntransalte = \"templates/translate.j2\"\n",
            "[glossary]\nmatcing = \"auto\"\n",
        ] {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{contents}").unwrap();

            let err = Config::load(file.path()).unwrap_err();
            assert!(err.to_string().contains("unknown field"), "error: {err}");
        }
    }

    #[test]
    fn provider_endpoint_accepts_https_and_loopback_http_when_allowed() {
        assert!(ProviderEndpoint::parse("https://api.openai.com/v1", false).is_ok());
        for endpoint in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:11434/v1",
        ] {
            assert!(
                ProviderEndpoint::parse(endpoint, true).is_ok(),
                "endpoint: {endpoint}"
            );
        }
    }

    #[test]
    fn provider_endpoint_rejects_unsafe_or_invalid_urls() {
        for (endpoint, allow_loopback) in [
            ("http://example.com/v1", false),
            ("http://example.com/v1", true),
            ("http://127.0.0.1:11434/v1", false),
            ("https://user@example.com/v1", false),
            ("https://user:password@example.com/v1", false),
            ("not a url", false),
            ("file:///tmp/socket", false),
        ] {
            assert!(
                ProviderEndpoint::parse(endpoint, allow_loopback).is_err(),
                "endpoint should be rejected: {endpoint}"
            );
        }
    }

    #[test]
    fn provider_endpoint_builds_request_urls_and_compares_origins() {
        let endpoint = ProviderEndpoint::parse("https://api.example.com/v1", false).unwrap();
        assert_eq!(
            endpoint.request_url("chat/completions").as_str(),
            "https://api.example.com/v1/chat/completions"
        );
        let same = ProviderEndpoint::parse("https://api.example.com/other", false).unwrap();
        let changed = ProviderEndpoint::parse("https://other.example.com/v1", false).unwrap();
        assert!(endpoint.same_origin(&same));
        assert!(!endpoint.same_origin(&changed));
    }

    #[test]
    fn config_allows_loopback_http_only_for_local_provider_profile() {
        let mut cfg = Config::default();
        cfg.provider.base_url = "http://127.0.0.1:11434/v1".into();
        assert!(cfg.validate().is_err());

        cfg.provider.kind = "ollama".into();
        assert!(cfg.validate().is_ok());

        cfg.provider.base_url = "http://example.com/v1".into();
        assert!(cfg.validate().is_err());
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
