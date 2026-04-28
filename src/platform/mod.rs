//! Cross-platform abstraction layer. Per the design doc, all
//! `#[cfg(target_os = …)]` and `#[cfg(unix)]` blocks in the codebase live
//! inside this module (M8 grep-lint enforces this with one exception
//! documented in `config::Modifier::resolve_native`).

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

/// Construct the active platform impl for this build.
pub fn current() -> ActivePlatform {
    // Per-OS impls start as unit structs but may grow fields; using `default()`
    // keeps the call site stable.
    #[allow(clippy::default_constructed_unit_structs)]
    ActivePlatform::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_constructs() {
        let _ = current();
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
}
