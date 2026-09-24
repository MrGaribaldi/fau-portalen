//! Slug minting and resolution (section 6; ADR-002 sections 3-4, amended by D4).

use super::text::{collapse, latin_fold, Letters};

/// Section 6, step 8.
pub const MAX_SLUG_LEN: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlugError {
    /// Nothing sluggable was left, e.g. a name in a non-Latin script only.
    Empty,
    /// A municipality number that is not four ASCII digits.
    InvalidMunicipalityNumber,
}

impl std::fmt::Display for SlugError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SlugError::Empty => "name has no sluggable characters",
            SlugError::InvalidMunicipalityNumber => "municipality number is not four digits",
        })
    }
}

impl std::error::Error for SlugError {}

/// What a path segment resolves to (ADR-002's resolution rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution<Id> {
    /// The current holder: serve it.
    Current(Id),
    /// A single former holder: 301 to its canonical path.
    Redirect(Id),
    /// Nobody, or several former holders: 404, never a guess.
    NotFound,
}

/// Section 6, steps 2-8, for a school's display name or a municipality's Norwegian name.
pub fn slugify(name: &str) -> Result<String, SlugError> {
    let slug = cap(collapse(&latin_fold(name, Letters::Transliterate), '-'));
    if slug.is_empty() {
        Err(SlugError::Empty)
    } else {
        Ok(slug)
    }
}

/// Cuts at the last hyphen within the cap, or hard at the cap if there is none. The
/// input is ASCII, so byte indices are character indices.
fn cap(slug: String) -> String {
    if slug.len() <= MAX_SLUG_LEN {
        return slug;
    }
    let head = &slug[..MAX_SLUG_LEN];
    if slug.as_bytes()[MAX_SLUG_LEN] == b'-' {
        return head.to_owned();
    }
    match head.rfind('-') {
        Some(i) => head[..i].to_owned(),
        None => head.to_owned(),
    }
}

/// `<kommunenr>-<navn>` (ADR-002 section 3), from the Norwegian name (D3).
pub fn municipality_slug(number: &str, norwegian_name: &str) -> Result<String, SlugError> {
    if number.len() != 4 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SlugError::InvalidMunicipalityNumber);
    }
    Ok(format!("{number}-{}", slugify(norwegian_name)?))
}

/// Section 6's collision rule within one municipality: the base slug, then with the
/// post town appended, then numbered from 2. `is_taken` must count a slug held in
/// history by a different, non-closed school as taken.
pub fn first_free_slug(
    base: &str,
    post_town: Option<&str>,
    is_taken: impl Fn(&str) -> bool,
) -> String {
    if !is_taken(base) {
        return base.to_owned();
    }
    let stem = match post_town.and_then(|t| slugify(t).ok()) {
        Some(town) => {
            let with_town = format!("{base}-{town}");
            if !is_taken(&with_town) {
                return with_town;
            }
            with_town
        }
        None => base.to_owned(),
    };
    (2u32..)
        .map(|n| format!("{stem}-{n}"))
        .find(|candidate| !is_taken(candidate))
        .expect("some numbered slug is free")
}

