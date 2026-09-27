use std::fmt;

/// Coarse on purpose: a caller learns that decryption failed, never why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    Randomness,
    Malformed,
    Decrypt,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Randomness => "the operating system's random source failed",
            Self::Malformed => "the stored value is not a recognised envelope",
            Self::Decrypt => "the value could not be decrypted",
        })
    }
}

impl std::error::Error for CryptoError {}
