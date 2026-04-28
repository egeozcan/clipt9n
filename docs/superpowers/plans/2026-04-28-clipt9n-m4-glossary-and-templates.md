# clipt9n M4 — Glossary + Template Overrides + SIGHUP Reload — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the spec's user-facing terminology and templating layer — a TOML glossary file with pair-scoped entries that auto-inject into the system prompt, four user-overridable Jinja templates, a glossary chip preview in the prompt window, and SIGHUP-driven hot reload.

**Architecture:** A new `src/glossary.rs` owns parsing/scoping/matching. The existing `src/llm/templates.rs` grows a `Templates` struct that loads built-ins at compile time and overlays file-based overrides at startup. `Translator::new` gains two parameters (`&Templates`, `&Glossary`) so the caller controls lifecycle. `App` holds an `Arc<RwLock<Glossary>>` so a SIGHUP-driven reload can atomically swap the entries without touching anything else. A new `src/platform/unix.rs` (cfg(unix), Linux + macOS) installs the SIGHUP handler via `tokio::signal::unix`.

**Tech Stack:** Rust 2021 / eframe 0.31 / egui 0.31 (already pinned). One new crate: `whatlang = "0.16"` (~1MB binary cost, dependency-free language detector). All cross-platform discipline rules from M2/M3 still apply: every `cfg(target_os)` and `cfg(unix)` block lives in `src/platform/`.

> **Branch:** This plan executes on `m4-glossary-and-templates`, branched from `main` (currently at `6ff8959`, post-M3 fast-forward). Working directory: `/Users/egecan/Code/clipt9n`.

---

## File structure

After M4, the tree gains:

```
src/
├── app.rs                       ← MODIFIED: load glossary+templates;
│                                              SIGHUP rx; drop _source_text param
├── config.rs                    ← MODIFIED: [glossary] + [templates] sections
├── error.rs                     ← MODIFIED: TranslateError::Glossary variant
├── glossary.rs                  ← NEW: load + scope + match + format glossary block
├── lib.rs                       ← MODIFIED: load Templates + Glossary in run()
├── llm/
│   └── templates.rs             ← MODIFIED: Templates struct + override loader
├── platform/
│   ├── mod.rs                   ← MODIFIED: install_sighup_reload free fn (cfg dispatch)
│   └── unix.rs                  ← NEW: tokio::signal::unix sighup listener
├── translator.rs                ← MODIFIED: Translator::new takes &Templates, &Glossary
└── ui/
    └── prompt.rs                ← MODIFIED: glossary chip strip above slot list
Cargo.toml                       ← MODIFIED: add whatlang = "0.16"
README.md                        ← MODIFIED: M4 section (glossary, overrides, SIGHUP)
```

Boundary discipline (unchanged from M3):
- `src/platform/` is the **only** place `#[cfg(target_os = …)]` and `#[cfg(unix)]` may appear (with the single audited exception in `config::Modifier::resolve_native`). This task adds `src/platform/unix.rs`; the cfg-dispatch lives in `src/platform/mod.rs`.
- `src/ui/` knows nothing about `tokio`, `reqwest`, `whatlang`, or platform specifics — it only paints frames and emits intents.
- `src/glossary.rs` knows nothing about `egui` — it's a pure data + algorithm module.
- `src/app.rs` is the seam between egui (sync), tokio (async), and the platform layer.

---

## Glossary of cross-cutting decisions (read once)

These come up repeatedly; agreeing on them up front prevents drift.

1. **Pair keys are 2-letter ISO 639-1.** The glossary file uses `"de->en"`-style strings (matching spec §5.4 example). At translation time, the source language is detected via `whatlang` (3-letter ISO 639-3), then mapped to its 2-letter form via a small table; the target language code comes from the picked slot (already 2-letter in `cfg.languages.slot_N.code`). Pair `*` matches all.

2. **`auto` matching strategy uses the 3-letter form for the substring/word-boundary decision.** Per spec §5.4, `auto` picks `substring` for `zho`, `jpn`, `tha`, `lao`, `mya`, `khm` and `word_boundary` for everything else (including `unknown`). This decision is made on the *whatlang-detected* code, before the iso2 mapping.

3. **whatlang confidence threshold = 0.5.** Below this, we treat detection as `unknown` and default to `word_boundary`. Spec §13 left the exact value open; 0.5 is the median of whatlang's `Detector::confidence()` range and is the conservative middle. Documented as a private const in `src/glossary.rs` (not user-configurable; can be revisited in M8 if false-negatives matter).

4. **Glossary failures degrade silently; template failures abort startup.** Spec §8 is explicit:
   - Glossary file malformed → log a warning, disable glossary for this session, app continues.
   - Template file malformed → startup error with file:line, abort.
   - Template references unknown variable → startup error with file:line, abort.
   The Translator never panics on missing/empty glossary input.

5. **Chip-strip placement: above the slot list.** Per the M3→M4 handoff, the chip strip is rendered between the preview block and the slot ScrollArea, capped at one wrap-row (~28px). This is a deliberate deviation from the design's JSX (which puts chips below the menu) — in egui, the slot ScrollArea has bounded height and adding chips below it competes for vertical space. Above means slot overflow is the casualty, not the chips. Conditional render: zero hits → strip is omitted entirely (no reserved space).

6. **Chip-strip preview ignores pair scoping.** At preview time the user hasn't picked a target language, so we cannot apply pair-key scoping. Show all entries whose source term matches the clipboard text (regardless of pair) as an informational hint. The actual translator path applies pair scoping correctly. Cap at 5 chips with `+N more` overflow text.

7. **Templates are loaded once at startup; glossary is loaded once at startup AND on SIGHUP.** This matches spec §5.3 + §5.4. Templates are immutable post-startup (per the design — startup-error semantics make sense only for static loading). Glossary is `Arc<RwLock<Glossary>>` so the SIGHUP handler thread can swap it atomically without restarting the app.

8. **Explicit four template overrides, nothing else.** Spec §6 names exactly `templates/translate.j2`, `templates/fix_grammar.j2`, `templates/rewrite.j2`, `templates/custom.j2`. Any other `.j2` file in the templates dir is ignored (not autoloaded). The `[templates]` config block is *paths* (relative to config_dir), letting users point any of the four to a custom location while keeping the rest as built-ins. Empty/null means "use built-in".

9. **`Translator::new` gains two parameters.** Becomes `Translator::new(cfg, provider, templates, glossary)`. All call sites — including the M1-shipped tests in `src/translator.rs`, M1's `lib.rs::run`, and the `src/app.rs` worker spawn — must update. Provide `Templates::built_in()` and `Glossary::empty()` constructors so test code can build instances cheaply.

10. **`decide_intent`'s `_source_text` parameter is dropped.** The handoff flagged this — glossary lookup happens at translator-execute time, not at slot-pick time, so the parameter is dead. Removed in Task 7 alongside the Translator signature change.

11. **Cross-platform SIGHUP discipline.** A new free function `platform::install_sighup_reload(rt, tx)` in `platform/mod.rs` cfg-dispatches to either the new `platform/unix.rs::install` (Linux + macOS via `cfg(unix)`) or a no-op stub for Windows. **Don't** put `cfg(unix)` in `app.rs` or `glossary.rs`.

12. **No new dependencies beyond whatlang.** `Arc<RwLock<_>>` uses `std::sync::RwLock`; SIGHUP listener uses `tokio::signal::unix::signal` (already available because we built tokio with `features = ["full"]`). No `nix`, no `signal-hook`.

---

## Pre-flight: Confirm starting state

- [ ] **Step 0.1: Verify branch and clean tree**

Run:
```bash
git rev-parse --abbrev-ref HEAD
git status --short
```
Expected: branch `m4-glossary-and-templates`, no working-tree changes.

If you're still on `main`, branch and check out:
```bash
git checkout -b m4-glossary-and-templates
```

- [ ] **Step 0.2: Verify M3 tests pass on this branch**

Run: `cargo test --all-features 2>&1 | grep "test result:"`
Expected: lines totaling **117 passed; 0 failed** across the lib, integration, and doctest test runs.

If either step fails, stop and report.

- [ ] **Step 0.3: Verify clippy + fmt are clean before any changes**

Run: `cargo clippy --all-features -- -D warnings 2>&1 | tail -3`
Expected: `Finished` clean, no warnings.

Run: `cargo fmt --check 2>&1 | tail -3`
Expected: empty output (already formatted).

---

## Task 1: Add `[glossary]` and `[templates]` config sections

**Files:**
- Modify: `src/config.rs:13-20` (top-level `Config` struct + new section structs)
- Modify: `src/config.rs::tests` (new tests at the end of the module)

**Why:** Both M4 features (glossary and template overrides) are gated by config sections per spec §6. We add them up front so all subsequent tasks can read configured values rather than threading hardcoded defaults.

- [ ] **Step 1.1: Write the failing tests**

Append to `src/config.rs`'s `tests` mod (after `loads_confirm_size_threshold_override`, around line 360):

