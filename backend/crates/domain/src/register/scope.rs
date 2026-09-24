//! Which NSR units can have an FAU (section 2.3, decision D2).

/// The NSR fields the filter reads. `primary_nace` is the `Naeringskoder` entry with
/// `Prioritet` 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrScopeFacts {
    pub is_school: bool,
    pub is_active: bool,
    pub is_primary_school: bool,
    pub municipality_number: String,
    pub category_ids: Vec<String>,
    pub primary_nace: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutOfScopeReason {
    NotASchool,
    Inactive,
    NotPrimarySchool,
    /// Norwegian schools abroad, pseudo-municipality 2599.
    Abroad,
    /// Category 10 or 25, or primary NACE 85.593.
    AdultEducation,
    /// Primary NACE 85.3xx, e.g. hospital schools such as Viti skole.
    UpperSecondary,
}

impl OutOfScopeReason {
    /// Stored in `register_source_records.scope_reason`.
    pub fn code(self) -> &'static str {
        match self {
            OutOfScopeReason::NotASchool => "not_a_school",
            OutOfScopeReason::Inactive => "inactive",
            OutOfScopeReason::NotPrimarySchool => "not_grunnskole",
            OutOfScopeReason::Abroad => "abroad",
            OutOfScopeReason::AdultEducation => "adult_education",
            OutOfScopeReason::UpperSecondary => "upper_secondary",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeDecision {
    InScope,
    OutOfScope(OutOfScopeReason),
}

impl ScopeDecision {
    pub fn code(self) -> &'static str {
        match self {
            ScopeDecision::InScope => "in_scope",
            ScopeDecision::OutOfScope(reason) => reason.code(),
        }
    }
}

/// Section 2.3's filter, checked in a fixed order so each unit gets one reason.
pub fn classify(facts: &NsrScopeFacts) -> ScopeDecision {
    use OutOfScopeReason::*;
    let out = ScopeDecision::OutOfScope;
    if !facts.is_school {
        return out(NotASchool);
    }
    if !facts.is_active {
        return out(Inactive);
    }
    if !facts.is_primary_school {
        return out(NotPrimarySchool);
    }
    if facts.municipality_number == "2599" {
        return out(Abroad);
    }
    if facts.category_ids.iter().any(|c| c == "10" || c == "25") {
        return out(AdultEducation);
    }
    match facts.primary_nace.as_deref() {
        Some("85.593") => out(AdultEducation),
        Some(code) if code.starts_with("85.3") => out(UpperSecondary),
        _ => ScopeDecision::InScope,
    }
}

/// `coalesce(scope_override, in_scope)`: an operator's decision wins (D2).
pub fn effective_in_scope(classified_in_scope: bool, operator_override: Option<bool>) -> bool {
    operator_override.unwrap_or(classified_in_scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hosle skole, 974552124, as NSR returned it on 24 September 2026.
    fn hosle() -> NsrScopeFacts {
        NsrScopeFacts {
            is_school: true,
            is_active: true,
            is_primary_school: true,
            municipality_number: "3201".into(),
            category_ids: vec!["1".into(), "3".into(), "5".into(), "32".into()],
            primary_nace: Some("85.201".into()),
        }
    }

    #[test]
    fn an_ordinary_grunnskole_is_in_scope() {
        assert_eq!(classify(&hosle()), ScopeDecision::InScope);
    }

    #[test]
    fn combined_and_special_schools_are_in() {
        // A combined school's secondary NACE is 85.310; only the primary code counts.
        let combined = hosle();
        assert_eq!(classify(&combined), ScopeDecision::InScope);
        let special = NsrScopeFacts {
            primary_nace: Some("85.202".into()),
            ..hosle()
        };
        assert_eq!(classify(&special), ScopeDecision::InScope);
    }

    #[test]
    fn exclusions_carry_their_reason() {
        use OutOfScopeReason::*;
        let cases = [
            (
                NsrScopeFacts {
                    is_school: false,
                    ..hosle()
                },
                NotASchool,
            ),
            (
                NsrScopeFacts {
                    is_active: false,
                    ..hosle()
                },
                Inactive,
            ),
            (
                NsrScopeFacts {
                    is_primary_school: false,
                    ..hosle()
                },
                NotPrimarySchool,
            ),
            (
                NsrScopeFacts {
                    municipality_number: "2599".into(),
                    ..hosle()
                },
                Abroad,
            ),
            (
                NsrScopeFacts {
                    category_ids: vec!["10".into()],
                    ..hosle()
                },
                AdultEducation,
            ),
            (
                NsrScopeFacts {
                    category_ids: vec!["25".into()],
                    ..hosle()
                },
                AdultEducation,
            ),
            (
                NsrScopeFacts {
                    primary_nace: Some("85.593".into()),
                    ..hosle()
                },
                AdultEducation,
            ),
            (
                NsrScopeFacts {
                    primary_nace: Some("85.320".into()),
                    ..hosle()
                },
                UpperSecondary,
            ),
        ];
        for (facts, reason) in cases {
            assert_eq!(
                classify(&facts),
                ScopeDecision::OutOfScope(reason),
                "{reason:?}"
            );
        }
    }

    #[test]
    fn reason_codes_are_stable() {
        use OutOfScopeReason::*;
        let codes: Vec<_> = [
            NotASchool,
            Inactive,
            NotPrimarySchool,
            Abroad,
            AdultEducation,
            UpperSecondary,
        ]
        .map(OutOfScopeReason::code)
        .into();
        assert_eq!(
            codes,
            [
                "not_a_school",
                "inactive",
                "not_grunnskole",
                "abroad",
                "adult_education",
                "upper_secondary"
            ]
        );
        assert_eq!(ScopeDecision::InScope.code(), "in_scope");
    }

    #[test]
    fn the_operator_override_wins() {
        assert!(effective_in_scope(true, None));
        assert!(
            !effective_in_scope(true, Some(false)),
            "Porsgrunn kommune Vikarer, marked out"
        );
        assert!(effective_in_scope(false, Some(true)));
        assert!(!effective_in_scope(false, None));
    }
}
