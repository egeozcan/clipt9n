use std::os::windows::ffi::OsStrExt;

use super::Platform;

#[link(name = "Kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

#[derive(Default)]
pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn replace_file(
        &self,
        source: &std::path::Path,
        destination: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: both buffers are owned, NUL-terminated UTF-16 paths and
        // remain alive for the duration of the call.
        let replaced = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

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

    fn paste_from_clipboard(&self) -> Result<(), crate::error::TranslateError> {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "$wshell = New-Object -ComObject wscript.shell; $wshell.SendKeys('^v')",
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
