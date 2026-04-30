//! macOS-specific platform integration.
//!
//! Provides Accessibility-permission detection so the app can surface a tray
//! warning when the global hotkey can't be registered.
//!
//! We call `AXIsProcessTrusted` directly via FFI rather than going through the
//! `objc2-application-services` wrapper. The wrapper's surface for the
//! `WithOptions` variant is awkward (NSDictionary + CFBoolean dance) and the
//! prompting behavior of `AXIsProcessTrustedWithOptions` is unreliable for
//! first-launch anyway — the binary has to already be in the Accessibility
//! list for macOS to show its own dialog.

use std::ffi::c_void;
use std::os::raw::{c_char, c_long};
use std::process::Command;

use super::Platform;
use crate::error::TranslateError;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

// AppKit is used for `[NSApplication sharedApplication]
// setActivationPolicy:]`. The lib-name link is what pulls the framework
// in; the actual symbol lookup goes through the Objective-C runtime
// below.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    // Declared as a bare `fn()` because objc_msgSend has no fixed
    // signature on arm64 — callers transmute it to the per-call
    // signature at the call site (the standard Rust idiom for
    // direct objc FFI without the `objc2` crate).
    fn objc_msgSend();
}

#[derive(Default)]
pub struct MacOsPlatform;

impl Platform for MacOsPlatform {
    fn ensure_hotkey_permissions(&self) -> Result<(), TranslateError> {
        accessibility_probe_result(is_process_trusted())
    }

    fn reduced_motion(&self) -> bool {
        // stderr is intentionally discarded: `domain ... does not exist` fires
        // for users who have never toggled NSReduceMotionEnabled and is the
        // expected case, not an error worth logging.
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

    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::error::TranslateError::Internal(format!("open: {e}")))
    }

    fn set_dock_visible(&self, visible: bool) {
        // Must be called on the main thread after NSApplication is
        // initialized — main.rs invokes this from the eframe creator
        // closure, which winit guarantees runs on the main thread
        // post-NSApp init.
        let policy = if visible {
            NS_APPLICATION_ACTIVATION_POLICY_REGULAR
        } else {
            NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY
        };
        unsafe { set_activation_policy(policy) };
    }

    fn activate_app(&self) {
        unsafe { activate_ignoring_other_apps() };
    }
}

// NSApplicationActivationPolicy values from AppKit/NSApplication.h.
const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: c_long = 0;
const NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY: c_long = 1;

type Id = *mut c_void;
type Sel = *mut c_void;
type Class = *mut c_void;

/// Resolve `[NSApplication sharedApplication]`. Safe to call from any
/// thread (lazily creates the singleton if missing), but anything
/// downstream of the returned id must be on the main thread.
unsafe fn shared_application() -> Id {
    let cls: Class = objc_getClass(c"NSApplication".as_ptr());
    let shared_sel: Sel = sel_registerName(c"sharedApplication".as_ptr());
    let shared: extern "C" fn(Class, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
    shared(cls, shared_sel)
}

/// Call `[[NSApplication sharedApplication] setActivationPolicy: policy]`.
///
/// Safety: must be called on the main thread.
unsafe fn set_activation_policy(policy: c_long) {
    let app: Id = shared_application();
    let policy_sel: Sel = sel_registerName(c"setActivationPolicy:".as_ptr());
    let set_policy: extern "C" fn(Id, Sel, c_long) -> bool =
        std::mem::transmute(objc_msgSend as *const ());
    let _ = set_policy(app, policy_sel, policy);
}

/// Call `[[NSApplication sharedApplication] activateIgnoringOtherApps:YES]`.
/// Required for accessory-policy apps to come to the foreground.
///
/// Safety: must be called on the main thread.
unsafe fn activate_ignoring_other_apps() {
    let app: Id = shared_application();
    let activate_sel: Sel = sel_registerName(c"activateIgnoringOtherApps:".as_ptr());
    let activate: extern "C" fn(Id, Sel, bool) = std::mem::transmute(objc_msgSend as *const ());
    activate(app, activate_sel, true);
}

/// Returns true if the current process has Accessibility permission.
fn is_process_trusted() -> bool {
    // Safety: `AXIsProcessTrusted` is a documented Apple C API in
    // ApplicationServices.framework. It takes no arguments, returns a Boolean,
    // and is safe to call from any thread.
    unsafe { AXIsProcessTrusted() }
}

fn accessibility_probe_result(is_trusted: bool) -> Result<(), TranslateError> {
    if is_trusted {
        Ok(())
    } else {
        Err(TranslateError::AccessibilityPermissionDenied)
    }
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
    fn accessibility_probe_is_side_effect_free() {
        assert!(accessibility_probe_result(true).is_ok());
        assert!(matches!(
            accessibility_probe_result(false),
            Err(TranslateError::AccessibilityPermissionDenied)
        ));
    }

    // Shells out to `defaults` on macOS; <50ms on real hardware but may slow
    // or fail in sandboxed CI environments. Mark `#[ignore]` if that happens.
    #[test]
    fn macos_reduced_motion_does_not_panic() {
        let _ = MacOsPlatform.reduced_motion();
    }
}
