//! The membership rules from the #3413 flow spec, as pure functions over values. No I/O:
//! persistence loads a snapshot, calls these, and writes the outcome in one transaction.

pub mod access;
pub mod period;
pub mod rules;
pub mod vocabulary;
