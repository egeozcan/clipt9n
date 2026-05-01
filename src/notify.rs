//! OS notifications. Currently used only for the post-translation
//! "Translation copied" toast.

use crate::{error::TranslateError, platform::Platform};
use std::sync::OnceLock;

const RESULT_PREVIEW_MAX_CHARS: usize = 120;

static NOTIFICATION_APPLICATION_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

/// Show a "Translation copied" toast. The body is a short identifier of
/// the action just performed (e.g., "Translate to Deutsch", "Fix grammar").
/// Failures are non-fatal — caller logs and continues.
pub fn translation_copied(action_label: &str, translated: &str) -> Result<(), TranslateError> {
    show(
        "Translation copied",
        &translation_copied_body(action_label, translated),
        3500,
    )
}

/// Show a translation-failure toast.
pub fn translation_failed(err: &TranslateError) -> Result<(), TranslateError> {
    show("Translation failed", &err.to_string(), 4000)
}

/// Show a selected-text capture failure toast.
pub fn selection_capture_failed(err: &TranslateError) -> Result<(), TranslateError> {
    show("No selected text copied", &err.to_string(), 3000)
}

fn show(summary: &str, body: &str, timeout_ms: u32) -> Result<(), TranslateError> {
    ensure_notification_application()?;
    notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .appname("clipt9n")
        .timeout(notify_rust::Timeout::Milliseconds(timeout_ms))
        .show()
        .map(|_| ())
        .map_err(notification_error)
}

fn notification_error<E: std::fmt::Display>(e: E) -> TranslateError {
    TranslateError::Config(format!("notification failed: {e}"))
}

fn translation_copied_body(action_label: &str, translated: &str) -> String {
    let preview = notification_preview(translated);
    if preview.is_empty() {
        action_label.to_string()
    } else {
        format!("{action_label}\n{preview}")
    }
}

fn notification_preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let preview: String = chars.by_ref().take(RESULT_PREVIEW_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn ensure_notification_application() -> Result<(), TranslateError> {
    NOTIFICATION_APPLICATION_RESULT
        .get_or_init(|| {
            crate::platform::current()
                .configure_notifications()
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(notification_error)
}

#[cfg(test)]
mod tests {
    // Notifications are inherently a side-effect on the user's session;
    // there's no headless way to assert delivery. We only verify that the
    // function compiles and the call doesn't panic when invoked from a
    // headless test runner. (`show()` may fail in CI; we accept that.)
    #[test]
    fn translation_copied_does_not_panic() {
        let _ = super::translation_copied("Fix grammar", "Fixed text.");
    }

    #[test]
    fn translation_copied_body_includes_action_and_result_preview() {
        let body = super::translation_copied_body(
            "Translate to Deutsch",
            "Das ist\n\n eine   kurze Vorschau.",
        );

        assert_eq!(body, "Translate to Deutsch\nDas ist eine kurze Vorschau.");
    }

    #[test]
    fn translation_preview_is_truncated_with_ellipsis() {
        let body = super::translation_copied_body("Rewrite for clarity", &"x".repeat(130));

        assert_eq!(
            body.chars().count(),
            "Rewrite for clarity\n".chars().count() + 121
        );
        assert!(body.ends_with('…'));
    }
}