```rust
    #[test]
    fn default_glossary_is_enabled_with_default_path() {
        let cfg = Config::default();
        assert!(cfg.glossary.enabled);
        assert_eq!(cfg.glossary.file, "glossary.toml");
        assert!(!cfg.glossary.case_sensitive);
        assert_eq!(cfg.glossary.matching, "auto");
    }

    #[test]
    fn loads_glossary_overrides() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[glossary]
enabled = false
file = "my-glossary.toml"
case_sensitive = true
matching = "word_boundary"
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert!(!cfg.glossary.enabled);
        assert_eq!(cfg.glossary.file, "my-glossary.toml");
        assert!(cfg.glossary.case_sensitive);
        assert_eq!(cfg.glossary.matching, "word_boundary");
    }

    #[test]
    fn default_template_paths_point_at_templates_dir() {
        let cfg = Config::default();
        assert_eq!(
            cfg.templates.translate.as_deref(),
            Some("templates/translate.j2")
        );
        assert_eq!(
            cfg.templates.fix_grammar.as_deref(),
            Some("templates/fix_grammar.j2")
        );
        assert_eq!(
            cfg.templates.rewrite.as_deref(),
            Some("templates/rewrite.j2")
        );
        assert_eq!(
            cfg.templates.custom.as_deref(),
            Some("templates/custom.j2")
        );
    }

    #[test]
    fn loads_template_overrides() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
[templates]
translate = "alt/translate.j2"
custom = ""
"#
        )
        .unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(
            cfg.templates.translate.as_deref(),
            Some("alt/translate.j2")
        );
        // Empty string is preserved as Some("") — Task 6 treats it as "use built-in".
        assert_eq!(cfg.templates.custom.as_deref(), Some(""));
        // Other templates default to their conventional paths.
        assert_eq!(
            cfg.templates.fix_grammar.as_deref(),
            Some("templates/fix_grammar.j2")
        );
    }
```

- [ ] **Step 1.2: Run tests to verify failure**

Run: `cargo test --lib config 2>&1 | tail -10`
Expected: compilation error on `cfg.glossary` and `cfg.templates` (fields don't exist).

- [ ] **Step 1.3: Add the new sections to `Config`**

In `src/config.rs`, modify the top-level `Config` struct (currently lines 13-20) to add the two new fields:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderConfig,
    pub languages: LanguagesConfig,
    pub hotkey: HotkeyConfig,
    pub ui: UiConfig,
    pub glossary: GlossaryConfig,
    pub templates: TemplatesConfig,
}
```

Append the two new structs after `UiConfig` (around line 144):

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GlossaryConfig {
    /// When false, the glossary loader is bypassed entirely and
    /// `{{ glossary_block }}` always renders empty.
    pub enabled: bool,
    /// Path to the glossary TOML file, relative to the config dir.
    pub file: String,
    /// Whether term matching against source text is case-sensitive.
    /// Default false (case-insensitive); spec §6 default.
    pub case_sensitive: bool,
    /// One of "auto", "word_boundary", "substring". Spec §5.4. The
    /// glossary parser validates this value at load; arbitrary strings
    /// fall back to "auto" with a warn log.
    pub matching: String,
}

impl Default for GlossaryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            file: "glossary.toml".into(),
            case_sensitive: false,
            matching: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TemplatesConfig {
    /// Path (relative to config dir) for an override file. `None` or
    /// `Some("")` means "use built-in for this action". Default values
    /// point at the conventional `templates/<action>.j2` paths; the
    /// override loader treats those as opt-in (file must exist).
    pub translate: Option<String>,
    pub fix_grammar: Option<String>,
    pub rewrite: Option<String>,
    pub custom: Option<String>,
}

impl Default for TemplatesConfig {
    fn default() -> Self {
        Self {
            translate: Some("templates/translate.j2".into()),
            fix_grammar: Some("templates/fix_grammar.j2".into()),
            rewrite: Some("templates/rewrite.j2".into()),
            custom: Some("templates/custom.j2".into()),
        }
    }
}
```

- [ ] **Step 1.4: Run tests to verify pass**

Run: `cargo test --lib config 2>&1 | tail -10`
Expected: all config tests pass (16 total in this module after the additions).

- [ ] **Step 1.5: Update the module-level doc comment**

The module's top-of-file doc still says glossary/templates are "loaded into the struct but not consumed by M1." Now they're consumed. **Replace** the doc comment at `src/config.rs:1-5`:

```rust
//! `config.toml` loader. Reads the spec §6 schema. M1–M3 only consume
//! `[provider]`, `[provider.api_key]`, `[languages]`, `[hotkey]`, `[ui]`.
//! M4 adds `[glossary]` and `[templates]`. Other sections (`[history]`,
//! `[tray]`, `[logging]`) are still loaded into the struct but unused
//! pending later milestones. Defaults applied when fields are absent.
```

- [ ] **Step 1.6: Commit**

```bash
git add src/config.rs
git commit -m "feat(M4): [glossary] + [templates] config sections"
```

---

## Task 2: Add `whatlang` dep + glossary types and TOML loader

**Files:**
- Modify: `Cargo.toml` (add `whatlang = "0.16"`)
- Create: `src/glossary.rs`
- Modify: `src/lib.rs` (add `pub mod glossary;`)
- Modify: `src/error.rs` (add `Glossary` variant)

**Why:** Before any matching or block-formatting logic, M4 needs:
1. The `whatlang` dependency (used in Task 3).
2. A typed representation of `glossary.toml`'s shape per spec §5.4.
3. A loader that returns `Glossary::empty()` on missing file (graceful) and `TranslateError::Glossary(...)` on malformed TOML (caller logs warn and proceeds with empty).

- [ ] **Step 2.1: Add the `whatlang` dependency**

In `Cargo.toml`, append `whatlang = "0.16"` to the `[dependencies]` section. Place it alphabetically near `tracing`. The exact line:

```toml
whatlang = "0.16"
```

After saving, run:
```bash
cargo build 2>&1 | tail -5
```
Expected: `Finished` clean. The dependency downloads and compiles (~3-5s on first run).

- [ ] **Step 2.2: Add the `TranslateError::Glossary` variant**

In `src/error.rs`, add to the enum (after `Internal`, around line 43):

```rust
    #[error("glossary error: {0}")]
    Glossary(String),
```

In the same file's `tests` mod (around line 53), add an assertion to `display_strings_are_user_facing`:

```rust
        assert_eq!(
            TranslateError::Glossary("malformed entry at line 5".into()).to_string(),
            "glossary error: malformed entry at line 5"
        );
```

Run: `cargo test --lib error 2>&1 | tail -5`
Expected: pass.

- [ ] **Step 2.3: Write the failing tests for `src/glossary.rs`**

Create `src/glossary.rs` with this initial content (types, loader, tests):

```rust
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
        let contents = std::fs::read_to_string(path).map_err(|e| {
            TranslateError::Glossary(format!("reading {}: {e}", path.display()))
        })?;
        let mut g: Self = toml::from_str(&contents).map_err(|e| {
            TranslateError::Glossary(format!("parsing {}: {e}", path.display()))
        })?;
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
        let g = Glossary::load(Path::new(
            "/tmp/clipt9n-nonexistent-glossary-12345.toml",
        ))
        .unwrap();
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
```

In `src/lib.rs`, add the module declaration alphabetically (after `error;`, around line 7):

```rust
pub mod glossary;
```

- [ ] **Step 2.4: Run tests to verify pass**

Run: `cargo test --lib glossary 2>&1 | tail -10`
Expected: 5 tests pass.

Run a full build to confirm nothing else broke: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

- [ ] **Step 2.5: Commit**

```bash
git add Cargo.toml Cargo.lock src/error.rs src/glossary.rs src/lib.rs
git commit -m "feat(M4): glossary types + TOML loader + whatlang dep"
```

---

## Task 3: Source-language detection + auto-matching strategy

**Files:**
- Modify: `src/glossary.rs` (append helpers to the existing module)

**Why:** The `auto` matching strategy (spec §5.4) decides per-translation between `word_boundary` and `substring` based on the detected source language. The decision uses whatlang's 3-letter ISO 639-3 form. For pair-key formatting we additionally need a 3→2 mapping (whatlang returns `deu`, glossary uses `de`).

- [ ] **Step 3.1: Write the failing tests**

Append to `src/glossary.rs`'s `tests` mod:

```rust
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
        assert_eq!(MatchingStrategy::parse("auto"), Some(MatchingStrategy::Auto));
        assert_eq!(
            MatchingStrategy::parse("word_boundary"),
            Some(MatchingStrategy::WordBoundary)
        );
        assert_eq!(
            MatchingStrategy::parse("substring"),
            Some(MatchingStrategy::Substring)
        );
        assert_eq!(MatchingStrategy::parse("AUTO"), Some(MatchingStrategy::Auto));
        assert!(MatchingStrategy::parse("garbage").is_none());
    }
```

- [ ] **Step 3.2: Run tests to verify failure**

Run: `cargo test --lib glossary 2>&1 | tail -10`
Expected: compilation errors on `detect_source_lang`, `iso3_to_iso2`, `default_strategy`, `MatchingStrategy` (none exist yet).

- [ ] **Step 3.3: Implement the helpers**

Append to `src/glossary.rs` (after the `impl Glossary` block, before the `tests` mod):

```rust
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
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "word_boundary" => Some(Self::WordBoundary),
            "substring" => Some(Self::Substring),
            _ => None,
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
```

- [ ] **Step 3.4: Run tests to verify pass**

Run: `cargo test --lib glossary 2>&1 | tail -15`
Expected: 14 tests pass (5 from Task 2 + 9 from this task).

