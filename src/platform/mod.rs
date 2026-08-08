//! Cross-platform abstraction layer. Per the design doc, all
//! `#[cfg(target_os = …)]` and `#[cfg(unix)]` blocks in the codebase live
//! inside this module (M8 grep-lint enforces this).

use crate::error::TranslateError;

/// Opaque identity for the exact desktop destination that currently owns
/// keyboard focus. On macOS this retains the Accessibility focused-window and
/// focused-UI-element identities. Platforms that cannot provide a reliable
/// identity return `None` instead of constructing this value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationIdentity(DestinationIdentityInner);

#[derive(Clone, Debug, PartialEq, Eq)]
enum DestinationIdentityInner {
    #[cfg(target_os = "macos")]
    MacOs(macos::MacOsDestinationIdentity),
    #[cfg(test)]
    Test {
        process_id: i32,
        destination_id: u64,
    },
}

impl DestinationIdentity {
    #[cfg(target_os = "macos")]
    fn from_macos(identity: macos::MacOsDestinationIdentity) -> Self {
        Self(DestinationIdentityInner::MacOs(identity))
    }

    #[cfg(test)]
    pub(crate) fn for_test(process_id: i32, destination_id: u64) -> Self {
        Self(DestinationIdentityInner::Test {
            process_id,
            destination_id,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAutomation {
    Supported,
    Unsupported(&'static str),
}

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

    /// Open a path in the OS default handler. macOS shells out to `open`,
    /// Linux to `xdg-open`, and Windows uses native `ShellExecuteW` (never a
    /// command shell). Best-effort; callers log failures and stay open.
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

    /// Whether copy/paste gesture automation is available in this desktop
    /// session. Native Wayland is explicitly unsupported until a portal or
    /// compositor adapter replaces the X11-only xdotool path.
    fn selection_automation(&self) -> SelectionAutomation {
        SelectionAutomation::Supported
    }

    /// Ask the foreground app to copy its current selection to the system
    /// clipboard. This intentionally performs only the copy gesture; callers
    /// decide whether and how to restore prior clipboard contents.
    fn copy_selection_to_clipboard(&self) -> Result<(), TranslateError> {
        Err(TranslateError::Internal(
            "selected-text capture is not implemented on this platform".into(),
        ))
    }

    /// Ask the foreground app to paste from the system clipboard.
    fn paste_from_clipboard(&self) -> Result<(), TranslateError> {
        Err(TranslateError::Internal(
            "clipboard paste is not implemented on this platform".into(),
        ))
    }

    /// Platform clipboard generation, when available. macOS exposes
    /// NSPasteboard.changeCount, which lets selection capture distinguish
    /// "copy produced the same text" from "copy did nothing".
    fn clipboard_change_count(&self) -> Option<i64> {
        None
    }

    /// Identity of the exact focused paste destination, when it can be
    /// captured and revalidated reliably. Returning `None` makes inline
    /// replacement clipboard-only; callers must never fall back to a PID.
    fn active_destination_identity(&self) -> Option<DestinationIdentity> {
        None
    }

    /// Configure OS notification delivery for the app. macOS needs the
    /// bundle identifier registered with `notify-rust`; other platforms
    /// use the default notification backend behavior.
    fn configure_notifications(&self) -> Result<(), TranslateError> {
        Ok(())
    }

    /// Return the PID (process identifier) of the currently frontmost
    /// (focused) application. Used so clipt9n can restore focus to the
    /// app the user was in before summoning the prompt window. Returns
    /// `None` on platforms where this isn't available or when the query
    /// fails.
    fn frontmost_app_pid(&self) -> Option<i32> {
        None
    }

    /// Activate (bring to foreground) the application with the given PID.
    /// Used after translation completes so the user lands back in their
    /// previous app automatically. No-op on unsupported platforms.
    fn activate_pid(&self, _pid: i32) {}

    /// Atomically replace `destination` with the same-filesystem `source`.
    /// Unix rename has replacement semantics; Windows overrides this with
    /// `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`.
    fn replace_file(
        &self,
        source: &std::path::Path,
        destination: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        std::fs::rename(source, destination)
    }
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacOsPlatform as ActivePlatform;

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxPlatform as ActivePlatform;

#[cfg(any(target_os = "windows", test))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // failure stages are constructed only by cfg(test) injection helpers
pub(crate) enum SecureWriteFailure {
    None,
    Permission,
    Rename,
}

/// Atomically replace a regular file with owner-only contents. Platforms
/// without enforceable owner-only creation semantics fail closed.
pub(crate) fn secure_atomic_write(
    path: &std::path::Path,
    contents: &[u8],
) -> Result<(), std::io::Error> {
    secure_atomic_write_impl(path, contents, SecureWriteFailure::None)
}

#[cfg(unix)]
pub(crate) fn secure_read_file(path: &std::path::Path) -> Result<Vec<u8>, std::io::Error> {
    unix::secure_read_file(path)
}

#[cfg(not(unix))]
pub(crate) fn secure_read_file(_path: &std::path::Path) -> Result<Vec<u8>, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure file-backed secret storage is unavailable on this platform; use the OS keychain",
    ))
}

/// Probe an optional legacy secret file without making keychain-only first
/// run depend on file-backed secret support. `Ok(None)` means the path is
/// absent; an existing path that cannot be read securely is an error.
#[cfg(unix)]
pub(crate) fn probe_secure_legacy_file(
    path: &std::path::Path,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    match secure_read_file(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
pub(crate) fn probe_secure_legacy_file(
    path: &std::path::Path,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!(
                "legacy secret exists at {} but secure file-backed secret storage is unavailable on this platform; move it to a Unix host for recovery",
                path.display()
            ),
        )),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn secure_atomic_write_impl(
    path: &std::path::Path,
    contents: &[u8],
    failure: SecureWriteFailure,
) -> Result<(), std::io::Error> {
    unix::secure_atomic_write(path, contents, failure)
}

#[cfg(not(unix))]
fn secure_atomic_write_impl(
    _path: &std::path::Path,
    _contents: &[u8],
    _failure: SecureWriteFailure,
) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure file-backed secret storage is unavailable on this platform; use the OS keychain",
    ))
}

