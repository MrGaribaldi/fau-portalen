//! `fau register export`: the pickable schools as CSV on stdout (§9), for #3431.
//!
//! RFC 4180 quoting (a field with a comma, a quote or a line break is quoted, and its
//! quotes doubled) and LF line ends. #3431's file is opened by people in spreadsheets, so a
//! field that a spreadsheet would run as a formula -- one starting with `=`, `+`, `@`, a tab
//! or a CR -- gets a leading `'` first (final review, item 5). A leading `-` is left alone:
//! real names can start with one.

use fau_persistence::register::{connect, export_rows, ExportRow, RegisterError};

use super::Exit;
use crate::config::RegisterConfig;

const HEADER: &str = "school_id,orgnr,municipality_number,display_name,path,fau_orgnr";

/// The characters a spreadsheet treats as the start of a formula, `-` deliberately excepted.
const FORMULA_STARTS: [char; 5] = ['=', '+', '@', '\t', '\r'];

fn field(value: &str) -> String {
    let value = if value.starts_with(FORMULA_STARTS) {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
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

    /// Final review, item 5: #3431's CSV is opened in spreadsheets, which run a cell that
    /// starts with `=`, `+`, `@`, a tab or a CR as a formula. Such a field gets a leading `'`.
    /// A leading `-` is left alone: real names can start with one.
    #[test]
    fn a_field_that_a_spreadsheet_would_run_as_a_formula_is_prefixed() {
        assert_eq!(field("=HYPERLINK(1)"), "'=HYPERLINK(1)");
        assert_eq!(field("+47 skole"), "'+47 skole");
        assert_eq!(field("@skole"), "'@skole");
        assert_eq!(field("\tskole"), "'\tskole");
        assert_eq!(field("\rskole"), "\"'\rskole\"");
        assert_eq!(field("=1,2"), "\"'=1,2\"");
        assert_eq!(field("-Skolen"), "-Skolen");
        assert_eq!(field("Skole = 1"), "Skole = 1");
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
