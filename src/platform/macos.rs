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

    fn reduced_motion(&self) -> bool {
        match Command::new("defaults")
            .args(["read", "-g", "NSReduceMotionEnabled"])
            .output()
        {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout);
                parse_reduce_motion_output(&s)
            }
            // Any failure (key unset, defaults missing, sandbox denied)
            // → assume reduce-motion is off. Spec a11y baseline accepts
            // false-negative > false-positive here.
            _ => false,
        }
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

/// Parse `defaults read -g NSReduceMotionEnabled` output. Treats "1" as
/// true; anything else (including missing-key, "0", garbage) as false.
fn parse_reduce_motion_output(s: &str) -> bool {
    s.trim() == "1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_process_trusted_does_not_panic() {
        let _ = is_process_trusted();
    }

    #[test]
    fn parse_defaults_output_handles_known_values() {
        assert!(parse_reduce_motion_output("1\n"));
        assert!(parse_reduce_motion_output(" 1 "));
        assert!(!parse_reduce_motion_output("0\n"));
        assert!(!parse_reduce_motion_output("garbage"));
        assert!(!parse_reduce_motion_output(""));
    }

    #[test]
    fn macos_reduced_motion_does_not_panic() {
        let _ = MacOsPlatform.reduced_motion();
    }
}
