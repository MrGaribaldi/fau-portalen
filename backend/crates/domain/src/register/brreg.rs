//! Brreg FAU entities (sections 2.5 and 4.6, D1). Addresses pass through these
//! functions in memory only; nothing here is ever stored (ruling, 24 September 2026).
//! A `c/o`/`v/` line never yields a plain [`address_keys`] key, only a
//! [`care_of_keys`] one, and only when it names the candidate school (Erik, 24
//! September 2026).

use super::search::search_query;

/// Lossy-folded words that mark a Brreg entity as an FAU, matched as whole words.
/// A samarbeidsutvalg is a different statutory body (section 2.5); a foreldreutvalg
/// is not a defined FAU term either, so neither is a marker here.
const FAU_WORDS: &[&str] = &[
    "fau",
    "arbeidsutvalg",
    "arbeidsutval",
    "arbeidsutvalet",
    "arbeidsutvalget",
    "foreldrerad",
    "foreldreradet",
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

/// Words that classify a name-core match as a school, so `care_of_keys` can tell
/// "Hosle skule" names the school "Hosle skole" (Erik, 24 September 2026).
const SCHOOL_WORDS: &[&str] = &[
    "skole",
    "skolen",
    "skule",
    "skulen",
    "barneskole",
    "barneskule",
    "ungdomsskole",
    "ungdomsskule",
    "oppvekstsenter",
];

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

/// Every usable address line as a key: lines naming a person (`c/o`, `v/`, wherever the
/// marker sits) or a post box are skipped, a line must carry a number, and the postcode
/// must be four digits. A marker line never yields a key here even if it happens to
/// name a school -- that is [`care_of_keys`]'s job, since it alone knows which school
/// is the candidate.
pub fn address_keys<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    postcode: Option<&str>,
) -> Vec<AddressKey> {
    let Some(postcode) = valid_postcode(postcode) else {
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

/// Keys from `c/o` and `v/` lines that name `school_name` (Erik, 24 September 2026):
/// "c/o Hosle skole, Bispeveien 73" counts for Hosle skole, but a line naming a person
/// or another school never does. Each key counts only towards that school, so the
/// matcher calls this once per candidate school sharing the postcode.
pub fn care_of_keys<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    postcode: Option<&str>,
    school_name: &str,
) -> Vec<AddressKey> {
    let Some(postcode) = valid_postcode(postcode) else {
        return Vec::new();
    };
    let school_lossy = fold_words(school_name);
    let school_core: Vec<String> = name_core(school_name)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    let mut keys: Vec<AddressKey> = lines
        .into_iter()
        .filter_map(|line| care_of_line(line, &school_lossy, &school_core))
        .map(|street| AddressKey {
            street,
            postcode: postcode.to_owned(),
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// The four-digit postcode `address_keys` and `care_of_keys` both require, trimmed.
fn valid_postcode(postcode: Option<&str>) -> Option<&str> {
    postcode
        .map(str::trim)
        .filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// A line's words, lossy-folded and space-split.
fn fold_words(line: &str) -> Vec<String> {
    search_query(line)
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

fn street_line(line: &str) -> Option<String> {
    let words = fold_words(line);
    if is_personal(&words) {
        return None;
    }
    normalise_street_words(&words)
}

/// A marker line's name part -- the words after the marker, up to (excluding) the
/// candidate school, if it is one there -- and the words after that match are the
/// street, normalised exactly like a plain [`street_line`].
fn care_of_line(line: &str, school_lossy: &[String], school_core: &[String]) -> Option<String> {
    let words = fold_words(line);
    let marker_end = marker_end(&words)?;
    let name_part_end = first_digit_index(&words).unwrap_or(words.len());
    if marker_end > name_part_end {
        return None;
    }
    let matched = school_match_end(&words[marker_end..name_part_end], school_lossy, school_core)?;
    normalise_street_words(&words[marker_end + matched..])
}

/// A line is personal (controller ruling) if the pair `c`, `o` appears anywhere among
/// its words, or a word `v` appears before the first word that carries a digit. A
/// street abbreviated `V.` folds the same way as `v/`, so it is treated the same way:
/// privacy wins over recall.
fn is_personal(words: &[String]) -> bool {
    marker_end(words).is_some()
}

/// Where a personal marker ends, if `words` has one: right after the `o` of a `c`,
/// `o` pair found anywhere, or right after the last `v` word before the first word
/// that carries a digit.
fn marker_end(words: &[String]) -> Option<usize> {
    if let Some(i) = words.windows(2).position(|w| w[0] == "c" && w[1] == "o") {
        return Some(i + 2);
    }
    let first_digit = first_digit_index(words)?;
    words[..first_digit]
        .iter()
        .rposition(|w| w == "v")
        .map(|i| i + 1)
}

fn first_digit_index(words: &[String]) -> Option<usize> {
    words
        .iter()
        .position(|w| w.bytes().any(|b| b.is_ascii_digit()))
}

/// The index right after `needle` where it occurs contiguously in `haystack`, using
/// the first (leftmost) occurrence.
fn find_sequence(haystack: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|start| start + needle.len())
}

/// Whether `name_part` names the school, and if so, the index (relative to
/// `name_part`) right after the matched name: the lossy school name in full, or its
/// core immediately followed by a school word (both are then part of the match, so
/// neither reaches the street part).
fn school_match_end(
    name_part: &[String],
    school_lossy: &[String],
    school_core: &[String],
) -> Option<usize> {
    if let Some(end) = find_sequence(name_part, school_lossy) {
        return Some(end);
    }
    let core_end = find_sequence(name_part, school_core)?;
    let next = name_part.get(core_end)?;
    SCHOOL_WORDS
        .contains(&next.as_str())
        .then_some(core_end + 1)
}

/// A post box, by an exact word or the adjacent pair `p`, `b` (e.g. "P.b. 264").
fn is_postbox(words: &[String]) -> bool {
    words
        .iter()
        .any(|w| w == "postboks" || w == "pb" || w == "boks")
        || words.windows(2).any(|w| w[0] == "p" && w[1] == "b")
}

/// A street line's words, once any personal or post-box check has already passed:
/// canonicalised, with a trailing house-letter merged, requiring a digit and no post
/// box.
fn normalise_street_words(words: &[String]) -> Option<String> {
    if is_postbox(words) || !words.iter().any(|w| w.bytes().any(|b| b.is_ascii_digit())) {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        // "18 a" and "18a" are the same house.
        if word.len() == 1 && word.as_bytes()[0].is_ascii_lowercase() {
            if let Some(prev) = out.last_mut() {
                if prev.bytes().all(|b| b.is_ascii_digit()) {
                    prev.push_str(word);
                    continue;
                }
            }
        }
        out.push(street_word(word));
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

/// Recases a word run by run: the non-alphanumeric characters between runs (and at
/// the word's ends) are kept exactly as they are, and each alphanumeric run is cased
/// on its own, so an acronym embedded in a compound token (`FAU-ET`, `SFO/FAU`) is
/// recognised independently of its neighbours. The word's first-run gets the
/// first-word capitalisation; every other run follows the "not first" rules.
fn recase(word: &str, first: bool, school_words: &[&str]) -> String {
    let mut out = String::with_capacity(word.len());
    let mut is_first_run = true;
    let mut rest = word;
    while !rest.is_empty() {
        match rest.find(char::is_alphanumeric) {
            None => {
                out.push_str(rest);
                break;
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                rest = &rest[start..];
                let end = rest
                    .find(|c: char| !c.is_alphanumeric())
                    .unwrap_or(rest.len());
                let (run, remainder) = rest.split_at(end);
                out.push_str(&recase_run(run, first && is_first_run, school_words));
                is_first_run = false;
                rest = remainder;
            }
        }
    }
    out
}

/// Cases one alphanumeric run: an acronym is kept verbatim, a run that matches a
/// school-name word (case-insensitively) takes that word's own spelling, the run
/// gets its own first letter capitalised when it is the word's first run and the
/// registered name's first word, and otherwise it is lowercased.
fn recase_run(run: &str, first_run: bool, school_words: &[&str]) -> String {
    let run_lower = run.to_lowercase();
    if ACRONYMS.contains(&run) {
        run.to_owned()
    } else if let Some(school) = school_words
        .iter()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|w| w.to_lowercase() == run_lower)
    {
        school.to_owned()
    } else if first_run {
        let mut chars = run_lower.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    } else {
        run_lower
    }
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
        for name in [
            "FAUSKE IDRETTSLAG",
            "ASKERIS FAUNA",
            "SAMFUNNSHUS AS",
            "SAMARBEIDSUTVALGET VED X SKOLE",
            "SAMARBEIDSUTVALG VED X SKOLE",
            "FORELDREUTVALGET I X KOMMUNE",
        ] {
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
        assert!(address_keys(["P.b. 264"], Some("1891")).is_empty());
    }

    #[test]
    fn personal_markers_are_found_wherever_they_sit() {
        assert!(address_keys(["FAU v/ Kari Nordmann, Hjemveien 12"], Some("5314")).is_empty());
        assert!(address_keys(["Skolen c/o Kari Nordmann 12"], Some("5314")).is_empty());
        assert_eq!(
            address_keys(["Colletts gate 3"], Some("0169")),
            vec![key("colletts gate 3", "0169")],
            "\"co\" is not \"c o\""
        );
        // A street abbreviated "V." is indistinguishable from "v/" after folding;
        // privacy wins.
        assert!(address_keys(["V. Slottsgate 2"], Some("0157")).is_empty());
    }

    #[test]
    fn a_line_needs_a_number_and_a_valid_postcode() {
        assert!(address_keys(["Skoleveien"], Some("1362")).is_empty());
        assert!(address_keys(["Bispeveien 73"], None).is_empty());
        assert!(address_keys(["Bispeveien 73"], Some("136")).is_empty());
    }

    #[test]
    fn the_postcode_is_trimmed_before_validation() {
        assert_eq!(
            address_keys(["Bispeveien 73"], Some(" 1362 ")),
            vec![key("bispevei 73", "1362")]
        );
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

    #[test]
    fn acronyms_inside_compound_tokens_are_recased_by_run() {
        assert_eq!(
            suggest_fau_name("FAU-ET VED BESTUM SKOLE", "Bestum skole"),
            "FAU-et ved Bestum skole"
        );
        assert_eq!(
            suggest_fau_name("BESTUM SKOLE SFO/FAU", "Bestum skole"),
            "Bestum skole SFO/FAU"
        );
        assert_eq!(suggest_fau_name("ÅSANE FAU", "Åsane skole"), "Åsane FAU");
    }

    #[test]
    fn care_of_keys_only_count_a_marker_line_that_names_the_school() {
        assert_eq!(
            care_of_keys(
                ["c/o Hosle skole, Bispeveien 73"],
                Some("1362"),
                "Hosle skole"
            ),
            vec![key("bispevei 73", "1362")]
        );
        assert_eq!(
            care_of_keys(
                ["v/ Hosle skole Bispeveien 73"],
                Some("1362"),
                "Hosle skole"
            ),
            vec![key("bispevei 73", "1362")]
        );
        assert!(care_of_keys(
            ["c/o Hosle skole, Bispeveien 73"],
            Some("1362"),
            "Bekkestua skole"
        )
        .is_empty());
        assert!(care_of_keys(
            ["c/o Kari Nordmann, Bispeveien 73"],
            Some("1362"),
            "Hosle skole"
        )
        .is_empty());
        assert!(care_of_keys(
            ["FAU v/ Bergenhus skole", "Storgata 61"],
            Some("1890"),
            "Bergenhus skole"
        )
        .is_empty());
        assert_eq!(
            care_of_keys(
                ["c/o Hosle skule Bispeveien 73"],
                Some("1362"),
                "Hosle skole"
            ),
            vec![key("bispevei 73", "1362")],
            "core plus a school word"
        );
        assert!(
            care_of_keys(["Bispeveien 73"], Some("1362"), "Hosle skole").is_empty(),
            "not a marker line"
        );
        assert!(address_keys(["c/o Hosle skole, Bispeveien 73"], Some("1362")).is_empty());
    }
}
