//! FAU's use of OpenBao (docs/key-service-design.md).

mod client;

pub use client::{Auth, KeyError, Keys, KeysConfig};
