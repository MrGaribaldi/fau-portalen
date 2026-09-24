//! Brreg FAU entities (sections 2.5 and 4.6, D1). Addresses pass through these
//! functions in memory only; nothing here is ever stored (ruling, 24 September 2026).

use super::search::search_query;

/// Lossy-folded words that mark a Brreg entity as an FAU, matched as whole words.
const FAU_WORDS: &[&str] = &[
    "fau",
    "arbeidsutvalg",
    "arbeidsutval",
    "arbeidsutvalet",
    "arbeidsutvalget",
    "foreldrerad",
    "foreldreradet",
    "foreldreutval",
    "foreldreutvalg",
    "samarbeidsutvalg",
];

/// Words that say what kind of body or school it is, not which one (section 2.5).
const GENERIC_WORDS: &[&str] = &[
    "fau",
    "foreldrenes",
    "foreldreradets",
    "foreldreradet",
    "foreldrerad",
    "arbeidsutvalg",
    "arbeidsutval",
    "arbeidsutvalet",
    "arbeidsutvalget",
    "ved",
    "pa",
    "for",
    "i",
    "og",
    "v",
    "skole",
    "skolen",
    "skoles",
    "skule",
    "skulen",
    "skules",
    "barneskole",
    "barneskule",
    "ungdomsskole",
    "ungdomsskule",
    "ungdomskole",
    "barne",
    "oppvekstsenter",
];

/// Acronyms kept in capitals when a registered name is re-cased.
const ACRONYMS: &[&str] = &["FAU", "SFO", "AU"];

/// Whether a registered name looks like an FAU (the caller also requires form FLI).
pub fn is_fau_name(name: &str) -> bool {
    search_query(name)
        .split(' ')
        .any(|w| FAU_WORDS.contains(&w))
}

/// A normalised street line and its postcode. Compared in memory; never stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AddressKey {
    pub street: String,
    pub postcode: String,
}

