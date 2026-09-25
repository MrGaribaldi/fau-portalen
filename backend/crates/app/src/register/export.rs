//! `fau register export`: the pickable schools as CSV on stdout (§9), for #3431.
//!
//! RFC 4180 quoting (a field with a comma, a quote or a line break is quoted, and its
//! quotes doubled) and LF line ends. Values are written as they are: whoever opens the file
//! in a spreadsheet imports it as text.

use fau_persistence::register::{connect, export_rows, ExportRow, RegisterError};

use super::Exit;
use crate::config::RegisterConfig;

const HEADER: &str = "school_id,orgnr,municipality_number,display_name,path,fau_orgnr";

fn field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

pub(super) fn csv(rows: &[ExportRow]) -> String {
    let mut out = format!("{HEADER}\n");
    for r in rows {
        let line = [
            r.school_id.to_string(),
            field(r.orgnr.as_deref().unwrap_or("")),
            field(&r.municipality_number),
            field(&r.display_name),
            field(&r.path),
            field(r.fau_orgnr.as_deref().unwrap_or("")),
        ]
        .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

pub(super) async fn export(config: &RegisterConfig) -> Exit {
    let rows: Result<Vec<ExportRow>, RegisterError> = async {
        let mut conn = connect(config.database_url.expose()).await?;
        export_rows(&mut conn).await
    }
    .await;
    match rows {
        Ok(rows) => {
            print!("{}", csv(&rows));
            tracing::info!(schools = rows.len(), "register export finished");
            Exit::Done
        }
        Err(e) => {
            tracing::error!(error = %e, "register export failed");
            Exit::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_field_is_quoted_only_when_it_must_be() {
        assert_eq!(field("Hosle skole"), "Hosle skole");
        assert_eq!(
            field("Skole \"Nord\", avd. 2"),
            "\"Skole \"\"Nord\"\", avd. 2\""
        );
        assert_eq!(field("to\nlinjer"), "\"to\nlinjer\"");
        assert_eq!(field(""), "");
    }

    #[test]
    fn missing_values_are_empty_fields() {
        let row = ExportRow {
            school_id: Uuid::from_u128(1),
            orgnr: None,
            municipality_number: "0301".into(),
            display_name: "Nyskolen".into(),
            path: "/s/00000000-0000-0000-0000-000000000001".into(),
            fau_orgnr: None,
        };
        assert_eq!(
            csv(&[row]),
            "school_id,orgnr,municipality_number,display_name,path,fau_orgnr\n\
             00000000-0000-0000-0000-000000000001,,0301,Nyskolen,/s/00000000-0000-0000-0000-000000000001,\n"
        );
    }
}
