//! The member directory (groups design §4; #3502): names, what each person represents and
//! a contact address; select people, then open a `mailto:` link or copy the addresses.
//!
//! Pure. Persistence authorizes and reads the directory (`member_directory`) and audits
//! every export of addresses (`export_addresses`); the session decrypts names and
//! addresses under the FAU's record key; this module orders the result for the viewer's
//! locale and builds the link or the text.

pub mod address;
