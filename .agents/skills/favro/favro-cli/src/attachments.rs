use serde_json::Value;
use std::collections::BTreeMap;

/// Count names rather than using a set: two uploads may share a filename.
fn names(card: &Value) -> Result<BTreeMap<String, usize>, String> {
    let mut names = BTreeMap::new();
    let attachments = match card.get("attachments") {
        None | Some(Value::Null) => return Ok(names),
        Some(Value::Array(attachments)) => attachments,
        _ => return Err("Favro returned an invalid attachment list.".into()),
    };
    for attachment in attachments {
        let name = attachment
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Favro returned an attachment without a name.")?;
        *names.entry(name.to_string()).or_default() += 1;
    }
    Ok(names)
}

/// Markdown PUT rebuilds Favro's attachment list from image nodes, including
/// non-image uploads. Preserve existing files in that same request using their
/// remote URLs; neither attachments nor addAttachments body fields work.
///
/// `omit` lists `fileURL`s to drop the node for instead of preserving -- since the
/// attachment list is rebuilt from these nodes, leaving one out is how
/// `attach --replace` and `detach` remove it. Every other attachment, including ones
/// sharing the same name, round-trips unchanged. Every attachment is still validated,
/// including omitted ones: refusing an unrepresentable attachment must not depend on
/// whether this write happens to remove it. Pass an empty `omit` to preserve everything.
pub fn preserve_in_markdown_except(
    card: &Value,
    description: &str,
    omit: &[&str],
) -> Result<String, String> {
    // Validate the metadata before any mutation, including unnamed attachments.
    names(card)?;
    let mut result = String::new();
    if let Some(Value::Array(attachments)) = card.get("attachments") {
        for attachment in attachments {
            let name = attachment["name"].as_str().unwrap();
            let url = attachment.get("fileURL").and_then(Value::as_str)
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
                .ok_or_else(|| format!("Cannot preserve attachment {name:?}: missing or invalid fileURL. Description was not changed."))?;
            if name.chars().any(char::is_control)
                || url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
                || url.contains(['<', '>', '\\'])
            {
                return Err(format!("Cannot safely represent attachment {name:?} in Markdown. Description was not changed."));
            }
            if omit.contains(&url) {
                continue;
            }
            let mut label = String::new();
            for ch in name.chars() {
                if ch.is_ascii_punctuation() {
                    label.push('\\');
                }
                label.push(ch);
            }
            result.push_str(&format!("![{label}](<{url}>)\n\n"));
        }
    }
    // Put nodes before user text: an unclosed code fence or HTML block at the
    // end of that text must not consume attachment links as literal content.
    result.push_str(description);
    Ok(result)
}

