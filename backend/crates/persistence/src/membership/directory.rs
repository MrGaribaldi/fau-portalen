//! The member directory's read (groups design §4; #3502): who a viewer may see and what
//! each person represents. `export.rs` hands over a selection's addresses.
//!
//! **What a viewer sees** (§4.4, §3.2, §3.3):
//! - the FAU-wide section, every current member and admin, for members and admins only --
//!   an FAU-wide audience never includes guests, as viewers or as entries;
//! - one section per group the viewer may read, through the same facts and the same rule as
//!   `list_groups` (`readable_groups`), listing its current members as `list_group_members`
//!   does (`group_members_sql`), guests included, **except an archived group** (controller
//!   ruling Q6, spec §4.1: the directory shows the present). An archived group's content
//!   stays readable through the group reads (`get_group`, `list_group_members`) -- archiving
//!   only removes it as a directory section;
//! - so a guest sees exactly the members of their own (non-archived) groups, and members
//!   see a guest only inside a group they can read.
//!
//! "Current" means standing today: a usable membership holding a role assignment valid
//! today. A person whose name an Article 17 erasure removed is not listed. Everyone listed
//! is therefore active, so under D3 (28 September 2026) their name still applies: an entry's
//! `name` is only ever `MemberName::Named` or `MemberName::Unnamed`, never `Ended` or
//! `Former`.
//!
//! **What a person represents** is derived, never typed: their role assignments valid
//! today (role name, plus unit or cohort), and the listed groups they are in (§4.1).
//!
//! Every read runs in one snapshot with the authorization it depends on. Names and contact
//! addresses leave this module as ciphertext; the session decrypts them under the FAU's
//! record key and orders the result with `fau_domain::directory::listing::arrange`.

use std::collections::{BTreeMap, HashSet};

use fau_crypto::Ciphertext;
use fau_domain::authz::{decide, Action, Target};
use fau_domain::directory::listing::{Place, RoleHeld, SectionId};
use fau_domain::email::Email;
use fau_domain::membership::access::Capability;
use fau_domain::time::Moment;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::access::membership_access;
use super::authz::{read_transaction, Viewer, ASSIGNMENT_VALID_ON_3};
use super::error::MembershipError;
use super::groups::{group_members_sql, readable_groups, standing_today_sql};
use super::names::MemberName;
use super::sql::date_param;

