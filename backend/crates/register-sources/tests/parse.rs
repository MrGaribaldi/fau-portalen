//! Parsing the recorded fixtures (tests/fixtures/README.md). These are real API responses,
//! and the expected values below are taken from them.

use fau_domain::register::scope::{classify, OutOfScopeReason, ScopeDecision};
use fau_register_sources::{nsr, Source, SourceErrorKind};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn unit(orgnr: &str) -> fau_domain::register::source::NsrUnit {
    nsr::parse_unit(&fixture(&format!("nsr/enhet-{orgnr}.json"))).unwrap()
}

#[test]
fn nsr_list_page_carries_paging_and_slim_units() {
    let page = nsr::parse_list_page(&fixture("nsr/list-page-1-of-5.json")).unwrap();
    assert_eq!((page.page, page.page_count, page.total), (1, 3670, 18350));
    let orgnrs: Vec<_> = page.units.iter().map(|u| u.orgnr.as_str()).collect();
    assert_eq!(
        orgnrs,
        [
            "U99999999",
            "U90099999",
            "U90099021",
            "U90099020",
            "U90099018"
        ]
    );
    assert_eq!(page.units[0].municipality_number, "2599");
    assert!(!page.units[0].is_active);
}

#[test]
fn nsr_municipality_list_for_baerum() {
    let page = nsr::parse_list_page(&fixture("nsr/kommune-3201.json")).unwrap();
    assert_eq!(page.units.len(), 218);
    assert_eq!(
        page.units
            .iter()
            .filter(|u| u.is_active && u.is_primary_school)
            .count(),
        45
    );
}

#[test]
fn hosle_skole_in_full() {
    let u = unit("974552124");
    assert_eq!(u.name, "Hosle skole");
    assert_eq!(u.municipality_number, "3201");
    assert!(u.is_school && u.is_active && u.is_primary_school && !u.is_private);
    assert_eq!(u.category_ids, ["1", "3", "5", "32"]);
    assert_eq!(u.nace, [(1, "85.201".to_owned())]);
    assert_eq!((u.grade_from, u.grade_to), (Some(1), Some(7)));
    assert_eq!(u.language.as_deref(), Some("nb"));
    assert_eq!(u.website.as_deref(), Some("www.hosle.no"));
    assert_eq!(u.visiting.street.as_deref(), Some("Bispeveien 73"));
    assert_eq!(u.visiting.postcode.as_deref(), Some("1362"));
    assert_eq!(u.visiting.post_town.as_deref(), Some("HOSLE"));
    assert_eq!(u.closure, None, "Utgaattype A means not closed");
    assert_eq!(u.changed_at.unwrap().to_string(), "2026-09-13T01:05:43.46Z");
    assert_eq!(classify(&u.scope_facts()), ScopeDecision::InScope);
}

#[test]
fn every_fixture_unit_classifies_as_section_2_3_says() {
    use OutOfScopeReason::*;
    let cases = [
        ("974552124", ScopeDecision::InScope),              // ordinary
        ("990672938", ScopeDecision::InScope),              // private
        ("998516897", ScopeDecision::InScope),              // combined
        ("998666783", ScopeDecision::InScope),              // special, 85.202
        ("974795655", ScopeDecision::InScope),              // Svalbard, 2100
        ("998245508", ScopeDecision::InScope),              // Nynorsk
        ("998670799", ScopeDecision::InScope),              // no website
        ("974554682", ScopeDecision::InScope),              // municipality prefix
        ("933181995", ScopeDecision::InScope),              // Stange, new number
        ("975270920", ScopeDecision::OutOfScope(Inactive)), // Stange, old number
        ("999038182", ScopeDecision::OutOfScope(AdultEducation)),
        ("986779795", ScopeDecision::OutOfScope(UpperSecondary)),
        ("U90099017", ScopeDecision::OutOfScope(Abroad)),
    ];
    for (orgnr, expected) in cases {
        assert_eq!(classify(&unit(orgnr).scope_facts()), expected, "{orgnr}");
    }
}

