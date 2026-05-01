//! Cross-platform abstraction layer. Per the design doc, all
//! `#[cfg(target_os = …)]` and `#[cfg(unix)]` blocks in the codebase live
//! inside this module (M8 grep-lint enforces this).

use crate::error::TranslateError;

/// Behavior an OS may need to provide to the rest of the app. Per-OS impls
/// in `macos.rs`, `linux.rs`, `windows.rs`. Defaults are no-ops.
pub trait Platform {
    /// Verify the OS-level prerequisites for registering a global hotkey.
    /// On macOS this checks Accessibility permission. On Linux/Windows this
    /// is a no-op. Returns an error with user-actionable messaging if the
    /// prereq is missing.
    fn ensure_hotkey_permissions(&self) -> Result<(), TranslateError> {
        Ok(())
    }

    /// Whether the user has requested reduced motion at the OS level.
    /// Default `false`. macOS reads `defaults read -g NSReduceMotionEnabled`;
    /// if the key is absent (most users) or the query fails, returns `false`.
    /// Other OSes use the default until a per-platform query is added.
    fn reduced_motion(&self) -> bool {
        false
    }

    /// Open a path in the OS default handler. macOS shells out to
    /// `open`; Linux to `xdg-open`; Windows to `cmd.exe /C start`.
    /// Best-effort — returns `Err` if the helper isn't on PATH; the
    /// caller (M6 wizard, M7 tray menu) logs warn and stays open.
    fn open_path(&self, path: &std::path::Path) -> Result<(), TranslateError> {
        let _ = path;
        Err(TranslateError::Internal(
            "open_path not implemented on this platform".into(),
        ))
    }

    /// Hide or show the app's Dock / taskbar / app-switcher presence.
    /// Menu-bar / tray-only apps call this with `false` at startup.
    /// macOS sets `NSApplicationActivationPolicyAccessory`. Windows
    /// taskbar suppression is handled at viewport-construction time
    /// via `ViewportBuilder::with_taskbar(false)`. Linux is a no-op
    /// because tray-only apps don't create top-level windows that
    /// would appear in the WM's task list. Best-effort: failures log
    /// at warn and are not propagated, since this is purely cosmetic.
    fn set_dock_visible(&self, visible: bool) {
        let _ = visible;
    }

    /// Bring the app to the foreground. Required for accessory-policy
    /// macOS apps that have no Dock icon — without this, sending
    /// `ViewportCommand::Focus` to a window when the user has tabbed
    /// away to another app does nothing because the OS keeps the other
    /// app activated. Calls `[NSApp activateIgnoringOtherApps:YES]` on
    /// macOS; no-op on Linux/Windows where window-level focus is
    /// sufficient.
    fn activate_app(&self) {}

    /// Open the OS settings page where the user can grant global hotkey /
    /// accessibility permission. macOS opens the Accessibility privacy pane;
    /// other platforms return an actionable unsupported error.
    fn open_accessibility_settings(&self) -> Result<(), TranslateError> {
        Err(TranslateError::Internal(
            "accessibility settings are not available on this platform".into(),
        ))
    }

    /// Ask the foreground app to copy its current selection to the system
    /// clipboard. This intentionally performs only the copy gesture; callers
    /// decide whether and how to restore prior clipboard contents.
    fn copy_selection_to_clipboard(&self) -> Result<(), TranslateError> {
        Err(TranslateError::Internal(
            "selected-text capture is not implemented on this platform".into(),
        ))
    }

    /// Platform clipboard generation, when available. macOS exposes
    /// NSPasteboard.changeCount, which lets selection capture distinguish
    /// "copy produced the same text" from "copy did nothing".
    fn clipboard_change_count(&self) -> Option<i64> {
        None
    }

