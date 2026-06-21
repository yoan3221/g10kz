//! Text normalisation pipeline.
//!
//! Two levels:
//! - [`normalize_input`]    — light (NFKC + zero-width strip): safe for LLM input
//! - [`normalize_for_scan`] — aggressive (+ full-width collapse + homoglyph folding +
//!   lowercase): used by [`crate::guard`] and [`crate::route`] for injection scanning
//!
//! Neither function is called in the hot display path — [`normalize_input`] is
//! applied once to the incoming message before sending to the LLM.

use unicode_normalization::UnicodeNormalization;

// ─── Zero-width / invisible characters ───────────────────────────────────────

/// Characters stripped before any further processing.
const ZERO_WIDTH: &[char] = &[
    '\u{200B}', // ZERO WIDTH SPACE
    '\u{200C}', // ZERO WIDTH NON-JOINER
    '\u{200D}', // ZERO WIDTH JOINER
    '\u{FEFF}', // ZERO WIDTH NO-BREAK SPACE (BOM)
    '\u{00AD}', // SOFT HYPHEN
    '\u{2060}', // WORD JOINER
    '\u{180E}', // MONGOLIAN VOWEL SEPARATOR
    '\u{034F}', // COMBINING GRAPHEME JOINER
    '\u{2061}', // FUNCTION APPLICATION (invisible)
    '\u{2062}', // INVISIBLE TIMES
    '\u{2063}', // INVISIBLE SEPARATOR
    '\u{2064}', // INVISIBLE PLUS
];

/// Strip zero-width / invisible characters.
fn strip_zero_width(s: &str) -> String {
    s.chars().filter(|c| !ZERO_WIDTH.contains(c)).collect()
}

// ─── Homoglyph folding table ─────────────────────────────────────────────────
//
// Maps visually-identical non-ASCII characters to their ASCII equivalents.
// Covers the most common injection-obfuscation vectors (Cyrillic, Greek,
// lookalike punctuation).

static HOMOGLYPHS: &[(char, char)] = &[
    // Cyrillic → Latin
    ('а', 'a'),
    ('е', 'e'),
    ('о', 'o'),
    ('р', 'p'),
    ('с', 'c'),
    ('х', 'x'),
    ('у', 'y'),
    ('В', 'B'),
    ('К', 'K'),
    ('М', 'M'),
    ('Н', 'H'),
    ('О', 'O'),
    ('Р', 'P'),
    ('С', 'C'),
    ('Т', 'T'),
    ('Х', 'X'),
    ('А', 'A'),
    ('Е', 'E'),
    ('і', 'i'),
    ('Ѕ', 'S'),
    ('ѕ', 's'),
    ('ԁ', 'd'),
    ('ɑ', 'a'),
    ('ɡ', 'g'),
    // Greek → Latin
    ('α', 'a'),
    ('ε', 'e'),
    ('ι', 'i'),
    ('ο', 'o'),
    ('υ', 'u'),
    ('χ', 'x'),
    ('τ', 't'),
    ('κ', 'k'),
    ('ρ', 'r'),
    ('η', 'n'),
    ('ν', 'v'),
    ('ω', 'w'),
    ('ϲ', 'c'),
    // Lookalike punctuation
    ('‐', '-'),
    ('‑', '-'),
    ('‒', '-'),
    ('–', '-'),
    ('—', '-'),
    ('‛', '\''),
    ('\u{2018}', '\''),
    ('\u{2019}', '\''),
    ('\u{201C}', '"'),
    ('\u{201D}', '"'),
    // Math / script letters (common injection vectors)
    ('ℓ', 'l'),
    ('℃', 'C'),
    ('ℊ', 'g'),
];

/// Fold homoglyphs to their ASCII equivalents.
fn fold_homoglyphs(s: &str) -> String {
    s.chars()
        .map(|c| {
            HOMOGLYPHS
                .iter()
                .find(|(src, _)| *src == c)
                .map(|(_, dst)| *dst)
                .unwrap_or(c)
        })
        .collect()
}

// ─── Full-width → ASCII ───────────────────────────────────────────────────────
//
// Full-width ASCII variants live in U+FF01–U+FF5E.
// U+3000 (ideographic space) → U+0020 (regular space).

fn fullwidth_to_ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            _ => c,
        })
        .collect()
}

// ─── Run-length collapse ──────────────────────────────────────────────────────

/// Collapse runs of repeated punctuation to at most 3 consecutive characters.
/// `!!!!!!` → `!!!`, `??????` → `???`, `......` → `...`
fn collapse_repeated_punct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run: char = '\0';
    let mut count: usize = 0;
    const MAX_RUN: usize = 3;

    for c in s.chars() {
        if c == run && "!?.,~-*".contains(c) {
            count += 1;
            if count <= MAX_RUN {
                out.push(c);
            }
        } else {
            run = c;
            count = 1;
            out.push(c);
        }
    }
    out
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Light normalisation: safe for LLM input.
///
/// Applies:
/// 1. NFC Unicode normalisation (canonical form — preserves fullwidth and ligatures)
/// 2. Zero-width / invisible character removal
///
/// Does NOT fold homoglyphs or collapse full-width — the LLM sees the
/// user's actual text.
pub fn normalize_input(input: &str) -> String {
    let nfc: String = input.nfc().collect();
    strip_zero_width(&nfc)
}

