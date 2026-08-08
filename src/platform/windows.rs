#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use super::Platform;

#[cfg(target_os = "windows")]
#[link(name = "Kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

#[cfg(target_os = "windows")]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
#[cfg(target_os = "windows")]
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

fn wide_null_terminated(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn wide_path_null_terminated(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[derive(Default)]
pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn replace_file(
        &self,
        source: &std::path::Path,
        destination: &std::path::Path,
    ) -> Result<(), std::io::Error> {
        #[cfg(target_os = "windows")]
        {
            let source = wide_path_null_terminated(source);
            let destination = wide_path_null_terminated(destination);
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
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (source, destination);
            unreachable!("WindowsPlatform::replace_file is only active on Windows")
        }
    }

    fn open_path(&self, path: &std::path::Path) -> Result<(), crate::error::TranslateError> {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::UI::Shell::ShellExecuteW;
            use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

            let operation = wide_null_terminated("open");
            let target = wide_path_null_terminated(path);
            // SAFETY: all pointers reference null-terminated buffers that
            // remain alive for the call; optional parameters are null.
            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                )
            };
            if result as isize > 32 {
                Ok(())
            } else {
                Err(crate::error::TranslateError::Internal(format!(
                    "ShellExecuteW failed with code {}",
                    result as isize
                )))
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = path;
            unreachable!("WindowsPlatform::open_path is only active on Windows")
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_execute_path_preserves_shell_metacharacters_as_data() {
        let path = r#"C:\A & B\pipe | name\quoted \"file\".txt"#;
        let encoded = wide_null_terminated(path);
        let decoded = String::from_utf16(&encoded[..encoded.len() - 1]).unwrap();

        assert_eq!(decoded, path);
        assert_eq!(encoded.last(), Some(&0));
    }
}
