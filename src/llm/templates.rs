//! Render prompt templates. Built-in defaults from `prompts.rs`; user
//! overrides loaded from `<config_dir>/templates/<action>.j2` per spec §5.3.
//!
//! `Templates::load(...)` runs at startup and validates each override:
//!   - Parse error → `TranslateError::Template("<file> line <N>: parse error: <detail>")`
//!   - Renders with all known variables stubbed to verify no undeclared
//!     references → unknown var → `TranslateError::Template("<file> line <N>: undefined variable or render error: <detail>")`
//!
//! Validation runs once; the resulting `Templates` is immutable for the
//! lifetime of the app (templates are NOT reloaded on SIGHUP — only the
//! glossary is, per spec §5.4).

use std::path::Path;

use minijinja::{context, Environment, UndefinedBehavior};

use super::prompts;
use crate::config::TemplatesConfig;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Translate,
    FixGrammar,
    Rewrite,
    Custom,
}

impl TemplateKind {
    fn name(self) -> &'static str {
        match self {
            TemplateKind::Translate => "translate",
            TemplateKind::FixGrammar => "fix_grammar",
            TemplateKind::Rewrite => "rewrite",
            TemplateKind::Custom => "custom",
        }
    }

    fn built_in_source(self) -> &'static str {
        match self {
            TemplateKind::Translate => prompts::TRANSLATE,
            TemplateKind::FixGrammar => prompts::FIX_GRAMMAR,
            TemplateKind::Rewrite => prompts::REWRITE,
            TemplateKind::Custom => prompts::CUSTOM,
        }
    }
}

/// Loaded template set. Holds the four template strings (built-in or
/// user-override) so `render` is a pure substitution.
#[derive(Debug, Clone)]
pub struct Templates {
    translate: String,
    fix_grammar: String,
    rewrite: String,
    custom: String,
}

impl Templates {
    /// Construct a `Templates` with the four built-in defaults from
    /// `prompts.rs`. Useful in tests and as a fallback when override
    /// loading is bypassed.
    pub fn built_in() -> Self {
        Self {
            translate: prompts::TRANSLATE.to_string(),
            fix_grammar: prompts::FIX_GRAMMAR.to_string(),
            rewrite: prompts::REWRITE.to_string(),
            custom: prompts::CUSTOM.to_string(),
        }
    }

    /// Load templates with overrides from disk. For each of the four
    /// kinds: if the configured path resolves to an existing file, parse
    /// and validate it and use it as the source; otherwise use the built-in.
    /// Empty / `None` paths in `cfg` mean "use built-in" for that kind.
    ///
    /// Validation errors abort startup (`Err`); missing files do not
    /// (the path is just treated as "no override configured").
    pub fn load(config_dir: &Path, cfg: &TemplatesConfig) -> Result<Self, TranslateError> {
        let translate = load_one(
            config_dir,
            cfg.translate.as_deref(),
            TemplateKind::Translate,
        )?;
        let fix_grammar = load_one(
            config_dir,
            cfg.fix_grammar.as_deref(),
            TemplateKind::FixGrammar,
        )?;
        let rewrite = load_one(config_dir, cfg.rewrite.as_deref(), TemplateKind::Rewrite)?;
        let custom = load_one(config_dir, cfg.custom.as_deref(), TemplateKind::Custom)?;
        Ok(Self {
            translate,
            fix_grammar,
            rewrite,
            custom,
        })
    }

    fn source(&self, kind: TemplateKind) -> &str {
        match kind {
            TemplateKind::Translate => &self.translate,
            TemplateKind::FixGrammar => &self.fix_grammar,
            TemplateKind::Rewrite => &self.rewrite,
            TemplateKind::Custom => &self.custom,
        }
    }
}

fn load_one(
    config_dir: &Path,
    rel_path: Option<&str>,
    kind: TemplateKind,
) -> Result<String, TranslateError> {
    let Some(rel) = rel_path.filter(|s| !s.is_empty()) else {
        return Ok(kind.built_in_source().to_string());
    };
    let abs = config_dir.join(rel);
    if !abs.exists() {
        return Ok(kind.built_in_source().to_string());
    }
    let source = std::fs::read_to_string(&abs)
        .map_err(|e| TranslateError::Template(format!("reading {}: {e}", abs.display())))?;
    validate_template_source(&source, &abs, kind)?;
    Ok(source)
}

