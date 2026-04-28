use thiserror::Error;

/// Unified error type for all translator operations.
///
/// Display strings are user-facing — they appear in stderr (CLI) and toast
/// notifications (later milestones). Keep them short and actionable.
#[derive(Debug, Error)]
pub enum TranslateError {
    #[error("clipboard is empty or not text")]
    EmptyOrNonTextClipboard,

    #[error("API key not found: set {env_var} or run setup wizard")]
    MissingApiKey { env_var: String },

    #[error("config error: {0}")]
    Config(String),

    #[error("template error: {0}")]
    Template(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("provider error ({status}): {message}")]
    Provider { status: u16, message: String },

    #[error("rate limited; try again later")]
    RateLimited,

    #[error("translation timed out")]
    Timeout,

    #[error("unsupported language code '{0}'; add a slot to [languages] in config.toml")]
    UnsupportedLanguage(String),

    #[error("invalid clipboard contents: {0}")]
    InvalidClipboard(String),

    #[error("macOS Accessibility permission not granted; the global hotkey cannot be registered without it. Open System Settings → Privacy & Security → Accessibility and enable clipt9n.")]
    AccessibilityPermissionDenied,

    #[error("internal error: {0}")]
    Internal(String),

    #[error("glossary error: {0}")]
    Glossary(String),
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
