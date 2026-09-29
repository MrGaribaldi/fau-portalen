//! The member directory's reads (groups design §4.3, §4.4; #3502): its sections agree with
//! the group reads, an entry shows what a person represents, and only people with standing
//! today are listed. The per-viewer matrix is in authorization.rs.

mod common;
use common::groups::*;
use common::membership::*;
use common::TestDb;
use fau_crypto::Ciphertext;
use fau_domain::directory::listing::{Place, RoleHeld, SectionId};
use fau_domain::membership::period::Period;
use fau_domain::membership::vocabulary::{CapabilityClass, Visibility};
use fau_persistence::membership::*;
use sqlx::PgPool;
use uuid::Uuid;

fn year() -> Period {
    period(day(2026, 9, 1), day(2027, 9, 1))
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort_unstable();
    v
}

/// Mutation check: give the directory its own copy of the member SQL, or let it skip
/// `decide` for groups, and one of these comparisons fails for some viewer.
#[tokio::test]
async fn sections_are_exactly_the_readable_groups_with_their_member_lists() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let w = world(&pool).await;
    let t0 = at(T0);
    for name in VIEWERS.iter().filter(|n| !n.starts_with("none")) {
        let viewer = w.viewer(name);
        let read = member_directory(&pool, viewer, t0).await.unwrap();
        let groups: Vec<Uuid> = read
            .sections
            .iter()
            .filter_map(|s| match s.section {
                SectionId::Group(g) => Some(g),
                SectionId::Fau => None,
            })
            .collect();
        let listed: Vec<Uuid> = list_groups(&pool, viewer, t0)
            .await
            .unwrap()
            .iter()
            .map(|g| g.group_id)
            .collect();
        assert_eq!(sorted(groups.clone()), sorted(listed), "{name}");
        for g in groups {
            let section = read
                .sections
                .iter()
                .find(|s| s.section == SectionId::Group(g))
                .unwrap();
            let members: Vec<Uuid> = list_group_members(&pool, viewer, g, t0)
                .await
                .unwrap()
                .iter()
                .map(|m| m.membership_id)
                .collect();
            assert_eq!(sorted(section.members.clone()), sorted(members), "{name}");
            assert!(section.encrypted_group_name.is_some());
        }
    }
}

async fn named_role(
    pool: &PgPool,
    tenant: Uuid,
    name: &str,
    unit: Option<Uuid>,
    cohort: Option<Uuid>,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into roles (tenant_id, id, name, capability_class, unit_id, cohort_id)
         values ($1, $2, $3, 'member', $4, $5)",
    )
    .bind(tenant)
    .bind(id)
    .bind(name)
    .bind(unit)
    .bind(cohort)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn plaintext_name(pool: &PgPool, table: &str, id: Uuid) -> String {
    sqlx::query_scalar(&format!("select name from {table} where id = $1"))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_entry_shows_what_the_person_represents_and_which_address_applies() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let unit = seed_unit(&pool, fau.tenant_id).await;
    let cohort = seed_cohort(&pool, fau.tenant_id).await;
    let on_unit = named_role(&pool, fau.tenant_id, "Kontaktforelder", Some(unit), None).await;
    let on_cohort = named_role(&pool, fau.tenant_id, "Trinnkontakt", None, Some(cohort)).await;
    let group = seed_bound_group(&pool, &fau, Visibility::Open, Some(unit), None).await;

    let kari = add_member(
        &pool,
        &fau,
        "kari@example.test",
        RoleChoice::Existing(on_unit),
        year(),
        t0,
    )
    .await
    .membership_id;
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            role: RoleChoice::Existing(on_cohort),
            period: year(),
        },
        t0,
    )
    .await
    .unwrap();
    // A past role is not shown: the directory shows the present.
    grant_role(
        &pool,
        GrantRole {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: kari,
            role: new_role("Tidligere kasserer", CapabilityClass::Member),
            period: period(day(2026, 9, 23), day(2026, 9, 24)),
        },
        t0,
    )
    .await
    .unwrap();
    let contact = {
        let mut v = vec![1u8, 77];
        v.extend_from_slice(&[0u8; 40]);
        Ciphertext::from_stored(v)
    };
    set_contact_email(
        &pool,
        SetContactEmail {
            tenant_id: fau.tenant_id,
            membership_id: kari,
            encrypted_contact_email: Some(contact.clone()),
        },
        t0,
    )
    .await
    .unwrap();
    let guest = add_guest(&pool, &fau, "gjest@example.test", group, year(), t0)
        .await
        .membership_id;

    let later = at("2026-09-25T10:00:00Z");
    let read = member_directory(
        &pool,
        Viewer {
            tenant_id: fau.tenant_id,
            membership_id: kari,
        },
        later,
    )
    .await
    .unwrap();
    let entry = |m: Uuid| read.people.iter().find(|p| p.membership_id == m).unwrap();

    let k = entry(kari);
    assert!(k.is_viewer && !k.is_guest);
    assert_eq!(k.address, DirectoryAddress::Contact(contact));
    assert_eq!(k.name, MemberName::Named(placeholder_envelope()));
    let mut roles = k.roles.clone();
    roles.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        roles,
        [
            RoleHeld {
                name: "Kontaktforelder".into(),
                place: Some(Place::Unit(
                    plaintext_name(&pool, "organization_units", unit).await
                )),
            },
            RoleHeld {
                name: "Trinnkontakt".into(),
                place: Some(Place::Cohort(
                    plaintext_name(&pool, "cohorts", cohort).await
                )),
            },
        ]
    );
    assert_eq!(k.group_ids, [group], "in the unit's group through the role");

    let g = entry(guest);
    assert!(g.is_guest && !g.is_viewer);
    assert_eq!(
        g.address,
        DirectoryAddress::Login(email("gjest@example.test"))
    );
    assert_eq!(g.group_ids, [group]);
    let fau_section = read
        .sections
        .iter()
        .find(|s| s.section == SectionId::Fau)
        .unwrap();
    assert!(
        !fau_section.members.contains(&guest),
        "never in the FAU-wide section"
    );
    assert!(fau_section.members.contains(&kari));
}

