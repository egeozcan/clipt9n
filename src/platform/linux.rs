#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{Platform, SelectionAutomation};

const WAYLAND_UNSUPPORTED: &str = "Selected-text copy/paste automation is unavailable on native Wayland. Log in to an X11 session to use the xdotool-based shortcuts.";

fn selection_automation_for_session(
    session_type: Option<&str>,
    wayland_display: Option<&str>,
) -> SelectionAutomation {
    let native_wayland = session_type
        .map(|value| value.eq_ignore_ascii_case("wayland"))
        .unwrap_or_else(|| wayland_display.is_some());
    if native_wayland {
        SelectionAutomation::Unsupported(WAYLAND_UNSUPPORTED)
    } else {
        SelectionAutomation::Supported
    }
}

fn require_selection_automation() -> Result<(), crate::error::TranslateError> {
    match selection_automation_for_session(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    ) {
        SelectionAutomation::Supported => Ok(()),
        SelectionAutomation::Unsupported(message) => {
            Err(crate::error::TranslateError::Internal(message.into()))
        }
    }
}

#[derive(Default)]
pub struct LinuxPlatform;

impl Platform for LinuxPlatform {
    fn selection_automation(&self) -> SelectionAutomation {
        selection_automation_for_session(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        )
    }

    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::error::TranslateError::Internal(format!("xdg-open: {e}")))
    }

    fn copy_selection_to_clipboard(&self) -> Result<(), crate::error::TranslateError> {
        require_selection_automation()?;
        std::process::Command::new("xdotool")
            .args(["key", "ctrl+c"])
            .status()
            .map_err(|e| crate::error::TranslateError::Internal(format!("xdotool: {e}")))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(crate::error::TranslateError::Internal(format!(
                        "xdotool exited with status {status}"
                    )))
                }
            })
    }

    fn paste_from_clipboard(&self) -> Result<(), crate::error::TranslateError> {
        require_selection_automation()?;
        std::process::Command::new("xdotool")
            .args(["key", "ctrl+v"])
            .status()
            .map_err(|e| crate::error::TranslateError::Internal(format!("xdotool: {e}")))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(crate::error::TranslateError::Internal(format!(
                        "xdotool exited with status {status}"
                    )))
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_wayland_reports_actionable_unsupported_capability() {
        let capability = selection_automation_for_session(Some("wayland"), Some("wayland-0"));

        let super::SelectionAutomation::Unsupported(message) = capability else {
            panic!("native Wayland must not claim xdotool automation support");
        };
        assert!(message.contains("Wayland"));
        assert!(message.contains("X11"));
    }

    #[test]
    fn x11_session_supports_xdotool_automation() {
        assert_eq!(
            selection_automation_for_session(Some("x11"), None),
            super::SelectionAutomation::Supported
        );
    }
}
