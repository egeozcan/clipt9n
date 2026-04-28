//! Glossary loading and term-matching per spec §5.4.
//!
//! The glossary is a TOML file with `[[entry]]` records, each pinning a
//! `source` term to a fixed `target` translation, optionally scoped to
//! specific language pairs (`"de->en"`) or all pairs (`"*"`).
//!
//! This module is pure — no egui, no tokio, no I/O beyond the loader.
//! The translator and the prompt window both consume `Glossary` via
//! `matching_entries(...)` (Task 4).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TranslateError;

/// A single glossary entry per spec §5.4.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct GlossaryEntry {
    /// The term to look for in source text. Matched per the configured
    /// strategy (auto / word_boundary / substring) and case-sensitivity.
    pub source: String,
    /// The mandated translation. Injected into the system prompt verbatim.
    pub target: String,
    /// Pair-key list; `"*"` matches any pair. Format: `"<src>-><tgt>"`
    /// where each side is a 2-letter ISO 639-1 code. Empty list is
    /// treated as `["*"]` per the loader normalization.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Optional free-form note shown after the target in the formatted
    /// glossary block. Spec §5.4 example: `"Always preserve as-is"`.
    #[serde(default)]
    pub note: Option<String>,
}

/// Loaded glossary.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Glossary {
    #[serde(default, rename = "entry")]
    entries: Vec<GlossaryEntry>,
}

impl Glossary {
    /// An empty glossary. Used in tests, on malformed input, and when
    /// `[glossary] enabled = false`.
    pub fn empty() -> Self {
        Self { entries: vec![] }
    }

    /// Read a glossary TOML file at `path`. Missing file → empty glossary
    /// (Ok). Malformed TOML → `Err(TranslateError::Glossary(...))`. The
    /// caller is expected to log warn and substitute `Glossary::empty()`
    /// per spec §8 ("Glossary file malformed → app continues without
    /// glossary").
    pub fn load(path: &Path) -> Result<Self, TranslateError> {
        if !path.exists() {
            return Ok(Self::empty());
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|e| TranslateError::Glossary(format!("reading {}: {e}", path.display())))?;
        let mut g: Self = toml::from_str(&contents)
            .map_err(|e| TranslateError::Glossary(format!("parsing {}: {e}", path.display())))?;
        // Normalize: empty `languages` is equivalent to `["*"]`.
        for entry in g.entries.iter_mut() {
            if entry.languages.is_empty() {
                entry.languages.push("*".into());
            }
        }
        Ok(g)
    }

    /// Direct access to the loaded entries. Used in tests; production
    /// callers go through `matching_entries` (Task 4).
    pub fn entries(&self) -> &[GlossaryEntry] {
        &self.entries
    }

    /// Total number of entries — useful for the chip-strip "+N more"
    /// counter and for tests.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries have been loaded — used by the chip-strip
    /// preview to short-circuit and by tests.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn empty_glossary_constructs_cleanly() {
        let g = Glossary::empty();
        assert!(g.is_empty());
        assert_eq!(g.len(), 0);
        assert_eq!(g.entries().len(), 0);
    }

    #[test]
    fn missing_file_returns_empty() {
        let g = Glossary::load(Path::new("/tmp/clipt9n-nonexistent-glossary-12345.toml")).unwrap();
        assert!(g.is_empty());
    }

    #[test]
    fn loads_canonical_spec_example() {
        // Verbatim from spec §5.4.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[[entry]]
source = "Smart Table"
target = "Smart Table"
languages = ["*"]

[[entry]]
source = "Vorgang"
target = "case"
languages = ["de->en"]

[[entry]]
source = "case"
target = "Vorgang"
languages = ["en->de"]

[[entry]]
source = "GIP"
target = "GIP"
languages = ["*"]
note = "Always preserve as-is"
"#
        )
        .unwrap();
        let g = Glossary::load(f.path()).unwrap();
        assert_eq!(g.len(), 4);
        let e0 = &g.entries()[0];
        assert_eq!(e0.source, "Smart Table");
        assert_eq!(e0.target, "Smart Table");
        assert_eq!(e0.languages, vec!["*"]);
        assert!(e0.note.is_none());

        let e3 = &g.entries()[3];
        assert_eq!(e3.source, "GIP");
        assert_eq!(e3.note.as_deref(), Some("Always preserve as-is"));
    }

    #[test]
    fn malformed_toml_returns_glossary_error() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "this is not [[[ valid").unwrap();
        let err = Glossary::load(f.path()).unwrap_err();
        match err {
            TranslateError::Glossary(msg) => assert!(msg.contains("parsing")),
            other => panic!("expected Glossary error, got {other:?}"),
        }
    }

    #[test]
    fn empty_languages_normalizes_to_wildcard() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[[entry]]
source = "FooBar"
target = "FooBar"
"#
        )
        .unwrap();
        let g = Glossary::load(f.path()).unwrap();
        assert_eq!(g.entries()[0].languages, vec!["*"]);
    }

    #[test]
    fn missing_required_fields_returns_error() {
        // `source` is required; `target` is required.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[[entry]]
source = "no-target-here"
"#
        )
        .unwrap();
        let err = Glossary::load(f.path()).unwrap_err();
        assert!(matches!(err, TranslateError::Glossary(_)));
    }
}
