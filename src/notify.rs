//! OS notifications. Currently used only for the post-translation
//! "Translation copied" toast.

use crate::{error::TranslateError, platform::Platform};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

const ERROR_PRESENTATION_MAX_CHARS: usize = 512;

static NOTIFICATION_APPLICATION_RESULT: OnceLock<Result<(), String>> = OnceLock::new();

/// Show a "Translation copied" toast. The body is a short identifier of
/// the action just performed (e.g., "Translate to Deutsch", "Fix grammar").
/// Failures are non-fatal — caller logs and continues.
pub fn translation_copied(action_label: &str, translated: &str) -> Result<(), TranslateError> {
    show(
        "Translation copied",
        &translation_copied_body(action_label, translated, false),
        3500,
    )
}

/// Show a translation-failure toast.
pub fn translation_failed(err: &TranslateError) -> Result<(), TranslateError> {
    show("Translation failed", &error_presentation(err), 4000)
}

/// Show a selected-text capture failure toast.
pub fn selection_capture_failed(err: &TranslateError) -> Result<(), TranslateError> {
    show("No selected text copied", &error_presentation(err), 3000)
}

/// Tell the user an inline result is ready but its original target is no longer safe.
pub fn inline_result_ready_for_manual_paste() -> Result<(), TranslateError> {
    show(
        "Inline result copied",
        "The original app is no longer active. Paste the result manually.",
        4000,
    )
}

/// Show a notification when inline replacement failed because slot is not inlineable.
pub fn inline_replace_not_inlineable(
    slot: u8,
    action_label: Option<&str>,
) -> Result<(), TranslateError> {
    let body = if let Some(label) = action_label {
        format!("Slot {slot} ({label}) is not inlineable.")
    } else {
        format!("Slot {slot} is not inlineable.")
    };
    show("Inline Replacement Skipped", &body, 3000)
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

fn translation_copied_body(
    action_label: &str,
    _translated: &str,
    _include_preview: bool,
) -> String {
    action_label.to_string()
}

fn error_presentation(err: &TranslateError) -> String {
    let sanitized = err
        .to_string()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if sanitized.chars().count() <= ERROR_PRESENTATION_MAX_CHARS {
        sanitized
    } else {
        let bounded: String = sanitized
            .chars()
            .take(ERROR_PRESENTATION_MAX_CHARS.saturating_sub(1))
            .collect();
        format!("{bounded}…")
    }
}

/// Once-per-session warning flag. Set to true when `configure_notifications`
/// fails so the warn log fires exactly once, not on every notification attempt.
static NOTIFICATION_WARNED: AtomicBool = AtomicBool::new(false);

fn ensure_notification_application() -> Result<(), TranslateError> {
    let result = NOTIFICATION_APPLICATION_RESULT.get_or_init(|| {
        crate::platform::current()
            .configure_notifications()
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            if !NOTIFICATION_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(error = %e, "notifications unavailable for this session");
            }
            Err(notification_error(e.clone()))
        }
    }
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
    fn translation_notification_omits_result_text_by_default() {
        let body =
            super::translation_copied_body("Translate to Deutsch", "private medical text", false);
        assert_eq!(body, "Translate to Deutsch");
        assert!(!body.contains("medical"));
    }

    #[test]
    fn error_presentation_is_bounded_and_sanitized() {
        let message = format!("{}\u{1b}[31m\u{7}", "x".repeat(100 * 1024));
        let err = crate::error::TranslateError::Provider {
            status: 500,
            message,
        };
        let presented = super::error_presentation(&err);

        assert_eq!(
            presented.chars().count(),
            super::ERROR_PRESENTATION_MAX_CHARS
        );
        assert!(!presented
            .chars()
            .any(|c| c.is_control() && !c.is_whitespace()));
    }
}