#[cfg(unix)]
pub(crate) fn rename_legacy_key_to_recovery(
    source: &std::path::Path,
    recovery: &std::path::Path,
) -> Result<(), std::io::Error> {
    unix::rename_legacy_key_to_recovery(source, recovery)
}

#[cfg(not(unix))]
pub(crate) fn rename_legacy_key_to_recovery(
    _source: &std::path::Path,
    _recovery: &std::path::Path,
) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "safe legacy-key recovery rename is unavailable on this platform",
    ))
}

#[cfg(test)]
pub(crate) fn secure_atomic_write_with_failure_for_test(
    path: &std::path::Path,
    contents: &[u8],
    failure: SecureWriteFailure,
) -> Result<(), std::io::Error> {
    secure_atomic_write_impl(path, contents, failure)
}

#[cfg(all(test, unix))]
pub(crate) fn create_file_symlink_for_test(
    target: &std::path::Path,
    link: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(all(test, windows))]
pub(crate) fn create_file_symlink_for_test(
    target: &std::path::Path,
    link: &std::path::Path,
) -> Result<(), std::io::Error> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(all(test, unix))]
pub(crate) fn secure_file_storage_supported_for_test() -> bool {
    true
}

#[cfg(all(test, not(unix)))]
pub(crate) fn secure_file_storage_supported_for_test() -> bool {
    false
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
pub fn install_sighup_reload(
    rt: &tokio::runtime::Runtime,
    tx: crossbeam_channel::Sender<()>,
    wake: impl Fn() + Send + 'static,
) {
    unix::install(rt, tx, wake);
}

#[cfg(not(unix))]
pub fn install_sighup_reload(
    _rt: &tokio::runtime::Runtime,
    _tx: crossbeam_channel::Sender<()>,
    _wake: impl Fn() + Send + 'static,
) {
}

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
    fn current_platform_atomically_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("candidate.tmp");
        let destination = dir.path().join("config.toml");
        std::fs::write(&source, "new").unwrap();
        std::fs::write(&destination, "old").unwrap();

        current().replace_file(&source, &destination).unwrap();

        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "new");
        assert!(!source.exists());
    }

    #[test]
    #[cfg(not(unix))]
    fn optional_legacy_probe_distinguishes_absent_from_unsupported_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(".history-key");
        assert_eq!(probe_secure_legacy_file(&path).unwrap(), None);

        std::fs::write(&path, [7u8; 32]).unwrap();
        let err = probe_secure_legacy_file(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        assert!(err.to_string().contains("legacy secret exists"));
    }

    #[test]
    fn install_sighup_reload_does_not_panic_on_current_platform() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (tx, _rx) = crossbeam_channel::unbounded::<()>();
        // Infallible — Unix installs a real listener, Windows is no-op.
        install_sighup_reload(&rt, tx, || {});
    }
}