- [ ] **Step 3.5: Commit**

```bash
git add src/glossary.rs
git commit -m "feat(M4): source-language detection + auto-strategy mapping"
```

---

## Task 4: Glossary matching helpers + `matching_entries` integration

**Files:**
- Modify: `src/glossary.rs` (append term-match + pair-scope + integrating function)

**Why:** With strategies and lang-detection in place, this task adds:
1. `term_matches(text, term, case_sensitive, strategy)` — the per-entry source-text scan.
2. `pair_matches(entry_pairs, current_pair)` — the pair-scope filter (`*` matches any).
3. `Glossary::matching_entries(...)` — the integrating function that the translator (Task 7) and the chip-strip preview (Task 9) both call.

- [ ] **Step 4.1: Write the failing tests**

Append to `src/glossary.rs`'s `tests` mod:

```rust
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
    fn term_with_regex_metacharacters_is_escaped() {
        // The implementation must escape regex metas before wrapping
        // in `\b…\b` — otherwise "C++" would crash or false-match.
        assert!(term_matches(
            "I write C++ code",
            "C++",
            false,
            MatchingStrategy::Substring,
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
        let mut cfg = crate::config::GlossaryConfig::default();
        cfg.enabled = false;
        let hits = g.matching_entries(
            "Smart Table",
            Some("en"),
            Some("de"),
            &cfg,
        );
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
        let mut cfg = crate::config::GlossaryConfig::default();
        cfg.matching = "auto".into();
        // No whitespace around 東京 — word_boundary would miss it.
        let hits = g.matching_entries(
            "私は東京に住んでいます。",
            Some("ja"),
            Some("en"),
            &cfg,
        );
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
```

- [ ] **Step 4.2: Run tests to verify failure**

Run: `cargo test --lib glossary 2>&1 | tail -15`
Expected: compilation errors on `term_matches`, `pair_matches`, `Glossary::matching_entries`, `Glossary::entries_mut`, `Glossary::preview_entries`.

- [ ] **Step 4.3: Implement the helpers**

Append to `src/glossary.rs` (after `default_strategy`, before the `tests` mod):

```rust
/// Test whether a single glossary term matches `source_text` under the
/// given strategy and case sensitivity. Word-boundary uses a regex
/// `\b<escaped term>\b` — the `\b` is Unicode-naive but accepts the same
/// boundaries as the spec example for whitespace-using scripts.
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
                let pre_ok = pre.map_or(true, |c| !is_word_char(c));
                let post_ok = post.map_or(true, |c| !is_word_char(c));
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
    entry_pairs
        .iter()
        .any(|p| p == "*" || p == current_pair)
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
        let strategy_cfg =
            MatchingStrategy::parse(&cfg.matching).unwrap_or(MatchingStrategy::Auto);
        // For `auto`, resolve once per call against the detected source
        // language's 3-letter form. Detection failed → "unknown" → falls
        // through to word_boundary in default_strategy.
        let detected_iso3 =
            detect_source_lang(source_text).unwrap_or_else(|| "unknown".to_string());
        let resolved = match strategy_cfg {
            MatchingStrategy::Auto => default_strategy(&detected_iso3),
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
        let strategy_cfg =
            MatchingStrategy::parse(&cfg.matching).unwrap_or(MatchingStrategy::Auto);
        // Prefer the caller-supplied source language (App detects once at
        // show_window time); fall back to local detection only if absent.
        let resolved = match strategy_cfg {
            MatchingStrategy::Auto => {
                let iso3 = source_lang_iso2
                    .and_then(iso2_to_iso3)
                    .unwrap_or_else(|| {
                        detect_source_lang(source_text)
                            .unwrap_or_else(|| "unknown".to_string())
                    });
                default_strategy(&iso3)
            }
            other => other,
        };
        self.entries
            .iter()
            .filter(|e| term_matches(source_text, &e.source, cfg.case_sensitive, resolved))
            .collect()
    }
}

/// Reverse of `iso3_to_iso2` for the small set of CJK/Thai languages that
/// drive the substring auto-strategy decision. Most callers don't need a
/// full inverse — just enough to pick the right strategy.
fn iso2_to_iso3(iso2: &str) -> Option<String> {
    match iso2 {
        "ja" => Some("jpn".into()),
        "zh" => Some("zho".into()),
        "th" => Some("tha".into()),
        "lo" => Some("lao".into()),
        "my" => Some("mya".into()),
        "km" => Some("khm".into()),
        // For all other pair-key languages, returning None means we'll
        // fall back to running detect_source_lang again — fine, since
        // those map to word_boundary anyway.
        _ => None,
    }
}
```

Note the `entries_mut` method is gated behind `cfg(any(test, feature = "internal-test-helpers"))`. **Add the feature** to `Cargo.toml` so external test code (none yet, but possible) can opt in:

In `Cargo.toml`, append below the `[dev-dependencies]` section:

```toml
[features]
internal-test-helpers = []
```

- [ ] **Step 4.4: Run tests to verify pass**

Run: `cargo test --lib glossary 2>&1 | tail -15`
Expected: 28 tests pass (14 from earlier + 14 new).

If any `unreachable!` panics fire, the test was incorrectly calling `term_matches` with `MatchingStrategy::Auto` — fix the test setup, not the production code.

- [ ] **Step 4.5: Commit**

```bash
git add Cargo.toml src/glossary.rs
git commit -m "feat(M4): glossary matching + pair scoping + preview"
```

---

## Task 5: Glossary block formatting

**Files:**
- Modify: `src/glossary.rs` (append `format_block`)

**Why:** The translator (Task 7) calls `format_block(matched)` and substitutes the result into `{{ glossary_block }}`. The output format is locked by spec §5.4:

```
GLOSSARY — these terms MUST be translated exactly as specified:
- "Smart Table" → "Smart Table"
- "Vorgang" → "case"
- "GIP" → "GIP" (Always preserve as-is)
```

Empty input → empty string (no header, no trailing whitespace per spec).

- [ ] **Step 5.1: Write the failing tests**

Append to `src/glossary.rs`'s `tests` mod:

```rust
    // ---- format_block ----

    #[test]
    fn format_block_empty_returns_empty_string() {
        let out = format_block(&[]);
        assert_eq!(out, "");
    }

    #[test]
    fn format_block_renders_canonical_spec_example() {
        let entries = vec![
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
        let entries = vec![GlossaryEntry {
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
        let entries = vec![GlossaryEntry {
            source: "say \"hi\"".into(),
            target: "say \"hello\"".into(),
            languages: vec!["*".into()],
            note: None,
        }];
        let refs: Vec<&GlossaryEntry> = entries.iter().collect();
        let out = format_block(&refs);
        assert!(out.contains("\"say \"hi\"\" → \"say \"hello\"\""));
    }
```

- [ ] **Step 5.2: Run tests to verify failure**