#[test]
fn nsr_empty_strings_and_codes_become_absent_or_mapped() {
    assert_eq!(
        unit("998516897").website,
        None,
        "Lerberg sends an empty website"
    );
    assert_eq!(unit("998670799").website, None, "Halsa sends none");
    assert_eq!(unit("998245508").language.as_deref(), Some("nn"));
    assert_eq!(
        unit("U90099017").visiting.postcode,
        None,
        "abroad: empty postcode"
    );
}

#[test]
fn a_closed_unit_carries_its_reason_and_time() {
    let old = unit("975270920");
    assert!(!old.is_active);
    let closure = old.closure.expect("Slettet for sammenslåing");
    assert_eq!(closure.code, "F");
    assert_eq!(closure.at.unwrap().to_string(), "2024-08-25T01:15:10.91Z");
}

#[test]
fn a_missing_required_field_fails_loudly() {
    let mut v: serde_json::Value =
        serde_json::from_slice(&fixture("nsr/enhet-974552124.json")).unwrap();
    v.as_object_mut().unwrap().remove("Navn");
    let err = nsr::parse_unit(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert_eq!(err.source, Source::Nsr);
    match err.kind {
        SourceErrorKind::Parse { detail } => assert!(detail.contains("Navn"), "{detail}"),
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[test]
fn a_parse_error_never_quotes_the_input() {
    let mut v: serde_json::Value =
        serde_json::from_slice(&fixture("nsr/enhet-974552124.json")).unwrap();
    v["ErAktiv"] = serde_json::Value::String("SECRET-LOOKING-VALUE".into());
    let err = nsr::parse_unit(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert!(!format!("{err:?} {err}").contains("SECRET-LOOKING-VALUE"));
}

use fau_domain::register::source::OfficialName;
use fau_register_sources::{kartverket, ssb};

fn names(n: &[(&str, &str, u8)]) -> Vec<OfficialName> {
    n.iter()
        .map(|(name, lang, p)| OfficialName {
            name: (*name).into(),
            language: (*lang).into(),
            priority: *p,
        })
        .collect()
}

#[test]
fn kartverket_lists_every_current_municipality() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    assert_eq!(all.len(), 357);
    assert!(all
        .iter()
        .all(|m| m.number.len() == 4 && m.county_number.len() == 2));
    assert!(
        !all.iter().any(|m| m.number == "2100"),
        "Svalbard is not a municipality (§2.2)"
    );
}

#[test]
fn kaafjord_has_three_official_names() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    let k = all.iter().find(|m| m.number == "5540").unwrap();
    assert_eq!(k.norwegian_name, "Kåfjord");
    assert_eq!(k.official_name, "Gáivuotna");
    assert_eq!(
        (k.county_number.as_str(), k.county_name.as_str()),
        ("55", "Troms")
    );
    assert_eq!(
        k.names,
        names(&[
            ("Gáivuotna", "se", 1),
            ("Kåfjord", "no", 2),
            ("Kaivuono", "fkv", 3)
        ])
    );
}

#[test]
fn kartverket_padding_entries_are_skipped() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    let oslo = all.iter().find(|m| m.number == "0301").unwrap();
    assert_eq!(oslo.names, names(&[("Oslo", "no", 1)]));
    assert_eq!(oslo.norwegian_name, "Oslo");
}

#[test]
fn every_kartverket_language_is_mapped() {
    let all = kartverket::parse_municipalities(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    let mut langs: Vec<_> = all
        .iter()
        .flat_map(|m| m.names.iter().map(|n| n.language.as_str()))
        .collect();
    langs.sort();
    langs.dedup();
    assert_eq!(langs, ["fkv", "no", "se", "sma", "smj"]);
}

#[test]
fn an_unknown_kartverket_language_is_an_error() {
    let mut v: serde_json::Value =
        serde_json::from_slice(&fixture("kartverket/fylkerkommuner.json")).unwrap();
    v[0]["kommuner"][0]["gyldigeNavn"][0]["sprak"] = "Klingon".into();
    let err = kartverket::parse_municipalities(&serde_json::to_vec(&v).unwrap()).unwrap_err();
    assert_eq!(
        err.kind,
        SourceErrorKind::UnexpectedValue { field: "sprak" }
    );
}

#[test]
fn ssb_changes_of_2024_include_the_renumbering_and_the_split() {
    let c = ssb::parse_changes(&fixture("ssb/changes-2024.json")).unwrap();
    assert_eq!(c.len(), 118);
    let has = |old: &str, new: &str| c.iter().any(|x| x.old_code == old && x.new_code == new);
    assert!(has("3024", "3201"), "Bærum renumbered");
    assert!(
        has("1507", "1508") && has("1507", "1580"),
        "Ålesund split into Ålesund and Haram"
    );
    assert!(c.iter().all(|x| x.occurred_on.to_string() == "2024-01-01"));
    let rana = c.iter().find(|x| x.old_code == "1833").unwrap();
    assert_eq!(
        (rana.new_code.as_str(), rana.new_name.as_str()),
        ("1833", "Rana - Raane"),
        "a name-only change"
    );
}

#[test]
fn ssb_changes_of_2026_include_the_boundary_adjustment() {
    let c = ssb::parse_changes(&fixture("ssb/changes-2026.json")).unwrap();
    assert_eq!(c.len(), 6);
    let from_3118: Vec<_> = c
        .iter()
        .filter(|x| x.old_code == "3118")
        .map(|x| x.new_code.as_str())
        .collect();
    assert_eq!(
        from_3118,
        ["3118", "3207", "3216"],
        "3118 continues: a boundary adjustment, not a merger"
    );
}

use fau_register_sources::brreg;
use std::io::Write;

fn gzipped(path: &str) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&fixture(path)).unwrap();
    enc.finish().unwrap()
}

