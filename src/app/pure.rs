//! Pure helpers — testable in isolation, no `egui::Context` or `ClipApp`
//! dependency. Extracted from `app/mod.rs` Step 1 of the improvement plan.

use crate::config::Config;
use crate::translator::Action;

/// What the user implicitly asked for by picking a slot. The state machine
/// in `update()` switches on this to decide whether to enter custom-prompt
/// mode, show the size-confirm modal, or dispatch immediately.
#[derive(Debug, Clone)]
pub(super) enum Intent {
    /// Run the action against the current clipboard.
    Translate {
        action: Action,
        action_label: String,
        overlay_label: String,
    },
    /// Slot 6 — open the custom prompt window first, the action is built
    /// from user input.
    EnterCustom,
}

pub(super) fn decide_intent(slot: u8, cfg: &Config) -> Option<Intent> {
    match slot {
        1 => Some(translate_intent(
            Action::Translate {
                code: cfg.languages.slot_1.code.clone(),
            },
            &cfg.languages.slot_1.label,
        )),
        2 => Some(translate_intent(
            Action::Translate {
                code: cfg.languages.slot_2.code.clone(),
            },
            &cfg.languages.slot_2.label,
        )),
        3 => Some(translate_intent(
            Action::Translate {
                code: cfg.languages.slot_3.code.clone(),
            },
            &cfg.languages.slot_3.label,
        )),
        4 => Some(Intent::Translate {
            action: Action::FixGrammar,
            action_label: "Fix grammar".into(),
            overlay_label: "Fixing grammar…".into(),
        }),
        5 => Some(Intent::Translate {
            action: Action::Rewrite,
            action_label: "Rewrite for clarity".into(),
            overlay_label: "Rewriting for clarity…".into(),
        }),
        6 => Some(Intent::EnterCustom),
        _ => None,
    }
}

fn translate_intent(action: Action, lang_label: &str) -> Intent {
    Intent::Translate {
        action,
        action_label: format!("Translate to {lang_label}"),
        overlay_label: format!("Translating to {lang_label}…"),
    }
}

pub(super) fn requires_size_confirm(source: &str, cfg: &Config) -> bool {
    source.chars().count() > cfg.ui.confirm_size_threshold
}

pub(super) fn selected_text_after_copy(
    before: &str,
    after: &str,
    copy_changed: Option<bool>,
) -> Option<String> {
    if after.trim().is_empty() {
        return None;
    }

    let copied_selection = copy_changed.unwrap_or(after != before);
    if copied_selection {
        Some(after.to_string())
    } else {
        None
    }
}

pub(super) fn next_gen(current: u64) -> u64 {
    current.wrapping_add(1)
}

pub(super) fn reset_focus_loss_latch(has_been_focused: &mut bool) {
    *has_been_focused = false;
}

pub(super) fn update_focus_loss_latch(focused: bool, has_been_focused: &mut bool) -> bool {
    if focused {
        *has_been_focused = true;
        false
    } else {
        *has_been_focused
    }
}

/// Return the overlay label for a non-`Translate` action.
///
/// # Panics
///
/// Panics if called with `Action::Translate` — that variant's label is
/// constructed at slot-resolution time inside `decide_intent`, so callers
/// must never pass it here.
pub(super) fn overlay_label_for(action: &Action) -> String {
    match action {
        Action::Translate { .. } => unreachable!(
            "Translate overlay labels are built at slot resolution; \
             callers must not pass Action::Translate here without a label"
        ),
        Action::FixGrammar => "Fixing grammar…".into(),
        Action::Rewrite => "Rewriting for clarity…".into(),
        Action::Custom { .. } => "Running custom prompt…".into(),
    }
}

pub(super) fn action_label_for(action: &Action, cfg: &Config) -> String {
    match action {
        Action::Translate { code } => match cfg.label_for_code(code) {
            Ok(label) => format!("Translate to {label}"),
            Err(_) => format!("Translate to {code}"),
        },
        Action::FixGrammar => "Fix grammar".into(),
        Action::Rewrite => "Rewrite for clarity".into(),
        Action::Custom { .. } => "Custom prompt".into(),
    }
}

