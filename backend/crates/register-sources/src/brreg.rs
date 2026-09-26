//! Brreg's Enhetsregisteret bulk file, `enheter/lastned`: a gzip JSON array of every main
//! unit, 1.18 million of them (§2.5). It is streamed one unit at a time, and only FAU-like
//! entities are kept.
//!
//! The DTO below declares no field for e-mail, phone, mobile, website or free text, so serde
//! never materialises them (Erik, 24 September 2026: contact data is fetched live by
//! outreach, never by the product). Address lines are kept only inside `BrregAddress`,
//! whose `Debug` redacts them.

use std::fmt;
use std::io::Read;

use fau_domain::register::brreg::is_fau_name;
use fau_domain::register::source::{BrregAddress, BrregFau};
use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::error::{Source, SourceError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BrregStats {
    pub units_seen: u64,
    pub faus: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnitDto {
    organisasjonsnummer: String,
    navn: String,
    organisasjonsform: FormDto,
    forretningsadresse: Option<AddressDto>,
    postadresse: Option<AddressDto>,
}

#[derive(Deserialize)]
struct FormDto {
    kode: String,
}

#[derive(Deserialize)]
struct AddressDto {
    // Brreg sends `"adresse": null` for some units, not just an absent field: `Option` accepts
    // both, and either way it means no address lines.
    #[serde(default)]
    adresse: Option<Vec<Option<String>>>,
    postnummer: Option<String>,
    kommunenummer: Option<String>,
}

fn address(a: Option<&AddressDto>) -> BrregAddress {
    match a {
        None => BrregAddress::default(),
        Some(a) => BrregAddress {
            lines: a
                .adresse
                .iter()
                .flatten()
                .flatten()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect(),
            postcode: a.postnummer.clone().filter(|p| !p.trim().is_empty()),
        },
    }
}

struct Each<'f, F: FnMut(BrregFau)> {
    f: &'f mut F,
    stats: &'f mut BrregStats,
}

impl<'de, F: FnMut(BrregFau)> DeserializeSeed<'de> for Each<'_, F> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de, F: FnMut(BrregFau)> Visitor<'de> for Each<'_, F> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of Enhetsregisteret units")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(u) = seq.next_element::<UnitDto>()? {
            self.stats.units_seen += 1;
            if u.organisasjonsform.kode != "FLI" || !is_fau_name(&u.navn) {
                continue;
            }
            self.stats.faus += 1;
            let municipality_number = u
                .forretningsadresse
                .as_ref()
                .and_then(|a| a.kommunenummer.clone())
                .filter(|n| !n.trim().is_empty());
            (self.f)(BrregFau {
                orgnr: u.organisasjonsnummer,
                registered_name: u.navn,
                organisation_form: u.organisasjonsform.kode,
                municipality_number,
                business_address: address(u.forretningsadresse.as_ref()),
                postal_address: address(u.postadresse.as_ref()),
            });
        }
        Ok(())
    }
}

/// Streams a gzipped Enhetsregisteret array, calling `f` for each FAU-like entity (FLI plus
/// an FAU word in the name). A truncated file or a unit missing a required field is an error,
/// never a silent partial result. On `Err`, `f` may already have been called with some FAU-er
/// from earlier in the stream: the caller must discard everything it collected rather than
/// treat the partial run as complete.
pub fn for_each_fau<R: Read>(
    gzipped: R,
    mut f: impl FnMut(BrregFau),
) -> Result<BrregStats, SourceError> {
    let reader = std::io::BufReader::new(flate2::read::GzDecoder::new(gzipped));
    let mut de = serde_json::Deserializer::from_reader(reader);
    let mut stats = BrregStats::default();
    Each {
        f: &mut f,
        stats: &mut stats,
    }
    .deserialize(&mut de)
    .and_then(|()| de.end())
    .map_err(|e| SourceError::parse(Source::Brreg, &e))?;
    Ok(stats)
}