    /// Configure OS notification delivery for the app. macOS needs the
    /// bundle identifier registered with `notify-rust`; other platforms
    /// use the default notification backend behavior.
    fn configure_notifications(&self) -> Result<(), TranslateError> {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacOsPlatform as ActivePlatform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as ActivePlatform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WindowsPlatform as ActivePlatform;

#[cfg(unix)]
mod unix;

/// Construct the active platform impl for this build.
pub fn current() -> ActivePlatform {
    // Per-OS impls start as unit structs but may grow fields; using `default()`
    // keeps the call site stable.
    #[allow(clippy::default_constructed_unit_structs)]
    ActivePlatform::default()
}

/// Resolve logical "cmd" to the OS-native hotkey modifier.
pub fn cmd_modifier() -> crate::config::NativeModifier {
    cmd_modifier_impl()
}

#[cfg(target_os = "macos")]
fn cmd_modifier_impl() -> crate::config::NativeModifier {
    crate::config::NativeModifier::Meta
}

#[cfg(not(target_os = "macos"))]
fn cmd_modifier_impl() -> crate::config::NativeModifier {
    crate::config::NativeModifier::Ctrl
}

#[cfg(unix)]
pub(crate) fn set_owner_only_permissions(path: &std::path::Path) -> Result<(), std::io::Error> {
    unix::set_owner_only_permissions(path)
}

#[cfg(not(unix))]
pub(crate) fn set_owner_only_permissions(_path: &std::path::Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn owner_only_permissions_are_enforced_for_test(
    path: &std::path::Path,
) -> Result<bool, std::io::Error> {
    owner_only_permissions_are_enforced_for_test_impl(path)
}

#[cfg(all(test, unix))]
fn owner_only_permissions_are_enforced_for_test_impl(
    path: &std::path::Path,
) -> Result<bool, std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
    Ok(mode == 0o600)
}

#[cfg(all(test, not(unix)))]
fn owner_only_permissions_are_enforced_for_test_impl(
    _path: &std::path::Path,
) -> Result<bool, std::io::Error> {
    Ok(true)
}

/// Install a SIGHUP-driven reload listener. On Unix (Linux/macOS) this
/// spawns a tokio task that forwards every SIGHUP delivery to `tx`. On
/// Windows this is a no-op (signal model differs; tray-menu "Reload
/// glossary" is the equivalent affordance there in M7).
///
/// Infallible: signal-install errors on Unix are logged inside the
/// spawned task rather than propagated. Callers can rely on this not
/// being a startup failure mode.
#[cfg(unix)]
pub fn install_sighup_reload(rt: &tokio::runtime::Runtime, tx: crossbeam_channel::Sender<()>) {
    unix::install(rt, tx);
}

#[cfg(not(unix))]
pub fn install_sighup_reload(_rt: &tokio::runtime::Runtime, _tx: crossbeam_channel::Sender<()>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_constructs() {
        let _ = current();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn cmd_modifier_resolves_to_macos_meta() {
        assert_eq!(cmd_modifier(), crate::config::NativeModifier::Meta);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn cmd_modifier_resolves_to_ctrl_off_macos() {
        assert_eq!(cmd_modifier(), crate::config::NativeModifier::Ctrl);
    }

    #[test]
    fn no_op_default_succeeds() {
        struct Stub;
        impl Platform for Stub {}
        assert!(Stub.ensure_hotkey_permissions().is_ok());
    }

    #[test]
    fn default_reduced_motion_is_false() {
        struct Stub;
        impl Platform for Stub {}
        assert!(!Stub.reduced_motion());
    }

    // Shells out to `defaults` on macOS; <50ms on real hardware but may slow
    // or fail in sandboxed CI environments. Mark `#[ignore]` if that happens.
    #[test]
    fn current_platform_reduced_motion_does_not_panic() {
        // Whatever the OS reports, we just need a clean call.
        let _ = current().reduced_motion();
    }

    #[test]
    fn install_sighup_reload_does_not_panic_on_current_platform() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (tx, _rx) = crossbeam_channel::unbounded::<()>();
        // Infallible — Unix installs a real listener, Windows is no-op.
        install_sighup_reload(&rt, tx);
    }
}