/// `expected_removals` lists attachment names this write intentionally removed, one
/// entry per removed attachment (names are not unique, so a name removed twice needs
/// two entries). Those counts are subtracted from the pre-write baseline before
/// comparing; anything missing beyond that still fails loudly -- the guard's value is
/// catching *unintended* loss, not blessing every decrease an intentional removal
/// happens to accompany.
pub fn verify(before: &Value, after: &Value, expected_removals: &[String]) -> Result<(), String> {
    let expected = names(before)?;
    let actual = names(after)?;
    let mut allowance: BTreeMap<&str, usize> = BTreeMap::new();
    for name in expected_removals {
        *allowance.entry(name.as_str()).or_default() += 1;
    }
    let missing: Vec<String> = expected
        .into_iter()
        .filter_map(|(name, count)| {
            let allowed_gone = allowance.get(name.as_str()).copied().unwrap_or_default();
            let floor = count.saturating_sub(allowed_gone);
            let remaining = actual.get(&name).copied().unwrap_or_default();
            (remaining < floor).then(|| format!("{name:?} ({} missing)", floor - remaining))
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Attachment loss detected after updating the card: {}. The update was applied; inspect the card before continuing.",
            missing.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_remote_files_with_markdown_safe_labels_and_urls() {
        let card = json!({"attachments": [{"name": "evidence [final].pdf", "fileURL": "https://example.com/file(1)?signature=abc&x=1"}]});
        let result = preserve_in_markdown_except(&card, "- [x] reviewed", &[]).unwrap();
        assert_eq!(result, "![evidence \\[final\\]\\.pdf](<https://example.com/file(1)?signature=abc&x=1>)\n\n- [x] reviewed");
        assert_eq!(
            preserve_in_markdown_except(&json!({}), "text", &[]).unwrap(),
            "text"
        );
    }

    #[test]
    fn refuses_unrepresentable_attachments_before_mutation() {
        for attachment in [
            json!({"name": "evidence.pdf"}),
            json!({"name": "evidence.pdf", "fileURL": "javascript:alert(1)"}),
            json!({"name": "evidence.pdf", "fileURL": "https://example.com/unsafe>"}),
            json!({"name": "evidence\n.pdf", "fileURL": "https://example.com/file"}),
        ] {
            assert!(preserve_in_markdown_except(
                &json!({"attachments": [attachment]}),
                "text",
                &[]
            )
            .is_err());
        }
    }

    #[test]
    fn detects_missing_names_and_duplicate_uploads() {
        let before = json!({"attachments": [{"name": "evidence.pdf"}, {"name": "evidence.pdf"}]});
        let after = json!({"attachments": [{"name": "evidence.pdf"}]});
        assert!(verify(&before, &after, &[])
            .unwrap_err()
            .contains("1 missing"));
        assert!(verify(&before, &json!({}), &[]).is_err());
    }

    #[test]
    fn permits_added_files_and_rotating_signed_urls() {
        let before = json!({"attachments": [{"name": "evidence.pdf", "fileURL": "old"}]});
        let after = json!({"attachments": [
            {"name": "extra.txt"}, {"name": "evidence.pdf", "fileURL": "new"}
        ]});
        assert!(verify(&before, &after, &[]).is_ok());
        assert!(verify(&json!({}), &json!({"attachments": []}), &[]).is_ok());
    }

    #[test]
    fn rejects_malformed_attachment_metadata() {
        assert!(verify(&json!({"attachments": {}}), &json!({}), &[]).is_err());
        assert!(verify(&json!({}), &json!({"attachments": [{}]}), &[]).is_err());
    }

    #[test]
    fn permits_expected_removals_but_still_catches_extra_loss() {
        let before = json!({"attachments": [
            {"name": "evidence.pdf"}, {"name": "evidence.pdf"}, {"name": "extra.txt"}
        ]});
        // Removing one of the two "evidence.pdf" copies is expected and passes.
        let after_intended =
            json!({"attachments": [{"name": "evidence.pdf"}, {"name": "extra.txt"}]});
        assert!(verify(&before, &after_intended, &["evidence.pdf".to_string()]).is_ok());
        // The same expected removal must not mask an unrelated file also disappearing.
        let after_extra_loss = json!({"attachments": [{"name": "evidence.pdf"}]});
        let err = verify(&before, &after_extra_loss, &["evidence.pdf".to_string()]).unwrap_err();
        assert!(err.contains("extra.txt"));
        assert!(!err.contains("evidence.pdf"));
    }

    #[test]
    fn omits_only_the_named_url_and_preserves_same_named_siblings() {
        let card = json!({"attachments": [
            {"name": "evidence.pdf", "fileURL": "https://example.com/old"},
            {"name": "evidence.pdf", "fileURL": "https://example.com/new"},
        ]});
        let result =
            preserve_in_markdown_except(&card, "body", &["https://example.com/old"]).unwrap();
        assert_eq!(
            result,
            "![evidence\\.pdf](<https://example.com/new>)\n\nbody"
        );
    }

    #[test]
    fn still_validates_omitted_attachments_before_mutation() {
        let card =
            json!({"attachments": [{"name": "evidence.pdf", "fileURL": "javascript:alert(1)"}]});
        assert!(preserve_in_markdown_except(&card, "body", &["javascript:alert(1)"]).is_err());
    }
}