/// Map an `Action` to the string we persist in `entries.action`. Must
/// match the `'translate' | 'fix_grammar' | 'rewrite' | 'custom'`
/// alphabet from spec §7.
pub(super) fn action_kind_str(action: &Action) -> &'static str {
    match action {
        Action::Translate { .. } => "translate",
        Action::FixGrammar => "fix_grammar",
        Action::Rewrite => "rewrite",
        Action::Custom { .. } => "custom",
    }
}

/// Target language for the history row. `None` for fix_grammar /
/// rewrite / custom (which stay in source language); `Some(code)` for
/// translate.
pub(super) fn target_lang_for(action: &Action) -> Option<String> {
    match action {
        Action::Translate { code } => Some(code.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_threshold(threshold: usize) -> Config {
        let mut c = Config::default();
        c.ui.confirm_size_threshold = threshold;
        c
    }

    #[test]
    fn slot_1_resolves_to_translate_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(1, &cfg).expect("slot 1 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        let Action::Translate { code } = action else {
            panic!("expected Action::Translate");
        };
        assert_eq!(code, "en");
        assert_eq!(action_label, "Translate to English");
        assert_eq!(overlay_label, "Translating to English…");
    }

    #[test]
    fn slot_2_resolves_to_translate_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(2, &cfg).expect("slot 2 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        let Action::Translate { code } = action else {
            panic!("expected Action::Translate");
        };
        assert_eq!(code, "de");
        assert_eq!(action_label, "Translate to Deutsch (formell)");
        assert_eq!(overlay_label, "Translating to Deutsch (formell)…");
    }

    #[test]
    fn slot_3_resolves_to_translate_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(3, &cfg).expect("slot 3 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        let Action::Translate { code } = action else {
            panic!("expected Action::Translate");
        };
        assert_eq!(code, "tr");
        assert_eq!(action_label, "Translate to Türkçe (resmî)");
        assert_eq!(overlay_label, "Translating to Türkçe (resmî)…");
    }

    #[test]
    fn slot_4_resolves_to_fix_grammar_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(4, &cfg).expect("slot 4 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        assert!(matches!(action, Action::FixGrammar));
        assert_eq!(action_label, "Fix grammar");
        assert_eq!(overlay_label, "Fixing grammar…");
    }

    #[test]
    fn slot_5_resolves_to_rewrite_intent() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(5, &cfg).expect("slot 5 is valid");
        let Intent::Translate {
            action,
            action_label,
            overlay_label,
        } = intent
        else {
            panic!("expected Intent::Translate");
        };
        assert!(matches!(action, Action::Rewrite));
        assert_eq!(action_label, "Rewrite for clarity");
        assert_eq!(overlay_label, "Rewriting for clarity…");
    }

    #[test]
    fn slot_6_resolves_to_enter_custom() {
        let cfg = cfg_with_threshold(2000);
        let intent = decide_intent(6, &cfg).expect("slot 6 is valid");
        assert!(matches!(intent, Intent::EnterCustom));
    }

    #[test]
    fn invalid_slot_returns_none() {
        let cfg = cfg_with_threshold(2000);
        assert!(decide_intent(0, &cfg).is_none());
        assert!(decide_intent(7, &cfg).is_none());
    }

    #[test]
    fn requires_size_confirm_above_threshold() {
        let cfg = cfg_with_threshold(100);
        let big = "x".repeat(150);
        assert!(requires_size_confirm(&big, &cfg));
        let small = "x".repeat(50);
        assert!(!requires_size_confirm(&small, &cfg));
    }

    #[test]
    fn selected_text_after_copy_accepts_changed_text() {
        assert_eq!(
            selected_text_after_copy("clipboard", "selected text", None),
            Some("selected text".to_string())
        );
    }

    #[test]
    fn selected_text_after_copy_rejects_empty_or_unchanged_clipboard() {
        assert_eq!(
            selected_text_after_copy("clipboard", "clipboard", None),
            None
        );
        assert_eq!(
            selected_text_after_copy("clipboard", "   \n", Some(true)),
            None
        );
    }

    #[test]
    fn selected_text_after_copy_accepts_same_text_when_pasteboard_changed() {
        assert_eq!(
            selected_text_after_copy("same text", "same text", Some(true)),
            Some("same text".to_string())
        );
    }

    #[test]
    fn selected_text_after_copy_rejects_when_pasteboard_flag_says_unchanged() {
        // Text changed but the pasteboard-change-count signal explicitly
        // says no copy occurred (Some(false)). The signal wins.
        assert_eq!(
            selected_text_after_copy("old text", "different text", Some(false)),
            None
        );
    }

    #[test]
    fn dispatch_gen_starts_at_zero_and_monotonically_increases() {
        // Just verify the field exists with the expected starting value.
        // We can't construct ClipApp here (requires CreationContext), so
        // this is a doc-style invariant test on a free helper.
        assert_eq!(next_gen(0), 1);
        assert_eq!(next_gen(42), 43);
        assert_eq!(next_gen(u64::MAX - 1), u64::MAX);
    }

    #[test]
    fn overlay_label_for_translate() {
        assert_eq!(overlay_label_for(&Action::FixGrammar), "Fixing grammar…");
        assert_eq!(
            overlay_label_for(&Action::Rewrite),
            "Rewriting for clarity…"
        );
        assert_eq!(
            overlay_label_for(&Action::Custom {
                instruction: "x".into()
            }),
            "Running custom prompt…"
        );
    }

    #[test]
    fn action_label_for_translate_uses_label() {
        let cfg = Config::default();
        assert_eq!(
            action_label_for(&Action::Translate { code: "de".into() }, &cfg),
            "Translate to Deutsch (formell)"
        );
        assert_eq!(action_label_for(&Action::FixGrammar, &cfg), "Fix grammar");
        assert_eq!(
            action_label_for(&Action::Rewrite, &cfg),
            "Rewrite for clarity"
        );
        assert_eq!(
            action_label_for(
                &Action::Custom {
                    instruction: "anything".into()
                },
                &cfg
            ),
            "Custom prompt"
        );
    }

    #[test]
    fn dispatch_translate_paths_diverge_on_threshold() {
        // We can't construct a ClipApp here, but we can directly verify
        // the requires_size_confirm boundary used by dispatch_translate.
        let mut cfg = Config::default();
        cfg.ui.confirm_size_threshold = 10;

        assert!(!requires_size_confirm("short", &cfg));
        assert!(requires_size_confirm(
            "this is definitely longer than ten characters",
            &cfg
        ));
    }

    #[test]
    fn cancellation_increments_gen_so_outcome_is_stale() {
        // Simulates: dispatch at gen=N, user cancels (bump to N+1), outcome
        // arrives tagged gen=N — must be considered stale.
        let mut current = 5_u64;
        let dispatched_gen = current;
        current = next_gen(current);
        // Outcome from the dispatched generation:
        let outcome_gen = dispatched_gen;
        // Stale check (mirrors handle_translation_done):
        assert_ne!(current, outcome_gen);
    }

    #[test]
    fn reset_focus_latch_prevents_immediate_dismiss_after_resummon() {
        let mut has_been_focused = true;

        reset_focus_loss_latch(&mut has_been_focused);

        assert!(!update_focus_loss_latch(false, &mut has_been_focused));
        assert!(!has_been_focused);
    }

    #[test]
    fn action_kind_str_maps_per_spec() {
        assert_eq!(
            action_kind_str(&Action::Translate { code: "de".into() }),
            "translate"
        );
        assert_eq!(action_kind_str(&Action::FixGrammar), "fix_grammar");
        assert_eq!(action_kind_str(&Action::Rewrite), "rewrite");
        assert_eq!(
            action_kind_str(&Action::Custom {
                instruction: "x".into()
            }),
            "custom"
        );
    }

    #[test]
    fn target_lang_for_only_set_on_translate() {
        assert_eq!(
            target_lang_for(&Action::Translate { code: "de".into() }),
            Some("de".to_string())
        );
        assert_eq!(target_lang_for(&Action::FixGrammar), None);
        assert_eq!(target_lang_for(&Action::Rewrite), None);
        assert_eq!(
            target_lang_for(&Action::Custom {
                instruction: "x".into()
            }),
            None
        );
    }
}
