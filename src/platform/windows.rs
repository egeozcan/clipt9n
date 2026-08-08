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

const VK_SHIFT_VALUE: u16 = 0x10;
const VK_CONTROL_VALUE: u16 = 0x11;
const VK_ALT_VALUE: u16 = 0x12;
const VK_C_VALUE: u16 = 0x43;
const VK_V_VALUE: u16 = 0x56;
const VK_LEFT_WINDOWS_VALUE: u16 = 0x5b;
const VK_RIGHT_WINDOWS_VALUE: u16 = 0x5c;
const MODIFIER_KEYS: [u16; 5] = [
    VK_CONTROL_VALUE,
    VK_ALT_VALUE,
    VK_SHIFT_VALUE,
    VK_LEFT_WINDOWS_VALUE,
    VK_RIGHT_WINDOWS_VALUE,
];
const MODIFIER_RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
const MODIFIER_RELEASE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
const MODIFIER_RELEASE_MAX_WAITS: usize = 50;

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

fn cleanup_for_inserted_prefix(
    inputs: &[KeyboardInputSpec],
    inserted: usize,
) -> Vec<KeyboardInputSpec> {
    let mut logically_down = Vec::new();
    for input in inputs.iter().take(inserted.min(inputs.len())) {
        if input.key_up {
            if let Some(position) = logically_down
                .iter()
                .rposition(|key| *key == input.virtual_key)
            {
                logically_down.remove(position);
            }
        } else if !logically_down.contains(&input.virtual_key) {
            logically_down.push(input.virtual_key);
        }
    }
    logically_down
        .into_iter()
        .rev()
        .map(KeyboardInputSpec::key_up)
        .collect()
}

fn send_keyboard_inputs_with(
    inputs: &[KeyboardInputSpec],
    max_waits: usize,
    mut modifiers_are_down: impl FnMut() -> bool,
    mut sleep: impl FnMut(std::time::Duration),
    mut insert: impl FnMut(&[KeyboardInputSpec]) -> usize,
) -> Result<(), crate::error::TranslateError> {
    for wait in 0..=max_waits {
        if !modifiers_are_down() {
            let inserted = insert(inputs);
            if inserted == inputs.len() {
                return Ok(());
            }

            let cleanup = cleanup_for_inserted_prefix(inputs, inserted);
            let cleanup_inserted = if cleanup.is_empty() {
                0
            } else {
                insert(&cleanup)
            };
            return Err(crate::error::TranslateError::Internal(format!(
                "SendInput inserted {inserted} of {} keyboard events; cleanup inserted \
                 {cleanup_inserted} of {} key-release events. Release any stuck keys and try again",
                inputs.len(),
                cleanup.len()
            )));
        }
        if wait == max_waits {
            return Err(crate::error::TranslateError::Internal(format!(
                "Windows hotkey modifiers remained pressed for {} ms; release Ctrl, Alt, Shift, or Windows keys and try again",
                MODIFIER_RELEASE_TIMEOUT.as_millis()
            )));
        }
        sleep(MODIFIER_RELEASE_POLL_INTERVAL);
    }
    unreachable!("bounded modifier wait always returns")
}

#[cfg(target_os = "windows")]
fn native_modifiers_are_down() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    MODIFIER_KEYS.iter().any(|key| {
        // SAFETY: GetAsyncKeyState accepts any virtual-key code. The high bit
        // is set while the key is physically down.
        unsafe { GetAsyncKeyState(i32::from(*key)) < 0 }
    })
}

#[cfg(target_os = "windows")]
fn insert_native_keyboard_inputs(inputs: &[KeyboardInputSpec]) -> usize {
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
    unsafe {
        SendInput(
            native_inputs.len() as u32,
            native_inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        ) as usize
    }
}

