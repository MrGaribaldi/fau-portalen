//! The directory as the screen shows it (groups design §4.3), after persistence has
//! authorized and read it and the session has decrypted names and addresses.
//!
//! [`arrange`] puts it in order: the FAU-wide section first, then groups by name; people in
//! every section by name; each person's roles by name. Every comparison of names is the
//! viewer's locale's collation ([`NameCollator`]), never byte order, and ties fall back to
//! the id so the order is stable. A person listed in two sections is one [`Person`].

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;

use uuid::Uuid;

use super::collation::NameCollator;
use crate::email::Email;
use crate::membership::vocabulary::DisplayName;

/// One heading of the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionId {
    /// Every current member and admin: "Hele FAU". Never shown to a guest, and never lists
    /// one (§3.2: an FAU-wide audience never includes guests).
    Fau,
    Group(Uuid),
}

/// One heading of the directory. `Debug` is hand-written: `title` is a decrypted group
/// name, so it must never appear in an error, log line or test failure message
/// (`implementer-rules.md`).
#[derive(Clone, PartialEq, Eq)]
pub struct Section {
    pub id: SectionId,
    /// The decrypted group name; `None` for [`SectionId::Fau`], whose heading the screen
    /// takes from the catalogue.
    pub title: Option<String>,
    pub members: Vec<Uuid>,
}

impl fmt::Debug for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Section")
            .field("id", &self.id)
            .field("title", &self.title.as_ref().map(|_| "[redacted]"))
            .field("members", &self.members)
            .finish()
    }
}

/// A name as the directory shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShownName {
    Named(DisplayName),
    /// A membership created before migration 0008, which has no name. Sorted last. Bokmål
    /// source string for the catalogue (#3439): "Navn ikke oppgitt".
    Unnamed,
}

/// Where a role sits in the school structure, when it sits anywhere (plaintext, ADR-003
/// decision 6). `Debug` is hand-written: both variants hold a decrypted unit or cohort
/// name.
#[derive(Clone, PartialEq, Eq)]
pub enum Place {
    Unit(String),
    Cohort(String),
}

impl fmt::Debug for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::Unit(_) => f.write_str("Place::Unit([redacted])"),
            Place::Cohort(_) => f.write_str("Place::Cohort([redacted])"),
        }
    }
}

/// One role held today, as "what a person represents" shows it: the role's name, plus the
/// unit or cohort it sits on (§4.1). Past roles are never listed. `Debug` is hand-written:
/// `name` is a decrypted role name.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleHeld {
    pub name: String,
    pub place: Option<Place>,
}

impl fmt::Debug for RoleHeld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoleHeld")
            .field("name", &"[redacted]")
            .field("place", &self.place)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub membership_id: Uuid,
    pub name: ShownName,
    /// The contact address, or the login address when the member set none (D3).
    pub address: Email,
    /// Marked "Gjest" on the screen (Bokmål source string for the catalogue).
    pub is_guest: bool,
    pub roles: Vec<RoleHeld>,
    /// The listed groups this person is in, in section order.
    pub groups: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub sections: Vec<Section>,
    /// Every `Person` the caller passed in, once (deduplicated by `membership_id`), in
    /// name order. Not filtered to people who appear in a `Section`: it is the caller's
    /// job to pass exactly the people it wants described.
    pub people: Vec<Person>,
}

fn compare_names(c: &NameCollator, a: &ShownName, b: &ShownName) -> Ordering {
    match (a, b) {
        (ShownName::Named(a), ShownName::Named(b)) => c.compare(a.as_str(), b.as_str()),
        (ShownName::Named(_), ShownName::Unnamed) => Ordering::Less,
        (ShownName::Unnamed, ShownName::Named(_)) => Ordering::Greater,
        (ShownName::Unnamed, ShownName::Unnamed) => Ordering::Equal,
    }
}

fn place_text(p: &Option<Place>) -> &str {
    match p {
        None => "",
        Some(Place::Unit(s) | Place::Cohort(s)) => s,
    }
}

