//! The whole chain against real Postgres and a real OpenBao (docs/key-service-design.md §7).

mod common;
use std::sync::Arc;
use std::time::Duration;

use common::membership::*;
use common::TestDb;
use fau_crypto::{decrypt, encrypt, Aad, Unit};
use fau_crypto::{Ciphertext, DataKey};
use fau_domain::directory::address::{CopySeparator, Mailto, RecipientField, Recipients};
use fau_domain::directory::collation::NameCollator;
use fau_domain::directory::listing::{arrange, Person, Section, ShownName};
use fau_domain::email::Email;
use fau_domain::membership::vocabulary::{CapabilityClass, DisplayName, GroupName, Visibility};
use fau_keys::{data_key, Auth, DataKeyError, KeyCache, KeyError, Keys, KeysConfig};
use fau_persistence::membership::*;
use jiff::SignedDuration;
use uuid::Uuid;

async fn keys() -> Arc<Keys> {
    Arc::new(
        Keys::connect(KeysConfig {
            address: std::env::var("TEST_OPENBAO_ADDR").unwrap(),
            ca_cert_path: None,
            auth: Auth::Token("dev-only-app".into()),
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap(),
    )
}

/// Stands in for the deletion job (Task 8): soft-delete every key of the FAU.
async fn soft_delete_fau(tenant: Uuid) {
    let addr = std::env::var("TEST_OPENBAO_ADDR").unwrap();
    let root = std::env::var("TEST_OPENBAO_TOKEN").unwrap();
    for name in [
        Unit::Record { tenant }.key_name(),
        Unit::Messages { tenant }.key_name(),
    ] {
        let s = reqwest::Client::new()
            .delete(format!("{addr}/v1/transit/keys/{name}/soft-delete"))
            .header("X-Vault-Token", &root)
            .send()
            .await
            .unwrap()
            .status();
        assert!(s.is_success(), "{name}");
    }
}

#[tokio::test]
async fn records_messages_and_shredding_work_end_to_end() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let keys = keys().await;
    let cache = KeyCache::new(
        keys.clone(),
        Arc::new(jiff::Timestamp::now),
        SignedDuration::from_mins(30),
    );
    let record = Unit::Record {
        tenant: fau.tenant_id,
    };

    // Envelope: the first session creates and stores the wrapped key; a second session unwraps it.
    let s1 = Uuid::now_v7();
    let k1 = {
        let mut c = pool.acquire().await.unwrap();
        data_key(&mut c, &keys, &cache, s1, &record).await.unwrap()
    };
    let row = Uuid::now_v7();
    let aad = Aad::new(fau.tenant_id, "groups", "encrypted_name", row);
    let ct = encrypt(&k1, &aad, "Juleballkomiteen").unwrap();
    let s2 = Uuid::now_v7();
    let k2 = {
        let mut c = pool.acquire().await.unwrap();
        data_key(&mut c, &keys, &cache, s2, &record).await.unwrap()
    };
    assert_eq!(
        decrypt(&k2, &aad, &ct).unwrap().as_str(),
        "Juleballkomiteen"
    );

    // No usable key material in Postgres: the plaintext data key appears nowhere in wrapped_keys.
    let stored: Vec<u8> =
        sqlx::query_scalar("select wrapped_key from wrapped_keys where tenant_id = $1")
            .bind(fau.tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!stored.windows(32).any(|w| w == k1.expose()));
    assert!(stored.starts_with(b"vault:v1:"));

    // An access-request message, encrypted with no session, read on the approval screen.
    let request_id = Uuid::now_v7();
    let (t, c) = ACCESS_REQUEST_MESSAGE_AAD;
    let msg_aad = Aad::new(fau.tenant_id, t, c, request_id);
    let sealed = keys
        .encrypt_message(fau.tenant_id, &msg_aad, "Jeg vil bli med i FAU")
        .await
        .unwrap();
    create_access_request(
        &pool,
        CreateAccessRequest {
            tenant_id: fau.tenant_id,
            requester: verified("ny@example.test"),
            message: Some(AccessRequestMessage {
                request_id,
                ciphertext: sealed,
            }),
        },
        t0,
    )
    .await
    .unwrap();
    let stored = access_request_message(
        &pool,
        fau.tenant_id,
        request_id,
        fau.admin_membership_id,
        t0,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        keys.decrypt_message(fau.tenant_id, &msg_aad, &stored)
            .await
            .unwrap()
            .as_str(),
        "Jeg vil bli med i FAU"
    );

    // Shredding: with the FAU's keys soft-deleted, a new session can read nothing.
    soft_delete_fau(fau.tenant_id).await;
    cache.release_session(s1);
    cache.release_session(s2);
    let s3 = Uuid::now_v7();
    let mut c = pool.acquire().await.unwrap();
    assert!(matches!(
        data_key(&mut c, &keys, &cache, s3, &record).await,
        Err(DataKeyError::Keys(KeyError::NotFound))
    ));
    assert_eq!(
        keys.decrypt_message(fau.tenant_id, &msg_aad, &stored)
            .await
            .unwrap_err(),
        KeyError::NotFound
    );
}

/// Groups design §3.1: a group's name is content, encrypted under the FAU's record key and
/// bound to its own row. Nothing readable reaches Postgres.
#[tokio::test]
async fn group_names_are_encrypted_under_the_record_key_and_bound_to_their_row() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let keys = keys().await;
    let cache = KeyCache::new(
        keys.clone(),
        Arc::new(jiff::Timestamp::now),
        SignedDuration::from_mins(30),
    );
    let record = Unit::Record {
        tenant: fau.tenant_id,
    };
    let session = Uuid::now_v7();
    let key = {
        let mut c = pool.acquire().await.unwrap();
        data_key(&mut c, &keys, &cache, session, &record)
            .await
            .unwrap()
    };

    let name = GroupName::parse("Oppfølging av sak med rektor").unwrap();
    let group_id = Uuid::now_v7();
    let (table, column) = GROUP_NAME_AAD;
    let aad = Aad::new(fau.tenant_id, table, column, group_id);
    create_group(
        &pool,
        CreateGroup {
            tenant_id: fau.tenant_id,
            actor_membership_id: fau.admin_membership_id,
            group_id,
            encrypted_name: encrypt(&key, &aad, name.as_str()).unwrap(),
            visibility: Visibility::Closed,
            binding: None,
        },
        t0,
    )
    .await
    .unwrap();

    let stored: Vec<u8> = sqlx::query_scalar("select encrypted_name from groups where id = $1")
        .bind(group_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let plain = name.as_str().as_bytes();
    assert!(!stored.windows(plain.len()).any(|w| w == plain));
    assert!(!stored.windows(6).any(|w| w == b"rektor"));

    let viewer = Viewer {
        tenant_id: fau.tenant_id,
        membership_id: fau.admin_membership_id,
    };
    let listed = list_groups(&pool, viewer, t0).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        decrypt(&key, &aad, &listed[0].encrypted_name)
            .unwrap()
            .as_str(),
        name.as_str()
    );
    // Bound to its row: the same ciphertext under another group's id does not open.
    let elsewhere = Aad::new(fau.tenant_id, table, column, Uuid::now_v7());
    assert!(decrypt(&key, &elsewhere, &listed[0].encrypted_name).is_err());
}