#[tokio::test]
async fn only_people_with_standing_today_are_listed() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let current = join(&pool, &fau, "current@example.test", year()).await;
    let future = join(
        &pool,
        &fau,
        "future@example.test",
        period(day(2026, 10, 1), day(2027, 9, 1)),
    )
    .await;
    let ended = join(
        &pool,
        &fau,
        "ended@example.test",
        period(day(2026, 9, 1), day(2026, 9, 24)),
    )
    .await;
    let disabled = join(&pool, &fau, "disabled@example.test", year()).await;
    let revoked = join(&pool, &fau, "revoked@example.test", year()).await;
    let erased = join(&pool, &fau, "erased@example.test", year()).await;
    sqlx::query("update accounts set disabled_at = now() where id = $1")
        .bind(disabled.account_id)
        .execute(&pool)
        .await
        .unwrap();
    revoke_membership(
        &pool,
        RevokeMembership {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            membership_id: revoked.membership_id,
            confirm_no_admin: false,
        },
        t0,
    )
    .await
    .unwrap();
    erase_member_names(&pool, erased.account_id, t0)
        .await
        .unwrap();

    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: current.membership_id,
    };
    let later = at("2026-09-25T10:00:00Z");
    let listed: Vec<Uuid> = member_directory(&pool, viewer, later)
        .await
        .unwrap()
        .people
        .iter()
        .map(|p| p.membership_id)
        .collect();
    assert_eq!(
        sorted(listed),
        sorted(vec![fau.admin_membership_id, current.membership_id])
    );
    // An erased membership still has its roles, so it is not listed but may still read.
    // #3426's erasure flow decides whether the membership ends with it.
    for absent in [future, ended, disabled, revoked] {
        let refused = member_directory(
            &pool,
            Viewer {
                tenant_id: fau.tenant_id,
                membership_id: absent.membership_id,
            },
            later,
        )
        .await;
        assert_eq!(refused, Err(MembershipError::NotAuthorized));
    }
}

async fn join(pool: &PgPool, fau: &Fau, address: &str, p: Period) -> Accepted {
    add_member(
        pool,
        fau,
        address,
        new_role("Medlem", CapabilityClass::Member),
        p,
        at(T0),
    )
    .await
}

#[tokio::test]
async fn a_viewer_from_another_fau_is_refused_and_sees_nothing_of_it() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let a = active_fau(&pool, "admin-a@example.test", t0).await;
    let b = active_fau(&pool, "admin-b@example.test", t0).await;
    // B's admin, naming A's tenant: no standing there.
    let crossed = Viewer {
        tenant_id: a.tenant_id,
        membership_id: b.admin_membership_id,
    };
    assert_eq!(
        member_directory(&pool, crossed, t0).await,
        Err(MembershipError::NotAuthorized)
    );
    // Positive control: B's admin sees only B.
    let own = member_directory(
        &pool,
        Viewer {
            tenant_id: b.tenant_id,
            membership_id: b.admin_membership_id,
        },
        t0,
    )
    .await
    .unwrap();
    let ids: Vec<Uuid> = own.people.iter().map(|p| p.membership_id).collect();
    assert_eq!(ids, [b.admin_membership_id]);
}

/// Q6 (controller ruling): the directory shows the present (spec §4.1), so an archived
/// group is not one of its sections, and a person listed only through it disappears from
/// the directory entirely. The group's content stays readable through the group reads
/// (`get_group`, `list_group_members`): archiving does not hide the group itself.
#[tokio::test]
async fn an_archived_group_the_viewer_is_in_is_not_a_section_and_its_only_member_disappears() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let group = seed_group(&pool, &fau, Visibility::Open).await;
    // A guest whose only role is a guest role naming the group (§3.1): with no other
    // standing, this is the only section that would ever list them.
    let solo = add_guest(&pool, &fau, "solo@example.test", group, year(), t0).await;

    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: fau.admin_membership_id,
    };
    // Before archiving: the group is a section, and its only member is listed.
    let before = member_directory(&pool, viewer, t0).await.unwrap();
    assert!(before
        .sections
        .iter()
        .any(|s| s.section == SectionId::Group(group)));
    assert!(before
        .people
        .iter()
        .any(|p| p.membership_id == solo.membership_id));

    archive_group(
        &pool,
        ArchiveGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id: group,
        },
        t0,
    )
    .await
    .unwrap();

    let after = member_directory(&pool, viewer, t0).await.unwrap();
    assert!(
        !after
            .sections
            .iter()
            .any(|s| s.section == SectionId::Group(group)),
        "an archived group is not a directory section"
    );
    assert!(
        !after
            .people
            .iter()
            .any(|p| p.membership_id == solo.membership_id),
        "a person listed only through the archived group disappears"
    );
    // The FAU-wide section is unaffected: the admin is still there.
    let fau_section = after
        .sections
        .iter()
        .find(|s| s.section == SectionId::Fau)
        .unwrap();
    assert!(fau_section.members.contains(&fau.admin_membership_id));

    // The group's content stays readable through the group reads (Ruling R13/Q6): the
    // group itself and its member list are still there for an admin.
    let still_there = get_group(&pool, viewer, group, t0).await.unwrap();
    assert!(still_there.archived);
    let members = list_group_members(&pool, viewer, group, t0).await.unwrap();
    assert!(members
        .iter()
        .any(|m| m.membership_id == solo.membership_id));
}
