//! macOS-specific platform integration.
//!
//! Provides Accessibility-permission detection so the app can surface a tray
//! warning when the global hotkey can't be registered.
//!
//! We call the Accessibility APIs directly via FFI rather than going through
//! the `objc2-application-services` wrapper. The prompt variant is best-effort
//! and paired with opening System Settings because local ad-hoc app rebuilds
//! can leave stale TCC entries behind.

use std::ffi::c_void;
use std::os::raw::{c_char, c_long};
use std::process::Command;

use super::{DestinationIdentity, Platform};
use crate::error::TranslateError;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFEqual(first: *const c_void, second: *const c_void) -> u8;
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        bytes: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    static kCFBooleanTrue: *const c_void;
    static kAXTrustedCheckOptionPrompt: *const c_void;
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
        accessibility_probe_result(is_process_trusted_with_prompt())
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

    fn open_accessibility_settings(&self) -> Result<(), TranslateError> {
        Command::new("open")
            .arg(ACCESSIBILITY_SETTINGS_URL)
            .spawn()
            .map(|_| ())
            .map_err(|e| TranslateError::Internal(format!("open accessibility settings: {e}")))
    }

    fn copy_selection_to_clipboard(&self) -> Result<(), TranslateError> {
        post_cmd_c()
    }

    fn paste_from_clipboard(&self) -> Result<(), TranslateError> {
        post_cmd_v()
    }

    fn clipboard_change_count(&self) -> Option<i64> {
        unsafe { pasteboard_change_count() }
    }

    fn active_destination_identity(&self) -> Option<DestinationIdentity> {
        unsafe { focused_destination_identity().map(DestinationIdentity::from_macos) }
    }

    fn configure_notifications(&self) -> Result<(), TranslateError> {
        notify_rust::set_application(notification_bundle_identifier())
            .map_err(|e| TranslateError::Config(format!("notification failed: {e}")))
    }

    fn frontmost_app_pid(&self) -> Option<i32> {
        unsafe { frontmost_application_pid() }
    }

    fn activate_pid(&self, pid: i32) {
        unsafe { activate_running_application(pid) };
    }
}

const NOTIFICATION_BUNDLE_ID: &str = "dev.egecan.clipt9n";
const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

fn notification_bundle_identifier() -> &'static str {
    NOTIFICATION_BUNDLE_ID
}

// NSApplicationActivationPolicy values from AppKit/NSApplication.h.
const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: c_long = 0;
const NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY: c_long = 1;

type Id = *mut c_void;
type Sel = *mut c_void;
type Class = *mut c_void;

/// Exact macOS paste destination. Accessibility elements are retained rather
/// than reduced to hashes or labels: `CFEqual` can therefore revalidate the
/// same focused window and focused UI element without collision-prone tokens.
pub(super) struct MacOsDestinationIdentity {
    process_id: i32,
    focused_window: RetainedAxElement,
    focused_ui_element: RetainedAxElement,
}

impl Clone for MacOsDestinationIdentity {
    fn clone(&self) -> Self {
        Self {
            process_id: self.process_id,
            focused_window: self.focused_window.clone(),
            focused_ui_element: self.focused_ui_element.clone(),
        }
    }
}

impl std::fmt::Debug for MacOsDestinationIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacOsDestinationIdentity")
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for MacOsDestinationIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.process_id == other.process_id
            && self.focused_window == other.focused_window
            && self.focused_ui_element == other.focused_ui_element
    }
}

impl Eq for MacOsDestinationIdentity {}

struct RetainedAxElement(*const c_void);

impl Clone for RetainedAxElement {
    fn clone(&self) -> Self {
        unsafe {
            CFRetain(self.0);
        }
        Self(self.0)
    }
}

impl PartialEq for RetainedAxElement {
    fn eq(&self, other: &Self) -> bool {
        unsafe { CFEqual(self.0, other.0) != 0 }
    }
}

impl Eq for RetainedAxElement {}

impl Drop for RetainedAxElement {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}

// AXUIElement is an immutable CFType reference. Accessibility queries may run
// from any thread, and Core Foundation retain/release is thread-safe. The
// identity is carried through the Send translation outcome but only compared
// by the main-thread desktop adapter.
unsafe impl Send for RetainedAxElement {}
unsafe impl Sync for RetainedAxElement {}