/// Validate a template by parsing it (catches syntax errors) and
/// rendering it with every known variable stubbed (catches references to
/// undeclared variables). Errors include `<file> line <N>` context.
fn validate_template_source(
    source: &str,
    path: &Path,
    kind: TemplateKind,
) -> Result<(), TranslateError> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_template(kind.name(), source).map_err(|e| {
        TranslateError::Template(format!(
            "{} line {}: parse error: {e}",
            path.display(),
            err_line(&e),
        ))
    })?;
    let tmpl = env
        .get_template(kind.name())
        .map_err(|e| TranslateError::Template(format!("{}: load error: {e}", path.display())))?;
    let mut undeclared: Vec<_> = tmpl
        .undeclared_variables(false)
        .into_iter()
        .filter(|name| {
            !matches!(
                name.as_str(),
                "source_language" | "target_language" | "user_instruction" | "glossary_block"
            )
        })
        .collect();
    undeclared.sort();
    let contexts = [
        context! {
            source_language => "stub",
            target_language => "Stub",
            user_instruction => "stub",
            glossary_block => "",
        },
        context! {
            source_language => "stub",
            target_language => "Stub",
            user_instruction => "stub",
            glossary_block => "GLOSSARY",
        },
    ];
    for ctx in contexts {
        if let Err(e) = tmpl.render(ctx) {
            let undeclared_note = if undeclared.is_empty() {
                String::new()
            } else {
                format!("; undeclared variables: {}", undeclared.join(", "))
            };
            return Err(TranslateError::Template(format!(
                "{} line {}: undefined variable or render error: {e}{undeclared_note}",
                path.display(),
                err_line(&e),
            )));
        }
    }
    Ok(())
}

