use std::fmt;

/// Maximum number of characters emitted when an error is formatted for a
/// user-facing surface or structured log field.
pub const ERROR_DISPLAY_MAX_CHARS: usize = 512;

/// Unified error type for all translator operations.
///
/// Dynamic values remain available structurally for programmatic handling,
/// while `Display` is bounded and control-character sanitized because it is
/// used directly by notifications, stderr, and tracing fields.
#[derive(Debug)]
pub enum TranslateError {
    EmptyOrNonTextClipboard,
    MissingApiKey { env_var: String },
    Config(String),
    Template(String),
    Network(String),
    Provider { status: u16, message: String },
    RateLimited,
    Timeout,
    UnsupportedLanguage(String),
    InvalidClipboard(String),
    AccessibilityPermissionDenied,
    Internal(String),
    Glossary(String),
    History(String),
    SetupWizard(String),
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let raw = match self {
            Self::EmptyOrNonTextClipboard => "clipboard is empty or not text".to_string(),
            Self::MissingApiKey { env_var } => {
                format!("API key not found: set {env_var} or run setup wizard")
            }
            Self::Config(message) => format!("config error: {message}"),
            Self::Template(message) => format!("template error: {message}"),
            Self::Network(message) => format!("network error: {message}"),
            Self::Provider { status, message } => {
                format!("provider error ({status}): {message}")
            }
            Self::RateLimited => "rate limited; try again later".to_string(),
            Self::Timeout => "translation timed out".to_string(),
            Self::UnsupportedLanguage(code) => format!(
                "unsupported language code '{code}'; add a slot to [languages] in config.toml"
            ),
            Self::InvalidClipboard(message) => {
                format!("invalid clipboard contents: {message}")
            }
            Self::AccessibilityPermissionDenied => "macOS Accessibility permission not granted; the global hotkey cannot be registered without it. Open System Settings → Privacy & Security → Accessibility and enable clipt9n.".to_string(),
            Self::Internal(message) => format!("internal error: {message}"),
            Self::Glossary(message) => format!("glossary error: {message}"),
            Self::History(message) => format!("history error: {message}"),
            Self::SetupWizard(message) => format!("setup wizard error: {message}"),
        };
        f.write_str(&bounded_sanitized(&raw))
    }
}

impl std::error::Error for TranslateError {}

fn bounded_sanitized(text: &str) -> String {
    let sanitized = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = sanitized.chars();
    let bounded: String = chars.by_ref().take(ERROR_DISPLAY_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_display_is_bounded_and_has_no_non_whitespace_controls() {
        let err = TranslateError::Provider {
            status: 500,
            message: format!("{}\u{1b}[31m\u{7}", "x".repeat(100 * 1024)),
        };

        let displayed = err.to_string();

        assert!(displayed.chars().count() <= ERROR_DISPLAY_MAX_CHARS + 1);
        assert!(!displayed
            .chars()
            .any(|c| c.is_control() && !c.is_whitespace()));
        assert!(displayed.starts_with("provider error (500):"));
    }

    #[test]
    fn display_strings_are_user_facing() {
        assert_eq!(
            TranslateError::EmptyOrNonTextClipboard.to_string(),
            "clipboard is empty or not text"
        );
        assert_eq!(
            TranslateError::MissingApiKey {
                env_var: "ANTHROPIC_API_KEY".into()
            }
            .to_string(),
            "API key not found: set ANTHROPIC_API_KEY or run setup wizard"
        );
        assert_eq!(
            TranslateError::Provider {
                status: 503,
                message: "service unavailable".into()
            }
            .to_string(),
            "provider error (503): service unavailable"
        );
        assert_eq!(
            TranslateError::UnsupportedLanguage("fr".into()).to_string(),
            "unsupported language code 'fr'; add a slot to [languages] in config.toml"
        );
        assert_eq!(
            TranslateError::Glossary("malformed entry at line 5".into()).to_string(),
            "glossary error: malformed entry at line 5"
        );
        assert_eq!(
            TranslateError::History("encrypted db unreadable".into()).to_string(),
            "history error: encrypted db unreadable"
        );
        assert_eq!(
            TranslateError::SetupWizard("keychain unavailable".into()).to_string(),
            "setup wizard error: keychain unavailable"
        );
    }
}
