//! NSR v4 (docs/school-register-design.md §2.1): the paged list and the unit detail.

use fau_domain::register::source::{NsrAddress, NsrClosure, NsrUnit};
use jiff::Timestamp;
use serde::Deserialize;

use crate::error::{Source, SourceError, SourceErrorKind};

/// One page of `/v4/enheter` or `/v4/enheter/kommune/{nr}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrListPage {
    pub page: u32,
    pub page_count: u32,
    pub total: u32,
    pub units: Vec<NsrListItem>,
}

/// The slim list model: enough to decide which details to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsrListItem {
    pub orgnr: String,
    pub name: String,
    pub municipality_number: String,
    pub is_active: bool,
    pub is_school: bool,
    pub is_primary_school: bool,
    pub changed_at: Option<Timestamp>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListDto {
    #[serde(default)]
    sidenummer: Option<u32>,
    #[serde(default)]
    antall_sider: Option<u32>,
    #[serde(default)]
    totalt_antall_enheter: Option<u32>,
    enhet_liste: Vec<ListItemDto>,
}

/// The municipality list (`/v4/enheter/kommune/{nr}`), unlike `/v4/enheter`, carries no paging
/// fields at all, so their absence there is normal and must not be an error.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PagedListDto {
    sidenummer: u32,
    antall_sider: u32,
    totalt_antall_enheter: u32,
    enhet_liste: Vec<ListItemDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListItemDto {
    organisasjonsnummer: String,
    navn: String,
    kommunenummer: String,
    er_aktiv: bool,
    er_skole: bool,
    er_grunnskole: bool,
    dato_endret: Option<String>,
}

// `Option<…>` fields below (Maalform, Utgaattype, the addresses, and so on) are optional
// attributes, which serde treats as absent when missing; that is accepted because they do not
// decide scope, unlike ErPrivatskole, Skolekategorier and Naeringskoder just below.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UnitDto {
    organisasjonsnummer: String,
    navn: String,
    kommune: KommuneDto,
    er_aktiv: bool,
    er_skole: bool,
    er_grunnskole: bool,
    er_privatskole: bool,
    skolekategorier: Vec<IdDto>,
    naeringskoder: Vec<NaceDto>,
    #[serde(rename = "SkoletrinnGSFra")]
    skoletrinn_gs_fra: Option<i16>,
    #[serde(rename = "SkoletrinnGSTil")]
    skoletrinn_gs_til: Option<i16>,
    maalform: Option<IdDto>,
    internettadresse: Option<String>,
    beliggenhetsadresse: Option<AddressDto>,
    postadresse: Option<AddressDto>,
    utgaattype: Option<IdDto>,
    utgaatt_dato: Option<String>,
    dato_endret: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct KommuneDto {
    kommunenummer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IdDto {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NaceDto {
    prioritet: i64,
    kode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AddressDto {
    adresse: Option<String>,
    postnummer: Option<String>,
    poststed: Option<String>,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn timestamp(s: Option<String>, field: &'static str) -> Result<Option<Timestamp>, SourceError> {
    match non_empty(s) {
        None => Ok(None),
        Some(s) => s
            .parse::<Timestamp>()
            .map(Some)
            .map_err(|_| SourceError::new(Source::Nsr, SourceErrorKind::UnexpectedValue { field })),
    }
}

fn address(a: Option<AddressDto>) -> NsrAddress {
    match a {
        None => NsrAddress::default(),
        Some(a) => NsrAddress {
            street: non_empty(a.adresse),
            postcode: non_empty(a.postnummer),
            post_town: non_empty(a.poststed),
        },
    }
}

fn list_items(items: Vec<ListItemDto>) -> Result<Vec<NsrListItem>, SourceError> {
    items
        .into_iter()
        .map(|u| {
            Ok(NsrListItem {
                orgnr: u.organisasjonsnummer,
                name: u.navn,
                municipality_number: u.kommunenummer,
                is_active: u.er_aktiv,
                is_school: u.er_skole,
                is_primary_school: u.er_grunnskole,
                changed_at: timestamp(u.dato_endret, "DatoEndret")?,
            })
        })
        .collect()
}

/// Parses `/v4/enheter/kommune/{nr}`, which carries no paging fields at all (unlike
/// `/v4/enheter`): a municipality's whole list comes back in one `EnhetListe`, so `Sidenummer`,
/// `AntallSider` and `TotaltAntallEnheter` are simply absent, not merely unpopulated, and
/// defaulting them here is correct rather than a leniency.
pub fn parse_list_page(bytes: &[u8]) -> Result<NsrListPage, SourceError> {
    let dto: ListDto =
        serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Nsr, &e))?;
    let units = list_items(dto.enhet_liste)?;
    let total = dto.totalt_antall_enheter.unwrap_or(units.len() as u32);
    Ok(NsrListPage {
        page: dto.sidenummer.unwrap_or(1),
        page_count: dto.antall_sider.unwrap_or(1),
        total,
        units,
    })
}

/// Parses one page of `/v4/enheter`, where `Sidenummer`, `AntallSider` and
/// `TotaltAntallEnheter` are REQUIRED: paging must fail loudly on a missing field, never fall
/// back to a guess that could hide a truncated run (§5.3).
pub fn parse_paged_list_page(bytes: &[u8]) -> Result<NsrListPage, SourceError> {
    let dto: PagedListDto =
        serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Nsr, &e))?;
    let units = list_items(dto.enhet_liste)?;
    Ok(NsrListPage {
        page: dto.sidenummer,
        page_count: dto.antall_sider,
        total: dto.totalt_antall_enheter,
        units,
    })
}

pub fn parse_unit(bytes: &[u8]) -> Result<NsrUnit, SourceError> {
    let d: UnitDto =
        serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Nsr, &e))?;
    let language = match d.maalform.map(|m| m.id).as_deref() {
        Some("B") => Some("nb".to_owned()),
        Some("N") => Some("nn".to_owned()),
        _ => None,
    };
    let closure = match d.utgaattype.map(|t| t.id) {
        Some(code) if code != "A" => Some(NsrClosure {
            code,
            at: timestamp(d.utgaatt_dato, "UtgaattDato")?,
        }),
        _ => None,
    };
    Ok(NsrUnit {
        orgnr: d.organisasjonsnummer,
        name: d.navn,
        municipality_number: d.kommune.kommunenummer,
        is_school: d.er_skole,
        is_active: d.er_aktiv,
        is_primary_school: d.er_grunnskole,
        is_private: d.er_privatskole,
        category_ids: d.skolekategorier.into_iter().map(|c| c.id).collect(),
        nace: d
            .naeringskoder
            .into_iter()
            .map(|n| (n.prioritet, n.kode))
            .collect(),
        grade_from: d.skoletrinn_gs_fra,
        grade_to: d.skoletrinn_gs_til,
        language,
        website: non_empty(d.internettadresse),
        visiting: address(d.beliggenhetsadresse),
        postal: address(d.postadresse),
        closure,
        changed_at: timestamp(d.dato_endret, "DatoEndret")?,
    })
}
