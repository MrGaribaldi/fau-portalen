//! Invitation tokens (spec 5.1): 32 bytes from the operating system's CSPRNG, shown as
//! 64 lower-case hex characters, stored only as the SHA-256 of that text. The raw token
//! is returned to the caller once, is never written to the database or the outbox, and
//! has a redacted `Debug` so it cannot reach a log line by accident.

use std::fmt;

use sha2::{Digest, Sha256};

use super::error::MembershipError;

/// A freshly generated invitation token. Deliberately neither `Clone` nor `Display`:
/// the only way to read it is [`InvitationToken::expose`], which is easy to grep for.
pub struct InvitationToken(String);

impl InvitationToken {
    pub(crate) fn generate() -> Result<Self, MembershipError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| MembershipError::Randomness)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    /// The raw token, for the link in the invitation email.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn hash(&self) -> Vec<u8> {
        hash_token(&self.0)
    }
}

impl fmt::Debug for InvitationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InvitationToken([redacted])")
    }
}

/// SHA-256 of the token's text, as stored in `invitations.token_hash`.
pub(crate) fn hash_token(raw: &str) -> Vec<u8> {
    Sha256::digest(raw.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_lowercase_hex_characters_and_unique() {
        let a = InvitationToken::generate().unwrap();
        let b = InvitationToken::generate().unwrap();
        let hex = a.expose();
        assert_eq!(hex.len(), 64);
        assert!(hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn the_hash_is_sha256_of_the_text() {
        // FIPS 180-2's "abc" test vector.
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let hex: String = hash_token("abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, expected);
        let t = InvitationToken::generate().unwrap();
        assert_eq!(t.hash(), hash_token(t.expose()));
        assert_eq!(t.hash().len(), 32);
    }

    #[test]
    fn debug_is_redacted() {
        let t = InvitationToken::generate().unwrap();
        let debug = format!("{t:?}");
        assert_eq!(debug, "InvitationToken([redacted])");
        assert!(!debug.contains(t.expose()));
    }
}
