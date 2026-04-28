//! OS notifications. Currently used only for the post-translation
//! "Translation copied" toast.

use crate::error::TranslateError;

/// Show a "Translation copied" toast. The body is a short identifier of
/// the action just performed (e.g., "Translate to Deutsch", "Fix grammar").
/// Failures are non-fatal — caller logs and continues.
pub fn translation_copied(action_label: &str) -> Result<(), TranslateError> {
    notify_rust::Notification::new()
        .summary("Translation copied")
        .body(action_label)
        .appname("clipt9n")
        .timeout(notify_rust::Timeout::Milliseconds(2500))
        .show()
        .map(|_| ())
        .map_err(|e| TranslateError::Config(format!("notification failed: {e}")))
}

#[cfg(test)]
mod tests {
    // Notifications are inherently a side-effect on the user's session;
    // there's no headless way to assert delivery. We only verify that the
    // function compiles and the call doesn't panic when invoked from a
    // headless test runner. (`show()` may fail in CI; we accept that.)
    #[test]
    fn translation_copied_does_not_panic() {
        let _ = super::translation_copied("Fix grammar");
    }
}