fn err_line(e: &minijinja::Error) -> String {
    e.line()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Render a template (built-in or override) with the given context.
/// Returns the rendered system prompt that gets sent to the LLM.
pub fn render(
    templates: &Templates,
    kind: TemplateKind,
    ctx: &TemplateContext<'_>,
) -> Result<String, TranslateError> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_template(kind.name(), templates.source(kind))
        .map_err(|e| {
            TranslateError::Template(format!("rendering '{}' failed at parse: {e}", kind.name()))
        })?;
    let tmpl = env
        .get_template(kind.name())
        .map_err(|e| TranslateError::Template(format!("'{}' not found: {e}", kind.name())))?;
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
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;
    use crate::config::TemplatesConfig;

    fn templates() -> Templates {
        Templates::built_in()
    }

    // ---- existing built-in coverage (preserved from M1) ----

    #[test]
    fn translate_renders_with_target_language_and_empty_glossary() {
        let t = templates();
        let ctx = TemplateContext::for_translate("German", "");
        let out = render(&t, TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("Translate the user's text into German."));
        assert!(out.contains("If the text is already in German, return it unchanged."));
        assert!(!out.contains("GLOSSARY"));
    }

    #[test]
    fn translate_renders_glossary_block_when_provided() {
        let t = templates();
        let glossary = "GLOSSARY — these terms MUST be translated exactly as specified:\n- \"Smart Table\" → \"Smart Table\"";
        let ctx = TemplateContext::for_translate("German", glossary);
        let out = render(&t, TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("Smart Table"));
    }

    #[test]
    fn fix_grammar_does_not_mention_target_language() {
        let t = templates();
        let ctx = TemplateContext::for_fix_grammar("");
        let out = render(&t, TemplateKind::FixGrammar, &ctx).unwrap();
        assert!(out.contains("IN THE SAME LANGUAGE"));
        assert!(!out.contains("Translate the user's text"));
    }

    #[test]
    fn rewrite_does_not_translate() {
        let t = templates();
        let ctx = TemplateContext::for_rewrite("");
        let out = render(&t, TemplateKind::Rewrite, &ctx).unwrap();
        assert!(out.contains("IN THE SAME LANGUAGE"));
        assert!(out.contains("MAY restructure sentences"));
    }

    #[test]
    fn custom_substitutes_user_instruction() {
        let t = templates();
        let ctx = TemplateContext::for_custom("translate to formal Spanish", "");
        let out = render(&t, TemplateKind::Custom, &ctx).unwrap();
        assert!(out.contains("translate to formal Spanish"));
    }

    #[test]
    fn empty_glossary_block_does_not_leave_trailing_whitespace() {
        let t = templates();
        let ctx = TemplateContext::for_fix_grammar("");
        let out = render(&t, TemplateKind::FixGrammar, &ctx).unwrap();
        let trailing = &out[out.len().saturating_sub(20)..];
        assert!(
            !trailing.contains("  "),
            "trailing whitespace found: {trailing:?}"
        );
    }

    // ---- override loader ----

    #[test]
    fn missing_template_files_fall_back_to_built_ins() {
        let dir = tempdir().unwrap();
        // No files in dir; default config points at templates/<name>.j2.
        let cfg = TemplatesConfig::default();
        let t = Templates::load(dir.path(), &cfg).unwrap();
        // Should behave identically to built-in.
        let ctx = TemplateContext::for_translate("German", "");
        let out = render(&t, TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("Translate the user's text into German."));
    }

    #[test]
    fn override_replaces_builtin_for_specified_action() {
        let dir = tempdir().unwrap();
        let templates_dir = dir.path().join("templates");
        std::fs::create_dir(&templates_dir).unwrap();
        let mut f = std::fs::File::create(templates_dir.join("translate.j2")).unwrap();
        // Custom template with a unique marker string.
        writeln!(
            f,
            "CUSTOM-OVERRIDE: translate to {{{{ target_language }}}}.\n{{{{ glossary_block }}}}"
        )
        .unwrap();
        let cfg = TemplatesConfig::default();
        let t = Templates::load(dir.path(), &cfg).unwrap();
        let ctx = TemplateContext::for_translate("German", "");
        let out = render(&t, TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("CUSTOM-OVERRIDE"));
        assert!(out.contains("translate to German."));
        // Other kinds remain built-ins.
        let ctx2 = TemplateContext::for_fix_grammar("");
        let out2 = render(&t, TemplateKind::FixGrammar, &ctx2).unwrap();
        assert!(out2.contains("IN THE SAME LANGUAGE"));
    }

    #[test]
    fn override_validation_checks_truthy_and_falsey_conditional_branches() {
        let dir = tempdir().unwrap();
        let templates_dir = dir.path().join("templates");
        std::fs::create_dir(&templates_dir).unwrap();
        std::fs::write(
            templates_dir.join("translate.j2"),
            "{% if glossary_block %}{{ glossary_block }} {{ missing_in_truthy }}{% else %}Translate to {{ target_language }}{% endif %}",
        )
        .unwrap();

        let err = Templates::load(dir.path(), &TemplatesConfig::default()).unwrap_err();
        match err {
            TranslateError::Template(msg) => {
                assert!(msg.contains("missing_in_truthy"), "msg: {msg}")
            }
            other => panic!("expected Template error, got {other:?}"),
        }
    }

    #[test]
    fn override_conditional_template_renders_both_paths_when_valid() {
        let dir = tempdir().unwrap();
        let templates_dir = dir.path().join("templates");
        std::fs::create_dir(&templates_dir).unwrap();
        std::fs::write(
            templates_dir.join("translate.j2"),
            "{% if glossary_block %}WITH {{ glossary_block }}{% else %}WITHOUT {{ target_language }}{% endif %}",
        )
        .unwrap();

        let t = Templates::load(dir.path(), &TemplatesConfig::default()).unwrap();
        let without = render(
            &t,
            TemplateKind::Translate,
            &TemplateContext::for_translate("German", ""),
        )
        .unwrap();
        let with = render(
            &t,
            TemplateKind::Translate,
            &TemplateContext::for_translate("German", "GLOSSARY"),
        )
        .unwrap();
        assert_eq!(without, "WITHOUT German");
        assert_eq!(with, "WITH GLOSSARY");
    }

    #[test]
    fn empty_path_string_means_use_builtin() {
        let dir = tempdir().unwrap();
        let cfg = TemplatesConfig {
            translate: Some(String::new()),
            ..TemplatesConfig::default()
        };
        let t = Templates::load(dir.path(), &cfg).unwrap();
        let ctx = TemplateContext::for_translate("German", "");
        let out = render(&t, TemplateKind::Translate, &ctx).unwrap();
        assert!(out.contains("Translate the user's text into German."));
    }

    #[test]
    fn malformed_template_returns_error_with_file_and_line() {
        let dir = tempdir().unwrap();
        let templates_dir = dir.path().join("templates");
        std::fs::create_dir(&templates_dir).unwrap();
        let mut f = std::fs::File::create(templates_dir.join("translate.j2")).unwrap();
        // Unclosed `{%` — minijinja reports a parse error with line info.
        writeln!(f, "Hello {{% if foo and").unwrap();
        let cfg = TemplatesConfig::default();
        let err = Templates::load(dir.path(), &cfg).unwrap_err();
        match err {
            TranslateError::Template(msg) => {
                assert!(msg.contains("translate.j2"), "msg: {msg}");
                assert!(msg.contains("line"), "msg: {msg}");
            }
            other => panic!("expected Template error, got {other:?}"),
        }
    }

    #[test]
    fn template_referencing_unknown_variable_returns_error() {
        let dir = tempdir().unwrap();
        let templates_dir = dir.path().join("templates");
        std::fs::create_dir(&templates_dir).unwrap();
        let mut f = std::fs::File::create(templates_dir.join("translate.j2")).unwrap();
        // `nonsense_var` is not in TemplateContext.
        writeln!(
            f,
            "Translate to {{{{ target_language }}}}. Use {{{{ nonsense_var }}}} setting."
        )
        .unwrap();
        let cfg = TemplatesConfig::default();
        let err = Templates::load(dir.path(), &cfg).unwrap_err();
        match err {
            TranslateError::Template(msg) => {
                assert!(msg.contains("translate.j2"), "msg: {msg}");
                assert!(
                    msg.contains("nonsense_var") || msg.contains("undefined"),
                    "msg: {msg}"
                );
            }
            other => panic!("expected Template error, got {other:?}"),
        }
    }
}