/// Every usable address line as a key: lines naming a person (`c/o`, `v/`) or a post
/// box are skipped, a line must carry a number, and the postcode must be four digits.
pub fn address_keys<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    postcode: Option<&str>,
) -> Vec<AddressKey> {
    let Some(postcode) = postcode.filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_digit()))
    else {
        return Vec::new();
    };
    let mut keys: Vec<AddressKey> = lines
        .into_iter()
        .filter_map(street_line)
        .map(|street| AddressKey {
            street,
            postcode: postcode.to_owned(),
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn street_line(line: &str) -> Option<String> {
    let words: Vec<String> = search_query(line)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let personal = matches!(words.as_slice(), [c, o, ..] if c == "c" && o == "o")
        || words.first().is_some_and(|w| w == "v");
    let postbox = words
        .iter()
        .any(|w| w == "postboks" || w == "pb" || w == "boks");
    if personal || postbox || !words.iter().any(|w| w.bytes().any(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        // "18 a" and "18a" are the same house.
        if word.len() == 1 && word.as_bytes()[0].is_ascii_lowercase() {
            if let Some(prev) = out.last_mut() {
                if prev.bytes().all(|b| b.is_ascii_digit()) {
                    prev.push_str(&word);
                    continue;
                }
            }
        }
        out.push(street_word(&word));
    }
    Some(out.join(" "))
}

/// Street-type spellings that differ between registrations of the same address.
fn street_word(word: &str) -> String {
    for (suffix, canonical) in [
        ("veien", "vei"),
        ("vegen", "vei"),
        ("veg", "vei"),
        ("gaten", "gate"),
        ("gata", "gate"),
    ] {
        if let Some(stem) = word.strip_suffix(suffix) {
            return format!("{stem}{canonical}");
        }
    }
    match word {
        "vn" | "v" => "vei".to_owned(),
        "gt" => "gate".to_owned(),
        _ => word.to_owned(),
    }
}

/// The identifying part of an FAU's or a school's name: lossy-folded, with the words
/// that name the kind of body or school removed.
pub fn name_core(name: &str) -> String {
    search_query(name)
        .split(' ')
        .filter(|w| !w.is_empty() && !GENERIC_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The signup form's FAU-name suggestion from a Brreg name, which Brreg stores in
/// capitals. A name with any lowercase letter is returned as registered.
pub fn suggest_fau_name(registered: &str, school_display_name: &str) -> String {
    let registered = registered.trim();
    if registered.chars().any(char::is_lowercase) {
        return registered.to_owned();
    }
    let school_words: Vec<&str> = school_display_name
        .split(|c: char| c.is_whitespace() || c == '-' || c == '/')
        .filter(|w| !w.is_empty())
        .collect();
    registered
        .split_whitespace()
        .enumerate()
        .map(|(i, word)| recase(word, i == 0, &school_words))
        .collect::<Vec<_>>()
        .join(" ")
}

fn recase(word: &str, first: bool, school_words: &[&str]) -> String {
    let core: &str = word.trim_matches(|c: char| !c.is_alphanumeric());
    if core.is_empty() {
        return word.to_lowercase();
    }
    let start = word.find(core).expect("core is a substring of word");
    let (prefix, suffix) = (&word[..start], &word[start + core.len()..]);
    let core_lower = core.to_lowercase();
    let cased = if ACRONYMS.contains(&core) {
        core.to_owned()
    } else if let Some(school) = school_words
        .iter()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|w| w.to_lowercase() == core_lower)
    {
        school.to_owned()
    } else if first {
        let mut chars = core_lower.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    } else {
        core_lower
    };
    format!("{}{cased}{}", prefix.to_lowercase(), suffix.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(street: &str, postcode: &str) -> AddressKey {
        AddressKey {
            street: street.into(),
            postcode: postcode.into(),
        }
    }

    #[test]
    fn fau_names_are_recognised_on_word_boundaries() {
        for name in [
            "BESTUM FAU",
            "FAU FAUSKANGER BARNE OG UNGDOMSKOLE",
            "FORELDRENES ARBEIDSUTVALG VED HOSLE SKOLE",
            "FORELDRERÅDETS ARBEIDSUTVALG TORSHOV SKOLE",
            "FORELDRERÅDET VED ÅSANE SKULE",
            "ARBEIDSUTVALET VED X",
        ] {
            assert!(is_fau_name(name), "{name}");
        }
        for name in ["FAUSKE IDRETTSLAG", "ASKERIS FAUNA", "SAMFUNNSHUS AS"] {
            assert!(!is_fau_name(name), "{name}");
        }
    }

    #[test]
    fn address_keys_normalise_street_names() {
        assert_eq!(
            address_keys(["Bispeveien 73"], Some("1362")),
            vec![key("bispevei 73", "1362")]
        );
        assert_eq!(
            address_keys(["Bispevegen 73"], Some("1362")),
            address_keys(["BISPEVEIEN 73"], Some("1362"))
        );
        assert_eq!(
            address_keys(["Storgata 61"], Some("1890")),
            address_keys(["Storgaten 61"], Some("1890"))
        );
        assert_eq!(
            address_keys(["Holgerslystveien 18 A"], Some("0280")),
            address_keys(["Holgerslystveien 18a"], Some("0280"))
        );
    }

    #[test]
    fn personal_and_postbox_lines_are_ignored() {
        // Section 2.5: 913 of 2,477 carry such a line, usually a parent's home address.
        assert_eq!(
            address_keys(["c/o Kari Nordmann", "Abbedissevegen 67"], Some("5314")),
            vec![key("abbedissevei 67", "5314")]
        );
        assert!(address_keys(["C/O Ola Nordmann 12"], Some("5314")).is_empty());
        assert!(address_keys(["v/ Ola Nordmann 12"], Some("5314")).is_empty());
        assert!(address_keys(["Postboks 264"], Some("1891")).is_empty());
        assert!(address_keys(["Pb 264"], Some("1891")).is_empty());
        assert!(address_keys(
            [
                "FAU v/ Bergenhus skole",
                "c/o Rakkestad kommune",
                "Postboks 264"
            ],
            Some("1891")
        )
        .is_empty());
    }

    #[test]
    fn a_line_needs_a_number_and_a_valid_postcode() {
        assert!(address_keys(["Skoleveien"], Some("1362")).is_empty());
        assert!(address_keys(["Bispeveien 73"], None).is_empty());
        assert!(address_keys(["Bispeveien 73"], Some("136")).is_empty());
    }

    #[test]
    fn keys_are_sorted_and_deduplicated() {
        let keys = address_keys(["Bispeveien 73", "Bispevegen 73", "Aveien 1"], Some("1362"));
        assert_eq!(
            keys,
            vec![key("avei 1", "1362"), key("bispevei 73", "1362")]
        );
    }

    #[test]
    fn name_cores_compare_an_fau_with_its_school() {
        assert_eq!(name_core("BESTUM FAU"), name_core("Bestum skole"));
        assert_eq!(
            name_core("FORELDRERÅDETS ARBEIDSUTVALG VED HOSLE SKOLE"),
            "hosle"
        );
        assert_eq!(
            name_core("FAU FAUSKANGER BARNE OG UNGDOMSKOLE"),
            name_core("Fauskanger barne- og ungdomsskule")
        );
        assert_ne!(name_core("Bekkestua skole"), name_core("Hosle skole"));
        assert_eq!(name_core("FAU"), "");
    }

    #[test]
    fn fau_names_are_suggested_in_readable_case() {
        assert_eq!(suggest_fau_name("BESTUM FAU", "Bestum skole"), "Bestum FAU");
        assert_eq!(
            suggest_fau_name(
                "FAU FAUSKANGER BARNE OG UNGDOMSKOLE",
                "Fauskanger barne- og ungdomsskule"
            ),
            "FAU Fauskanger barne og ungdomskole"
        );
        assert_eq!(
            suggest_fau_name(
                "FORELDRERÅDETS ARBEIDSUTVALG VED HOSLE SKOLE",
                "Hosle skole"
            ),
            "Foreldrerådets arbeidsutvalg ved Hosle skole"
        );
        assert_eq!(
            suggest_fau_name("FAU V/ ST. SVITHUN SKOLE", "St. Svithun skole"),
            "FAU v/ St. Svithun skole"
        );
        assert_eq!(
            suggest_fau_name("Bestum FAU", "Bestum skole"),
            "Bestum FAU",
            "mixed case is kept as registered"
        );
    }
}