/// Orders the directory for one viewer's locale. Repeated people and repeated section
/// members are dropped, keeping the first; a section member with no [`Person`] is dropped
/// too, so a section can never list someone the people list does not describe.
pub fn arrange(sections: Vec<Section>, people: Vec<Person>, collator: &NameCollator) -> Listing {
    let mut seen = HashSet::new();
    let mut people: Vec<Person> = people
        .into_iter()
        .filter(|p| seen.insert(p.membership_id))
        .collect();
    people.sort_by(|a, b| {
        compare_names(collator, &a.name, &b.name).then(a.membership_id.cmp(&b.membership_id))
    });
    let rank: HashMap<Uuid, usize> = people
        .iter()
        .enumerate()
        .map(|(i, p)| (p.membership_id, i))
        .collect();

    let mut sections = sections;
    sections.sort_by(|a, b| match (a.id, b.id) {
        (SectionId::Fau, SectionId::Fau) => Ordering::Equal,
        (SectionId::Fau, SectionId::Group(_)) => Ordering::Less,
        (SectionId::Group(_), SectionId::Fau) => Ordering::Greater,
        (SectionId::Group(x), SectionId::Group(y)) => collator
            .compare(
                a.title.as_deref().unwrap_or(""),
                b.title.as_deref().unwrap_or(""),
            )
            .then(x.cmp(&y)),
    });
    for s in &mut sections {
        let mut in_section = HashSet::new();
        s.members
            .retain(|m| rank.contains_key(m) && in_section.insert(*m));
        s.members.sort_by_key(|m| rank[m]);
    }
    let group_rank: HashMap<Uuid, usize> = sections
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s.id {
            SectionId::Group(g) => Some((g, i)),
            SectionId::Fau => None,
        })
        .collect();

    for p in &mut people {
        p.roles.sort_by(|a, b| {
            collator
                .compare(&a.name, &b.name)
                .then_with(|| collator.compare(place_text(&a.place), place_text(&b.place)))
        });
        let mut in_person = HashSet::new();
        p.groups
            .retain(|g| group_rank.contains_key(g) && in_person.insert(*g));
        p.groups.sort_by_key(|g| group_rank[g]);
    }
    Listing { sections, people }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn person(n: u128, name: Option<&str>, groups: &[u128]) -> Person {
        Person {
            membership_id: id(n),
            name: match name {
                Some(s) => ShownName::Named(DisplayName::parse(s).unwrap()),
                None => ShownName::Unnamed,
            },
            address: Email::parse(&format!("p{n}@example.no")).unwrap(),
            is_guest: false,
            roles: Vec::new(),
            groups: groups.iter().map(|g| id(*g)).collect(),
        }
    }

    fn section(sid: SectionId, title: Option<&str>, members: &[u128]) -> Section {
        Section {
            id: sid,
            title: title.map(str::to_owned),
            members: members.iter().map(|m| id(*m)).collect(),
        }
    }

    fn names(l: &Listing, s: usize) -> Vec<String> {
        l.sections[s]
            .members
            .iter()
            .map(|m| {
                match &l
                    .people
                    .iter()
                    .find(|p| p.membership_id == *m)
                    .unwrap()
                    .name
                {
                    ShownName::Named(n) => n.as_str().to_owned(),
                    ShownName::Unnamed => "-".to_owned(),
                }
            })
            .collect()
    }

    #[test]
    fn a_person_in_two_groups_is_one_person_listed_in_both() {
        let people = vec![
            person(1, Some("Åse"), &[100, 101]),
            person(2, Some("Berit"), &[100]),
            person(1, Some("Åse"), &[100, 101]),
        ];
        let sections = vec![
            section(SectionId::Group(id(101)), Some("Styret"), &[1]),
            section(SectionId::Group(id(100)), Some("Dugnad"), &[1, 2, 1]),
        ];
        let l = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
        assert_eq!(l.people.len(), 2);
        assert_eq!(names(&l, 0), ["Berit", "Åse"], "Dugnad sorts before Styret");
        assert_eq!(names(&l, 1), ["Åse"]);
        let aase = l.people.iter().find(|p| p.membership_id == id(1)).unwrap();
        assert_eq!(aase.groups, [id(100), id(101)], "in section order");
    }

    #[test]
    fn names_follow_the_viewers_locale_and_unnamed_sort_last() {
        let people = vec![
            person(1, Some("Åse"), &[]),
            person(2, None, &[]),
            person(3, Some("Øvre"), &[]),
            person(4, Some("Anders"), &[]),
        ];
        let sections = vec![section(SectionId::Fau, None, &[1, 2, 3, 4])];
        let nb = arrange(
            sections.clone(),
            people.clone(),
            &NameCollator::for_locale("nb-NO"),
        );
        assert_eq!(names(&nb, 0), ["Anders", "Øvre", "Åse", "-"]);
        let en = arrange(sections, people, &NameCollator::for_locale("en"));
        assert_eq!(names(&en, 0), ["Anders", "Åse", "Øvre", "-"]);
    }

    #[test]
    fn the_fau_section_comes_first_and_groups_follow_by_name() {
        let people = vec![person(1, Some("Kari"), &[])];
        let sections = vec![
            section(SectionId::Group(id(10)), Some("Årsmøtekomiteen"), &[1]),
            section(SectionId::Group(id(11)), Some("Øvingsgruppa"), &[1]),
            section(SectionId::Fau, None, &[1]),
            section(SectionId::Group(id(12)), Some("Dugnad"), &[1]),
        ];
        let l = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
        let order: Vec<SectionId> = l.sections.iter().map(|s| s.id).collect();
        assert_eq!(
            order,
            [
                SectionId::Fau,
                SectionId::Group(id(12)),
                SectionId::Group(id(11)),
                SectionId::Group(id(10)),
            ]
        );
    }

    #[test]
    fn a_section_never_lists_someone_the_people_list_does_not_describe() {
        let l = arrange(
            vec![section(SectionId::Fau, None, &[1, 9])],
            vec![person(1, Some("Kari"), &[77])],
            &NameCollator::for_locale("nb-NO"),
        );
        assert_eq!(l.sections[0].members, [id(1)]);
        assert!(l.people[0].groups.is_empty(), "no section for group 77");
    }

    #[test]
    fn roles_are_ordered_by_name_then_place() {
        let mut p = person(1, Some("Kari"), &[]);
        p.roles = vec![
            RoleHeld {
                name: "Kontaktforelder".into(),
                place: Some(Place::Unit("7B".into())),
            },
            RoleHeld {
                name: "Kasserer".into(),
                place: None,
            },
            RoleHeld {
                name: "Kontaktforelder".into(),
                place: Some(Place::Unit("7A".into())),
            },
        ];
        let l = arrange(Vec::new(), vec![p], &NameCollator::for_locale("nb-NO"));
        let shown: Vec<(&str, &str)> = l.people[0]
            .roles
            .iter()
            .map(|r| (r.name.as_str(), place_text(&r.place)))
            .collect();
        assert_eq!(
            shown,
            [
                ("Kasserer", ""),
                ("Kontaktforelder", "7A"),
                ("Kontaktforelder", "7B")
            ]
        );
    }

    /// Section titles, role names and place names are decrypted content (a group name, a
    /// role name, a unit or cohort name): `Debug` must never print them, only a redaction
    /// marker (`implementer-rules.md`, controller ruling Q3). Names chosen to be
    /// recognisable if they leaked.
    #[test]
    fn debug_never_prints_a_decrypted_name() {
        let s = section(
            SectionId::Group(id(1)),
            Some("Oppfølging av sak med rektor"),
            &[],
        );
        let debug = format!("{s:?}");
        assert!(!debug.contains("Oppfølging"));
        assert!(!debug.contains("rektor"));
        assert!(!debug.contains("chars"), "no length side-channel: {debug}");
        assert!(debug.contains("[redacted]"), "{debug}");

        let role = RoleHeld {
            name: "Kontaktforelder".into(),
            place: Some(Place::Unit("Nordbytoppen skole".into())),
        };
        let debug = format!("{role:?}");
        assert!(!debug.contains("Kontaktforelder"));
        assert!(!debug.contains("Nordbytoppen"));
        assert!(!debug.contains("chars"), "no length side-channel: {debug}");

        let place = Place::Cohort("Klasse 7B ved Nordre Skole".into());
        let debug = format!("{place:?}");
        assert!(!debug.contains("Klasse"));
        assert!(!debug.contains("Nordre"));
        assert!(!debug.contains("chars"), "no length side-channel: {debug}");

        // Containers that hold these types must not leak through their own derived
        // `Debug` either: a `Person`'s roles, and a `Listing`'s sections.
        let mut p = person(1, Some("Kari Nordmann"), &[]);
        p.roles = vec![RoleHeld {
            name: "Leder for foreldrekomiteen".into(),
            place: Some(Place::Cohort("Klasse 7B ved Nordre Skole".into())),
        }];
        let debug = format!("{p:?}");
        assert!(!debug.contains("Leder for foreldrekomiteen"));
        assert!(!debug.contains("Nordre Skole"));
        assert!(!debug.contains("Kari"));
        assert!(!debug.contains("chars"), "no length side-channel: {debug}");

        let listing = Listing {
            sections: vec![section(
                SectionId::Group(id(2)),
                Some("Oppfølging av sak med rektor"),
                &[],
            )],
            people: vec![p],
        };
        let debug = format!("{listing:?}");
        assert!(!debug.contains("Oppfølging"));
        assert!(!debug.contains("rektor"));
        assert!(!debug.contains("chars"), "no length side-channel: {debug}");
    }

    /// Deleting `.then(a.membership_id.cmp(&b.membership_id))` from the people sort in
    /// `arrange` leaves every other test green, because they all use names that already
    /// differ. Two people who share a `ShownName` need the id as the tiebreaker to get a
    /// stable order (controller ruling Q9). Mutation-checked: removing that `.then(...)`
    /// makes this test fail (shown, then reverted).
    #[test]
    fn ties_in_name_break_by_membership_id() {
        // Same collation key ("Åse" twice), different ids: the lower id must sort first,
        // deterministically, however `sort_by` happens to compare them.
        let people = vec![person(9, Some("Åse"), &[]), person(2, Some("Åse"), &[])];
        let sections = vec![section(SectionId::Fau, None, &[9, 2])];
        let l = arrange(sections, people, &NameCollator::for_locale("nb-NO"));
        assert_eq!(
            l.people.iter().map(|p| p.membership_id).collect::<Vec<_>>(),
            [id(2), id(9)],
            "the lower membership id must come first when names tie"
        );
    }

    /// Same as above for groups: two `Group` sections sharing a title need the group id as
    /// the tiebreaker. Deleting `.then(x.cmp(&y))` from the section sort in `arrange` leaves
    /// every other test green, because none of them gives two groups the same title.
    /// Mutation-checked: removing that `.then(...)` makes this test fail (shown, then
    /// reverted).
    #[test]
    fn ties_in_group_title_break_by_group_id() {
        let sections = vec![
            section(SectionId::Group(id(9)), Some("Dugnad"), &[]),
            section(SectionId::Group(id(2)), Some("Dugnad"), &[]),
        ];
        let l = arrange(sections, Vec::new(), &NameCollator::for_locale("nb-NO"));
        assert_eq!(
            l.sections.iter().map(|s| s.id).collect::<Vec<_>>(),
            [SectionId::Group(id(2)), SectionId::Group(id(9))],
            "the lower group id must come first when titles tie"
        );
    }
}
