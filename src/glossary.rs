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

/// Test whether a single glossary term matches `source_text` under the
/// given strategy and case sensitivity. Word-boundary checks that the
/// surrounding characters aren't ASCII alphanumerics or `_`; this is
/// Unicode-naive but matches the spec's whitespace-language assumption.
/// `Substring` deliberately ignores boundaries, even on whitespace
/// languages — opt-in via `[glossary] matching = "substring"`.
pub fn term_matches(
    source_text: &str,
    term: &str,
    case_sensitive: bool,
    strategy: MatchingStrategy,
) -> bool {
    if term.is_empty() || source_text.is_empty() {
        return false;
    }
    let resolved = match strategy {
        MatchingStrategy::Auto => unreachable!(
            "term_matches is called only with a resolved strategy; \
             callers must convert Auto via default_strategy first"
        ),
        s => s,
    };
    let (haystack, needle) = if case_sensitive {
        (source_text.to_string(), term.to_string())
    } else {
        (source_text.to_lowercase(), term.to_lowercase())
    };
    match resolved {
        MatchingStrategy::WordBoundary => {
            // Plain `contains` for the substring half of the check.
            // Then verify the surrounding characters aren't ASCII letters
            // / digits / underscores. Since whatlang's word-boundary
            // languages are whitespace-separated, this naive boundary test
            // is sufficient and avoids the `regex` crate entirely.
            let mut start = 0;
            while let Some(pos) = haystack[start..].find(&needle) {
                let abs = start + pos;
                let end = abs + needle.len();
                let pre = haystack[..abs].chars().last();
                let post = haystack[end..].chars().next();
                let pre_ok = pre.is_none_or(|c| !is_word_char(c));
                let post_ok = post.is_none_or(|c| !is_word_char(c));
                if pre_ok && post_ok {
                    return true;
                }
                start = abs + needle.chars().next().map_or(1, |c| c.len_utf8());
            }
            false
        }
        MatchingStrategy::Substring => haystack.contains(&needle),
        MatchingStrategy::Auto => unreachable!(),
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Test whether an entry's `languages` list matches the current pair key
/// (e.g. `"de->en"`). `*` in the entry list matches any pair.
pub fn pair_matches(entry_pairs: &[String], current_pair: &str) -> bool {
    entry_pairs.iter().any(|p| p == "*" || p == current_pair)
}

/// Render the spec §5.4 glossary block from matched entries. Empty input
/// → empty string (no header, no trailing whitespace; matches the
/// "renders cleanly" invariant in the spec).
pub fn format_block(entries: &[&GlossaryEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(entries.len() * 64);
    out.push_str("GLOSSARY — these terms MUST be translated exactly as specified:");
    for e in entries {
        out.push('\n');
        out.push_str("- \"");
        out.push_str(&e.source);
        out.push_str("\" → \"");
        out.push_str(&e.target);
        out.push('"');
        if let Some(note) = &e.note {
            if !note.is_empty() {
                out.push_str(" (");
                out.push_str(note);
                out.push(')');
            }
        }
    }
    out
}

impl Glossary {
    /// Mutable access to entries (test-only and for in-place SIGHUP swaps).
    /// Production callers should not modify entries directly — load a new
    /// `Glossary` and replace the existing one wholesale.
    #[cfg(any(test, feature = "internal-test-helpers"))]
    pub fn entries_mut(&mut self) -> &mut Vec<GlossaryEntry> {
        &mut self.entries
    }

    /// Return the entries that should inject into `{{ glossary_block }}`
    /// for a translation of `source_text` from `source_lang_iso2` to
    /// `target_lang_iso2`. Both lang codes are 2-letter ISO 639-1; pass
    /// `None` for `source_lang_iso2` when detection failed (treated as
    /// `unknown` — only `*`-scoped entries apply).
    ///
    /// When `cfg.enabled = false`, returns empty.
    pub fn matching_entries(
        &self,
        source_text: &str,
        source_lang_iso2: Option<&str>,
        target_lang_iso2: Option<&str>,
        cfg: &crate::config::GlossaryConfig,
    ) -> Vec<&GlossaryEntry> {
        if !cfg.enabled {
            return vec![];
        }
        let pair_key = format!(
            "{}->{}",
            source_lang_iso2.unwrap_or("unknown"),
            target_lang_iso2.unwrap_or("unknown"),
        );
        let strategy_cfg = MatchingStrategy::parse(&cfg.matching).unwrap_or(MatchingStrategy::Auto);
        // For `auto`, prefer the caller-supplied iso2 (already detected
        // for the pair key); fall back to text detection only if absent.
        // Non-Auto strategies skip detection entirely.
        let resolved = match strategy_cfg {
            MatchingStrategy::Auto => {
                default_strategy(&resolve_iso3(source_text, source_lang_iso2))
            }
            other => other,
        };
        self.entries
            .iter()
            .filter(|e| pair_matches(&e.languages, &pair_key))
            .filter(|e| term_matches(source_text, &e.source, cfg.case_sensitive, resolved))
            .collect()
    }

    /// Pair-scope-agnostic preview for the prompt window's chip strip.
    /// Surfaces every entry whose source term matches the clipboard,
    /// regardless of pair, so the user sees what *might* inject before
    /// they pick a target slot. The actual translator path applies pair
    /// scoping in `matching_entries`.
    pub fn preview_entries(
        &self,
        source_text: &str,
        source_lang_iso2: Option<&str>,
        cfg: &crate::config::GlossaryConfig,
    ) -> Vec<&GlossaryEntry> {
        if !cfg.enabled {
            return vec![];
        }
        let strategy_cfg = MatchingStrategy::parse(&cfg.matching).unwrap_or(MatchingStrategy::Auto);
        // Prefer the caller-supplied source language (App detects once at
        // show_window time); fall back to local detection only if absent.
        let resolved = match strategy_cfg {
            MatchingStrategy::Auto => {
                default_strategy(&resolve_iso3(source_text, source_lang_iso2))
            }
            other => other,
        };
        self.entries
            .iter()
            .filter(|e| term_matches(source_text, &e.source, cfg.case_sensitive, resolved))
            .collect()
    }
}

/// Resolve a 3-letter ISO 639-3 language code for the auto-strategy
/// decision. Prefers the caller-supplied iso2 (cheap reverse-map); falls
/// back to whatlang detection on text. Returns `"unknown"` when both fail
/// — `default_strategy` treats that as `WordBoundary`.
fn resolve_iso3(source_text: &str, source_lang_iso2: Option<&str>) -> String {
    if let Some(iso3) = source_lang_iso2.and_then(iso2_to_iso3) {
        return iso3.to_string();
    }
    detect_source_lang(source_text).unwrap_or_else(|| "unknown".to_string())
}

/// Reverse of `iso3_to_iso2` for the small set of CJK/Thai languages that
/// drive the substring auto-strategy decision. Most callers don't need a
/// full inverse — just enough to pick the right strategy.
fn iso2_to_iso3(iso2: &str) -> Option<&'static str> {
    match iso2 {
        "ja" => Some("jpn"),
        "zh" => Some("zho"),
        "th" => Some("tha"),
        "lo" => Some("lao"),
        "my" => Some("mya"),
        "km" => Some("khm"),
        // For all other pair-key languages, returning None means we'll
        // fall back to running detect_source_lang again — fine, since
        // those map to word_boundary anyway.
        _ => None,
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

    // ---- term_matches ----

    #[test]
    fn word_boundary_matches_full_word() {
        assert!(term_matches(
            "We have a Smart Table here",
            "Smart Table",
            false,
            MatchingStrategy::WordBoundary,
        ));
    }

    #[test]
    fn word_boundary_does_not_match_inside_word() {
        // "case" inside "casein" — classic spec example.
        assert!(!term_matches(
            "We use casein protein",
            "case",
            false,
            MatchingStrategy::WordBoundary,
        ));
    }

    #[test]
    fn substring_matches_inside_word() {
        // CJK/Thai-style: substring would catch "case" in "casein".
        assert!(term_matches(
            "We use casein protein",
            "case",
            false,
            MatchingStrategy::Substring,
        ));
    }

    #[test]
    fn case_insensitive_match_is_default() {
        assert!(term_matches(
            "we have a SMART TABLE here",
            "Smart Table",
            false,
            MatchingStrategy::WordBoundary,
        ));
    }

    #[test]
    fn case_sensitive_match_respects_flag() {
        assert!(!term_matches(
            "we have a SMART TABLE here",
            "Smart Table",
            true,
            MatchingStrategy::WordBoundary,
        ));
        assert!(term_matches(
            "We have a Smart Table here",
            "Smart Table",
            true,
            MatchingStrategy::WordBoundary,
        ));
    }

    #[test]
    fn substring_handles_punctuation_terms() {
        // Substring pathway: "C++" should match anywhere it appears.
        assert!(term_matches(
            "I write C++ code",
            "C++",
            false,
            MatchingStrategy::Substring,
        ));
    }

    #[test]
    fn word_boundary_handles_punctuation_terms() {
        // Word-boundary's char-class check treats `+` as non-word, so
        // "C++" hits when surrounded by whitespace and misses when
        // jammed against alphanumerics — proves the boundary check
        // actually fires (the historical regex-escaping concern).
        assert!(term_matches(
            "I write C++ code",
            "C++",
            false,
            MatchingStrategy::WordBoundary,
        ));
        assert!(!term_matches(
            "I write C++code",
            "C++",
            false,
            MatchingStrategy::WordBoundary,
        ));
    }

    // ---- pair_matches ----

    #[test]
    fn wildcard_pair_matches_any_pair() {
        assert!(pair_matches(&["*".into()], "de->en"));
        assert!(pair_matches(&["*".into()], "tr->de"));
        assert!(pair_matches(&["*".into()], "unknown->en"));
    }

    #[test]
    fn specific_pair_matches_only_itself() {
        assert!(pair_matches(&["de->en".into()], "de->en"));
        assert!(!pair_matches(&["de->en".into()], "en->de"));
        assert!(!pair_matches(&["de->en".into()], "tr->en"));
    }

    #[test]
    fn multiple_pairs_match_any() {
        let pairs = vec!["de->en".into(), "en->de".into()];
        assert!(pair_matches(&pairs, "de->en"));
        assert!(pair_matches(&pairs, "en->de"));
        assert!(!pair_matches(&pairs, "fr->en"));
    }

    // ---- Glossary::matching_entries ----

    fn build_glossary() -> Glossary {
        let mut g = Glossary::empty();
        g.entries_mut().push(GlossaryEntry {
            source: "Smart Table".into(),
            target: "Smart Table".into(),
            languages: vec!["*".into()],
            note: None,
        });
        g.entries_mut().push(GlossaryEntry {
            source: "Vorgang".into(),
            target: "case".into(),
            languages: vec!["de->en".into()],
            note: None,
        });
        g.entries_mut().push(GlossaryEntry {
            source: "case".into(),
            target: "Vorgang".into(),
            languages: vec!["en->de".into()],
            note: None,
        });
        g
    }

    #[test]
    fn matching_entries_filters_by_pair_scope() {
        let g = build_glossary();
        let cfg = crate::config::GlossaryConfig::default();
        // Source is German, target English — "Vorgang" scoped to de->en applies.
        let hits = g.matching_entries(
            "Wir öffnen einen neuen Vorgang.",
            Some("de"),
            Some("en"),
            &cfg,
        );
        let sources: Vec<&str> = hits.iter().map(|e| e.source.as_str()).collect();
        assert!(sources.contains(&"Vorgang"));
        assert!(
            !sources.contains(&"case"),
            "case is en->de; should not fire on de->en source"
        );
    }

    #[test]
    fn matching_entries_includes_wildcard_scoped_in_any_pair() {
        let g = build_glossary();
        let cfg = crate::config::GlossaryConfig::default();
        let hits = g.matching_entries(
            "Buy a Smart Table for the kitchen.",
            Some("en"),
            Some("de"),
            &cfg,
        );
        let sources: Vec<&str> = hits.iter().map(|e| e.source.as_str()).collect();
        assert!(sources.contains(&"Smart Table"));
    }

    #[test]
    fn matching_entries_unknown_source_lang_falls_back_to_wildcard_only() {
        let g = build_glossary();
        let cfg = crate::config::GlossaryConfig::default();
        // Source language unknown → only `*`-scoped entries apply.
        let hits = g.matching_entries(
            "Smart Table und Vorgang", // Both terms present, but lang unknown.
            None,
            Some("en"),
            &cfg,
        );
        let sources: Vec<&str> = hits.iter().map(|e| e.source.as_str()).collect();
        assert!(sources.contains(&"Smart Table"));
        assert!(
            !sources.contains(&"Vorgang"),
            "Vorgang's scope is de->en; unknown source must not match"
        );
    }

    #[test]
    fn matching_entries_skips_non_matching_terms() {
        let g = build_glossary();
        let cfg = crate::config::GlossaryConfig::default();
        let hits = g.matching_entries(
            "This text has none of the glossary terms in it.",
            Some("en"),
            Some("de"),
            &cfg,
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn matching_entries_disabled_returns_empty() {
        let g = build_glossary();
        let cfg = crate::config::GlossaryConfig {
            enabled: false,
            ..Default::default()
        };
        let hits = g.matching_entries("Smart Table", Some("en"), Some("de"), &cfg);
        assert!(hits.is_empty());
    }

    #[test]
    fn matching_entries_uses_substring_for_cjk_source_under_auto() {
        // Construct a Japanese-source entry with substring-style matching.
        let mut g = Glossary::empty();
        g.entries_mut().push(GlossaryEntry {
            source: "東京".into(),
            target: "Tokyo".into(),
            languages: vec!["*".into()],
            note: None,
        });
        let cfg = crate::config::GlossaryConfig {
            matching: "auto".into(),
            ..Default::default()
        };
        // No whitespace around 東京 — word_boundary would miss it.
        let hits = g.matching_entries("私は東京に住んでいます。", Some("ja"), Some("en"), &cfg);
        assert_eq!(hits.len(), 1);
    }

    // ---- preview_entries (chip strip) ----

    #[test]
    fn preview_entries_ignores_pair_scope() {
        // At preview time, target is unknown — we still want to surface
        // entries whose source term matches, regardless of pair.
        let g = build_glossary();
        let cfg = crate::config::GlossaryConfig::default();
        let hits = g.preview_entries("Buy a Smart Table for the Vorgang.", Some("de"), &cfg);
        let sources: Vec<&str> = hits.iter().map(|e| e.source.as_str()).collect();
        // Both Smart Table (* scope) and Vorgang (de->en scope) appear.
        assert!(sources.contains(&"Smart Table"));
        assert!(sources.contains(&"Vorgang"));
    }

    // ---- format_block ----

    #[test]
    fn format_block_empty_returns_empty_string() {
        let out = format_block(&[]);
        assert_eq!(out, "");
    }

    #[test]
    fn format_block_renders_canonical_spec_example() {
        let entries = [
            GlossaryEntry {
                source: "Smart Table".into(),
                target: "Smart Table".into(),
                languages: vec!["*".into()],
                note: None,
            },
            GlossaryEntry {
                source: "Vorgang".into(),
                target: "case".into(),
                languages: vec!["de->en".into()],
                note: None,
            },
            GlossaryEntry {
                source: "GIP".into(),
                target: "GIP".into(),
                languages: vec!["*".into()],
                note: Some("Always preserve as-is".into()),
            },
        ];
        let refs: Vec<&GlossaryEntry> = entries.iter().collect();
        let out = format_block(&refs);
        let expected = "GLOSSARY — these terms MUST be translated exactly as specified:\n\
- \"Smart Table\" → \"Smart Table\"\n\
- \"Vorgang\" → \"case\"\n\
- \"GIP\" → \"GIP\" (Always preserve as-is)";
        assert_eq!(out, expected);
    }

    #[test]
    fn format_block_no_trailing_whitespace() {
        let entries = [GlossaryEntry {
            source: "FOO".into(),
            target: "BAR".into(),
            languages: vec!["*".into()],
            note: None,
        }];
        let refs: Vec<&GlossaryEntry> = entries.iter().collect();
        let out = format_block(&refs);
        assert!(!out.ends_with(' '));
        assert!(!out.ends_with('\n'));
    }

    #[test]
    fn format_block_handles_quotes_in_terms() {
        // No escaping: terms are passed through verbatim. Quote characters
        // in source/target are unusual and the LLM tolerates them.
        let entries = [GlossaryEntry {
            source: "say \"hi\"".into(),
            target: "say \"hello\"".into(),
            languages: vec!["*".into()],
            note: None,
        }];
        let refs: Vec<&GlossaryEntry> = entries.iter().collect();
        let out = format_block(&refs);
        assert!(out.contains("\"say \"hi\"\" → \"say \"hello\"\""));
    }
}
