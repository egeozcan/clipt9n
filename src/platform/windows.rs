use super::Platform;

#[derive(Default)]
pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::error::TranslateError::Internal(format!("start: {e}")))
    }

    fn copy_selection_to_clipboard(&self) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "$wshell = New-Object -ComObject wscript.shell; $wshell.SendKeys('^c')",
            ])
            .status()
            .map_err(|e| crate::error::TranslateError::Internal(format!("powershell: {e}")))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(crate::error::TranslateError::Internal(format!(
                        "powershell exited with status {status}"
                    )))
                }
            })
    }
}
