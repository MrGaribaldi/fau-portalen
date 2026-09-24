//! Email addresses. An address is a login address, not an identity: the account ID is
//! the identity (flow spec §1, #3437), so nothing here assumes an address is permanent.
//!
//! `Debug` is redacted on both types. Errors and log lines in this workspace never carry
//! personal data (app-foundation design §10), and a derived `Debug` on any struct that
//! holds an address would otherwise print it.

use std::fmt;

/// A syntactically plausible, normalised address: trimmed and lower-cased. Deliverability
/// is proven only by the provider's passcode, which is what [`VerifiedEmail`] records.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailError {
    Empty,
    TooLong,
    Malformed,
}

impl Email {
    /// RFC 5321's practical ceiling on a forward path.
    pub const MAX_LEN: usize = 254;

    pub fn parse(raw: &str) -> Result<Self, EmailError> {
        let s = raw.trim();
        if s.is_empty() {
            return Err(EmailError::Empty);
        }
        if s.len() > Self::MAX_LEN {
            return Err(EmailError::TooLong);
        }
        if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(EmailError::Malformed);
        }
        let (local, domain) = s.rsplit_once('@').ok_or(EmailError::Malformed)?;
        if local.is_empty()
            || local.contains('@')
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
        {
            return Err(EmailError::Malformed);
        }
        Ok(Self(s.to_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Email([redacted])")
    }
}

/// An address the identity provider has just verified with a passcode. The type is the
/// proof: persistence functions that require a verified address take this, so an
/// unverified one cannot reach them.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedEmail(Email);

impl VerifiedEmail {
    /// Construct only from the session the HTTP layer established with Hanko (#3417),
    /// never from request input.
    pub fn from_provider(email: Email) -> Self {
        Self(email)
    }

    pub fn email(&self) -> &Email {
        &self.0
    }
}

impl fmt::Debug for VerifiedEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VerifiedEmail([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trims_and_lowercases() {
        let e = Email::parse("  Kari.Nordmann@Example.TEST ").unwrap();
        assert_eq!(e.as_str(), "kari.nordmann@example.test");
    }

    #[test]
    fn parse_rejects_empty_long_and_malformed() {
        assert_eq!(Email::parse("   "), Err(EmailError::Empty));
        let long = format!("{}@example.test", "a".repeat(250));
        assert_eq!(Email::parse(&long), Err(EmailError::TooLong));
        for bad in [
            "no-at-sign",
            "@example.test",
            "a@b@example.test",
            "a@localhost",
            "a@.example.test",
            "a@example.test.",
            "a b@example.test",
        ] {
            assert_eq!(Email::parse(bad), Err(EmailError::Malformed), "{bad}");
        }
    }

    #[test]
    fn debug_never_prints_the_address() {
        let e = Email::parse("kari@example.test").unwrap();
        assert_eq!(format!("{e:?}"), "Email([redacted])");
        let v = VerifiedEmail::from_provider(e);
        assert_eq!(format!("{v:?}"), "VerifiedEmail([redacted])");
        assert_eq!(v.email().as_str(), "kari@example.test");
    }
}
