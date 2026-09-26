//! Trigram similarity, computed as PostgreSQL's pg_trgm computes `similarity()`, over the
//! §7 query form of each name. The planner runs in memory, so it cannot ask the database.

use std::collections::BTreeSet;

use crate::register::search::search_query;

/// §4.5: a new NSR unit this similar to a pending or verified submitted school in the same
/// municipality is held for review. "0.45 to start; tune it on real review outcomes".
pub const SUBMISSION_MATCH_THRESHOLD: f32 = 0.45;

/// §4.5 "the same or a very similar name": a new NSR unit this similar to a school closing in
/// the same run and municipality is its re-registration.
pub const REREGISTRATION_THRESHOLD: f32 = 0.6;

/// pg_trgm's `similarity(a, b)` over `search_query(a)` and `search_query(b)`: each word padded
/// as `"  " + word + " "`, the set of its 3-character windows, then |A ∩ B| / |A ∪ B|. Two
/// names with no trigrams at all are 0, as in pg_trgm.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (ta, tb) = (trigrams(&search_query(a)), trigrams(&search_query(b)));
    let common = ta.intersection(&tb).count();
    let union = ta.len() + tb.len() - common;
    if union == 0 {
        return 0.0;
    }
    common as f32 / union as f32
}

/// `search_query` output is ASCII `[a-z0-9]` words joined by single spaces, so byte windows
/// are character windows.
fn trigrams(query: &str) -> BTreeSet<[u8; 3]> {
    let mut set = BTreeSet::new();
    for word in query.split(' ').filter(|w| !w.is_empty()) {
        let padded = format!("  {word} ");
        for w in padded.as_bytes().windows(3) {
            set.insert([w[0], w[1], w[2]]);
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values are PostgreSQL 17's `select similarity(a, b)` with pg_trgm, run on the
    /// `search_query` forms of the names, 24 September 2026.
    #[test]
    fn matches_pg_trgm() {
        let cases: [(&str, &str, f32); 8] = [
            ("Stange ungdomsskole", "Stange ungdomskole", 0.857_142_87),
            ("Stange ungdomsskole", "Stange ungdomsskule", 0.739_130_44),
            ("Stange skole", "Stange ungdomsskole", 0.523_809_55),
            ("Hosle skole", "Hosle skule", 0.571_428_6),
            ("Hosle skole", "Bekkestua skole", 0.285_714_3),
            ("Fornebu", "Fornebu skole", 0.571_428_6),
            ("Fornebu skule", "Fornebu skole", 0.647_058_84),
            (
                "Lerberg skole",
                "Lerberg skole og kompetansesenter",
                0.411_764_7,
            ),
        ];
        for (a, b, expected) in cases {
            let got = similarity(a, b);
            assert!(
                (got - expected).abs() < 1e-6,
                "{a} / {b}: {got} != {expected}"
            );
            assert_eq!(similarity(b, a), got, "symmetric: {a} / {b}");
        }
    }

    #[test]
    fn hand_traced_fractions() {
        // "hosle skole": hosle gives "  h", " ho", hos, osl, sle, "le "; skole gives "  s",
        // " sk", sko, kol, ole and "le " again: 11 trigrams. "hosle skule" also has 11, and
        // they share 8, so 8 / (11 + 11 - 8) = 8 / 14.
        assert_eq!(similarity("Hosle skole", "Hosle skule"), 8.0 / 14.0);
        // "oksenoya" has 9 trigrams, "oksenoya skole" 15, sharing all 9: exactly 9 / 15.
        assert_eq!(similarity("Oksenøya", "Oksenøya skole"), 9.0 / 15.0);
    }

    #[test]
    fn the_thresholds_are_inclusive() {
        // 9 / 15 rounds to the same f32 as the literal 0.6.
        assert!(similarity("Oksenøya", "Oksenøya skole") >= REREGISTRATION_THRESHOLD);
        assert!(similarity("Fornebu", "Fornebu skole") >= SUBMISSION_MATCH_THRESHOLD);
        assert!(similarity("Fornebu", "Fornebu skole") < REREGISTRATION_THRESHOLD);
        assert!(
            similarity("Lerberg skole", "Lerberg skole og kompetansesenter")
                < SUBMISSION_MATCH_THRESHOLD
        );
    }

    #[test]
    fn names_are_compared_in_their_query_form() {
        assert_eq!(similarity("Bærum", "barum"), 1.0);
        assert_eq!(
            similarity("STANGE  Ungdomsskole", "stange ungdomsskole"),
            1.0
        );
    }

    #[test]
    fn nothing_to_compare_is_zero() {
        assert_eq!(similarity("", ""), 0.0);
        assert_eq!(similarity("?!", "Школа"), 0.0);
        assert_eq!(similarity("abc", ""), 0.0);
        assert_eq!(similarity("a", "b"), 0.0);
    }
}
