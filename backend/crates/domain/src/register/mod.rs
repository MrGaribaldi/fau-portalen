//! The school and municipality register (#3441, docs/school-register-design.md):
//! pure rules over names and register facts. Fetching, storing and syncing live in
//! other crates.

pub mod brreg;
pub mod scope;
pub mod search;
pub mod slug;
mod text;
