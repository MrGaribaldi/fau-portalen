//! SSB Klass classification 131, `/changes?from=&to=` (§2.2): municipality code changes.

use fau_domain::register::source::CodeChange;
use jiff::civil::Date;
use serde::Deserialize;

use crate::error::{Source, SourceError, SourceErrorKind};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangesDto {
    code_changes: Vec<ChangeDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeDto {
    old_code: String,
    #[serde(default)]
    old_name: String,
    new_code: String,
    #[serde(default)]
    new_name: String,
    change_occurred: String,
}

pub fn parse_changes(bytes: &[u8]) -> Result<Vec<CodeChange>, SourceError> {
    let dto: ChangesDto =
        serde_json::from_slice(bytes).map_err(|e| SourceError::parse(Source::Ssb, &e))?;
    dto.code_changes
        .into_iter()
        .map(|c| {
            let occurred_on = c.change_occurred.parse::<Date>().map_err(|_| {
                SourceError::new(
                    Source::Ssb,
                    SourceErrorKind::UnexpectedValue {
                        field: "changeOccurred",
                    },
                )
            })?;
            Ok(CodeChange {
                old_code: c.old_code,
                old_name: c.old_name,
                new_code: c.new_code,
                new_name: c.new_name,
                occurred_on,
            })
        })
        .collect()
}
