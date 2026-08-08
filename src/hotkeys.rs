//! Validation and registration for every configurable global hotkey.

use std::fmt;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

use crate::config::{Modifier, NativeModifier};

#[derive(Debug, Clone, Copy)]
pub struct HotkeyBinding<'a> {
    pub name: &'a str,
    pub modifier: &'a str,
    pub option: bool,
    pub shift: bool,
    pub key: &'a str,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyWarning {
    InvalidModifier { binding: String, modifier: String },
    InvalidKey { binding: String, key: String },
    RegistrationFailed { binding: String, reason: String },
}

impl fmt::Display for HotkeyWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModifier { binding, modifier } => {
                write!(f, "{binding} hotkey has unknown modifier {modifier:?}")
            }
            Self::InvalidKey { binding, key } => {
                write!(f, "{binding} hotkey has unsupported key {key:?}")
            }
            Self::RegistrationFailed { binding, reason } => {
                write!(f, "{binding} hotkey registration failed: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationOutcome {
    pub id: Option<u32>,
    pub warning: Option<HotkeyWarning>,
}

/// Validate and register one binding. Invalid or disabled bindings never call
/// `register`, so there is no implicit fallback shortcut.
pub fn register_binding<E>(
    binding: HotkeyBinding<'_>,
    register: impl FnOnce(HotKey) -> Result<(), E>,
) -> RegistrationOutcome
where
    E: fmt::Display,
{
    if !binding.enabled {
        return RegistrationOutcome {
            id: None,
            warning: None,
        };
    }

    let Some(modifier) = Modifier::parse(binding.modifier) else {
        return RegistrationOutcome {
            id: None,
            warning: Some(HotkeyWarning::InvalidModifier {
                binding: binding.name.to_owned(),
                modifier: binding.modifier.to_owned(),
            }),
        };
    };
    let Some(key) = letter_to_code(binding.key) else {
        return RegistrationOutcome {
            id: None,
            warning: Some(HotkeyWarning::InvalidKey {
                binding: binding.name.to_owned(),
                key: binding.key.to_owned(),
            }),
        };
    };

    let mut modifiers = match modifier.resolve_native() {
        NativeModifier::Ctrl => Modifiers::CONTROL,
        NativeModifier::Alt => Modifiers::ALT,
        NativeModifier::Meta => Modifiers::META,
    };
    if binding.option {
        modifiers |= Modifiers::ALT;
    }
    if binding.shift {
        modifiers |= Modifiers::SHIFT;
    }

    let hotkey = HotKey::new(Some(modifiers), key);
    let id = hotkey.id();
    match register(hotkey) {
        Ok(()) => RegistrationOutcome {
            id: Some(id),
            warning: None,
        },
        Err(error) => RegistrationOutcome {
            id: None,
            warning: Some(HotkeyWarning::RegistrationFailed {
                binding: binding.name.to_owned(),
                reason: error.to_string(),
            }),
        },
    }
}

pub fn has_registration_warnings<'a>(
    outcomes: impl IntoIterator<Item = &'a RegistrationOutcome>,
) -> bool {
    outcomes
        .into_iter()
        .any(|outcome| outcome.warning.is_some())
}

fn letter_to_code(key: &str) -> Option<Code> {
    match key.to_ascii_uppercase().as_str() {
        "A" => Some(Code::KeyA),
        "B" => Some(Code::KeyB),
        "C" => Some(Code::KeyC),
        "D" => Some(Code::KeyD),
        "E" => Some(Code::KeyE),
        "F" => Some(Code::KeyF),
        "G" => Some(Code::KeyG),
        "H" => Some(Code::KeyH),
        "I" => Some(Code::KeyI),
        "J" => Some(Code::KeyJ),
        "K" => Some(Code::KeyK),
        "L" => Some(Code::KeyL),
        "M" => Some(Code::KeyM),
        "N" => Some(Code::KeyN),
        "O" => Some(Code::KeyO),
        "P" => Some(Code::KeyP),
        "Q" => Some(Code::KeyQ),
        "R" => Some(Code::KeyR),
        "S" => Some(Code::KeyS),
        "T" => Some(Code::KeyT),
        "U" => Some(Code::KeyU),
        "V" => Some(Code::KeyV),
        "W" => Some(Code::KeyW),
        "X" => Some(Code::KeyX),
        "Y" => Some(Code::KeyY),
        "Z" => Some(Code::KeyZ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn binding<'a>(modifier: &'a str, key: &'a str, enabled: bool) -> HotkeyBinding<'a> {
        HotkeyBinding {
            name: "history",
            modifier,
            option: true,
            shift: true,
            key,
            enabled,
        }
    }

    #[test]
    fn invalid_modifier_does_not_attempt_registration() {
        let attempts = Cell::new(0);
        let outcome = register_binding(binding("hyper", "H", true), |_| {
            attempts.set(attempts.get() + 1);
            Ok::<(), &'static str>(())
        });

        assert_eq!(attempts.get(), 0);
        assert_eq!(outcome.id, None);
        assert_eq!(
            outcome.warning,
            Some(HotkeyWarning::InvalidModifier {
                binding: "history".into(),
                modifier: "hyper".into(),
            })
        );
    }

    #[test]
    fn invalid_key_does_not_register_or_fall_back() {
        let attempts = Cell::new(0);
        let outcome = register_binding(binding("cmd", "F12", true), |_| {
            attempts.set(attempts.get() + 1);
            Ok::<(), &'static str>(())
        });

        assert_eq!(
            attempts.get(),
            0,
            "invalid input must not register a fallback"
        );
        assert_eq!(outcome.id, None);
        assert_eq!(
            outcome.warning,
            Some(HotkeyWarning::InvalidKey {
                binding: "history".into(),
                key: "F12".into(),
            })
        );
    }

    #[test]
    fn disabled_binding_is_not_validated_or_registered() {
        let attempts = Cell::new(0);
        let outcome = register_binding(binding("invalid", "invalid", false), |_| {
            attempts.set(attempts.get() + 1);
            Ok::<(), &'static str>(())
        });

        assert_eq!(attempts.get(), 0);
        assert_eq!(
            outcome,
            RegistrationOutcome {
                id: None,
                warning: None,
            }
        );
    }

    #[test]
    fn registration_conflict_is_a_warning() {
        let attempts = Cell::new(0);
        let outcome = register_binding(binding("cmd", "H", true), |_| {
            attempts.set(attempts.get() + 1);
            Err("already registered")
        });

        assert_eq!(attempts.get(), 1);
        assert_eq!(outcome.id, None);
        assert_eq!(
            outcome.warning,
            Some(HotkeyWarning::RegistrationFailed {
                binding: "history".into(),
                reason: "already registered".into(),
            })
        );
    }

    #[test]
    fn history_warning_is_included_in_aggregate() {
        let prompt = register_binding(binding("cmd", "T", true), |_| Ok::<(), &str>(()));
        let history = register_binding(binding("cmd", "H", true), |_| Err("conflict"));

        assert!(has_registration_warnings([&prompt, &history]));
    }
}
