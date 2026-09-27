//! The units that each have their own OpenBao transit key (docs/key-service-design.md §3.1).

use std::fmt;

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChatMonth {
    pub year: i16,
    pub month: i8,
}

impl ChatMonth {
    pub fn parse(s: &str) -> Option<Self> {
        let (y, m) = s.split_once('-')?;
        if y.len() != 4 || m.len() != 2 || !y.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let (year, month) = (y.parse().ok()?, m.parse().ok()?);
        (1..=12).contains(&month).then_some(Self { year, month })
    }

    pub fn index(self) -> i32 {
        i32::from(self.year) * 12 + i32::from(self.month) - 1
    }
}

impl fmt::Display for ChatMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    Record { tenant: Uuid },
    Document { tenant: Uuid, document: Uuid },
    Chat { tenant: Uuid, month: ChatMonth },
    Messages { tenant: Uuid },
}

impl Unit {
    pub fn tenant(&self) -> Uuid {
        match *self {
            Unit::Record { tenant }
            | Unit::Document { tenant, .. }
            | Unit::Chat { tenant, .. }
            | Unit::Messages { tenant } => tenant,
        }
    }

    pub fn key_name(&self) -> String {
        match self {
            Unit::Record { tenant } => format!("fau-{tenant}-record"),
            Unit::Document { tenant, document } => format!("fau-{tenant}-doc-{document}"),
            Unit::Chat { tenant, month } => format!("fau-{tenant}-chat-{month}"),
            Unit::Messages { tenant } => format!("fau-{tenant}-messages"),
        }
    }

    /// `(unit, scope)` in `wrapped_keys`; `None` for units without a data key.
    pub fn storage(&self) -> Option<(&'static str, Option<String>)> {
        match self {
            Unit::Record { .. } => Some(("record", None)),
            Unit::Document { document, .. } => Some(("document", Some(document.to_string()))),
            Unit::Chat { month, .. } => Some(("chat", Some(month.to_string()))),
            Unit::Messages { .. } => None,
        }
    }
}

fn has_transit_prefix(s: &str) -> bool {
    s.strip_prefix("vault:v")
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(v, body)| {
            !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && !body.is_empty()
        })
}

/// A data key as OpenBao wrapped it (`vault:v1:…`). Useless once its transit key is gone.
/// Its `Debug` prints the length only (spec §7).
#[derive(Clone, PartialEq, Eq)]
pub struct WrappedKey(String);

impl WrappedKey {
    pub fn new(s: String) -> Option<Self> {
        has_transit_prefix(&s).then_some(Self(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WrappedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WrappedKey({} bytes)", self.0.len())
    }
}

/// A message encrypted by OpenBao transit `encrypt` (`vault:v1:…`).
#[derive(Clone, PartialEq, Eq)]
pub struct MessageCiphertext(String);

impl MessageCiphertext {
    pub fn new(s: String) -> Option<Self> {
        has_transit_prefix(&s).then_some(Self(s))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MessageCiphertext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MessageCiphertext({} bytes)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn key_names_follow_the_spec() {
        let t = Uuid::parse_str("0192d3f4-0000-7000-8000-000000000001").unwrap();
        let d = Uuid::parse_str("0192d3f4-0000-7000-8000-0000000000aa").unwrap();
        assert_eq!(
            Unit::Record { tenant: t }.key_name(),
            "fau-0192d3f4-0000-7000-8000-000000000001-record"
        );
        assert_eq!(
            Unit::Document {
                tenant: t,
                document: d
            }
            .key_name(),
            "fau-0192d3f4-0000-7000-8000-000000000001-doc-0192d3f4-0000-7000-8000-0000000000aa"
        );
        assert_eq!(
            Unit::Chat {
                tenant: t,
                month: ChatMonth::parse("2026-09").unwrap()
            }
            .key_name(),
            "fau-0192d3f4-0000-7000-8000-000000000001-chat-2026-09"
        );
        assert_eq!(
            Unit::Messages { tenant: t }.key_name(),
            "fau-0192d3f4-0000-7000-8000-000000000001-messages"
        );
    }

    #[test]
    fn storage_rows_match_the_wrapped_keys_table() {
        let t = Uuid::now_v7();
        let d = Uuid::now_v7();
        assert_eq!(Unit::Record { tenant: t }.storage(), Some(("record", None)));
        assert_eq!(
            Unit::Document {
                tenant: t,
                document: d
            }
            .storage(),
            Some(("document", Some(d.to_string())))
        );
        assert_eq!(
            Unit::Chat {
                tenant: t,
                month: ChatMonth::parse("2026-09").unwrap()
            }
            .storage(),
            Some(("chat", Some("2026-09".into())))
        );
        assert_eq!(Unit::Messages { tenant: t }.storage(), None);
    }

    #[test]
    fn chat_months_parse_strictly() {
        assert_eq!(ChatMonth::parse("2026-09").unwrap().to_string(), "2026-09");
        for bad in ["2026-13", "2026-00", "26-09", "2026-9", "2026/09", ""] {
            assert!(ChatMonth::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(
            ChatMonth::parse("2027-10").unwrap().index()
                - ChatMonth::parse("2026-09").unwrap().index(),
            13
        );
    }

    #[test]
    fn transit_ciphertexts_must_carry_the_vault_prefix() {
        assert!(WrappedKey::new("vault:v1:abc".into()).is_some());
        assert!(WrappedKey::new("plain".into()).is_none());
        assert!(MessageCiphertext::new("vault:v12:abc".into()).is_some());
        assert!(MessageCiphertext::new("vault:x:abc".into()).is_none());
    }

    /// Spec §7: a type holding ciphertext prints its length, never its value.
    #[test]
    fn transit_ciphertexts_debug_redacts_the_value() {
        let body = "SECRETCIPHERTEXTBODY";
        let wrapped = WrappedKey::new(format!("vault:v1:{body}")).unwrap();
        let message = MessageCiphertext::new(format!("vault:v1:{body}")).unwrap();
        for debug in [format!("{wrapped:?}"), format!("{message:?}")] {
            assert!(!debug.contains(body), "{debug}");
            assert!(debug.contains("29 bytes"), "{debug}");
        }
    }
}
