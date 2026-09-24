//! Shared Latin folding for slugs (section 6) and search (section 7). One table of
//! letters, so a slug and a search form never disagree about a letter.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// How the Norwegian letters and their neighbours are spelled in ASCII.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Letters {
    /// ADR-002's reversible form: æ→ae, ø→oe, å→aa, ä→ae, ö→oe.
    Transliterate,
    /// What people type without Norwegian letters: æ→a, ø→o, å→a, ä→a, ö→o.
    Lossy,
}

/// NFC and full lowercase, the letter table, then NFKD with combining marks dropped.
/// Letters of other scripts pass through; `collapse` drops them.
pub(crate) fn latin_fold(input: &str, letters: Letters) -> String {
    let lower = input.nfc().collect::<String>().to_lowercase();
    let mut mapped = String::with_capacity(lower.len() + 8);
    for c in lower.chars() {
        match (c, letters) {
            ('æ' | 'ä', Letters::Transliterate) => mapped.push_str("ae"),
            ('ø' | 'ö', Letters::Transliterate) => mapped.push_str("oe"),
            ('å', Letters::Transliterate) => mapped.push_str("aa"),
            ('æ' | 'ä' | 'å', Letters::Lossy) => mapped.push('a'),
            ('ø' | 'ö', Letters::Lossy) => mapped.push('o'),
            ('đ', _) => mapped.push('d'),
            ('ŋ', _) => mapped.push('n'),
            ('ŧ', _) => mapped.push('t'),
            ('ß', _) => mapped.push_str("ss"),
            _ => mapped.push(c),
        }
    }
    mapped.nfkd().filter(|c| !is_combining_mark(*c)).collect()
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02BC}' | '`')
}

/// Keeps `[a-z0-9]` (ASCII-lowercasing anything NFKD produced in capitals), drops
/// apostrophes without a separator, and turns every other run into one `sep`,
/// trimmed at both ends.
pub(crate) fn collapse(input: &str, sep: char) -> String {
    let mut out = String::with_capacity(input.len());
    let mut gap = false;
    for c in input.chars() {
        if is_apostrophe(c) {
            continue;
        }
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push(sep);
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}
