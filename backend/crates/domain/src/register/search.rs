//! Search normalisation (section 7). Done in Rust at write time and at query time,
//! never in the database, so matching does not depend on its collation or LC_CTYPE.

use unicode_normalization::UnicodeNormalization;

use super::text::{collapse, latin_fold, Letters};

/// The space-joined matching forms of every name, for a `search_text` column: each
/// name folded (NFC, lowercase, letters kept), transliterated (`tromsoe`) and lossy
/// (`tromso`). Duplicate forms are kept once, in first-seen order.
///
/// The folded form (e.g. `tromsø`) can never match a [`search_query`] word-prefix
/// query, because `search_query` is always lossy ASCII and so never contains a
/// non-ASCII letter to prefix-match against. It is kept in `search_text` anyway, for
/// trigram similarity, which compares it directly against the un-folded query.
pub fn search_text<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut forms: Vec<String> = Vec::new();
    for name in names {
        let candidates = [
            folded(name),
            collapse(&latin_fold(name, Letters::Transliterate), ' '),
            collapse(&latin_fold(name, Letters::Lossy), ' '),
        ];
        for form in candidates {
            if !form.is_empty() && !forms.contains(&form) {
                forms.push(form);
            }
        }
    }
    forms.join(" ")
}

/// The query form: lossy, because that is what people type, and it matches all three
/// stored forms' lossy member.
///
/// Returns `""` when the query has no searchable characters (e.g. only punctuation,
/// or only characters `latin_fold`/`collapse` drop). A caller must treat that empty
/// result as "no rows", never run it through `like '%' || q || '%'` -- an empty
/// pattern there matches every row in the table, which is the opposite of what an
/// empty query means.
pub fn search_query(q: &str) -> String {
    collapse(&latin_fold(q, Letters::Lossy), ' ')
}

fn folded(name: &str) -> String {
    let lower = name.nfc().collect::<String>().to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SQL the persistence plan runs, in Rust: a word-prefix match.
    fn finds(names: &[&str], q: &str) -> bool {
        format!(" {}", search_text(names.iter().copied()))
            .contains(&format!(" {}", search_query(q)))
    }

    #[test]
    fn tromsoe_every_way_people_type_it() {
        for q in ["tromso", "tromsoe", "Tromsø", "TROMSØ", "troms"] {
            assert!(finds(&["Tromsø"], q), "{q}");
        }
    }

    #[test]
    fn every_official_name_is_searchable() {
        let names = ["Karasjok", "Kárášjohka"];
        assert!(finds(&names, "karasjok"));
        assert!(finds(&names, "kárášjohka"));
        assert!(finds(&names, "karasjohka"));
    }

    #[test]
    fn prefixes_of_any_word() {
        assert!(finds(&["Ålesund"], "aal"));
        assert!(finds(&["Ålesund"], "ale"));
        assert!(finds(&["Nordre Follo"], "follo"));
        assert!(
            !finds(&["Nordre Follo"], "ollo"),
            "word prefixes only, not infixes"
        );
    }

    #[test]
    fn baerum_three_ways() {
        for q in ["bærum", "baerum", "barum", "Bærum"] {
            assert!(finds(&["Bærum"], q), "{q}");
        }
    }

    #[test]
    fn forms_are_deduplicated_and_single_spaced() {
        assert_eq!(search_text(["Oslo"]), "oslo");
        assert_eq!(search_text(["Tromsø"]), "tromsø tromsoe tromso");
        let t = search_text(["Nordre  Follo", "Nordre-Follo"]);
        assert!(!t.contains("  "));
        assert_eq!(t, "nordre follo");
    }

    #[test]
    fn a_query_is_lossy_and_single_spaced() {
        assert_eq!(search_query("  Nordre--Follo "), "nordre follo");
        assert_eq!(search_query("Åsane"), "asane");
        assert_eq!(search_query("?!"), "");
    }
}
