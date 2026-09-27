//! Backend-side encryption types for FAU (docs/key-service-design.md §3).

mod envelope;
mod error;
mod key;
mod unit;

pub use envelope::{decrypt, encrypt, Aad, Ciphertext};
pub use error::CryptoError;
pub use key::DataKey;
pub use unit::{ChatMonth, MessageCiphertext, Unit, WrappedKey};
pub use zeroize::Zeroizing;
