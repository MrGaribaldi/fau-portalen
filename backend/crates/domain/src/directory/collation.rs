//! Locale-aware ordering of names (groups design §4.3, #3439). Names are encrypted, so the
//! database cannot sort them: the backend sorts after decrypting, inside the session.
//!
//! Byte order is wrong even for Bokmål (it puts Å before Æ and Ø), and #3439 rules out any
//! assumption of Latin collation or of two locales. So ordering is the Unicode Collation
//! Algorithm with CLDR's per-locale tailoring, through ICU4X (`icu_collator`), the Unicode
//! Consortium's own library. It is already in the dependency tree through `url`'s IDNA
//! support, so it adds one crate, not an ecosystem.
//!
//! **Data gap, recorded rather than hidden:** ICU4X's compiled data carries CLDR's tailoring
//! for `nb` and `nn` but not for the Sámi languages (`se`, `sma`, `smj`), which fall back to
//! the root order. Adding a Sámi locale therefore means generating ICU4X data that includes
//! it; the constructor below is the one place that changes.

use std::cmp::Ordering;
use std::fmt;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale_core::{locale, Locale};

/// The default and fallback locale (#3439).
pub const DEFAULT_LOCALE: &str = "nb-NO";

pub struct NameCollator {
    collator: CollatorBorrowed<'static>,
}

impl fmt::Debug for NameCollator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NameCollator")
    }
}

impl NameCollator {
    /// A collator for a BCP 47 tag such as `nb-NO`, `nn-NO` or `en`. A tag that does not
    /// parse falls back to [`DEFAULT_LOCALE`]; a well-formed tag without its own data falls
    /// back along CLDR's chain to the root order.
    pub fn for_locale(tag: &str) -> Self {
        let locale: Locale = tag.parse().unwrap_or(locale!("nb-NO"));
        let collator = Collator::try_new(locale.into(), CollatorOptions::default())
            .or_else(|_| Collator::try_new(locale!("nb-NO").into(), CollatorOptions::default()))
            .expect("ICU4X compiled data covers nb-NO");
        Self { collator }
    }

    pub fn compare(&self, a: &str, b: &str) -> Ordering {
        self.collator.compare(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(tag: &str, names: &[&'static str]) -> Vec<&'static str> {
        let c = NameCollator::for_locale(tag);
        let mut v = names.to_vec();
        v.sort_by(|a, b| c.compare(a, b));
        v
    }

    const NAMES: [&str; 6] = ["Åse", "Øvre", "Ærlig", "Zakariassen", "Berit", "Anders"];

    #[test]
    fn bokmaal_puts_ae_oe_aa_after_z() {
        assert_eq!(
            sorted("nb-NO", &NAMES),
            ["Anders", "Berit", "Zakariassen", "Ærlig", "Øvre", "Åse"]
        );
        assert_eq!(sorted("nn-NO", &NAMES), sorted("nb-NO", &NAMES));
    }

    #[test]
    fn it_is_not_byte_order() {
        let mut bytes = NAMES.to_vec();
        bytes.sort_unstable();
        assert_eq!(
            bytes,
            ["Anders", "Berit", "Zakariassen", "Åse", "Ærlig", "Øvre"]
        );
        assert_ne!(sorted("nb-NO", &NAMES), bytes);
    }

    #[test]
    fn another_locale_orders_differently() {
        // English folds the letters into A and O: a third order, from the same names.
        assert_eq!(
            sorted("en", &NAMES),
            ["Ærlig", "Anders", "Åse", "Berit", "Øvre", "Zakariassen"]
        );
    }

    #[test]
    fn case_does_not_split_a_name_from_its_neighbours() {
        assert_eq!(
            sorted("nb-NO", &["bjørn", "Anne", "Bjørg", "anders"]),
            ["anders", "Anne", "Bjørg", "bjørn"]
        );
    }

    #[test]
    fn an_unparsable_tag_falls_back_to_bokmaal() {
        assert_eq!(sorted("not a tag!", &NAMES), sorted("nb-NO", &NAMES));
        assert_eq!(sorted("", &NAMES), sorted("nb-NO", &NAMES));
    }
}