fn send_keyboard_inputs(inputs: &[KeyboardInputSpec]) -> Result<(), crate::error::TranslateError> {
    #[cfg(target_os = "windows")]
    {
        send_keyboard_inputs_with(
            inputs,
            MODIFIER_RELEASE_MAX_WAITS,
            native_modifiers_are_down,
            std::thread::sleep,
            insert_native_keyboard_inputs,
        )
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
    use std::cell::{Cell, RefCell};

    #[test]
    fn already_released_modifiers_inject_without_sleeping() {
        let checks = Cell::new(0);
        let sleeps = Cell::new(0);
        let injections = Cell::new(0);
        let inputs = copy_chord_inputs();

        let result = send_keyboard_inputs_with(
            &inputs,
            MODIFIER_RELEASE_MAX_WAITS,
            || {
                checks.set(checks.get() + 1);
                false
            },
            |_| sleeps.set(sleeps.get() + 1),
            |batch| {
                injections.set(injections.get() + 1);
                batch.len()
            },
        );

        assert!(result.is_ok());
        assert_eq!(checks.get(), 1);
        assert_eq!(sleeps.get(), 0);
        assert_eq!(injections.get(), 1);
    }

    #[test]
    fn modifiers_releasing_before_timeout_then_injects() {
        let checks = Cell::new(0);
        let sleeps = Cell::new(0);
        let injections = Cell::new(0);
        let inputs = paste_chord_inputs();

        let result = send_keyboard_inputs_with(
            &inputs,
            MODIFIER_RELEASE_MAX_WAITS,
            || {
                let check = checks.get();
                checks.set(check + 1);
                check < 2
            },
            |_| sleeps.set(sleeps.get() + 1),
            |batch| {
                injections.set(injections.get() + 1);
                batch.len()
            },
        );

        assert!(result.is_ok());
        assert_eq!(checks.get(), 3);
        assert_eq!(sleeps.get(), 2);
        assert_eq!(injections.get(), 1);
    }

    #[test]
    fn modifier_release_timeout_returns_actionable_error_without_injection() {
        let checks = Cell::new(0);
        let sleeps = Cell::new(0);
        let injections = Cell::new(0);
        let inputs = copy_chord_inputs();

        let error = send_keyboard_inputs_with(
            &inputs,
            MODIFIER_RELEASE_MAX_WAITS,
            || {
                checks.set(checks.get() + 1);
                true
            },
            |_| sleeps.set(sleeps.get() + 1),
            |batch| {
                injections.set(injections.get() + 1);
                batch.len()
            },
        )
        .unwrap_err();

        assert_eq!(checks.get(), MODIFIER_RELEASE_MAX_WAITS + 1);
        assert_eq!(sleeps.get(), MODIFIER_RELEASE_MAX_WAITS);
        assert_eq!(injections.get(), 0);
        assert!(error
            .to_string()
            .contains("release Ctrl, Alt, Shift, or Windows"));
        assert!(error.to_string().contains("try again"));
    }

    #[test]
    fn partial_insertion_attempts_key_up_cleanup_and_reports_counts() {
        let inputs = copy_chord_inputs();
        let batches = RefCell::new(Vec::<Vec<KeyboardInputSpec>>::new());
        let calls = Cell::new(0);

        let error = send_keyboard_inputs_with(
            &inputs,
            0,
            || false,
            |_| unreachable!("already released"),
            |batch| {
                batches.borrow_mut().push(batch.to_vec());
                let call = calls.get();
                calls.set(call + 1);
                if call == 0 {
                    2
                } else {
                    batch.len()
                }
            },
        )
        .unwrap_err();

        assert_eq!(
            batches.into_inner(),
            vec![
                inputs.to_vec(),
                vec![
                    KeyboardInputSpec::key_up(VK_C_VALUE),
                    KeyboardInputSpec::key_up(VK_CONTROL_VALUE),
                ],
            ]
        );
        assert!(error.to_string().contains("inserted 2 of 4"));
        assert!(error.to_string().contains("cleanup inserted 2 of 2"));
    }

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
