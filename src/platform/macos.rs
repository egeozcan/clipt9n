//! macOS-specific platform integration.
//!
//! Provides Accessibility-permission detection so the user gets a clear error
//! (and a one-click open of System Settings) when the global hotkey can't be
//! registered.
//!
//! We call `AXIsProcessTrusted` directly via FFI rather than going through the
//! `objc2-application-services` wrapper. The wrapper's surface for the
//! `WithOptions` variant is awkward (NSDictionary + CFBoolean dance) and the
//! prompting behavior of `AXIsProcessTrustedWithOptions` is unreliable for
//! first-launch anyway — the binary has to already be in the Accessibility
//! list for macOS to show its own dialog. Our `open` call covers that case
//! more reliably.

use std::process::Command;

use super::Platform;
use crate::error::TranslateError;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[derive(Default)]
pub struct MacOsPlatform;

impl Platform for MacOsPlatform {
    fn ensure_hotkey_permissions(&self) -> Result<(), TranslateError> {
        if is_process_trusted() {
            return Ok(());
        }
        // Best-effort: bring the user to the right pane in System Settings.
        // Failure (e.g. `open` missing) doesn't change the outcome — we still
        // surface the permission error to the caller.
        let _ = open_accessibility_settings();
        Err(TranslateError::AccessibilityPermissionDenied)
    }
}

/// Returns true if the current process has Accessibility permission.
fn is_process_trusted() -> bool {
    // Safety: `AXIsProcessTrusted` is a documented Apple C API in
    // ApplicationServices.framework. It takes no arguments, returns a Boolean,
    // and is safe to call from any thread.
    unsafe { AXIsProcessTrusted() }
}

fn open_accessibility_settings() -> std::io::Result<()> {
    Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_process_trusted_does_not_panic() {
        let _ = is_process_trusted();
    }
}
