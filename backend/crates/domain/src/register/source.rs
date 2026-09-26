//! Source records as the register's rules see them (docs/school-register-design.md §2). The
//! crate `fau-register-sources` fetches and parses the APIs, and hands over these plain values.
//! Brreg addresses exist only inside [`BrregAddress`], in memory, and its `Debug` never
//! prints them (planning-decisions, 24 September 2026).

use jiff::civil::Date;
use jiff::Timestamp;

use super::scope::{primary_nace, NsrScopeFacts};

/// One official name of a municipality, in one language (Kartverket `gyldigeNavn`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialName {
    pub name: String,
    /// BCP 47: `no`, `se`, `sma`, `smj`, `fkv`.
    pub language: String,
    pub priority: u8,
}

/// A current municipality from Kartverket (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MunicipalityRecord {
    pub number: String,
    /// `kommunenavnNorsk`, which the slug follows (D3).
    pub norwegian_name: String,
    /// `kommunenavn`, the priority-1 official name.
    pub official_name: String,
    pub county_number: String,
    pub county_name: String,
    pub names: Vec<OfficialName>,
}

/// One SSB Klass 131 code change (§2.2): a renumber, a rename, a split, a merger or a
/// boundary adjustment. The planner tells them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeChange {
    pub old_code: String,
    pub old_name: String,
    pub new_code: String,
    pub new_name: String,
    pub occurred_on: Date,
}

/// An institution's address in NSR. Public data, not personal (§8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NsrAddress {
    pub street: Option<String>,
    pub postcode: Option<String>,
    pub post_town: Option<String>,
}

/// NSR `Utgaattype` (any code but `A`) and `UtgaattDato`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrClosure {
    /// D, F, N, O, S or U (§2.1).
    pub code: String,
    pub at: Option<Timestamp>,
}

/// One NSR unit, from the detail endpoint (§2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrUnit {
    pub orgnr: String,
    pub name: String,
    pub municipality_number: String,
    pub is_school: bool,
    pub is_active: bool,
    pub is_primary_school: bool,
    pub is_private: bool,
    pub category_ids: Vec<String>,
    /// `(Prioritet, Kode)` pairs, in NSR's order.
    pub nace: Vec<(i64, String)>,
    pub grade_from: Option<i16>,
    pub grade_to: Option<i16>,
    /// BCP 47 from `Maalform`: `nb` or `nn`.
    pub language: Option<String>,
    pub website: Option<String>,
    pub visiting: NsrAddress,
    pub postal: NsrAddress,
    pub closure: Option<NsrClosure>,
    pub changed_at: Option<Timestamp>,
}

impl NsrUnit {
    /// The facts the scope filter reads (§2.3).
    pub fn scope_facts(&self) -> NsrScopeFacts {
        NsrScopeFacts {
            is_school: self.is_school,
            is_active: self.is_active,
            is_primary_school: self.is_primary_school,
            municipality_number: self.municipality_number.clone(),
            category_ids: self.category_ids.clone(),
            primary_nace: primary_nace(self.nace.iter().map(|(p, c)| (*p, c.as_str())))
                .map(str::to_owned),
        }
    }
}

/// A Brreg address. The lines are often a parent's home address, so they stay in memory and
/// `Debug` prints only how many there are.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct BrregAddress {
    pub lines: Vec<String>,
    pub postcode: Option<String>,
}

impl std::fmt::Debug for BrregAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrregAddress")
            .field("lines", &format_args!("<{} redacted>", self.lines.len()))
            .field("postcode", &self.postcode)
            .finish()
    }
}

/// A Brreg entity that looks like an FAU (§2.5). There are no e-mail or phone fields, on
/// purpose (Erik, 24 September 2026).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrregFau {
    pub orgnr: String,
    pub registered_name: String,
    pub organisation_form: String,
    pub municipality_number: Option<String>,
    pub business_address: BrregAddress,
    pub postal_address: BrregAddress,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::scope::{classify, OutOfScopeReason, ScopeDecision};

    fn unit() -> NsrUnit {
        NsrUnit {
            orgnr: "998516897".into(),
            name: "Lerberg skole og kompetansesenter".into(),
            municipality_number: "3314".into(),
            is_school: true,
            is_active: true,
            is_primary_school: true,
            is_private: false,
            category_ids: vec!["1".into(), "2".into(), "3".into(), "6".into(), "32".into()],
            nace: vec![(2, "85.310".into()), (1, "85.201".into())],
            grade_from: Some(8),
            grade_to: Some(10),
            language: Some("nb".into()),
            website: None,
            visiting: NsrAddress::default(),
            postal: NsrAddress::default(),
            closure: None,
            changed_at: None,
        }
    }

    #[test]
    fn scope_facts_take_the_priority_one_nace_code() {
        let facts = unit().scope_facts();
        assert_eq!(facts.primary_nace.as_deref(), Some("85.201"));
        assert_eq!(facts.municipality_number, "3314");
        assert_eq!(
            classify(&facts),
            ScopeDecision::InScope,
            "a combined school is in scope"
        );
        let vgs = NsrUnit {
            nace: vec![(1, "85.320".into())],
            ..unit()
        };
        assert_eq!(
            classify(&vgs.scope_facts()),
            ScopeDecision::OutOfScope(OutOfScopeReason::UpperSecondary)
        );
    }

    #[test]
    fn brreg_addresses_never_reach_debug_output() {
        let fau = BrregFau {
            orgnr: "913591100".into(),
            registered_name: "FAU STORØYA SKOLE".into(),
            organisation_form: "FLI".into(),
            municipality_number: Some("3201".into()),
            business_address: BrregAddress {
                lines: vec!["c/o Kari Nordmann".into(), "Oppdiktet vei 1".into()],
                postcode: Some("1364".into()),
            },
            postal_address: BrregAddress::default(),
        };
        let shown = format!("{fau:?}");
        assert!(
            !shown.contains("Nordmann") && !shown.contains("Oppdiktet"),
            "{shown}"
        );
        assert!(
            shown.contains("913591100") && shown.contains("1364"),
            "orgnr and postcode are not personal: {shown}"
        );
    }
}
