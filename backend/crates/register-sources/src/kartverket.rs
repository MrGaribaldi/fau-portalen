//! Kartverket's Administrative enheter API, `/fylkerkommuner` (§2.2): every current
//! municipality with its county and every official name, in one call.

use fau_domain::register::source::{MunicipalityRecord, OfficialName};
use serde::Deserialize;

use crate::error::{Source, SourceError, SourceErrorKind};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CountyDto {
    fylkesnummer: String,
    fylkesnavn: String,
    kommuner: Vec<MunicipalityDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MunicipalityDto {
    kommunenummer: String,
    kommunenavn: String,
    kommunenavn_norsk: String,
    #[serde(default)]
    gyldige_navn: Vec<NameDto>,
}

#[derive(Deserialize)]
struct NameDto {
    navn: Option<String>,
    prioritet: u8,
    sprak: Option<String>,
}

/// Kartverket's language names to BCP 47. A new language must be added here deliberately.
fn language(sprak: &str) -> Option<&'static str> {
    Some(match sprak {
        "Norwegian" => "no",
        "Northern Sami" => "se",
        "Southern Sami" => "sma",
        "Lule Sami" => "smj",
        "Kven Finnish" => "fkv",
        _ => return None,
    })
}

pub fn parse_municipalities(bytes: &[u8]) -> Result<Vec<MunicipalityRecord>, SourceError> {
    let counties: Vec<CountyDto> =
        serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Kartverket, &e))?;
    let mut out = Vec::new();
    for county in counties {
        for m in county.kommuner {
            let mut names = Vec::new();
            for n in m.gyldige_navn {
                // Kartverket pads the list with {navn: null, sprak: null}.
                let (Some(name), Some(sprak)) = (n.navn, n.sprak) else {
                    continue;
                };
                let language = language(&sprak).ok_or_else(|| {
                    SourceError::new(
                        Source::Kartverket,
                        SourceErrorKind::UnexpectedValue { field: "sprak" },
                    )
                })?;
                names.push(OfficialName {
                    name,
                    language: language.to_owned(),
                    priority: n.prioritet,
                });
            }
            names.sort_by_key(|n| n.priority);
            out.push(MunicipalityRecord {
                number: m.kommunenummer,
                norwegian_name: m.kommunenavn_norsk,
                official_name: m.kommunenavn,
                county_number: county.fylkesnummer.clone(),
                county_name: county.fylkesnavn.clone(),
                names,
            });
        }
    }
    Ok(out)
}
