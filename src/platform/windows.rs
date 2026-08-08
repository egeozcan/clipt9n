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

const VK_CONTROL_VALUE: u16 = 0x11;
const VK_C_VALUE: u16 = 0x43;
const VK_V_VALUE: u16 = 0x56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyboardInputSpec {
    virtual_key: u16,
    key_up: bool,
}

impl KeyboardInputSpec {
    const fn key_down(virtual_key: u16) -> Self {
        Self {
            virtual_key,
            key_up: false,
        }
    }

    const fn key_up(virtual_key: u16) -> Self {
        Self {
            virtual_key,
            key_up: true,
        }
    }
}

fn keyboard_chord_inputs(key: u16) -> [KeyboardInputSpec; 4] {
    [
        KeyboardInputSpec::key_down(VK_CONTROL_VALUE),
        KeyboardInputSpec::key_down(key),
        KeyboardInputSpec::key_up(key),
        KeyboardInputSpec::key_up(VK_CONTROL_VALUE),
    ]
}

fn copy_chord_inputs() -> [KeyboardInputSpec; 4] {
    keyboard_chord_inputs(VK_C_VALUE)
}

fn paste_chord_inputs() -> [KeyboardInputSpec; 4] {
    keyboard_chord_inputs(VK_V_VALUE)
}

fn send_keyboard_inputs(inputs: &[KeyboardInputSpec]) -> Result<(), crate::error::TranslateError> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        };

        let native_inputs: Vec<INPUT> = inputs
            .iter()
            .map(|input| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: input.virtual_key,
                        wScan: 0,
                        dwFlags: if input.key_up { KEYEVENTF_KEYUP } else { 0 },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
            .collect();
        // SAFETY: `native_inputs` is a contiguous initialized INPUT array and
        // remains alive for the duration of the call.
        let inserted = unsafe {
            SendInput(
                native_inputs.len() as u32,
                native_inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if inserted == native_inputs.len() as u32 {
            Ok(())
        } else {
            Err(crate::error::TranslateError::Internal(format!(
                "SendInput inserted {inserted} of {} keyboard events",
                native_inputs.len()
            )))
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = inputs;
        unreachable!("Windows keyboard input is only active on Windows")
    }
}

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
        send_keyboard_inputs(&copy_chord_inputs())
    }

    fn paste_from_clipboard(&self) -> Result<(), crate::error::TranslateError> {
        send_keyboard_inputs(&paste_chord_inputs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_chord_constructs_balanced_native_keyboard_inputs() {
        assert_eq!(
            copy_chord_inputs(),
            [
                KeyboardInputSpec::key_down(VK_CONTROL_VALUE),
                KeyboardInputSpec::key_down(VK_C_VALUE),
                KeyboardInputSpec::key_up(VK_C_VALUE),
                KeyboardInputSpec::key_up(VK_CONTROL_VALUE),
            ]
        );
    }

    #[test]
    fn paste_chord_constructs_balanced_native_keyboard_inputs() {
        assert_eq!(
            paste_chord_inputs(),
            [
                KeyboardInputSpec::key_down(VK_CONTROL_VALUE),
                KeyboardInputSpec::key_down(VK_V_VALUE),
                KeyboardInputSpec::key_up(VK_V_VALUE),
                KeyboardInputSpec::key_up(VK_CONTROL_VALUE),
            ]
        );
    }

    #[test]
    fn windows_desktop_io_has_no_process_launch_path() {
        let source = include_str!("windows.rs");
        let process_launch = ["Command", "::new"].concat();
        let legacy_shell = ["power", "shell"].concat();
        let command_shell = ["cmd", ".exe"].concat();
        assert!(!source.contains(&process_launch));
        assert!(!source.contains(&legacy_shell));
        assert!(!source.contains(&command_shell));
    }

    #[test]
    fn shell_execute_path_preserves_shell_metacharacters_as_data() {
        let path = r#"C:\A & B\pipe | name\quoted \"file\".txt"#;
        let encoded = wide_null_terminated(path);
        let decoded = String::from_utf16(&encoded[..encoded.len() - 1]).unwrap();

        assert_eq!(decoded, path);
        assert_eq!(encoded.last(), Some(&0));
    }
}