fn seal(
    key: &DataKey,
    tenant: Uuid,
    aad: (&'static str, &'static str),
    row: Uuid,
    text: &str,
) -> Ciphertext {
    encrypt(key, &Aad::new(tenant, aad.0, aad.1, row), text).unwrap()
}

fn open(
    key: &DataKey,
    tenant: Uuid,
    aad: (&'static str, &'static str),
    row: Uuid,
    ct: &Ciphertext,
) -> String {
    decrypt(key, &Aad::new(tenant, aad.0, aad.1, row), ct)
        .unwrap()
        .as_str()
        .to_owned()
}

/// Groups design §4 end to end (#3502): names and contact addresses enter under the record
/// key bound to their membership, nothing readable reaches Postgres, and the session turns
/// the directory into an ordered listing and a `mailto:` link.
#[tokio::test]
async fn the_directory_end_to_end_under_the_record_key() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let t0 = at(T0);
    let fau = active_fau(&pool, "admin@example.test", t0).await;
    let tenant = fau.tenant_id;
    let keys = keys().await;
    let cache = KeyCache::new(
        keys.clone(),
        Arc::new(jiff::Timestamp::now),
        SignedDuration::from_mins(30),
    );
    let key = {
        let mut c = pool.acquire().await.unwrap();
        data_key(
            &mut c,
            &keys,
            &cache,
            Uuid::now_v7(),
            &Unit::Record { tenant },
        )
        .await
        .unwrap()
    };

    // The admin names themself; two people join with names, one with a contact address.
    set_display_name(
        &pool,
        SetDisplayName {
            tenant_id: tenant,
            actor_membership_id: fau.admin_membership_id,
            membership_id: fau.admin_membership_id,
            encrypted_display_name: seal(
                &key,
                tenant,
                DISPLAY_NAME_AAD,
                fau.admin_membership_id,
                "Åse Admin",
            ),
        },
        t0,
    )
    .await
    .unwrap();
    let mut joined = Vec::new();
    for (address, name, contact) in [
        (
            "kari@example.test",
            "Kari Nordmann",
            Some("kari.privat@example.no"),
        ),
        ("oystein@example.test", "Øystein", None),
    ] {
        let token = issue_invitation(
            &pool,
            IssueInvitation {
                tenant_id: tenant,
                actor_membership_id: fau.admin_membership_id,
                recipient: email(address),
                roles: vec![OfferedRole {
                    role: new_role("Medlem", CapabilityClass::Member),
                    period: period(day(2026, 9, 1), day(2027, 9, 1)),
                }],
                handover_grant_id: None,
                message: None,
            },
            t0,
        )
        .await
        .unwrap()
        .token
        .expose()
        .to_owned();
        let target = prepare_acceptance(&pool, &token, &verified(address), t0)
            .await
            .unwrap();
        let m = target.membership_id;
        let name = DisplayName::parse(name).unwrap();
        accept_invitation(
            &pool,
            AcceptInvitation {
                token,
                acceptor: verified(address),
                admin_end_override: None,
                profile: MemberProfile {
                    membership_id: m,
                    encrypted_display_name: seal(&key, tenant, DISPLAY_NAME_AAD, m, name.as_str()),
                    encrypted_contact_email: contact.map(|c| {
                        seal(
                            &key,
                            tenant,
                            CONTACT_EMAIL_AAD,
                            m,
                            Email::parse(c).unwrap().as_str(),
                        )
                    }),
                },
            },
            t0,
        )
        .await
        .unwrap();
        joined.push(m);
    }
    let (kari, oystein) = (joined[0], joined[1]);

    // Nothing readable in Postgres.
    let stored: Vec<Vec<u8>> = sqlx::query_scalar(
        "select coalesce(encrypted_display_name, '') || coalesce(encrypted_contact_email, '')
           from memberships where tenant_id = $1",
    )
    .bind(tenant)
    .fetch_all(&pool)
    .await
    .unwrap();
    for bytes in &stored {
        for plain in ["Kari", "Admin", "privat", "ystein"] {
            assert!(
                !bytes.windows(plain.len()).any(|w| w == plain.as_bytes()),
                "{plain}"
            );
        }
    }

    // The session decrypts the directory and orders it for Bokmål: Æ, Ø, Å after Z.
    let viewer = Viewer {
        tenant_id: tenant,
        membership_id: kari,
    };
    let read = member_directory(&pool, viewer, t0).await.unwrap();
    let people: Vec<Person> = read
        .people
        .iter()
        .map(|p| Person {
            membership_id: p.membership_id,
            name: match &p.name {
                MemberName::Named(ct) => ShownName::Named(
                    DisplayName::parse(&open(&key, tenant, DISPLAY_NAME_AAD, p.membership_id, ct))
                        .unwrap(),
                ),
                _ => ShownName::Unnamed,
            },
            address: match &p.address {
                DirectoryAddress::Contact(ct) => {
                    Email::parse(&open(&key, tenant, CONTACT_EMAIL_AAD, p.membership_id, ct))
                        .unwrap()
                }
                DirectoryAddress::Login(e) => e.clone(),
            },
            is_guest: p.is_guest,
            roles: p.roles.clone(),
            groups: p.group_ids.clone(),
        })
        .collect();
    let sections: Vec<Section> = read
        .sections
        .iter()
        .map(|s| Section {
            id: s.section,
            title: None,
            members: s.members.clone(),
        })
        .collect();
    let listing = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
    assert_eq!(
        listing.sections[0].members,
        [kari, oystein, fau.admin_membership_id]
    );

    // "Skriv e-post" for everyone: the audited export, decrypted, becomes one link.
    let exported = export_addresses(
        &pool,
        viewer,
        AddressExport {
            purpose: ExportPurpose::Mailto,
            scope: ExportScope::Fau,
            membership_ids: listing.sections[0].members.clone(),
        },
        t0,
    )
    .await
    .unwrap();
    let recipients = Recipients::new(exported.iter().map(|r| match &r.address {
        DirectoryAddress::Contact(ct) => {
            Email::parse(&open(&key, tenant, CONTACT_EMAIL_AAD, r.membership_id, ct)).unwrap()
        }
        DirectoryAddress::Login(e) => e.clone(),
    }));
    assert_eq!(recipients.default_field(), RecipientField::To);
    assert_eq!(
        recipients.mailto(RecipientField::To),
        Mailto::Link(
            "mailto:kari.privat@example.no,oystein@example.test,admin@example.test".into()
        )
    );
    assert_eq!(
        recipients.copy_text(CopySeparator::Comma),
        "kari.privat@example.no, oystein@example.test, admin@example.test"
    );

    // Bound to its row: Kari's name does not open as Øystein's.
    let kari_name = match &read
        .people
        .iter()
        .find(|p| p.membership_id == kari)
        .unwrap()
        .name
    {
        MemberName::Named(ct) => ct.clone(),
        other => panic!("{other:?}"),
    };
    let (t, c) = DISPLAY_NAME_AAD;
    assert!(decrypt(&key, &Aad::new(tenant, t, c, oystein), &kari_name).is_err());
    let (t, c) = CONTACT_EMAIL_AAD;
    assert!(decrypt(&key, &Aad::new(tenant, t, c, kari), &kari_name).is_err());
}
