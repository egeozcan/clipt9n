//! Translator orchestration.
//!
//! Selects template → renders with context → calls provider → post-processes.
//!
//! M1 callers wire one of:
//!   - `Action::Translate { code: "de" }`
//!   - `Action::FixGrammar`
//!   - `Action::Rewrite`
//!   - `Action::Custom { instruction: "..." }`
//!
//! ...with a `LlmProvider` impl and a `Config`. The translator does no I/O of
//! its own — clipboard read/write is the caller's concern.

use crate::config::Config;
use crate::error::TranslateError;
use crate::llm::templates::{render, TemplateContext, TemplateKind};
use crate::llm::LlmProvider;

/// What the user wants to do with their clipboard text.
#[derive(Debug, Clone)]
pub enum Action {
    /// Translate to the language identified by ISO code (must match a slot in config).
    Translate { code: String },
    FixGrammar,
    Rewrite,
    Custom { instruction: String },
}

pub struct Translator<'a> {
    config: &'a Config,
    provider: &'a dyn LlmProvider,
}

impl<'a> Translator<'a> {
    pub fn new(config: &'a Config, provider: &'a dyn LlmProvider) -> Self {
        Self { config, provider }
    }

    /// Run the requested action against `clipboard_text` and return the
    /// post-processed result ready to write back to the clipboard.
    pub async fn execute(&self, action: &Action, clipboard_text: &str) -> Result<String, TranslateError> {
        let (kind, target_label, instruction) = self.resolve_template_inputs(action)?;
        // Glossary is M4. M1 always passes empty.
        let ctx = match kind {
            TemplateKind::Translate => TemplateContext::for_translate(target_label.as_deref().unwrap_or(""), ""),
            TemplateKind::FixGrammar => TemplateContext::for_fix_grammar(""),
            TemplateKind::Rewrite => TemplateContext::for_rewrite(""),
            TemplateKind::Custom => TemplateContext::for_custom(instruction.as_deref().unwrap_or(""), ""),
        };
        let system = render(kind, &ctx)?;
        let model_output = self.provider.complete(&system, clipboard_text).await?;
        Ok(post_process(&model_output, clipboard_text))
    }

    fn resolve_template_inputs(
        &self,
        action: &Action,
    ) -> Result<(TemplateKind, Option<String>, Option<String>), TranslateError> {
        Ok(match action {
            Action::Translate { code } => {
                let label = self.config.label_for_code(code)?.to_string();
                (TemplateKind::Translate, Some(label), None)
            }
            Action::FixGrammar => (TemplateKind::FixGrammar, None, None),
            Action::Rewrite => (TemplateKind::Rewrite, None, None),
            Action::Custom { instruction } => {
                if instruction.trim().is_empty() {
                    return Err(TranslateError::InvalidClipboard("custom instruction is empty".into()));
                }
                (TemplateKind::Custom, None, Some(instruction.clone()))
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Post-processing (spec §5.6)
// ---------------------------------------------------------------------------

pub fn post_process(model_output: &str, source_text: &str) -> String {
    let trimmed = model_output.trim();
    let dequoted = strip_wrapping_quotes_if_safe(trimmed, source_text);
    strip_preamble(&dequoted).into_owned()
}

const QUOTE_PAIRS: &[(char, char)] = &[
    ('"', '"'),
    ('\u{201C}', '\u{201D}'),
    ('\u{00AB}', '\u{00BB}'),
    ('\u{201E}', '\u{201C}'),
];

fn strip_wrapping_quotes_if_safe(text: &str, source: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return text.to_string();
    }
    let first = chars[0];
    let last = chars[chars.len() - 1];

    if !QUOTE_PAIRS.iter().any(|(o, c)| first == *o && last == *c) {
        return text.to_string();
    }

    if let Some(src_first) = source.trim().chars().next() {
        if src_first == first {
            return text.to_string();
        }
    }

    chars[1..chars.len() - 1].iter().collect()
}

fn strip_preamble(text: &str) -> std::borrow::Cow<'_, str> {
    const PREAMBLES: &[&str] = &[
        "Here is the translation:",
        "Translation:",
        "Übersetzung:",
    ];
    for p in PREAMBLES {
        if text.len() >= p.len() {
            let prefix = &text[..p.len()];
            if prefix.eq_ignore_ascii_case(p) {
                let rest = &text[p.len()..];
                return std::borrow::Cow::Owned(rest.trim_start().to_string());
            }
        }
    }
    std::borrow::Cow::Borrowed(text)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;

    // -------- post_process tests (preserved from Task 5) ---------------------

    #[test]
    fn trims_whitespace() {
        assert_eq!(post_process("  hello  \n", "input"), "hello");
    }

    #[test]
    fn strips_wrapping_ascii_quotes_when_source_unquoted() {
        assert_eq!(post_process("\"hello\"", "input"), "hello");
    }

    #[test]
    fn preserves_quotes_when_source_was_also_quoted() {
        assert_eq!(post_process("\"hello\"", "\"input\""), "\"hello\"");
    }

    #[test]
    fn strips_curly_quotes() {
        assert_eq!(post_process("\u{201C}hello\u{201D}", "input"), "hello");
    }

    #[test]
    fn strips_german_low_9_quotes() {
        assert_eq!(post_process("\u{201E}hallo\u{201C}", "input"), "hallo");
    }

    #[test]
    fn strips_french_guillemets() {
        assert_eq!(post_process("\u{00AB}bonjour\u{00BB}", "input"), "bonjour");
    }

    #[test]
    fn strips_translation_preamble() {
        assert_eq!(post_process("Translation: Hallo", "Hello"), "Hallo");
    }

    #[test]
    fn strips_german_preamble() {
        assert_eq!(post_process("Übersetzung: Hallo", "Hello"), "Hallo");
    }

    #[test]
    fn strips_full_translation_preamble() {
        assert_eq!(post_process("Here is the translation: Hallo", "Hello"), "Hallo");
    }

    #[test]
    fn preamble_is_case_insensitive() {
        assert_eq!(post_process("translation: Hallo", "Hello"), "Hallo");
    }

    #[test]
    fn preserves_internal_formatting() {
        let input = "Line 1\nLine 2\n\n- bullet";
        let model = "Line 1\nLine 2\n\n- bullet";
        assert_eq!(post_process(model, input), input);
    }

    #[test]
    fn preserves_internal_quotes() {
        let model = "She said \"hello\" politely";
        assert_eq!(post_process(model, "input"), "She said \"hello\" politely");
    }

    #[test]
    fn no_op_on_clean_text() {
        assert_eq!(post_process("Hallo, Welt.", "Hello, world."), "Hallo, Welt.");
    }

    // -------- Translator tests --------------------------------------------

    /// Mock provider that captures the system prompt and returns a fixed reply.
    struct CapturingProvider {
        captured: Mutex<Option<(String, String)>>,
        reply: String,
    }

    impl CapturingProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self { captured: Mutex::new(None), reply: reply.into() }
        }
        fn captured(&self) -> (String, String) {
            self.captured.lock().unwrap().clone().expect("provider was never called")
        }
    }

    #[async_trait]
    impl LlmProvider for CapturingProvider {
        async fn complete(&self, system: &str, user: &str) -> Result<String, TranslateError> {
            *self.captured.lock().unwrap() = Some((system.to_string(), user.to_string()));
            Ok(self.reply.clone())
        }
    }

    #[tokio::test]
    async fn translate_action_passes_target_label_to_template() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("Hallo, Welt.");
        let t = Translator::new(&cfg, &provider);
        let result = t
            .execute(&Action::Translate { code: "de".into() }, "Hello, world.")
            .await
            .unwrap();
        assert_eq!(result, "Hallo, Welt.");
        let (system, user) = provider.captured();
        assert!(system.contains("Translate the user's text into Deutsch."));
        assert_eq!(user, "Hello, world.");
    }

