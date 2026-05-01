use super::Platform;

#[derive(Default)]
pub struct LinuxPlatform;

impl Platform for LinuxPlatform {
    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::error::TranslateError::Internal(format!("xdg-open: {e}")))
    }

    fn copy_selection_to_clipboard(&self) -> Result<(), crate::error::TranslateError> {
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
}
