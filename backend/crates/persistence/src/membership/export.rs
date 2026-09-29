//! Handing over a selection's addresses for "Skriv e-post" or "Kopier adresser" (groups
//! design §4.3, §4.4; #3502), audited in the same transaction.

use std::collections::HashSet;

use fau_domain::directory::listing::SectionId;
use fau_domain::time::Moment;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::authz::Viewer;
use super::directory::{load, DirectoryAddress, DirectoryRead};
use super::error::MembershipError;
use super::sql::{write_audit, Audit};

/// "Skriv e-post" or "Kopier adresser" (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPurpose {
    Mailto,
    Copy,
}

impl ExportPurpose {
    fn code(self) -> &'static str {
        match self {
            ExportPurpose::Mailto => "mailto",
            ExportPurpose::Copy => "copy",
        }
    }
}

/// Which part of the directory the selection was made in: the screen's group filter, or
/// none of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    /// Every section the viewer sees.
    All,
    Fau,
    Group(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressExport {
    pub purpose: ExportPurpose,
    pub scope: ExportScope,
    /// The people ticked, in order; a person ticked in two groups may appear twice.
    pub membership_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedRecipient {
    pub membership_id: Uuid,
    pub address: DirectoryAddress,
}

/// Hands over the addresses of a selection for a `mailto:` link or a copy, and audits it
/// in the same transaction (§4.4): the purpose, the count and the scope, **never the
/// addresses** and never who was selected, because pulling out a batch of addresses is
/// itself a disclosure. Each person comes back once, in first-selected order; the session
/// decrypts contact addresses and builds the link with `fau_domain::directory::address`.
///
/// `recipient_count` counts **people**, each once. It can exceed the number of distinct
/// addresses in the link or the copied text, since two people may share an address and
/// `Recipients::new` keeps it once (final review M4).
///
/// Refusals, in this order, so none reveals more than the one before it:
/// 1. `EmptySelection`;
/// 2. `NotAuthorized`: no standing, or a guest asking for the FAU-wide section;
/// 3. `UnknownGroup`: a group the viewer cannot read, exactly as for an id that does not
///    exist;
/// 4. `UnknownMembership`: a selected person the scope does not list for this viewer,
///    whether or not the id exists elsewhere.
///
/// Allowed while the FAU is frozen: reading continues (ADR-003 7a).
pub async fn export_addresses(
    pool: &PgPool,
    viewer: Viewer,
    req: AddressExport,
    at: Moment,
) -> Result<Vec<ExportedRecipient>, MembershipError> {
    if req.membership_ids.is_empty() {
        return Err(MembershipError::EmptySelection);
    }
    let mut tx = pool.begin().await?;
    // One snapshot for the authorization and the read, as in `read_transaction`, but
    // writable: the audit entry commits with it.
    sqlx::query("set transaction isolation level repeatable read")
        .execute(&mut *tx)
        .await?;
    let read = load(&mut tx, viewer, at).await?;
    let in_scope = match req.scope {
        ExportScope::Fau => {
            scope_members(&read, req.scope).ok_or(MembershipError::NotAuthorized)?
        }
        ExportScope::Group(_) => {
            scope_members(&read, req.scope).ok_or(MembershipError::UnknownGroup)?
        }
        ExportScope::All => scope_members(&read, req.scope)
            .expect("scope_members(All) always lists every person the viewer sees"),
    };
    let mut seen = HashSet::new();
    let mut chosen = Vec::new();
    for id in &req.membership_ids {
        if !in_scope.contains(id) {
            return Err(MembershipError::UnknownMembership);
        }
        if seen.insert(*id) {
            chosen.push(*id);
        }
    }
    let recipients: Vec<ExportedRecipient> = chosen
        .iter()
        .map(|id| {
            let p = read
                .people
                .iter()
                .find(|p| p.membership_id == *id)
                .expect("every section member is a listed person");
            ExportedRecipient {
                membership_id: *id,
                address: p.address.clone(),
            }
        })
        .collect();
    let (scope, group_id) = match req.scope {
        ExportScope::All => ("all", None),
        ExportScope::Fau => ("fau", None),
        ExportScope::Group(g) => ("group", Some(g)),
    };
    write_audit(
        &mut tx,
        at,
        Audit::member(
            viewer.tenant_id,
            viewer.membership_id,
            "directory.addresses_exported",
            "tenant",
            viewer.tenant_id,
            json!({
                "purpose": req.purpose.code(),
                "recipient_count": recipients.len(),
                "scope": scope,
                "group_id": group_id,
            }),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(recipients)
}

/// The members of the section `scope` names, if the viewer may see it.
fn scope_members(read: &DirectoryRead, scope: ExportScope) -> Option<HashSet<Uuid>> {
    let pick = |id: SectionId| {
        read.sections
            .iter()
            .find(|s| s.section == id)
            .map(|s| s.members.iter().copied().collect())
    };
    match scope {
        ExportScope::All => Some(read.people.iter().map(|p| p.membership_id).collect()),
        ExportScope::Fau => pick(SectionId::Fau),
        ExportScope::Group(g) => pick(SectionId::Group(g)),
    }
}