    #[tokio::test]
    async fn fix_grammar_action_uses_fix_grammar_template() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("He doesn't know.");
        let t = Translator::new(&cfg, &provider);
        let result = t.execute(&Action::FixGrammar, "He dont know.").await.unwrap();
        assert_eq!(result, "He doesn't know.");
        let (system, _) = provider.captured();
        assert!(system.contains("IN THE SAME LANGUAGE"));
        assert!(system.contains("MINIMUM changes"));
    }

    #[tokio::test]
    async fn rewrite_action_uses_rewrite_template() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("Concise version.");
        let t = Translator::new(&cfg, &provider);
        let _ = t.execute(&Action::Rewrite, "verbose original").await.unwrap();
        let (system, _) = provider.captured();
        assert!(system.contains("MAY restructure sentences"));
    }

    #[tokio::test]
    async fn custom_action_includes_user_instruction() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("formal output");
        let t = Translator::new(&cfg, &provider);
        let _ = t
            .execute(
                &Action::Custom { instruction: "make this sound diplomatic".into() },
                "raw text",
            )
            .await
            .unwrap();
        let (system, _) = provider.captured();
        assert!(system.contains("make this sound diplomatic"));
    }

    #[tokio::test]
    async fn translate_action_with_unknown_code_returns_unsupported_language() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("");
        let t = Translator::new(&cfg, &provider);
        let err = t
            .execute(&Action::Translate { code: "fr".into() }, "Hello")
            .await
            .unwrap_err();
        assert!(matches!(err, TranslateError::UnsupportedLanguage(_)));
    }

    #[tokio::test]
    async fn custom_action_with_empty_instruction_errors() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("");
        let t = Translator::new(&cfg, &provider);
        let err = t
            .execute(&Action::Custom { instruction: "   ".into() }, "Hello")
            .await
            .unwrap_err();
        assert!(matches!(err, TranslateError::InvalidClipboard(_)));
    }

    #[tokio::test]
    async fn provider_output_is_post_processed_before_returning() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("\"Hallo, Welt.\"");
        let t = Translator::new(&cfg, &provider);
        let result = t
            .execute(&Action::Translate { code: "de".into() }, "Hello, world.")
            .await
            .unwrap();
        assert_eq!(result, "Hallo, Welt."); // wrapping quotes stripped
    }
}