/// Capture both the focused window and the focused UI element. If either is
/// unavailable (including missing Accessibility permission), refuse to create
/// an identity so inline replacement degrades to clipboard-only.
unsafe fn focused_destination_identity() -> Option<MacOsDestinationIdentity> {
    let system_wide = RetainedAxElement::from_created(AXUIElementCreateSystemWide())?;
    let application = copy_ax_element_attribute(&system_wide, c"AXFocusedApplication".as_ptr())?;
    let mut process_id = 0;
    if AXUIElementGetPid(application.0, &mut process_id) != 0 || process_id <= 0 {
        return None;
    }
    let focused_window = copy_ax_element_attribute(&application, c"AXFocusedWindow".as_ptr())?;
    let focused_ui_element =
        copy_ax_element_attribute(&application, c"AXFocusedUIElement".as_ptr())?;

    Some(MacOsDestinationIdentity {
        process_id,
        focused_window,
        focused_ui_element,
    })
}

impl RetainedAxElement {
    unsafe fn from_created(element: *const c_void) -> Option<Self> {
        if element.is_null() || CFGetTypeID(element) != AXUIElementGetTypeID() {
            if !element.is_null() {
                CFRelease(element);
            }
            return None;
        }
        Some(Self(element))
    }
}

unsafe fn copy_ax_element_attribute(
    element: &RetainedAxElement,
    attribute_name: *const c_char,
) -> Option<RetainedAxElement> {
    // kCFStringEncodingUTF8 from CFString.h. AX attribute constants are
    // CFSTR macros rather than exported symbols, so construct their exact
    // string values here.
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    let attribute =
        CFStringCreateWithCString(std::ptr::null(), attribute_name, K_CF_STRING_ENCODING_UTF8);
    if attribute.is_null() {
        return None;
    }

    let mut value = std::ptr::null();
    let result = AXUIElementCopyAttributeValue(element.0, attribute, &mut value);
    CFRelease(attribute);
    if result != 0 {
        return None;
    }
    RetainedAxElement::from_created(value)
}

/// Resolve `[NSApplication sharedApplication]`. Safe to call from any
/// thread (lazily creates the singleton if missing), but anything
/// downstream of the returned id must be on the main thread.
unsafe fn shared_application() -> Id {
    let cls: Class = objc_getClass(c"NSApplication".as_ptr());
    let shared_sel: Sel = sel_registerName(c"sharedApplication".as_ptr());
    let shared: extern "C" fn(Class, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
    shared(cls, shared_sel)
}

/// Return `[NSPasteboard generalPasteboard].changeCount`.
unsafe fn pasteboard_change_count() -> Option<i64> {
    let cls: Class = objc_getClass(c"NSPasteboard".as_ptr());
    if cls.is_null() {
        return None;
    }
    let general_sel: Sel = sel_registerName(c"generalPasteboard".as_ptr());
    let general: extern "C" fn(Class, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
    let pasteboard = general(cls, general_sel);
    if pasteboard.is_null() {
        return None;
    }
    let change_count_sel: Sel = sel_registerName(c"changeCount".as_ptr());
    let change_count: extern "C" fn(Id, Sel) -> c_long =
        std::mem::transmute(objc_msgSend as *const ());
    Some(change_count(pasteboard, change_count_sel) as i64)
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

fn is_process_trusted_with_prompt() -> bool {
    if is_process_trusted() {
        return true;
    }

    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        if options.is_null() {
            return false;
        }
        let trusted = AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
        trusted
    }
}

fn post_cmd_c() -> Result<(), TranslateError> {
    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 1 << 20;
    const MACOS_VIRTUAL_KEY_C: u16 = 0x08;

    unsafe {
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), MACOS_VIRTUAL_KEY_C, true);
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), MACOS_VIRTUAL_KEY_C, false);
        if down.is_null() || up.is_null() {
            if !down.is_null() {
                CFRelease(down.cast_const());
            }
            if !up.is_null() {
                CFRelease(up.cast_const());
            }
            return Err(TranslateError::Internal(
                "creating Cmd+C keyboard event failed".into(),
            ));
        }
        CGEventSetFlags(down, K_CG_EVENT_FLAG_MASK_COMMAND);
        CGEventSetFlags(up, K_CG_EVENT_FLAG_MASK_COMMAND);
        CGEventPost(K_CG_HID_EVENT_TAP, down);
        CGEventPost(K_CG_HID_EVENT_TAP, up);
        CFRelease(down.cast_const());
        CFRelease(up.cast_const());
    }

    Ok(())
}