/// ADR-002's resolution: the current holder; else a single former holder; else none.
pub fn resolve<Id: Copy + PartialEq>(current: Option<Id>, historical: &[Id]) -> Resolution<Id> {
    if let Some(id) = current {
        return Resolution::Current(id);
    }
    match historical.split_first() {
        Some((first, rest)) if rest.iter().all(|h| h == first) => Resolution::Redirect(*first),
        _ => Resolution::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> String {
        slugify(name).unwrap()
    }

    #[test]
    fn section_six_examples_from_the_real_data() {
        assert_eq!(municipality_slug("3911", "Færder").unwrap(), "3911-faerder");
        assert_eq!(municipality_slug("1515", "Herøy").unwrap(), "1515-heroey");
        assert_eq!(municipality_slug("1818", "Herøy").unwrap(), "1818-heroey");
        assert_eq!(
            municipality_slug("5540", "Kåfjord").unwrap(),
            "5540-kaafjord"
        );
        assert_eq!(
            municipality_slug("5610", "Karasjok").unwrap(),
            "5610-karasjok"
        );
        assert_eq!(municipality_slug("3201", "Bærum").unwrap(), "3201-baerum");
        assert_eq!(municipality_slug("0301", "Oslo").unwrap(), "0301-oslo");
        assert_eq!(s("Grünerløkka skole"), "grunerloekka-skole");
        assert_eq!(s("St. Svithun skole"), "st-svithun-skole");
        assert_eq!(s("Deanu Sàmeskuvla"), "deanu-sameskuvla");
        assert_eq!(
            s("Máze Skuvla/Masi skole Máze skole"),
            "maze-skuvla-masi-skole-maze-skole"
        );
        assert_eq!(s("Hosle skole"), "hosle-skole");
    }

    #[test]
    fn same_number_different_era() {
        assert_eq!(municipality_slug("0716", "Våle").unwrap(), "0716-vaale");
        assert_eq!(municipality_slug("0716", "Re").unwrap(), "0716-re");
    }

    #[test]
    fn sami_names_fold_to_ascii() {
        assert_eq!(s("Gáivuotna"), "gaivuotna");
        assert_eq!(s("Kárášjohka"), "karasjohka");
        assert_eq!(s("Guovdageaidnu"), "guovdageaidnu");
        assert_eq!(s("Aarborte"), "aarborte");
        assert_eq!(s("Unjárga"), "unjarga");
        assert_eq!(s("Deatnu đŋŧ"), "deatnu-dnt");
        assert_eq!(s("Stïentje"), "stientje");
    }

    #[test]
    fn d4_letters_beyond_ae_oe_aa() {
        assert_eq!(s("Märta Östgård"), "maerta-oestgaard");
        assert_eq!(s("Straße"), "strasse");
        assert_eq!(s("Čáhcesuolu"), "cahcesuolu");
    }

    #[test]
    fn punctuation() {
        assert_eq!(
            s("Children's International School"),
            "childrens-international-school"
        );
        assert_eq!(s("Children's"), "childrens");
        assert_eq!(s("Fossen skole 1.-4. skole"), "fossen-skole-1-4-skole");
        assert_eq!(
            s("Viti skole avd Nordbyhagen Sone 1, 2 & 3"),
            "viti-skole-avd-nordbyhagen-sone-1-2-3"
        );
        assert_eq!(
            s("  --Elverum kommune - Ydalir skole--  "),
            "elverum-kommune-ydalir-skole"
        );
    }

    #[test]
    fn decomposed_input_is_composed_first() {
        // "å" as a + U+030A, as some sources send it.
        assert_eq!(s("Ka\u{030A}fjord"), "kaafjord");
    }

    #[test]
    fn the_cap_cuts_at_a_hyphen() {
        let long = "abcdefghij ".repeat(10); // 10 words of 10 letters
        let slug = s(&long);
        assert!(
            slug.len() <= MAX_SLUG_LEN,
            "{} > {MAX_SLUG_LEN}",
            slug.len()
        );
        assert_eq!(
            slug,
            ["abcdefghij"; 7].join("-"),
            "76 characters, cut before the eighth word"
        );
        assert!(!slug.ends_with('-'));
        let one_word = "a".repeat(100);
        assert_eq!(
            s(&one_word).len(),
            MAX_SLUG_LEN,
            "no hyphen to cut at: hard cut"
        );
    }

    #[test]
    fn slugging_a_slug_changes_nothing() {
        for name in [
            "Grünerløkka skole",
            "Máze Skuvla/Masi skole Máze skole",
            "Children's",
            &"abcdefghij ".repeat(10),
        ] {
            let once = s(name);
            assert_eq!(s(&once), once, "{name}");
        }
    }

    #[test]
    fn case_only_changes_give_the_same_slug() {
        assert_eq!(s("HOSLE SKOLE"), s("Hosle skole"));
    }

    #[test]
    fn nothing_sluggable_is_an_error() {
        assert_eq!(slugify("Школа"), Err(SlugError::Empty));
        assert_eq!(slugify(" - "), Err(SlugError::Empty));
        assert_eq!(
            municipality_slug("301", "Oslo"),
            Err(SlugError::InvalidMunicipalityNumber)
        );
        assert_eq!(
            municipality_slug("03O1", "Oslo"),
            Err(SlugError::InvalidMunicipalityNumber)
        );
    }

    #[test]
    fn municipality_segments_always_start_with_four_digits_and_a_hyphen() {
        // ADR-002: no register segment can collide with a root path or locale code.
        for (n, name) in [("0301", "Oslo"), ("2100", "Svalbard"), ("5501", "Tromsø")] {
            let seg = municipality_slug(n, name).unwrap();
            assert!(
                seg.len() > 5
                    && seg.as_bytes()[..4].iter().all(u8::is_ascii_digit)
                    && seg.as_bytes()[4] == b'-'
            );
        }
    }

    #[test]
    fn collisions_take_the_post_town_then_a_number() {
        let taken = ["hosle-skole", "hosle-skole-hosle", "hosle-skole-hosle-2"];
        let is_taken = |c: &str| taken.contains(&c);
        assert_eq!(
            first_free_slug("bekkestua-skole", Some("Hosle"), is_taken),
            "bekkestua-skole"
        );
        assert_eq!(
            first_free_slug("hosle-skole", Some("HOSLE"), is_taken),
            "hosle-skole-hosle-3"
        );
        let only_base = |c: &str| c == "hosle-skole";
        assert_eq!(
            first_free_slug("hosle-skole", Some("Hosle"), only_base),
            "hosle-skole-hosle"
        );
        assert_eq!(
            first_free_slug("hosle-skole", None, only_base),
            "hosle-skole-2"
        );
        assert_eq!(
            first_free_slug("hosle-skole", Some("Школа"), only_base),
            "hosle-skole-2"
        );
    }

    #[test]
    fn resolution_follows_adr_002() {
        assert_eq!(resolve(Some(1), &[2, 3]), Resolution::Current(1));
        assert_eq!(resolve(None, &[2]), Resolution::Redirect(2));
        assert_eq!(
            resolve(None, &[2, 2]),
            Resolution::Redirect(2),
            "one holder, two history rows"
        );
        assert_eq!(
            resolve(None, &[2, 3]),
            Resolution::NotFound,
            "never a guess"
        );
        assert_eq!(resolve::<u8>(None, &[]), Resolution::NotFound);
    }
}
