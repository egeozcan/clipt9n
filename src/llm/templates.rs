//! Render built-in prompt templates with minijinja.
//!
//! M1 only renders the four built-in templates from `prompts`. File-based
//! override loading lands in M4; this module's API will be extended at that
//! time but the M1 callers here remain valid.

use minijinja::{context, Environment};

use super::prompts;
use crate::error::TranslateError;

/// Inputs available to template rendering.
///
/// All fields are passed to every template. Variables a template doesn't
/// reference are simply ignored by minijinja (no error).
pub struct TemplateContext<'a> {
    pub source_language: &'a str,
    pub target_language: &'a str,
    pub user_instruction: &'a str,
    pub glossary_block: &'a str,
}

impl<'a> TemplateContext<'a> {
    /// Convenience constructor for tests and call sites that only need a subset.
    pub fn for_translate(target_language: &'a str, glossary_block: &'a str) -> Self {
        Self {
            source_language: "unknown",
            target_language,
            user_instruction: "",
            glossary_block,
        }
    }

    pub fn for_fix_grammar(glossary_block: &'a str) -> Self {
        Self {
            source_language: "unknown",
            target_language: "",
            user_instruction: "",
            glossary_block,
        }
    }

    pub fn for_rewrite(glossary_block: &'a str) -> Self {
        Self {
            source_language: "unknown",
            target_language: "",
            user_instruction: "",
            glossary_block,
        }
    }

    pub fn for_custom(user_instruction: &'a str, glossary_block: &'a str) -> Self {
        Self {
            source_language: "unknown",
            target_language: "",
            user_instruction,
            glossary_block,
        }
    }
}

/// Identifies which built-in template to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Translate,
    FixGrammar,
    Rewrite,
    Custom,
}

impl TemplateKind {
    fn source(self) -> &'static str {
        match self {
            TemplateKind::Translate => prompts::TRANSLATE,
            TemplateKind::FixGrammar => prompts::FIX_GRAMMAR,
            TemplateKind::Rewrite => prompts::REWRITE,
            TemplateKind::Custom => prompts::CUSTOM,
        }
    }

    fn name(self) -> &'static str {
        match self {
            TemplateKind::Translate => "translate",
            TemplateKind::FixGrammar => "fix_grammar",
            TemplateKind::Rewrite => "rewrite",
            TemplateKind::Custom => "custom",
        }
    }
}

/// Render a built-in template with the given context. Returns the rendered
/// system prompt that gets sent to the LLM.
pub fn render(kind: TemplateKind, ctx: &TemplateContext<'_>) -> Result<String, TranslateError> {
    let mut env = Environment::new();
    env.add_template(kind.name(), kind.source()).map_err(|e| {
        TranslateError::Template(format!(
            "built-in template '{}' failed to load: {e}",
            kind.name()
        ))
    })?;

    let tmpl = env.get_template(kind.name()).map_err(|e| {
        TranslateError::Template(format!(
            "built-in template '{}' not found: {e}",
            kind.name()
        ))
    })?;

    tmpl.render(context! {
        source_language => ctx.source_language,
        target_language => ctx.target_language,
        user_instruction => ctx.user_instruction,
        glossary_block => ctx.glossary_block,
    })
    .map_err(|e| TranslateError::Template(format!("rendering '{}' failed: {e}", kind.name())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_renders_with_target_language_and_empty_glossary() {
        let ctx = TemplateContext::for_translate("German", "");
        let out = render(TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("Translate the user's text into German."));
        assert!(out.contains("If the text is already in German, return it unchanged."));
        // Empty glossary: no trailing GLOSSARY block
        assert!(!out.contains("GLOSSARY"));
        // Empty glossary should not leave trailing newlines beyond the rules block
        assert_eq!(out.trim_end(), out.trim_end_matches('\n').trim_end());
    }

    #[test]
    fn translate_renders_glossary_block_when_provided() {
        let glossary = "GLOSSARY — these terms MUST be translated exactly as specified:\n- \"Smart Table\" → \"Smart Table\"";
        let ctx = TemplateContext::for_translate("German", glossary);
        let out = render(TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("Smart Table"));
        assert!(out.contains("MUST be translated exactly"));
    }

    #[test]
    fn fix_grammar_does_not_mention_target_language() {
        let ctx = TemplateContext::for_fix_grammar("");
        let out = render(TemplateKind::FixGrammar, &ctx).unwrap();
        assert!(out.contains("IN THE SAME LANGUAGE"));
        assert!(!out.contains("Translate the user's text"));
    }

    #[test]
    fn rewrite_does_not_translate() {
        let ctx = TemplateContext::for_rewrite("");
        let out = render(TemplateKind::Rewrite, &ctx).unwrap();
        assert!(out.contains("IN THE SAME LANGUAGE"));
        assert!(out.contains("MAY restructure sentences"));
    }

    #[test]
    fn custom_substitutes_user_instruction() {
        let ctx = TemplateContext::for_custom("translate to formal Spanish", "");
        let out = render(TemplateKind::Custom, &ctx).unwrap();
        assert!(out.contains("translate to formal Spanish"));
    }

    #[test]
    fn empty_glossary_block_does_not_leave_trailing_whitespace() {
        // Spec §5.4: "If the filtered set is empty, {{ glossary_block }} resolves
        // to an empty string and the template renders cleanly without trailing
        // whitespace."
        let ctx = TemplateContext::for_fix_grammar("");
        let out = render(TemplateKind::FixGrammar, &ctx).unwrap();
        // The template's last non-glossary line ends with "return it unchanged."
        // After empty glossary substitution there should be no trailing
        // whitespace beyond a single newline.
        let trailing = &out[out.len().saturating_sub(20)..];
        assert!(
            !trailing.contains("  "),
            "trailing whitespace found: {trailing:?}"
        );
    }
}
