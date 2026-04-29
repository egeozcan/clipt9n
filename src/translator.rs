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
use crate::glossary::Glossary;
use crate::llm::templates::{render, TemplateContext, TemplateKind, Templates};
use crate::llm::LlmProvider;

/// What the user wants to do with their clipboard text.
#[derive(Debug, Clone)]
pub enum Action {
    /// Translate to the language identified by ISO code (must match a slot in config).
    Translate {
        code: String,
    },
    FixGrammar,
    Rewrite,
    Custom {
        instruction: String,
    },
}

pub struct Translator<'a> {
    config: &'a Config,
    provider: &'a dyn LlmProvider,
    templates: &'a Templates,
    glossary: &'a Glossary,
}

impl<'a> Translator<'a> {
    pub fn new(
        config: &'a Config,
        provider: &'a dyn LlmProvider,
        templates: &'a Templates,
        glossary: &'a Glossary,
    ) -> Self {
        Self {
            config,
            provider,
            templates,
            glossary,
        }
    }

    /// Run the requested action against `clipboard_text` and return the
    /// post-processed result ready to write back to the clipboard.
    pub async fn execute(
        &self,
        action: &Action,
        clipboard_text: &str,
    ) -> Result<String, TranslateError> {
        let (kind, target_label, instruction, target_iso2) =
            self.resolve_template_inputs(action)?;

        // Resolve glossary block. Detection is best-effort.
        let detected_iso3 = crate::glossary::detect_source_lang(clipboard_text);
        let source_iso2 = detected_iso3
            .as_deref()
            .and_then(crate::glossary::iso3_to_iso2);
        let matched = self.glossary.matching_entries(
            clipboard_text,
            source_iso2,
            target_iso2.as_deref(),
            &self.config.glossary,
        );
        let glossary_block = crate::glossary::format_block(&matched);

        let ctx = match kind {
            TemplateKind::Translate => TemplateContext::for_translate(
                target_label.as_deref().unwrap_or(""),
                &glossary_block,
            ),
            TemplateKind::FixGrammar => TemplateContext::for_fix_grammar(&glossary_block),
            TemplateKind::Rewrite => TemplateContext::for_rewrite(&glossary_block),
            TemplateKind::Custom => {
                TemplateContext::for_custom(instruction.as_deref().unwrap_or(""), &glossary_block)
            }
        };
        let system = render(self.templates, kind, &ctx)?;
        let model_output = self.provider.complete(&system, clipboard_text).await?;
        Ok(post_process(&model_output, clipboard_text))
    }

