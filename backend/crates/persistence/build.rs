//! `sqlx::migrate!` embeds each migration file via `include_str!` at the call site,
//! resolved once when the macro expands. It does not itself notice a *new* file
//! appearing in `migrations/` on stable Rust -- directory-level change tracking
//! (`proc_macro::tracked_path`) needs the `track-path` feature, which is nightly
//! only. Without this build script, `cargo build` sees no tracked input file has
//! changed and reports this crate `Fresh`, so a binary built before a migration was
//! added keeps running with the old, embedded migration set even though the file is
//! sitting right there on disk.
//!
//! This is sqlx's own documented remedy: a manual `cargo:rerun-if-changed` on the
//! migrations directory forces Cargo to invalidate and rebuild whenever anything
//! under it changes, regardless of what the macro itself can see.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
