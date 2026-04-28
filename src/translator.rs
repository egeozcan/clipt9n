//! Translator orchestration. M1 contents:
//!   - `post_process()` — clean LLM output before clipboard write (spec §5.6)
//!
//! Task 12 adds the `Action` enum, the `Translator` struct, and orchestration.

/// Apply spec §5.6 post-processing to model output before writing to clipboard.
///
/// Steps (in order):
///   1. Trim leading/trailing whitespace.
///   2. If the entire response is wrapped in matching `"..."`, `"..."`, `«...»`,
///      or `„..."` quotes AND the source text was not, strip the wrapping quotes.
///   3. Strip a leading "Here is the translation:" / "Translation:" /
///      "Übersetzung:" preamble (regex fallback for prompt failures).
///   4. Preserve all internal formatting (line breaks, lists, code blocks).
pub fn post_process(model_output: &str, source_text: &str) -> String {
    // Step 1: trim outer whitespace
    let trimmed = model_output.trim();

    // Step 2: strip wrapping quotes if source wasn't quoted
    let dequoted = strip_wrapping_quotes_if_safe(trimmed, source_text);

    // Step 3: strip preamble
    strip_preamble(&dequoted).into_owned()
}

/// Quote pairs we recognize. Each tuple is (open, close).
const QUOTE_PAIRS: &[(char, char)] = &[
    ('"', '"'),    // ASCII straight quote
    ('\u{201C}', '\u{201D}'),  // Curly: " "
    ('\u{00AB}', '\u{00BB}'),  // Guillemets: « »
    ('\u{201E}', '\u{201C}'),  // German low-9 / left double: „ "
];

fn strip_wrapping_quotes_if_safe(text: &str, source: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return text.to_string();
    }
    let first = chars[0];
    let last = chars[chars.len() - 1];

    let matches_pair = QUOTE_PAIRS.iter().any(|(o, c)| first == *o && last == *c);
    if !matches_pair {
        return text.to_string();
    }

    // Don't strip if the source itself starts with the same opening quote —
    // the user clearly intended the quotes to be there.
    let source_trimmed = source.trim();
    if let Some(src_first) = source_trimmed.chars().next() {
        if src_first == first {
            return text.to_string();
        }
    }

    // Strip exactly one quote from each end.
    let stripped: String = chars[1..chars.len() - 1].iter().collect();
    stripped
}

fn strip_preamble(text: &str) -> std::borrow::Cow<'_, str> {
    // Spec §5.6 lists three concrete preambles. We match them case-insensitively
    // at the very start of the text, optionally followed by whitespace/newline.
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // The string contains an embedded quoted phrase but isn't itself wrapped
        // in matching outer quotes, so nothing should be stripped.
        let model = "She said \"hello\" politely";
        assert_eq!(post_process(model, "input"), "She said \"hello\" politely");
    }

    #[test]
    fn no_op_on_clean_text() {
        assert_eq!(post_process("Hallo, Welt.", "Hello, world."), "Hallo, Welt.");
    }
}