Run: `cargo test --lib glossary 2>&1 | tail -10`
Expected: compilation error on `format_block` (function doesn't exist).

- [ ] **Step 5.3: Implement `format_block`**

Append to `src/glossary.rs` (after `pair_matches`, before the `impl Glossary` extension):

```rust
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
```

- [ ] **Step 5.4: Run tests to verify pass**

Run: `cargo test --lib glossary 2>&1 | tail -10`
Expected: 32 tests pass (28 from earlier + 4 new).

- [ ] **Step 5.5: Commit**

```bash
git add src/glossary.rs
git commit -m "feat(M4): glossary block formatting (spec §5.4)"
```

---

## Task 6: Template override loader

**Files:**
- Modify: `src/llm/templates.rs` (add `Templates` struct + override loader; update `render` signature)

**Why:** Per spec §5.3, if a user has placed `templates/translate.j2` (or any of the four) in their config dir, that file replaces the built-in for that action. Malformed templates abort startup with `file:line`; templates referencing undeclared variables also abort startup with `file:line`. M1's `render(kind, ctx)` always uses built-ins — M4 introduces a `Templates` struct that holds either a built-in or a parsed override per kind, and `render(&templates, kind, ctx)` consults it.

- [ ] **Step 6.1: Write the failing tests**

In `src/llm/templates.rs`, replace the existing `tests` mod's contents with the following — keeping the seven existing assertions (renamed to call into the new `Templates::built_in()` API) and adding new assertions for override loading:

```rust
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
    fn empty_path_string_means_use_builtin() {
        let dir = tempdir().unwrap();
        let mut cfg = TemplatesConfig::default();
        cfg.translate = Some(String::new());
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
```

- [ ] **Step 6.2: Run tests to verify failure**

Run: `cargo test --lib llm::templates 2>&1 | tail -15`
Expected: compilation errors on `Templates::built_in`, `Templates::load`, and the new `render(&t, kind, ctx)` signature.

- [ ] **Step 6.3: Implement `Templates`**

In `src/llm/templates.rs`, **replace** the existing `render` function (currently at lines 91-116) with this new module-level structure. The full new content from line 1 onward:

```rust
//! Render prompt templates. Built-in defaults from `prompts.rs`; user
//! overrides loaded from `<config_dir>/templates/<action>.j2` per spec §5.3.
//!
//! `Templates::load(...)` runs at startup and validates each override:
//!   - Parse error → `TranslateError::Template("<file>:<line>: <detail>")`
//!   - Renders with all known variables stubbed to verify no undeclared
//!     references → unknown var → `TranslateError::Template(...)`
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
    /// + validate it and use it as the source; otherwise use the built-in.
    /// Empty / `None` paths in `cfg` mean "use built-in" for that kind.
    ///
    /// Validation errors abort startup (`Err`); missing files do not
    /// (the path is just treated as "no override configured").
    pub fn load(config_dir: &Path, cfg: &TemplatesConfig) -> Result<Self, TranslateError> {
        let translate =
            load_one(config_dir, cfg.translate.as_deref(), TemplateKind::Translate)?;
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
    let source = std::fs::read_to_string(&abs).map_err(|e| {
        TranslateError::Template(format!("reading {}: {e}", abs.display()))
    })?;
    validate_template_source(&source, &abs, kind)?;
    Ok(source)
}

/// Validate a template by parsing it (catches syntax errors) and
/// rendering it with every known variable stubbed (catches references to
/// undeclared variables). Errors include `<file>:<line>` context.
fn validate_template_source(
    source: &str,
    path: &Path,
    kind: TemplateKind,
) -> Result<(), TranslateError> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_template(kind.name(), source).map_err(|e| {
        TranslateError::Template(format!(
            "{}:{}: parse error: {e}",
            path.display(),
            err_line(&e),
        ))
    })?;
    let tmpl = env.get_template(kind.name()).map_err(|e| {
        TranslateError::Template(format!("{}: load error: {e}", path.display()))
    })?;
    let render_result = tmpl.render(context! {
        source_language => "stub",
        target_language => "Stub",
        user_instruction => "stub",
        glossary_block => "",
    });
    if let Err(e) = render_result {
        return Err(TranslateError::Template(format!(
            "{}:{}: undefined variable or render error: {e}",
            path.display(),
            err_line(&e),
        )));
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
            TranslateError::Template(format!(
                "rendering '{}' failed at parse: {e}",
                kind.name()
            ))
        })?;
    let tmpl = env
        .get_template(kind.name())
        .map_err(|e| {
            TranslateError::Template(format!("'{}' not found: {e}", kind.name()))
        })?;
    tmpl.render(context! {
        source_language => ctx.source_language,
        target_language => ctx.target_language,
        user_instruction => ctx.user_instruction,
        glossary_block => ctx.glossary_block,
    })
    .map_err(|e| TranslateError::Template(format!("rendering '{}' failed: {e}", kind.name())))
}
```

The existing `tests` mod (replaced in Step 6.1 above) sits at the end as before.

- [ ] **Step 6.4: Run tests to verify pass**

Run: `cargo test --lib llm::templates 2>&1 | tail -15`
Expected: 11 tests pass (6 preserved + 5 new).

- [ ] **Step 6.5: Commit**

```bash
git add src/llm/templates.rs
git commit -m "feat(M4): template override loader with file:line errors"
```

---

## Task 7: Wire `Templates` and `Glossary` through `Translator`

**Files:**
- Modify: `src/translator.rs` (constructor signature; render path uses templates + glossary)
- Modify: `src/llm/mod.rs` (re-export the new types if needed; usually unchanged)
- Modify: `src/app.rs::dispatch_intent` (drop `_source_text` param)

**Why:** With `Templates` and `Glossary` in place as standalone modules, the translator integrates them. The constructor takes both; `execute` resolves the pair key, runs the glossary lookup, formats the block, and passes it to `render(&templates, kind, ctx)`. The previously-unused `_source_text` parameter on `decide_intent` is dropped (the handoff flagged it).

- [ ] **Step 7.1: Write the failing tests**

In `src/translator.rs`, replace the existing translator tests (the `Translator tests` section at lines ~258-388) — keep the `CapturingProvider` mock and rewrite each `Translator::new` call site to pass templates + glossary:

```rust
    // ----------- Translator tests (M4 signature) -----------

    use crate::glossary::{Glossary, GlossaryEntry};
    use crate::llm::templates::Templates;

    fn templates() -> Templates {
        Templates::built_in()
    }

    fn empty_glossary() -> Glossary {
        Glossary::empty()
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
        let _ = t.execute(&Action::Rewrite, "verbose original").await.unwrap();
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
        assert_eq!(result, "Hallo, Welt.");
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
```

- [ ] **Step 7.2: Run tests to verify failure**

Run: `cargo test --lib translator 2>&1 | tail -15`
Expected: compilation errors on `Translator::new(&cfg, &provider, &templates, &glossary)` (signature still has 2 params).

- [ ] **Step 7.3: Update `Translator` to accept `&Templates` and `&Glossary`**

In `src/translator.rs`, **replace** the existing `Translator` struct and its `impl` block (currently lines 33-88) with:

```rust
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
            TemplateKind::Custom => TemplateContext::for_custom(
                instruction.as_deref().unwrap_or(""),
                &glossary_block,
            ),
        };
        let system = render(self.templates, kind, &ctx)?;
        let model_output = self.provider.complete(&system, clipboard_text).await?;
        Ok(post_process(&model_output, clipboard_text))
    }

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
                (
                    TemplateKind::Custom,
                    None,
                    Some(instruction.clone()),
                    None,
                )
            }
        })
    }
}
```

Update the imports at the top of `src/translator.rs` (around lines 14-17) to include the new types:

```rust
use crate::config::Config;
use crate::error::TranslateError;
use crate::glossary::Glossary;
use crate::llm::templates::{render, TemplateContext, TemplateKind, Templates};
use crate::llm::LlmProvider;
```

- [ ] **Step 7.4: Run translator tests to verify pass**

Run: `cargo test --lib translator 2>&1 | tail -15`
Expected: all translator tests pass (16 + 2 new = 18; the existing post-process tests are untouched).

- [ ] **Step 7.5: Drop `_source_text` from `decide_intent`**

In `src/app.rs`, remove the unused parameter from `decide_intent` (currently at line ~671):

```rust
pub(crate) fn decide_intent(slot: u8, cfg: &Config) -> Option<Intent> {
```

Update the call site in `dispatch` (currently around line 179):

```rust
        let Some(intent) = decide_intent(slot, &self.cfg) else {
            tracing::info!(slot, "invalid slot ignored");
            return;
        };
```

Update all `decide_intent(...)` test calls in `src/app.rs::tests` to drop the source-text argument:

```rust
let intent = decide_intent(1, &cfg).expect("slot 1 is valid");
```

(Apply to all 7 call sites in the tests.)

- [ ] **Step 7.6: Run app tests**

Run: `cargo test --lib app 2>&1 | tail -10`
Expected: all 13 app tests pass.

- [ ] **Step 7.7: Commit**

```bash
git add src/translator.rs src/app.rs
git commit -m "refactor(M4): Translator takes &Templates + &Glossary; drop dead source_text param"
```

---

## Task 8: Load `Templates` and `Glossary` in `App::new` and `lib.rs::run`

**Files:**
- Modify: `src/app.rs` (`ClipApp::new` accepts and stores them; `start_translation` uses them)
- Modify: `src/lib.rs` (CLI `run` builds them and passes through)
- Modify: `src/main.rs` if it constructs `ClipApp` directly

**Why:** With Translator's new signature, the GUI app and the CLI `run` path both need to construct `Templates::load(...)` (strict) and `Glossary::load(...)` (graceful) at startup and thread them through. The glossary is wrapped in `Arc<RwLock<_>>` so a SIGHUP handler (Task 10) can swap it atomically.

- [ ] **Step 8.1: Update `ClipApp` to hold `Templates` + `Arc<RwLock<Glossary>>`**

In `src/app.rs`, modify the `ClipApp` struct (currently lines 64-99) to add the new fields:

```rust
pub struct ClipApp {
    cfg: Config,
    state_path: PathBuf,
    state: State,

    /// Boxed for shared ownership across async tasks.
    provider: std::sync::Arc<dyn LlmProvider>,

    /// Compiled templates (built-in + user overrides). Immutable for the
    /// lifetime of the app — overrides are validated at startup and
    /// changes require a restart.
    templates: std::sync::Arc<crate::llm::templates::Templates>,

    /// Glossary loaded from `<config_dir>/<cfg.glossary.file>`. Wrapped
    /// in `Arc<RwLock<_>>` so a SIGHUP handler (Task 10) can swap it
    /// atomically without touching anything else. Read access in
    /// `start_translation` is uncontended (no concurrent writes during
    /// rendering since the render path takes a read lock and the SIGHUP
    /// handler takes a write lock).
    glossary: std::sync::Arc<std::sync::RwLock<crate::glossary::Glossary>>,

    /// Path to the glossary file on disk. The SIGHUP handler reuses
    /// this for re-reads.
    glossary_path: PathBuf,

    runtime: Runtime,
    hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    result_tx: mpsc::Sender<TranslationOutcome>,
    result_rx: mpsc::Receiver<TranslationOutcome>,

    /// SIGHUP / glossary-reload signal receiver. Empty Ok(()) sent each
    /// time the user requests a reload.
    glossary_reload_rx: CrossbeamReceiver<()>,

    app_state: AppState,
    prompt_model: prompt::PromptModel,
    has_been_focused: bool,
    initial_focus_pending: bool,
    dispatch_gen: u64,
    reduced_motion: bool,
}
```

Update `ClipApp::new` to accept the new dependencies:

```rust
impl ClipApp {
    pub fn new(
        cc: &CreationContext<'_>,
        cfg: Config,
        provider: std::sync::Arc<dyn LlmProvider>,
        templates: std::sync::Arc<crate::llm::templates::Templates>,
        glossary: std::sync::Arc<std::sync::RwLock<crate::glossary::Glossary>>,
        glossary_path: PathBuf,
        glossary_reload_rx: CrossbeamReceiver<()>,
        _secrets: Box<dyn Secrets>,
        state_path: PathBuf,
        hotkey_rx: CrossbeamReceiver<GlobalHotKeyEvent>,
    ) -> Self {
        cc.egui_ctx.set_visuals(theme::visuals());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("clipt9n-async")
            .build()
            .expect("tokio runtime");

        let (result_tx, result_rx) = mpsc::channel();
        let state = State::load(&state_path);
        let reduced_motion = crate::platform::current().reduced_motion();

        Self {
            prompt_model: prompt::PromptModel {
                clipboard_text: String::new(),
                detected_lang: None,
                last_slot: state.last_slot,
                glossary_hits: vec![],
            },
            cfg,
            state_path,
            state,
            provider,
            templates,
            glossary,
            glossary_path,
            runtime,
            hotkey_rx,
            result_tx,
            result_rx,
            glossary_reload_rx,
            app_state: AppState::Idle,
            has_been_focused: false,
            initial_focus_pending: false,
            dispatch_gen: 0,
            reduced_motion,
        }
    }
    /* ... */
```

Note: `glossary_hits: vec![]` requires the `PromptModel` to grow this field — Task 9 adds it. For now, the app side compiles only after Task 9's struct change. **Sequence the tests carefully:** if you're running this out of order, do Step 8.1's struct edit + Step 9's `PromptModel` edit together, then test.

- [ ] **Step 8.2: Update `start_translation` to clone the templates + glossary into the worker closure**

In `src/app.rs::start_translation` (currently around line 237), update the worker spawn so it constructs the Translator with the new signature. Replace the `worker = self.runtime.spawn(async move {...})` block with:

```rust
        let templates = self.templates.clone();
        let glossary = self.glossary.clone();
        let worker = self.runtime.spawn(async move {
            // Take a read snapshot of the glossary at dispatch time. If a
            // SIGHUP-driven reload arrives mid-translation, the running
            // worker uses the snapshot it captured here; the next dispatch
            // sees the new entries.
            let g_snapshot = glossary.read().expect("glossary RwLock poisoned").clone();
            let translator = Translator::new(&cfg, provider.as_ref(), &templates, &g_snapshot);
            let result = translator.execute(&action, &source_text).await;
            TranslationOutcome {
                result,
                action_label,
                slot,
                gen,
            }
        });
```

The watcher spawn below it (for panic recovery via `JoinHandle::Err`) is unchanged.

- [ ] **Step 8.3: Wire up loading + plumbing in `lib.rs::run` (CLI path)**

In `src/lib.rs::run`, after `let cfg = Config::load(&cfg_path)?;`, add (around line 117):

```rust
    let config_dir = cfg_path.parent().map(|p| p.to_path_buf()).ok_or_else(|| {
        TranslateError::Config("config path has no parent dir".into())
    })?;
    let templates = std::sync::Arc::new(crate::llm::templates::Templates::load(
        &config_dir,
        &cfg.templates,
    )?);
    let glossary_path = config_dir.join(&cfg.glossary.file);
    let glossary = match crate::glossary::Glossary::load(&glossary_path) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, "glossary load failed; continuing without glossary");
            crate::glossary::Glossary::empty()
        }
    };
```

Update both `Translator::new(&cfg, provider.as_ref())` call sites in `lib.rs::run` (the test-only path around line 130 and the real-clipboard path around line 156) to pass templates + glossary:

```rust
        let translator = Translator::new(&cfg, provider.as_ref(), &templates, &glossary);
```

- [ ] **Step 8.4: Update `main.rs` (GUI binary) to pass new args to `ClipApp::new`**

Read `src/main.rs` to identify how `ClipApp::new` is currently called. If it's invoked from a `gui_main()` or similar function, add the loader calls there:

```rust
    let templates = std::sync::Arc::new(crate::llm::templates::Templates::load(
        &config_dir,
        &cfg.templates,
    )?);
    let glossary_path = config_dir.join(&cfg.glossary.file);
    let glossary_inner = match crate::glossary::Glossary::load(&glossary_path) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!(error = %e, "glossary load failed; continuing without glossary");
            crate::glossary::Glossary::empty()
        }
    };
    let glossary = std::sync::Arc::new(std::sync::RwLock::new(glossary_inner));

    // Glossary reload channel — wired up to SIGHUP in Task 10.
    let (glossary_reload_tx, glossary_reload_rx) = crossbeam_channel::unbounded::<()>();
    // Task 10: install_sighup_reload(rt, glossary_reload_tx) — wired in step 10.x.
    let _ = glossary_reload_tx; // suppress unused-variable warning until Task 10.
```

Pass the new args to `ClipApp::new(...)`. The exact call-site change depends on how M3 left `main.rs`; preserve all other arguments.

If `src/main.rs` does not currently construct `ClipApp` (e.g., the GUI is in a separate function), inspect that function and apply the same change there. Use `cargo check 2>&1 | tail -20` to surface any compile errors and fix the call sites guided by the diagnostics.

- [ ] **Step 8.5: Run the full test suite**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

Run: `cargo test --all-features 2>&1 | grep "test result:"`
Expected: still passing — total now ~150 tests.

- [ ] **Step 8.6: Commit**

```bash
git add src/app.rs src/lib.rs src/main.rs
git commit -m "feat(M4): wire Templates + Glossary load into App::new and CLI run"
```

---

## Task 9: Glossary chip strip in the prompt window

**Files:**
- Modify: `src/ui/prompt.rs` (add `GlossaryHit`, extend `PromptModel`, render chip strip above slot list)
- Modify: `src/app.rs::show_window` (compute `glossary_hits` from clipboard snapshot)

**Why:** The chip strip is the design's surfacing of "these glossary terms will likely inject" — a one-row preview between the preview block and the slot list. Per the cross-cutting decisions, placement is *above* the slot list (not below as in the JSX), capped at one wrap-row, and pair-scope-agnostic (uses `Glossary::preview_entries`).

- [ ] **Step 9.1: Add `GlossaryHit` and extend `PromptModel`**

In `src/ui/prompt.rs`, add after `SlotKind` (around line 60):

```rust
/// One glossary entry that matched the current clipboard. Rendered as a
/// chip in the prompt window's strip, between the preview and the slot
/// list. Pair-scope-agnostic: at preview time the user hasn't picked a
/// target, so we surface every source-term match regardless of pair.
/// The actual translator path applies pair scoping correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryHit {
    pub source: String,
    pub target: String,
}
```

Extend `PromptModel` (around line 13):

```rust
#[derive(Debug, Clone)]
pub struct PromptModel {
    pub clipboard_text: String,
    pub detected_lang: Option<String>,
    pub last_slot: Option<u8>,
    /// Glossary entries that match the clipboard text (pair-agnostic).
    /// Computed by `App::show_window` once per summon.
    pub glossary_hits: Vec<GlossaryHit>,
}
```

- [ ] **Step 9.2: Write the failing tests**

Append to `src/ui/prompt.rs`'s `tests` mod:

```rust
    #[test]
    fn truncate_chips_returns_all_when_under_limit() {
        let hits = vec![
            GlossaryHit {
                source: "A".into(),
                target: "A".into(),
            },
            GlossaryHit {
                source: "B".into(),
                target: "B".into(),
            },
        ];
        let (visible, more) = truncate_chips(&hits, 5);
        assert_eq!(visible.len(), 2);
        assert_eq!(more, 0);
    }

    #[test]
    fn truncate_chips_caps_at_max_and_returns_overflow_count() {
        let hits: Vec<GlossaryHit> = (0..8)
            .map(|i| GlossaryHit {
                source: format!("s{i}"),
                target: format!("t{i}"),
            })
            .collect();
        let (visible, more) = truncate_chips(&hits, 5);
        assert_eq!(visible.len(), 5);
        assert_eq!(more, 3);
    }

    #[test]
    fn glossary_hit_constructs_from_glossary_entry() {
        // Smoke test that the conversion path used by show_window
        // (Glossary::preview_entries → Vec<&GlossaryEntry> → Vec<GlossaryHit>)
        // produces the expected source/target pairing.
        let entry = crate::glossary::GlossaryEntry {
            source: "Smart Table".into(),
            target: "Smart Table".into(),
            languages: vec!["*".into()],
            note: None,
        };
        let hit = GlossaryHit {
            source: entry.source.clone(),
            target: entry.target.clone(),
        };
        assert_eq!(hit.source, "Smart Table");
        assert_eq!(hit.target, "Smart Table");
    }
```

- [ ] **Step 9.3: Run tests to verify failure**

Run: `cargo test --lib ui::prompt 2>&1 | tail -10`
Expected: compilation error on `truncate_chips` (function doesn't exist).

- [ ] **Step 9.4: Implement the chip-strip helper + draw block**

Append to `src/ui/prompt.rs` (after `should_warn_large_paste`, before the `tests` mod):

```rust
/// Cap the chip list at `max` items; return the visible slice and the
/// overflow count for the "+N more" indicator.
pub fn truncate_chips(hits: &[GlossaryHit], max: usize) -> (&[GlossaryHit], usize) {
    if hits.len() <= max {
        (hits, 0)
    } else {
        (&hits[..max], hits.len() - max)
    }
}

/// Maximum number of glossary chips rendered before "+N more" replaces
/// the overflow. Picked to fit one wrap-row at 480px window width.
const MAX_CHIPS: usize = 5;

/// Draw the glossary chip strip. Conditional render: zero hits → nothing
/// is painted (no reserved height; layout collapses cleanly). Chips
/// render in the design's `pw.chip` style: `var(--panel-3)` background,
/// monospace 11px, source/arrow/target three-segment colors.
fn draw_glossary_chips(ui: &mut egui::Ui, hits: &[GlossaryHit]) {
    if hits.is_empty() {
        return;
    }
    let (visible, more) = truncate_chips(hits, MAX_CHIPS);
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(0xff, 0xb8, 0x4d, 0x10))
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(0xff, 0xb8, 0x4d, 0x2e),
        ))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().spacing.item_spacing.x = 6.0;
                ui.label(
                    RichText::new("GLOSSARY WILL INJECT:")
                        .color(theme::WARN)
                        .size(10.0)
                        .strong(),
                );
                for hit in visible {
                    chip(ui, hit);
                }
                if more > 0 {
                    ui.label(
                        RichText::new(format!("+{more} more"))
                            .color(theme::INK_3)
                            .monospace()
                            .size(11.0),
                    );
                }
            });
        });
    ui.add_space(10.0);
}

fn chip(ui: &mut egui::Ui, hit: &GlossaryHit) {
    egui::Frame::new()
        .fill(theme::PANEL_3)
        .stroke(Stroke::new(1.0, theme::LINE))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.style_mut().spacing.item_spacing.x = 4.0;
                ui.label(
                    RichText::new(&hit.source)
                        .color(theme::INK_2)
                        .monospace()
                        .size(11.0),
                );
                ui.label(
                    RichText::new("→")
                        .color(theme::INK_3)
                        .monospace()
                        .size(11.0),
                );
                ui.label(
                    RichText::new(&hit.target)
                        .color(theme::ACCENT)
                        .monospace()
                        .size(11.0),
                );
            });
        });
}
```

- [ ] **Step 9.5: Wire the chip strip into `draw_populated`**

In `src/ui/prompt.rs::draw_populated`, find the section between the preview block (ends with `ui.add_space(14.0);` at line ~250) and the slot ScrollArea (`// ----- Slot rows -----` at line ~252). Insert the chip-strip render between them:

```rust
            // ----- Glossary chip strip (above slot list, per M4) -----
            draw_glossary_chips(ui, &model.glossary_hits);

            // ----- Slot rows -----
```

Also delete the now-stale comment at line ~272-274:

```rust
            // ----- Glossary chip area (M2 always empty; M4 fills it) -----
            // Empty placeholder reserved so the layout doesn't shift when M4
            // adds chips. Render nothing; the gap above the footer is enough.
```

- [ ] **Step 9.6: Compute `glossary_hits` in `App::show_window`**

In `src/app.rs::show_window` (around line 168), populate the new field after `clipboard_text` is captured:

```rust
    fn show_window(&mut self, ctx: &egui::Context) {
        self.prompt_model.clipboard_text = self.snapshot_clipboard();
        self.prompt_model.last_slot = self.state.last_slot;

        // Compute pair-agnostic glossary hits for the chip strip preview.
        // The translator applies pair scoping correctly at execute time;
        // this preview is informational only.
        let detected_iso3 =
            crate::glossary::detect_source_lang(&self.prompt_model.clipboard_text);
        let source_iso2 = detected_iso3
            .as_deref()
            .and_then(crate::glossary::iso3_to_iso2);
        self.prompt_model.detected_lang = source_iso2.map(String::from);
        let g = self.glossary.read().expect("glossary RwLock poisoned");
        self.prompt_model.glossary_hits = g
            .preview_entries(
                &self.prompt_model.clipboard_text,
                source_iso2,
                &self.cfg.glossary,
            )
            .into_iter()
            .map(|e| prompt::GlossaryHit {
                source: e.source.clone(),
                target: e.target.clone(),
            })
            .collect();
        drop(g); // release the read lock before mutating other state.

        self.has_been_focused = false;
        self.initial_focus_pending = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.app_state = AppState::Showing;
    }
```

- [ ] **Step 9.7: Run tests + build**

Run: `cargo test --lib ui::prompt 2>&1 | tail -10`
Expected: 9 tests pass (6 existing + 3 new).

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

- [ ] **Step 9.8: Manual smoke**

Run: `cargo run --release` in a terminal. Place a glossary file at `~/Library/Application Support/clipboard-translator/glossary.toml` containing the spec example (Smart Table, Vorgang, GIP). Copy text containing one of those terms (e.g., "We sell a Smart Table for the kitchen.") and trigger Cmd+Shift+T.

Expected: chip strip appears above the slot list with the warn-orange container, "GLOSSARY WILL INJECT:" label, and a chip showing `Smart Table → Smart Table` in monospace. Strip is one wrap-row, no layout shift visible. Copying text without glossary terms and re-triggering: no chip strip is rendered.

- [ ] **Step 9.9: Commit**

```bash
git add src/app.rs src/ui/prompt.rs
git commit -m "feat(M4): glossary chip strip above slot list (1-wrap-row preview)"
```

---

## Task 10: SIGHUP handler + glossary reload wiring

**Files:**
- Create: `src/platform/unix.rs`
- Modify: `src/platform/mod.rs` (add `install_sighup_reload` free function with cfg dispatch)
- Modify: `src/app.rs::drain_channels` (consume reload signals; swap glossary)
- Modify: `src/main.rs` (call `install_sighup_reload` after building the runtime; pass tx to channel)

**Why:** Per spec §5.4 + §6, glossaries should reload on SIGHUP without restart. The handler lives in `src/platform/unix.rs` (cfg(unix), Linux + macOS only); Windows gets a no-op. The reload path: SIGHUP fires → tokio task forwards `()` to a `crossbeam_channel` → `App::drain_channels` consumes → `Glossary::load(...)` runs and `RwLock::write()` swaps the entries.

- [ ] **Step 10.1: Create `src/platform/unix.rs`**

Create `src/platform/unix.rs`:

```rust
//! Unix (Linux + macOS) signal handling. Currently exposes a SIGHUP
//! listener that forwards reload requests to a sync channel. Lives in
//! `platform/` per the cross-platform discipline rule (no `cfg(unix)` in
//! `app.rs` or anywhere else).

use crossbeam_channel::Sender;
use tokio::runtime::Runtime;

use crate::error::TranslateError;

/// Spawn a tokio task on `rt` that listens for SIGHUP and forwards a
/// `()` to `tx` each time the signal arrives. Caller drains `tx`'s
/// receiver in its event loop and triggers whatever reload it owns.
///
/// The task runs until the runtime is dropped; if `tx` is dropped, sends
/// fail silently (logged at debug) and the task exits.
pub(crate) fn install(rt: &Runtime, tx: Sender<()>) -> Result<(), TranslateError> {
    rt.spawn(async move {
        let mut sighup =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "failed to install SIGHUP listener");
                    return;
                }
            };
        loop {
            match sighup.recv().await {
                Some(()) => {
                    tracing::info!("SIGHUP received; forwarding reload signal");
                    if tx.send(()).is_err() {
                        tracing::debug!("reload channel closed; SIGHUP listener exiting");
                        return;
                    }
                }
                None => {
                    tracing::debug!("SIGHUP stream ended; listener exiting");
                    return;
                }
            }
        }
    });
    Ok(())
}
```

- [ ] **Step 10.2: Add the cfg-dispatched free function to `platform/mod.rs`**

In `src/platform/mod.rs`, add (after the existing `current()` function, before the `tests` mod):

```rust
#[cfg(unix)]
mod unix;

/// Install a SIGHUP-driven reload listener. On Unix (Linux/macOS) this
/// spawns a tokio task that forwards every SIGHUP delivery to `tx`. On
/// Windows this is a no-op (signal model differs; tray-menu "Reload
/// glossary" is the equivalent affordance there in M7).
///
/// Returns `Ok(())` when the listener is installed (or when the OS does
/// not support it).
pub fn install_sighup_reload(
    rt: &tokio::runtime::Runtime,
    tx: crossbeam_channel::Sender<()>,
) -> Result<(), crate::error::TranslateError> {
    #[cfg(unix)]
    {
        return unix::install(rt, tx);
    }
    #[cfg(not(unix))]
    {
        let _ = (rt, tx);
        Ok(())
    }
}
```

- [ ] **Step 10.3: Add a smoke test for the platform-level dispatch**

Append to `src/platform/mod.rs`'s `tests` mod:

```rust
    #[test]
    fn install_sighup_reload_does_not_panic_on_current_platform() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (tx, _rx) = crossbeam_channel::unbounded::<()>();
        // Returns Ok regardless of OS — Unix installs a real listener,
        // Windows is no-op.
        assert!(install_sighup_reload(&rt, tx).is_ok());
    }
```

- [ ] **Step 10.4: Consume the reload signal in `App::drain_channels`**

In `src/app.rs::drain_channels` (around line 349), add a third drain block after the translation-results drain:

```rust
    fn drain_channels(&mut self, ctx: &egui::Context) {
        // Hotkey events
        while let Ok(_event) = self.hotkey_rx.try_recv() {
            if matches!(self.app_state, AppState::Idle) {
                self.show_window(ctx);
            } else {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
        }
        // Translation results
        while let Ok(outcome) = self.result_rx.try_recv() {
            self.handle_translation_done(outcome);
        }
        // Glossary reload requests (SIGHUP, tray menu in M7)
        let mut reload_requested = false;
        while self.glossary_reload_rx.try_recv().is_ok() {
            reload_requested = true;
        }
        if reload_requested {
            self.reload_glossary();
        }
    }

    fn reload_glossary(&mut self) {
        match crate::glossary::Glossary::load(&self.glossary_path) {
            Ok(g) => {
                let entry_count = g.len();
                let mut w = self.glossary.write().expect("glossary RwLock poisoned");
                *w = g;
                drop(w);
                tracing::info!(
                    path = %self.glossary_path.display(),
                    entries = entry_count,
                    "glossary reloaded"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %self.glossary_path.display(),
                    "glossary reload failed; keeping previous entries"
                );
            }
        }
    }
```

- [ ] **Step 10.5: Wire the SIGHUP installer in `main.rs`**

Find where the tokio runtime is constructed and where `ClipApp::new` is called in `src/main.rs`. Right after the runtime exists and the `glossary_reload_tx`/`glossary_reload_rx` pair is constructed (Step 8.4), call:

```rust
    crate::platform::install_sighup_reload(&runtime, glossary_reload_tx.clone())?;
```

If the runtime is owned by `ClipApp` (constructed inside `ClipApp::new`), the cleanest path is:
- Move runtime construction outside `ClipApp::new` — construct it in `main.rs`, pass it to `ClipApp::new`, install the SIGHUP listener before passing.

OR (simpler): keep runtime inside `ClipApp::new`, expose a `ClipApp::install_glossary_reload(&self, tx: Sender<()>) -> Result<()>` method that calls `install_sighup_reload` against the inner runtime. Pick whichever requires fewer signature changes.

Pragmatic choice: add a method:

```rust
    pub fn install_glossary_reload(
        &self,
        tx: crossbeam_channel::Sender<()>,
    ) -> Result<(), TranslateError> {
        crate::platform::install_sighup_reload(&self.runtime, tx)
    }
```

And in `main.rs`, after `ClipApp::new(...)`:

```rust
    app.install_glossary_reload(glossary_reload_tx)?;
```

- [ ] **Step 10.6: Run tests + build**

Run: `cargo test --lib platform 2>&1 | tail -10`
Expected: 5 tests pass (4 existing + 1 new).

Run: `cargo test --all-features 2>&1 | grep "test result:"`
Expected: total now ~155 tests, all passing.

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` clean.

- [ ] **Step 10.7: Manual smoke**

Run: `cargo run --release` in one terminal. In another, find the running PID and edit `~/Library/Application Support/clipboard-translator/glossary.toml` to add a new entry. Send SIGHUP:

```bash
pkill -HUP clipt9n
```

Look at the running app's stderr (or wherever tracing logs land). Expected: `INFO ... "SIGHUP received; forwarding reload signal"` followed by `INFO ... "glossary reloaded"` with the new entry count.

Trigger Cmd+Shift+T: the chip strip should now reflect the new entry if the clipboard contains the new term.

- [ ] **Step 10.8: Commit**

```bash
git add src/app.rs src/main.rs src/platform/mod.rs src/platform/unix.rs
git commit -m "feat(M4): SIGHUP-driven glossary reload (cfg(unix) listener in platform/)"
```

---

## Task 11: README updates + final verification

**Files:**
- Modify: `README.md`

**Why:** Document M4's new features and limitations for users. Verify the full test suite, clippy, fmt, and grep-lint all pass.

- [ ] **Step 11.1: Add an M4 features sub-section to `README.md`**

Append the following to `README.md` after the M3 section:

```markdown
### M4: Glossary + custom templates + SIGHUP reload

#### Glossary file

Place a `glossary.toml` in your config dir (macOS:
`~/Library/Application Support/clipboard-translator/glossary.toml`). Each
entry pins a source term to a fixed translation, optionally scoped to
language pairs:

```toml
[[entry]]
source = "Smart Table"
target = "Smart Table"
languages = ["*"]            # applies to every language pair

[[entry]]
source = "Vorgang"
target = "case"
languages = ["de->en"]       # only when translating German → English

[[entry]]
source = "GIP"
target = "GIP"
languages = ["*"]
note = "Always preserve as-is"
```

When the prompt window opens, matched terms appear in a chip strip above
the slot list ("GLOSSARY WILL INJECT: ..."). At translation time, the
pair-scoped subset is rendered into the system prompt as:

```
GLOSSARY — these terms MUST be translated exactly as specified:
- "Smart Table" → "Smart Table"
- "GIP" → "GIP" (Always preserve as-is)
```

Configure matching strategy via `[glossary]` in `config.toml`:

```toml
[glossary]
enabled = true
file = "glossary.toml"
case_sensitive = false
matching = "auto"            # auto | word_boundary | substring
```

`auto` (default): word_boundary for whitespace-using languages; substring
for `zho`, `jpn`, `tha`, `lao`, `mya`, `khm`. If your source-language
detection lands below the confidence threshold, `auto` falls back to
word_boundary (the safer choice for most target languages).

If `glossary.toml` is malformed, the app logs a warning at startup and
continues with no glossary. Editing the file and sending `SIGHUP` to the
running process reloads it without a restart:

```bash
pkill -HUP clipt9n
```

(SIGHUP reload is Linux + macOS only. The M7 tray menu's "Reload
glossary" item will provide a cross-platform alternative.)

#### Custom template overrides

The four built-in prompt templates are overridable via files in your
config dir's `templates/` folder. To override one, create the file at the
path listed in `[templates]` (defaults are `templates/<action>.j2`):

```
~/Library/Application Support/clipboard-translator/
└── templates/
    ├── translate.j2     ← overrides the built-in translate template
    ├── fix_grammar.j2
    ├── rewrite.j2
    └── custom.j2
```

Available variables:
- `{{ source_language }}` — auto-detected via whatlang; may be `unknown`
- `{{ target_language }}` — human-readable name (e.g., `"Deutsch"`)
- `{{ user_instruction }}` — only set in the `custom` template
- `{{ glossary_block }}` — pre-rendered glossary directives, or empty

Malformed templates abort startup with a `<file>:<line>` error.
References to undeclared variables likewise abort startup. Templates
are NOT reloaded on SIGHUP — restart the app after editing.

To force a built-in for a specific action, set its path to `""` in
`config.toml`:

```toml
[templates]
translate = ""               # use built-in regardless of file presence
custom = "templates/custom.j2"
```

#### M4 limitations (carried forward)

- whatlang's confidence threshold is hard-coded at 0.5. Misclassification
  on very short clipboards is best-effort; pair-scoped glossary entries
  may not fire when the language is low-confidence.
- The chip strip preview is pair-agnostic — it shows every term that
  matches the clipboard regardless of whether the pair will scope it
  out at translation time. This is informational; the translator
  applies pair scoping correctly.
- Templates can't be reloaded without a restart (only the glossary is
  hot-reloadable). M8 may add a tray-menu "Reload templates" action if
  there's demand.
```

- [ ] **Step 11.2: Run the full test suite**

Run: `cargo test --all-features 2>&1 | grep "test result:" | head -10`
Expected: ~155 passed; 0 failed.

- [ ] **Step 11.3: Cross-platform discipline check**

Verify M4 added no new `cfg(target_os)` or `cfg(unix)` blocks outside `src/platform/`:

```bash
grep -rn '#\[cfg(target_os' src/ | grep -v '^src/platform/' | grep -v '^src/config.rs:'
```
Expected: empty output.

```bash
grep -rn '#\[cfg(unix' src/ | grep -v '^src/platform/'
```
Expected: empty output.

If either grep returns a result, route the offending code into `src/platform/`.

- [ ] **Step 11.4: Clippy + fmt clean**

Run: `cargo clippy --all-features -- -D warnings 2>&1 | tail -3`
Expected: `Finished` clean, no warnings.

Run: `cargo fmt --check 2>&1 | tail -3`
Expected: empty output.

If either fails, fix and re-run.

- [ ] **Step 11.5: Verify only `whatlang` was added to deps**

Run: `git diff main..HEAD -- Cargo.toml` and confirm:
- The only new top-level dependency line is `whatlang = "0.16"`.
- The only new `[features]` section is `internal-test-helpers = []` (added in Task 4).

Run: `git diff main..HEAD -- .github/workflows/` and confirm CI is unchanged from M3.

- [ ] **Step 11.6: Manual M4 smoke matrix**

Build the release binary: `cargo build --release 2>&1 | tail -3`
Expected: `Finished` clean.

Run `target/release/clipt9n`. Exercise:

1. **Glossary hit, English source, German target**
   - Place `glossary.toml` with the spec example.
   - Copy `"Buy a Smart Table for the Vorgang."`
   - Trigger Cmd+Shift+T → chip strip shows `Smart Table → Smart Table` and `Vorgang → case`.
   - Press 2 (Deutsch) → translation output should preserve "Smart Table" verbatim.

2. **Pair-scoped glossary entry skipped on wrong pair**
   - Same clipboard.
   - Press 1 (English) → translator should not enforce `Vorgang → case` (en target, but the entry is scoped to de->en, not en->X). Output may still contain "case" naturally, but the system prompt visible in tracing logs should NOT include the Vorgang glossary block.

3. **Malformed glossary degrades silently**
   - Edit `glossary.toml` to `garbage [[[`.
   - Restart app → tracing shows `WARN ... "glossary load failed; continuing without glossary"`.
   - Translations still work, no chip strip.
   - Restore the file and send `pkill -HUP clipt9n` → tracing shows reload success → chip strip returns on next prompt.

4. **Template override**
   - Place a custom `templates/translate.j2` with text `"OVERRIDE: translate {{ source_language }} → {{ target_language }}: {{ glossary_block }}"`.
   - Restart, copy text, press a translate slot.
   - Trace logs show the system prompt starts with "OVERRIDE:".

5. **Malformed template aborts startup**
   - Replace `templates/translate.j2` with `"{% if foo and"`.
   - Restart → app exits with `template error: <path>/translate.j2:<line>: parse error: ...`.

6. **UTF-8 source still works under M4 changes**
   - Restore template + glossary.
   - Copy `"Bir aracıyı kullanarak..."` (Turkish).
   - Press Slot 1 (English) → completes normally; no panic. (Regression check: the M3 panic in `strip_preamble` should still be fixed.)

- [ ] **Step 11.7: Commit and finalize**

```bash
git add README.md
git commit -m "docs(M4): glossary, template overrides, SIGHUP reload"
```

Once all M4 commits are on `m4-glossary-and-templates`:

```bash
git log --oneline main..m4-glossary-and-templates
```

Expected output: ~11 commits, each starting with `feat(M4):`, `refactor(M4):`, `docs(M4):`, or `chore(M4):` (no merge commits inside the branch).

The branch is now ready for user review. Merge strategy mirrors M2 and M3: fast-forward to `main` once approved.

---

## Self-Review

Run this checklist after writing the plan; fix issues inline.

**1. Spec coverage (M4 row of design doc):**

| Spec deliverable | Plan task |
|---|---|
| `src/glossary.rs` — load `glossary.toml` per spec §5.4 | Tasks 2 (types + loader), 4 (matching), 5 (block formatting). |
| Pair-key matching (`*`, `de->en`, etc.) | Task 4 (`pair_matches`, integration in `matching_entries`). |
| Three matching strategies (auto / word_boundary / substring) | Tasks 3 (strategy enum + parse + `default_strategy`), 4 (`term_matches` per-strategy). |
| `whatlang` for source-language detection (with `unknown` fallback) | Task 3 (`detect_source_lang` with confidence threshold const = 0.5; `unknown` triggers `word_boundary` fallback). |
| Glossary block formatting that injects into `{{ glossary_block }}` | Tasks 5 (`format_block`), 7 (translator wires it into `TemplateContext`). |
| Template override loader; if file exists, replaces built-in for that action | Task 6 (`Templates::load`, validate parse + render, file:line errors). |
| Glossary chip preview in prompt window | Task 9 (chip strip render, `PromptModel.glossary_hits`, `App::show_window` populates it via `Glossary::preview_entries`). |
| SIGHUP handler to reload glossary without restart | Task 10 (`platform/unix.rs` listener + `install_sighup_reload` cfg dispatch + `App::reload_glossary`). |
| Tray "Reload glossary" menu item placeholder (real menu in M7) | Documented in Task 11 README; the `glossary_reload_rx` channel is the integration point M7 will add a tray sender to. No new menu rendered in M4. |

**Exit criteria from the design doc, M4 row:**

| Exit criterion | Plan coverage |
|---|---|
| 1. Glossary entries that match source text inject `{{ glossary_block }}` block correctly per spec §5.4 | Tasks 4 + 5 + 7. Verified by `glossary_block_is_injected_when_entries_match` and `glossary_block_omitted_when_no_entries_match` (translator tests). |
| 2. Glossary scope `*` and `de->en` filter correctly | Task 4 — `matching_entries_filters_by_pair_scope`, `matching_entries_includes_wildcard_scoped_in_any_pair`. |
| 3. Auto-matching uses word_boundary for de/en/tr/fr/es and substring for zho/jpn/tha (test cases for both) | Task 3 — `default_strategy_for_lang_uses_substring_for_no_whitespace_scripts`, `default_strategy_for_lang_uses_word_boundary_otherwise`. Task 4 — `matching_entries_uses_substring_for_cjk_source_under_auto`. |
| 4. Empty glossary block renders cleanly without trailing whitespace | Task 5 — `format_block_no_trailing_whitespace`. The existing `empty_glossary_block_does_not_leave_trailing_whitespace` test in templates.rs still passes (preserved in Task 6 rewrite). |
| 5. Malformed glossary disables glossary for the session, logs warn at startup, app continues | Task 8 — both `lib.rs::run` and `main.rs` callers wrap `Glossary::load` in a `match` that logs `WARN` and substitutes `Glossary::empty()` on `Err`. Verified by manual smoke #3 in Step 11.6. |
| 6. Override `templates/translate.j2` replaces built-in. Malformed template aborts startup with file+line. Template referencing an unknown variable aborts startup with file+line | Task 6 — `override_replaces_builtin_for_specified_action`, `malformed_template_returns_error_with_file_and_line`, `template_referencing_unknown_variable_returns_error`. |
| 7. SIGHUP reloads glossary without restart | Task 10 — `App::reload_glossary` swaps the entries via `RwLock::write()`. Verified by manual smoke #3 (SIGHUP path). |

**Design-doc cross-cutting items (M4 must inherit):**

| Item | Plan coverage |
|---|---|
| Cross-platform discipline — every `cfg(target_os)` and `cfg(unix)` block in `platform/` | Task 10 puts the new `cfg(unix)` dispatch inside `src/platform/mod.rs` and the impl in `src/platform/unix.rs`. Step 11.3 verifies via grep. |
| Worker panic recovery via `TranslateError::Internal` (M3) is preserved | Task 8.2 shows the worker spawn's panic-watcher pattern is unchanged — only the inner Translator construction is updated. |
| `_secrets` parameter on `ClipApp::new` is currently dead — M6 revives it | Task 8.1's new `ClipApp::new` signature still accepts and drops `_secrets: Box<dyn Secrets>` per M3. Unchanged. |

**2. Placeholder scan:** No "TBD", "implement later", "etc.", "similar to Task N", or naked "add error handling" appearances. Each step contains the actual code or actual command. Code blocks are complete (no `// ...` truncations within the action items). The README block in Task 11 contains the full user-facing text.

**3. Type consistency:**
- `GlossaryEntry { source, target, languages, note }` — same shape across Tasks 2 (defined), 4 (consumed in `matching_entries`), 5 (consumed in `format_block`), 9 (read for chip strip).
- `Glossary { entries }` — single private field, accessed via `entries()` (read) or `entries_mut()` (test+SIGHUP).
- `MatchingStrategy::{Auto, WordBoundary, Substring}` — declared in Task 3, consumed in Tasks 4 (`term_matches`) and 4 (`matching_entries`'s strategy resolution).
- `Templates { translate, fix_grammar, rewrite, custom }` — Task 6 declared, Task 7 consumed via `render(&templates, kind, ctx)`.
- `TemplatesConfig { translate, fix_grammar, rewrite, custom }` — Task 1 declared as `Option<String>` per kind, Task 6 `load_one` consumes it.
- `GlossaryConfig { enabled, file, case_sensitive, matching }` — Task 1 declared, Tasks 4 + 9 consumed.
- `Translator::new(cfg, provider, templates, glossary)` — declared in Task 7, called from Tasks 7 (tests), 8.2 (worker), 8.3 (CLI run).
- `PromptModel { clipboard_text, detected_lang, last_slot, glossary_hits }` — Task 9 grew it; `App::new` (Task 8) and `App::show_window` (Task 9) populate it; `prompt::draw` consumes it.
- `GlossaryHit { source, target }` — Task 9 declared, populated in `App::show_window`, consumed in `draw_glossary_chips`.
- `install_sighup_reload(rt, tx)` — Task 10 free function in `platform/mod.rs`; `unix::install` (Task 10.1) is the cfg(unix) impl.
- `ClipApp` field set: `templates: Arc<Templates>`, `glossary: Arc<RwLock<Glossary>>`, `glossary_path: PathBuf`, `glossary_reload_rx: CrossbeamReceiver<()>` — Task 8.1 declared, Tasks 8.2 + 9.6 + 10.4 consumed.
- `decide_intent(slot, cfg)` — Task 7.5 dropped `_source_text` parameter; all 8 call sites in `app.rs::tests` updated.
- `format_block`, `term_matches`, `pair_matches`, `default_strategy`, `detect_source_lang`, `iso3_to_iso2`, `iso2_to_iso3`, `MatchingStrategy::parse`, `LANG_CONFIDENCE_THRESHOLD`, `MAX_CHIPS`, `truncate_chips`, `draw_glossary_chips`, `chip` — all declared exactly once and consumed at named call sites.

No drift. Plan is consistent end-to-end.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-28-clipt9n-m4-glossary-and-templates.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration. Mirrors M1/M2/M3 execution flow.
2. **Inline Execution** — execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?
