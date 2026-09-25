//! The school and municipality register (#3441, docs/school-register-design.md):
//! pure rules over names and register facts, and the sync planner. Fetching and storing
//! live in other crates.

pub mod brreg;
pub mod scope;
pub mod search;
pub mod slug;
pub mod source;
pub mod sync;
mod text;