/// The address the directory shows for a person (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryAddress {
    /// The member's own contact address, encrypted with `CONTACT_EMAIL_AAD`.
    Contact(Ciphertext),
    /// No contact address is set, so the login address is shown.
    Login(Email),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPerson {
    pub membership_id: Uuid,
    /// The viewer's own entry, where the screen offers editing.
    pub is_viewer: bool,
    /// `Named`, or `Unnamed` for a membership created before migration 0008.
    pub name: MemberName,
    pub address: DirectoryAddress,
    /// Holds only guest-class roles today: marked "Gjest".
    pub is_guest: bool,
    /// What their role assignments valid today say they represent. Plaintext school
    /// structure; the session orders them by name.
    pub roles: Vec<RoleHeld>,
    /// The listed groups this person is in, by group id.
    pub group_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorySection {
    pub section: SectionId,
    /// The group's name under the record key (`GROUP_NAME_AAD`); `None` for the FAU-wide
    /// section.
    pub encrypted_group_name: Option<Ciphertext>,
    /// Ordered by membership id; the session orders by name after decrypting.
    pub members: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryRead {
    /// The FAU-wide section first when the viewer may see it, then groups by id.
    pub sections: Vec<DirectorySection>,
    /// Everyone listed in any section, once, by membership id.
    pub people: Vec<DirectoryPerson>,
}

type PersonRow = (Uuid, bool, Option<Vec<u8>>, Option<Vec<u8>>, String, bool);

/// Loads the directory for `viewer` on the caller's snapshot. `member_directory` and
/// `export_addresses` both read through it, so an export can only ever reach people the
/// directory would show.
pub(crate) async fn load(
    conn: &mut PgConnection,
    viewer: Viewer,
    at: Moment,
) -> Result<DirectoryRead, MembershipError> {
    let access =
        membership_access(conn, viewer.tenant_id, viewer.membership_id, at.today()).await?;
    if access.capability == Capability::None {
        return Err(MembershipError::NotAuthorized);
    }
    let today = date_param(at.today());

    // Everyone with standing today, and whether they hold anything but guest roles.
    let sql = format!(
        "select m.id, m.id = $2, m.encrypted_display_name, m.encrypted_contact_email, a.email,
                not exists (select 1 from role_assignments ra
                              join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
                             where ra.tenant_id = m.tenant_id and ra.membership_id = m.id
                               and {ASSIGNMENT_VALID_ON_3} and r.capability_class <> 'guest')
           from memberships m
           join accounts a on a.id = m.account_id
          where m.tenant_id = $1 and {standing} and m.name_erased_at is null
          order by m.id",
        standing = standing_today_sql()
    );
    let rows: Vec<PersonRow> = sqlx::query_as(&sql)
        .bind(viewer.tenant_id)
        .bind(viewer.membership_id)
        .bind(&today)
        .fetch_all(&mut *conn)
        .await?;
    let mut standing: BTreeMap<Uuid, DirectoryPerson> = BTreeMap::new();
    for (id, is_viewer, name, contact, login, is_guest) in rows {
        let address = match contact {
            Some(ct) => DirectoryAddress::Contact(Ciphertext::from_stored(ct)),
            None => DirectoryAddress::Login(
                Email::parse(&login).map_err(|_| MembershipError::decode())?,
            ),
        };
        standing.insert(
            id,
            DirectoryPerson {
                membership_id: id,
                is_viewer,
                name: name
                    .map(|n| MemberName::Named(Ciphertext::from_stored(n)))
                    .unwrap_or(MemberName::Unnamed),
                address,
                is_guest,
                roles: Vec::new(),
                group_ids: Vec::new(),
            },
        );
    }

    let mut sections = Vec::new();
    if decide(access.capability, Target::Fau, Action::Read).is_ok() {
        sections.push(DirectorySection {
            section: SectionId::Fau,
            encrypted_group_name: None,
            members: standing
                .values()
                .filter(|p| !p.is_guest)
                .map(|p| p.membership_id)
                .collect(),
        });
    }
    // The directory shows the present (§4.1, controller ruling Q6): an archived group is
    // readable through the group reads, but it is never a directory section.
    let groups: Vec<_> = readable_groups(conn, viewer, access.capability, at)
        .await?
        .into_iter()
        .filter(|g| !g.archived)
        .collect();
    let group_ids: Vec<Uuid> = groups.iter().map(|g| g.group_id).collect();
    let rows: Vec<(Uuid, Uuid, bool, bool)> = sqlx::query_as(&group_members_sql("= any($2)"))
        .bind(viewer.tenant_id)
        .bind(&group_ids)
        .bind(&today)
        .fetch_all(&mut *conn)
        .await?;
    for g in groups {
        sections.push(DirectorySection {
            section: SectionId::Group(g.group_id),
            encrypted_group_name: Some(g.encrypted_name),
            members: rows
                .iter()
                .filter(|(group, m, _, _)| *group == g.group_id && standing.contains_key(m))
                .map(|(_, m, _, _)| *m)
                .collect(),
        });
    }

    let listed: HashSet<Uuid> = sections
        .iter()
        .flat_map(|s| s.members.iter().copied())
        .collect();
    standing.retain(|id, _| listed.contains(id));
    for s in &sections {
        if let SectionId::Group(g) = s.section {
            for m in &s.members {
                if let Some(p) = standing.get_mut(m) {
                    p.group_ids.push(g);
                }
            }
        }
    }

    // What each listed person represents: role assignments valid today.
    let ids: Vec<Uuid> = standing.keys().copied().collect();
    let roles: Vec<(Uuid, String, Option<String>, Option<String>)> = sqlx::query_as(&format!(
        "select ra.membership_id, r.name, u.name, c.name
           from role_assignments ra
           join roles r on r.tenant_id = ra.tenant_id and r.id = ra.role_id
           left join organization_units u on u.tenant_id = r.tenant_id and u.id = r.unit_id
           left join cohorts c            on c.tenant_id = r.tenant_id and c.id = r.cohort_id
          where ra.tenant_id = $1 and ra.membership_id = any($2) and {ASSIGNMENT_VALID_ON_3}
          order by ra.membership_id, ra.id"
    ))
    .bind(viewer.tenant_id)
    .bind(&ids)
    .bind(&today)
    .fetch_all(&mut *conn)
    .await?;
    for (m, name, unit, cohort) in roles {
        let place = match (unit, cohort) {
            (Some(u), _) => Some(Place::Unit(u)),
            (None, Some(c)) => Some(Place::Cohort(c)),
            (None, None) => None,
        };
        if let Some(p) = standing.get_mut(&m) {
            p.roles.push(RoleHeld { name, place });
        }
    }

    Ok(DirectoryRead {
        sections,
        people: standing.into_values().collect(),
    })
}

/// The directory as `viewer` may see it today (§4.3, §4.4). A viewer with no standing gets
/// `NotAuthorized`.
pub async fn member_directory(
    pool: &PgPool,
    viewer: Viewer,
    at: Moment,
) -> Result<DirectoryRead, MembershipError> {
    let mut tx = read_transaction(pool).await?;
    let read = load(&mut tx, viewer, at).await?;
    tx.commit().await?;
    Ok(read)
}
