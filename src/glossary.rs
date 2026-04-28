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

/// Configured matching strategy. Spec §5.4 + §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchingStrategy {
    /// Per-translation: substring for CJK/Thai/Lao/etc., word_boundary otherwise.
    Auto,
    /// Always wrap term in `\b…\b` regex.
    WordBoundary,
    /// Plain (case-insensitive when `case_sensitive=false`) contains check.
    Substring,
}

impl MatchingStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("auto") {
            Some(Self::Auto)
        } else if s.eq_ignore_ascii_case("word_boundary") {
            Some(Self::WordBoundary)
        } else if s.eq_ignore_ascii_case("substring") {
            Some(Self::Substring)
        } else {
            None
        }
    }
}

/// Whatlang confidence threshold below which detection is treated as
/// `unknown`. Conservative middle of whatlang's typical 0..1 range.
/// Spec §13 left this open; we lock it down here.
const LANG_CONFIDENCE_THRESHOLD: f64 = 0.5;

/// Detect the source language of `text`, returning a 3-letter ISO 639-3
/// code (whatlang's `Lang::code()`). Returns `None` for empty input,
/// for input below the confidence threshold, or when whatlang declines
/// to classify (very short input, mixed scripts, etc.).
pub fn detect_source_lang(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let detector = whatlang::Detector::new();
    let info = detector.detect(trimmed)?;
    if info.confidence() < LANG_CONFIDENCE_THRESHOLD {
        return None;
    }
    Some(info.lang().code().to_string())
}

/// Map a whatlang ISO 639-3 code to ISO 639-1 (2-letter) for pair-key
/// formation. Unknown languages return `None` — callers treat that as
/// "use `*` scope only" at translation time.
pub fn iso3_to_iso2(iso3: &str) -> Option<&'static str> {
    // Covers languages whatlang detects that can plausibly appear in
    // clipboard text. Add more here on demand; an unknown code just means
    // pair-specific glossary entries don't fire (still falls back to `*`).
    match iso3 {
        "eng" => Some("en"),
        "deu" => Some("de"),
        "tur" => Some("tr"),
        "fra" => Some("fr"),
        "spa" => Some("es"),
        "ita" => Some("it"),
        "por" => Some("pt"),
        "nld" => Some("nl"),
        "rus" => Some("ru"),
        "ukr" => Some("uk"),
        "pol" => Some("pl"),
        "swe" => Some("sv"),
        "dan" => Some("da"),
        "nor" => Some("no"),
        "fin" => Some("fi"),
        "ces" => Some("cs"),
        "ell" => Some("el"),
        "heb" => Some("he"),
        "ara" => Some("ar"),
        "hin" => Some("hi"),
        "ben" => Some("bn"),
        "jpn" => Some("ja"),
        "kor" => Some("ko"),
        "zho" => Some("zh"),
        "tha" => Some("th"),
        "vie" => Some("vi"),
        "ind" => Some("id"),
        _ => None,
    }
}

/// Resolve `MatchingStrategy::Auto` against the detected source language,
/// returning a concrete `WordBoundary` or `Substring`. For non-`Auto`
/// strategies, the caller short-circuits and never consults this function.
///
/// `lang_iso3` may be the literal string `"unknown"` or any unrecognized
/// 3-letter code; both fall through to `WordBoundary` (the safer choice
/// per spec §5.4).
pub fn default_strategy(lang_iso3: &str) -> MatchingStrategy {
    match lang_iso3 {
        "zho" | "jpn" | "tha" | "lao" | "mya" | "khm" => MatchingStrategy::Substring,
        _ => MatchingStrategy::WordBoundary,
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

    #[test]
    fn detect_source_lang_recognizes_german() {
        // Long enough sample that whatlang's confidence exceeds 0.5.
        let g = "Das ist ein deutscher Satz mit ganz normalen deutschen Wörtern.";
        let detected = detect_source_lang(g);
        assert_eq!(detected.as_deref(), Some("deu"));
    }

    #[test]
    fn detect_source_lang_recognizes_english() {
        let s = "This is a regular English sentence with some normal English words.";
        let detected = detect_source_lang(s);
        assert_eq!(detected.as_deref(), Some("eng"));
    }

    #[test]
    fn detect_source_lang_returns_none_for_too_short_input() {
        // whatlang's confidence on 1-2 chars is well below threshold.
        let detected = detect_source_lang("ok");
        assert!(detected.is_none(), "got {:?}", detected);
    }

    #[test]
    fn detect_source_lang_returns_none_for_empty() {
        assert!(detect_source_lang("").is_none());
        assert!(detect_source_lang("   \n\t  ").is_none());
    }

    #[test]
    fn iso3_to_iso2_known_languages() {
        assert_eq!(iso3_to_iso2("eng"), Some("en"));
        assert_eq!(iso3_to_iso2("deu"), Some("de"));
        assert_eq!(iso3_to_iso2("tur"), Some("tr"));
        assert_eq!(iso3_to_iso2("fra"), Some("fr"));
        assert_eq!(iso3_to_iso2("spa"), Some("es"));
        assert_eq!(iso3_to_iso2("ita"), Some("it"));
        assert_eq!(iso3_to_iso2("jpn"), Some("ja"));
        assert_eq!(iso3_to_iso2("zho"), Some("zh"));
        // Unknown is unmapped — caller treats this as `unknown` for pair-key.
        assert!(iso3_to_iso2("xxx").is_none());
    }

    #[test]
    fn default_strategy_for_lang_uses_substring_for_no_whitespace_scripts() {
        // Spec §5.4: substring for zho/jpn/tha/lao/mya/khm.
        assert_eq!(default_strategy("zho"), MatchingStrategy::Substring);
        assert_eq!(default_strategy("jpn"), MatchingStrategy::Substring);
        assert_eq!(default_strategy("tha"), MatchingStrategy::Substring);
        assert_eq!(default_strategy("lao"), MatchingStrategy::Substring);
        assert_eq!(default_strategy("mya"), MatchingStrategy::Substring);
        assert_eq!(default_strategy("khm"), MatchingStrategy::Substring);
    }

    #[test]
    fn default_strategy_for_lang_uses_word_boundary_otherwise() {
        // Spec §5.4: word_boundary for whitespace-using languages.
        assert_eq!(default_strategy("eng"), MatchingStrategy::WordBoundary);
        assert_eq!(default_strategy("deu"), MatchingStrategy::WordBoundary);
        assert_eq!(default_strategy("tur"), MatchingStrategy::WordBoundary);
        assert_eq!(default_strategy("fra"), MatchingStrategy::WordBoundary);
        assert_eq!(default_strategy("spa"), MatchingStrategy::WordBoundary);
        // Unknown defaults to word_boundary (the safer choice for the
        // target-language set per spec §5.4).
        assert_eq!(default_strategy("xxx"), MatchingStrategy::WordBoundary);
    }

    #[test]
    fn matching_strategy_parse_round_trips() {
        assert_eq!(
            MatchingStrategy::parse("auto"),
            Some(MatchingStrategy::Auto)
        );
        assert_eq!(
            MatchingStrategy::parse("word_boundary"),
            Some(MatchingStrategy::WordBoundary)
        );
        assert_eq!(
            MatchingStrategy::parse("substring"),
            Some(MatchingStrategy::Substring)
        );
        assert_eq!(
            MatchingStrategy::parse("AUTO"),
            Some(MatchingStrategy::Auto)
        );
        assert!(MatchingStrategy::parse("garbage").is_none());
    }
}
