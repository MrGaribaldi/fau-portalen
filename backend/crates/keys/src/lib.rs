//! FAU's use of OpenBao (docs/key-service-design.md).

mod cache;
mod client;
mod data_key;

pub use cache::{CacheClock, KeyCache};
pub use client::{Auth, KeyError, Keys, KeysConfig};
pub use data_key::{data_key, DataKeyError};