    #[allow(clippy::type_complexity)]
    fn resolve_template_inputs(
        &self,
        action: &Action,
    ) -> Result<(TemplateKind, Option<String>, Option<String>, Option<String>), TranslateError>
    {
        Ok(match action {
            Action::Translate { code } => {
                let label = self.config.label_for_code(code)?.to_string();
                (
                    TemplateKind::Translate,
                    Some(label),
                    None,
                    Some(code.clone()),
                )
            }
            Action::FixGrammar => (TemplateKind::FixGrammar, None, None, None),
            Action::Rewrite => (TemplateKind::Rewrite, None, None, None),
            Action::Custom { instruction } => {
                if instruction.trim().is_empty() {
                    return Err(TranslateError::InvalidClipboard(
                        "custom instruction is empty".into(),
                    ));
                }
                (TemplateKind::Custom, None, Some(instruction.clone()), None)
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
    const PREAMBLES: &[&str] = &["Here is the translation:", "Translation:", "Übersetzung:"];
    for p in PREAMBLES {
        // `text.get(..n)` returns `None` if `n` is not a UTF-8 char boundary,
        // unlike `&text[..n]` which would panic. This matters for source-text
        // languages whose first char is multi-byte (e.g., Turkish `ı`, Cyrillic,
        // CJK) — without `get`, byte-indexing into the prefix length panics.
        let Some(prefix) = text.get(..p.len()) else {
            continue;
        };
        if prefix.eq_ignore_ascii_case(p) {
            // Safe: we just confirmed `p.len()` is a char boundary via `get`.
            let rest = &text[p.len()..];
            return std::borrow::Cow::Owned(rest.trim_start().to_string());
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
        assert_eq!(
            post_process("Here is the translation: Hallo", "Hello"),
            "Hallo"
        );
    }

    #[test]
    fn preamble_is_case_insensitive() {
        assert_eq!(post_process("translation: Hallo", "Hello"), "Hallo");
    }

    #[test]
    fn does_not_panic_on_utf8_first_char_shorter_than_preamble() {
        // Regression: the original byte-index slice panicked on text whose
        // first multi-byte char straddled the preamble's byte length.
        // Turkish "ı" is 2 bytes; "Bir aracıyı" has bytes that are not on
        // char boundaries at offsets 12 / 13 — exactly within the
        // "Translation:" (12) and "Übersetzung:" (13) prefix probes.
        let turkish = "Bir aracıyı M1→M2 ve M2→M3 şablonlarını takip ederek...";
        // Should not panic; preamble doesn't match, so output equals input
        // (modulo trim).
        assert_eq!(post_process(turkish, "irrelevant"), turkish);
    }

    #[test]
    fn handles_multibyte_text_with_matching_preamble() {
        // The preamble itself is ASCII; the rest can be anything.
        let model = "Translation: Hallöchen, schöne Welt 🌍";
        assert_eq!(post_process(model, "Hello"), "Hallöchen, schöne Welt 🌍");
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
        assert_eq!(
            post_process("Hallo, Welt.", "Hello, world."),
            "Hallo, Welt."
        );
    }

    // ----------- Translator tests (M4 signature) -----------

    use crate::glossary::{Glossary, GlossaryEntry};
    use crate::llm::templates::Templates;

    fn templates() -> Templates {
        Templates::built_in()
    }

    fn empty_glossary() -> Glossary {
        Glossary::empty()
    }

    /// Mock provider that captures the system prompt and returns a fixed reply.
    struct CapturingProvider {
        captured: Mutex<Option<(String, String)>>,
        reply: String,
    }

    impl CapturingProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                captured: Mutex::new(None),
                reply: reply.into(),
            }
        }
        fn captured(&self) -> (String, String) {
            self.captured
                .lock()
                .unwrap()
                .clone()
                .expect("provider was never called")
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
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
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
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        let result = t
            .execute(&Action::FixGrammar, "He dont know.")
            .await
            .unwrap();
        assert_eq!(result, "He doesn't know.");
        let (system, _) = provider.captured();
        assert!(system.contains("IN THE SAME LANGUAGE"));
        assert!(system.contains("MINIMUM changes"));
    }

    #[tokio::test]
    async fn rewrite_action_uses_rewrite_template() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("Concise version.");
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        let _ = t
            .execute(&Action::Rewrite, "verbose original")
            .await
            .unwrap();
        let (system, _) = provider.captured();
        assert!(system.contains("MAY restructure sentences"));
    }

    #[tokio::test]
    async fn custom_action_includes_user_instruction() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("formal output");
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        let _ = t
            .execute(
                &Action::Custom {
                    instruction: "make this sound diplomatic".into(),
                },
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
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
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
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        let err = t
            .execute(
                &Action::Custom {
                    instruction: "   ".into(),
                },
                "Hello",
            )
            .await
            .unwrap_err();
        assert!(matches!(err, TranslateError::InvalidClipboard(_)));
    }

    #[tokio::test]
    async fn provider_output_is_post_processed_before_returning() {
        let cfg = Config::default();
        let provider = CapturingProvider::new("\"Hallo, Welt.\"");
        let templates = templates();
        let glossary = empty_glossary();
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        let result = t
            .execute(&Action::Translate { code: "de".into() }, "Hello, world.")
            .await
            .unwrap();
        assert_eq!(result, "Hallo, Welt."); // wrapping quotes stripped
    }

    #[tokio::test]
    async fn glossary_block_is_injected_when_entries_match() {
        // Spec §5.4: matching entries inject into `{{ glossary_block }}`.
        let cfg = Config::default();
        let provider = CapturingProvider::new("Hallo, Welt.");
        let templates = templates();
        let mut glossary = Glossary::empty();
        glossary.entries_mut().push(GlossaryEntry {
            source: "Smart Table".into(),
            target: "Smart Table".into(),
            languages: vec!["*".into()],
            note: None,
        });
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        // Source contains the term.
        let _ = t
            .execute(
                &Action::Translate { code: "de".into() },
                "We sell a Smart Table for the kitchen.",
            )
            .await
            .unwrap();
        let (system, _) = provider.captured();
        assert!(
            system.contains("GLOSSARY"),
            "system prompt should contain glossary block; got: {system}"
        );
        assert!(system.contains("Smart Table"));
    }

    #[tokio::test]
    async fn glossary_block_omitted_when_no_entries_match() {
        // No matching entries → block is empty → no `GLOSSARY` header.
        let cfg = Config::default();
        let provider = CapturingProvider::new("Hallo, Welt.");
        let templates = templates();
        let mut glossary = Glossary::empty();
        glossary.entries_mut().push(GlossaryEntry {
            source: "Vorgang".into(),
            target: "case".into(),
            languages: vec!["de->en".into()],
            note: None,
        });
        let t = Translator::new(&cfg, &provider, &templates, &glossary);
        // Source has no glossary terms; pair is en->de which doesn't
        // match Vorgang's de->en scope anyway.
        let _ = t
            .execute(
                &Action::Translate { code: "de".into() },
                "There is nothing relevant in this text.",
            )
            .await
            .unwrap();
        let (system, _) = provider.captured();
        assert!(
            !system.contains("GLOSSARY"),
            "expected no glossary block; got: {system}"
        );
    }
}