fn post_cmd_v() -> Result<(), TranslateError> {
    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 1 << 20;
    const MACOS_VIRTUAL_KEY_V: u16 = 0x09;

    unsafe {
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), MACOS_VIRTUAL_KEY_V, true);
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), MACOS_VIRTUAL_KEY_V, false);
        if down.is_null() || up.is_null() {
            if !down.is_null() {
                CFRelease(down.cast_const());
            }
            if !up.is_null() {
                CFRelease(up.cast_const());
            }
            return Err(TranslateError::Internal(
                "creating Cmd+V keyboard event failed".into(),
            ));
        }
        CGEventSetFlags(down, K_CG_EVENT_FLAG_MASK_COMMAND);
        CGEventSetFlags(up, K_CG_EVENT_FLAG_MASK_COMMAND);
        CGEventPost(K_CG_HID_EVENT_TAP, down);
        CGEventPost(K_CG_HID_EVENT_TAP, up);
        CFRelease(down.cast_const());
        CFRelease(up.cast_const());
    }

    Ok(())
}

/// Return the PID of the frontmost (focused) application via
/// `[[NSWorkspace sharedWorkspace] frontmostApplication]`.
unsafe fn frontmost_application_pid() -> Option<i32> {
    let cls: Class = objc_getClass(c"NSWorkspace".as_ptr());
    if cls.is_null() {
        return None;
    }
    let shared_sel: Sel = sel_registerName(c"sharedWorkspace".as_ptr());
    let shared: extern "C" fn(Class, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
    let workspace = shared(cls, shared_sel);
    if workspace.is_null() {
        return None;
    }
    let frontmost_sel: Sel = sel_registerName(c"frontmostApplication".as_ptr());
    let frontmost: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const ());
    let app = frontmost(workspace, frontmost_sel);
    if app.is_null() {
        return None;
    }
    let pid_sel: Sel = sel_registerName(c"processIdentifier".as_ptr());
    let pid_fn: extern "C" fn(Id, Sel) -> c_long = std::mem::transmute(objc_msgSend as *const ());
    let pid = pid_fn(app, pid_sel) as i32;
    if pid <= 0 {
        return None;
    }
    Some(pid)
}

/// Activate the application with the given PID using
/// `[NSRunningApplication runningApplicationWithProcessIdentifier:]
///  activateWithOptions: NSApplicationActivateIgnoringOtherApps]`.
///
/// Safety: must be called on the main thread.
unsafe fn activate_running_application(pid: i32) {
    let cls: Class = objc_getClass(c"NSRunningApplication".as_ptr());
    if cls.is_null() {
        return;
    }
    let running_sel: Sel = sel_registerName(c"runningApplicationWithProcessIdentifier:".as_ptr());
    let running: extern "C" fn(Class, Sel, c_long) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    let app = running(cls, running_sel, pid as c_long);
    if app.is_null() {
        return;
    }
    // NSApplicationActivateIgnoringOtherApps = 1
    let activate_sel: Sel = sel_registerName(c"activateWithOptions:".as_ptr());
    let activate: extern "C" fn(Id, Sel, c_long) = std::mem::transmute(objc_msgSend as *const ());
    activate(app, activate_sel, 1);
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

    #[test]
    fn accessibility_settings_url_is_stable() {
        assert_eq!(
            ACCESSIBILITY_SETTINGS_URL,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        );
    }

    #[test]
    fn notifications_use_app_bundle_identifier() {
        assert_eq!(notification_bundle_identifier(), "dev.egecan.clipt9n");
    }

    // Shells out to `defaults` on macOS; <50ms on real hardware but may slow
    // or fail in sandboxed CI environments. Mark `#[ignore]` if that happens.
    #[test]
    fn macos_reduced_motion_does_not_panic() {
        let _ = MacOsPlatform.reduced_motion();
    }

    #[test]
    fn focused_destination_capture_is_safe_when_available_or_unsupported() {
        if let Some(identity) = MacOsPlatform.active_destination_identity() {
            assert_eq!(identity, identity.clone());
        }
    }
}