fn faus() -> (
    Vec<fau_domain::register::source::BrregFau>,
    brreg::BrregStats,
) {
    let mut out = Vec::new();
    let stats =
        brreg::for_each_fau(&gzipped("brreg/enheter-sample.json")[..], |f| out.push(f)).unwrap();
    (out, stats)
}

#[test]
fn only_fli_units_with_an_fau_name_are_kept() {
    let (faus, stats) = faus();
    assert_eq!(stats.units_seen, 9);
    assert_eq!(stats.faus, 8, "FAUSKE IDRETTSLAG ALPINT is not an FAU");
    assert!(!faus.iter().any(|f| f.orgnr == "988936871"));
    let hosle = faus.iter().find(|f| f.orgnr == "918316450").unwrap();
    assert_eq!(hosle.registered_name, "HOSLE FAU");
    assert_eq!(hosle.organisation_form, "FLI");
    assert_eq!(hosle.municipality_number.as_deref(), Some("3201"));
    assert_eq!(hosle.business_address.lines, ["Bispeveien 73"]);
    assert_eq!(hosle.business_address.postcode.as_deref(), Some("1362"));
    assert_eq!(hosle.postal_address, Default::default());
}

#[test]
fn contact_fields_are_never_read() {
    // 913591100 carries an invented e-mail and mobile (fixtures README). The value types have
    // no field for them, and nothing the parser returns may contain them.
    let (faus, _) = faus();
    let shown = format!("{faus:?}");
    for needle in ["example.invalid", "+4700000000", "Nordmann", "Oppdiktet"] {
        assert!(!shown.contains(needle), "{needle} leaked into {shown}");
    }
}

#[test]
fn a_truncated_file_is_an_error() {
    let full = fixture("brreg/enheter-sample.json");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&full[..full.len() / 2]).unwrap();
    let err = brreg::for_each_fau(&enc.finish().unwrap()[..], |_| {}).unwrap_err();
    assert_eq!(err.source, fau_register_sources::Source::Brreg);
}

#[test]
fn a_unit_without_a_name_fails_loudly() {
    let mut v: serde_json::Value =
        serde_json::from_slice(&fixture("brreg/enheter-sample.json")).unwrap();
    v[0].as_object_mut().unwrap().remove("navn");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&serde_json::to_vec(&v).unwrap()).unwrap();
    let err = brreg::for_each_fau(&enc.finish().unwrap()[..], |_| {}).unwrap_err();
    assert!(matches!(err.kind, SourceErrorKind::Parse { .. }));
}