/// Aggressive normalisation: for injection scanning only.
///
/// Applies everything in [`normalize_input`], then additionally:
/// 3. Full-width ASCII → ASCII
/// 4. Lowercase (before homoglyph folding — required for idempotency)
/// 5. Homoglyph folding (Cyrillic/Greek/lookalikes → ASCII)
/// 6. Repeated punctuation collapse
pub fn normalize_for_scan(input: &str) -> String {
    let light = normalize_input(input);
    let fw = fullwidth_to_ascii(&light);
    let lower = fw.to_lowercase(); // must precede fold for idempotency
    let hg = fold_homoglyphs(&lower);
    collapse_repeated_punct(&hg)
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── normalize_input (light) ───────────────────────────────────────────────

    #[test]
    fn nfc_preserves_ligatures() {
        // NFC does not split ligatures: ﬁ stays as-is
        let lig = "ﬁ";
        assert_eq!(normalize_input(lig), lig);
    }

    #[test]
    fn nfc_composes_combining_chars() {
        // A + combining acute -> Á (precomposed NFC)
        let decomposed = "Á";
        assert_eq!(normalize_input(decomposed), "Á");
    }

    #[test]
    fn strip_zwsp() {
        let s = "hello\u{200B}world";
        assert_eq!(normalize_input(s), "helloworld");
    }

    #[test]
    fn strip_zwnj() {
        let s = "abc\u{200C}def";
        assert_eq!(normalize_input(s), "abcdef");
    }

    #[test]
    fn strip_bom() {
        let s = "\u{FEFF}hello";
        assert_eq!(normalize_input(s), "hello");
    }

    #[test]
    fn strip_soft_hyphen() {
        let s = "soft\u{00AD}hyphen";
        assert_eq!(normalize_input(s), "softhyphen");
    }

    #[test]
    fn strip_multiple_zero_width() {
        let s = "a\u{200B}\u{200C}\u{200D}b";
        assert_eq!(normalize_input(s), "ab");
    }

    #[test]
    fn light_preserves_fullwidth() {
        // light normalize does NOT convert full-width — that's scan-only
        let s = "ａｂｃ";
        assert_eq!(normalize_input(s), "ａｂｃ");
    }

    #[test]
    fn light_preserves_cyrillic() {
        // light normalize does NOT fold homoglyphs
        let s = "сirсle"; // с = Cyrillic
        let result = normalize_input(s);
        assert!(result.contains('с'));
    }

    // ── normalize_for_scan (aggressive) ──────────────────────────────────────

    #[test]
    fn fullwidth_digits_to_ascii() {
        assert_eq!(normalize_for_scan("０１２３"), "0123");
    }

    #[test]
    fn fullwidth_lowercase_to_ascii() {
        assert_eq!(normalize_for_scan("ａｂｃ"), "abc");
    }

    #[test]
    fn fullwidth_uppercase_to_ascii() {
        assert_eq!(normalize_for_scan("ＡＢＣ"), "abc"); // uppercase → lowercase too
    }

    #[test]
    fn ideographic_space_to_ascii_space() {
        assert_eq!(normalize_for_scan("a\u{3000}b"), "a b");
    }

    #[test]
    fn cyrillic_a_to_ascii_a() {
        // Cyrillic 'а' (U+0430) → 'a'
        let cyrillic_a = '\u{0430}';
        let s = cyrillic_a.to_string();
        assert_eq!(normalize_for_scan(&s), "a");
    }

    #[test]
    fn cyrillic_o_to_ascii_o() {
        let s = "о"; // Cyrillic о
        assert_eq!(normalize_for_scan(s), "o");
    }

    #[test]
    fn greek_alpha_to_ascii_a() {
        assert_eq!(normalize_for_scan("α"), "a");
    }

    #[test]
    fn greek_omicron_to_ascii_o() {
        assert_eq!(normalize_for_scan("ο"), "o");
    }

    #[test]
    fn collapse_repeated_exclamation() {
        assert_eq!(normalize_for_scan("wow!!!!!!"), "wow!!!");
    }

    #[test]
    fn collapse_repeated_question() {
        assert_eq!(normalize_for_scan("what??????"), "what???");
    }

    #[test]
    fn collapse_repeated_dots() {
        assert_eq!(normalize_for_scan("hmm......"), "hmm...");
    }

    #[test]
    fn scan_is_lowercase() {
        assert_eq!(normalize_for_scan("HELLO"), "hello");
    }

    #[test]
    fn clean_ascii_passthrough() {
        assert_eq!(normalize_for_scan("hello world"), "hello world");
    }

    #[test]
    fn homoglyph_mixed_injection() {
        // "ѕystem рrompt" using Cyrillic look-alikes
        let s = "ѕystem рrompt";
        let result = normalize_for_scan(s);
        assert!(result.contains("system"), "got: {result}");
        assert!(result.contains("prompt"), "got: {result}");
    }

    // ── property-based tests ─────────────────────────────────────────────────

    proptest! {
        #[test]
        fn normalize_input_is_idempotent(s in "\\PC*") {
            let once = normalize_input(&s);
            let twice = normalize_input(&once);
            prop_assert_eq!(once, twice);
        }

        #[test]
        fn normalize_for_scan_is_idempotent(s in "\\PC*") {
            let once = normalize_for_scan(&s);
            let twice = normalize_for_scan(&once);
            prop_assert_eq!(once, twice);
        }

        #[test]
        fn scan_has_no_zero_width(s in "\\PC*") {
            let result = normalize_for_scan(&s);
            for c in ZERO_WIDTH {
                prop_assert!(!result.contains(*c), "found zero-width {:?}", c);
            }
        }

        #[test]
        fn scan_ascii_letters_are_lowercased(s in "[a-zA-Z0-9 ！？。、]+") {
            // ASCII letters in the scan output are always lowercase
            let result = normalize_for_scan(&s);
            let has_uppercase_ascii = result.chars().any(|c| c.is_ascii_uppercase());
            prop_assert!(!has_uppercase_ascii, "uppercase ASCII in output: {:?}", result);
        }
    }
}
