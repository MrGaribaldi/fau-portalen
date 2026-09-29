//! How history shows someone whose membership has ended (Erik's D3, 28 September 2026).
//!
//! A display name exists only while the membership is active. Once it ends, history -- an
//! author, a minute-taker, an audit line -- shows the role the person held and its years,
//! "Leder 2025–2026", never a name. Persistence (`member_names`) hands over the roles the
//! membership actually held, read from `role_assignments` when history is rendered, and
//! [`role_label`] picks the one that fits the moment being shown.
//!
//! **The rule** (Rulings D3-3 and D3-5):
//! - the role held on the day of the event; among several held that day, an admin-class
//!   role before a member-class one before a guest's, then the one that started first;
//! - an event after every role ended shows the last role that ended, and an event before any
//!   role began shows the first one to begin;
//! - one role only, never a list, and never the unit or cohort it sits on, since a place
//!   narrows a role down to a person;
//! - the years are the calendar years of the days the assignment was actually held, from
//!   its first day to its last.
//!
//! The screen renders a [`RoleLabel`] through the catalogue (#3439), never as prose built
//! here. The proposed Bokmål source strings are `member.endedRole.oneYear` = "{role} {year}"
//! and `member.endedRole.years` = "{role} {firstYear}–{lastYear}", with the years passed as
//! strings so ICU does not group their digits.
//!
//! **A returner** (#3511, Erik's M1): someone active again after an earlier period is shown
//! by name only for events in the current period; earlier events keep their role and year
//! ([`active_period`]).

use std::fmt;

use jiff::civil::Date;

use crate::membership::vocabulary::CapabilityClass;

/// One role a membership held, over the days it was actually held: from `from` up to, but
/// not including, `until`. An assignment revoked early ends where it was revoked. Built
/// only for non-empty spans, so `from < until`. `Debug` is hand-written: `name` is a role
/// name, which is never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct HeldRole {
    pub name: String,
    pub class: CapabilityClass,
    pub from: Date,
    pub until: Date,
}

impl fmt::Debug for HeldRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeldRole")
            .field("name", &"[redacted]")
            .field("class", &self.class)
            .field("from", &self.from)
            .field("until", &self.until)
            .finish()
    }
}

/// What history shows for an ended membership: a role and its years. `Debug` is
/// hand-written for the same reason as [`HeldRole`]'s.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleLabel {
    pub role: String,
    pub first_year: i16,
    pub last_year: i16,
}

impl RoleLabel {
    /// Whether the catalogue's one-year form applies ("Kasserer 2026") rather than the span
    /// ("Leder 2025–2026").
    pub fn is_one_year(&self) -> bool {
        self.first_year == self.last_year
    }
}

impl fmt::Debug for RoleLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoleLabel")
            .field("role", &"[redacted]")
            .field("first_year", &self.first_year)
            .field("last_year", &self.last_year)
            .finish()
    }
}

fn rank(class: CapabilityClass) -> u8 {
    match class {
        CapabilityClass::Admin => 0,
        CapabilityClass::Member => 1,
        CapabilityClass::Guest => 2,
    }
}

/// The label for an event on `on`, from the roles an ended membership held. `None` when it
/// held none, which the screen shows as "Tidligere medlem".
pub fn role_label(held: &[HeldRole], on: Date) -> Option<RoleLabel> {
    let during = held
        .iter()
        .filter(|r| r.from <= on && on < r.until)
        .min_by_key(|r| (rank(r.class), r.from));
    let after = || {
        held.iter()
            .filter(|r| r.until <= on)
            .min_by_key(|r| (std::cmp::Reverse(r.until), rank(r.class), r.from))
    };
    let before = || held.iter().min_by_key(|r| (r.from, rank(r.class)));
    let chosen = during.or_else(after).or_else(before)?;
    let last_day = chosen.until.yesterday().unwrap_or(chosen.from);
    Some(RoleLabel {
        role: chosen.name.clone(),
        first_year: chosen.from.year(),
        last_year: last_day.year(),
    })
}

/// The day a membership ended (#3511, plan Ruling R2): the latest day any of its roles
/// stopped being held, never later than `today`. A membership revoked long after its roles
/// ran out ended when they ran out. `today` when it held no role at all.
pub fn ended_on(held: &[HeldRole], today: Date) -> Date {
    held.iter()
        .map(|r| r.until)
        .max()
        .map_or(today, |d| d.min(today))
}

/// An active membership's history split at its current period (#3511, M1): the period
/// started on `since`, and `earlier` holds the roles of every earlier one. Most members
/// have nothing earlier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePeriod {
    pub since: Date,
    pub earlier: Vec<HeldRole>,
}

impl ActivePeriod {
    /// How an event on `on` shows the person: `None` for their name, or the role and year
    /// from an earlier period.
    pub fn label_on(&self, on: Date) -> Option<RoleLabel> {
        if on >= self.since {
            None
        } else {
            role_label(&self.earlier, on)
        }
    }
}

/// Splits an active membership's roles at its current period (plan Rulings R5, R6).
/// Periods are runs of held spans that touch or overlap; a gap of a day or more starts a
/// new one. The current period is the last run, counted from today if it is still to
/// start.
pub fn active_period(held: &[HeldRole], today: Date) -> ActivePeriod {
    let mut spans: Vec<(Date, Date)> = held.iter().map(|r| (r.from, r.until)).collect();
    spans.sort();
    let mut current: Option<(Date, Date)> = None;
    for (from, until) in spans {
        current = match current {
            Some((f, u)) if from <= u => Some((f, u.max(until))),
            _ => Some((from, until)),
        };
    }
    let start = current.map_or(today, |(from, _)| from);
    ActivePeriod {
        since: start.min(today),
        earlier: held.iter().filter(|r| r.until < start).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn held(name: &str, class: CapabilityClass, from: Date, until: Date) -> HeldRole {
        HeldRole {
            name: name.to_owned(),
            class,
            from,
            until,
        }
    }

    fn shown(held: &[HeldRole], on: Date) -> Option<(String, i16, i16)> {
        role_label(held, on).map(|l| (l.role, l.first_year, l.last_year))
    }

    fn two_years() -> Vec<HeldRole> {
        vec![
            held(
                "Kasserer",
                CapabilityClass::Member,
                date(2024, 9, 1),
                date(2025, 9, 1),
            ),
            held(
                "Leder",
                CapabilityClass::Admin,
                date(2025, 9, 1),
                date(2026, 9, 1),
            ),
        ]
    }

    #[test]
    fn the_role_held_on_the_day_is_shown_with_its_years() {
        let h = two_years();
        assert_eq!(
            shown(&h, date(2025, 1, 10)),
            Some(("Kasserer".into(), 2024, 2025))
        );
        assert_eq!(
            shown(&h, date(2026, 3, 1)),
            Some(("Leder".into(), 2025, 2026))
        );
        // The first day belongs to the new role, the day before to the old one.
        assert_eq!(
            shown(&h, date(2025, 9, 1)),
            Some(("Leder".into(), 2025, 2026))
        );
        assert_eq!(
            shown(&h, date(2025, 8, 31)),
            Some(("Kasserer".into(), 2024, 2025))
        );
    }

    #[test]
    fn outside_every_role_the_nearest_end_or_the_first_start_is_shown() {
        let h = two_years();
        assert_eq!(
            shown(&h, date(2027, 1, 1)),
            Some(("Leder".into(), 2025, 2026)),
            "after the end: the last role that ended"
        );
        assert_eq!(
            shown(&h, date(2024, 1, 1)),
            Some(("Kasserer".into(), 2024, 2025)),
            "before any start: the first role to begin"
        );
    }

    #[test]
    fn on_one_day_an_admin_role_wins_then_the_earliest_start() {
        let h = vec![
            held(
                "Kontaktforelder",
                CapabilityClass::Member,
                date(2025, 8, 1),
                date(2026, 8, 1),
            ),
            held(
                "Leder",
                CapabilityClass::Admin,
                date(2025, 9, 1),
                date(2026, 9, 1),
            ),
            held(
                "Dugnadsansvarlig",
                CapabilityClass::Member,
                date(2025, 7, 1),
                date(2026, 7, 1),
            ),
        ];
        assert_eq!(shown(&h, date(2025, 10, 1)).unwrap().0, "Leder");
        assert_eq!(
            shown(&h, date(2025, 8, 15)).unwrap().0,
            "Dugnadsansvarlig",
            "two member roles: the one that started first"
        );
    }

    #[test]
    fn a_role_within_one_calendar_year_has_one_year() {
        let whole = [held(
            "Sekretær",
            CapabilityClass::Member,
            date(2026, 1, 1),
            date(2027, 1, 1),
        )];
        let label = role_label(&whole, date(2026, 6, 1)).unwrap();
        assert_eq!((label.first_year, label.last_year), (2026, 2026));
        assert!(label.is_one_year());
        let short = [held(
            "Medlem",
            CapabilityClass::Member,
            date(2026, 9, 1),
            date(2026, 10, 1),
        )];
        assert!(role_label(&short, date(2026, 9, 2)).unwrap().is_one_year());
        assert!(!role_label(&two_years(), date(2026, 3, 1))
            .unwrap()
            .is_one_year());
    }

    #[test]
    fn nothing_held_is_no_label() {
        assert_eq!(shown(&[], date(2026, 1, 1)), None);
    }

    #[test]
    fn debug_never_prints_a_role_name() {
        let h = two_years();
        let label = role_label(&h, date(2026, 3, 1)).unwrap();
        assert!(!format!("{h:?}").contains("Leder"));
        assert!(!format!("{h:?}").contains("Kasserer"));
        assert!(!format!("{label:?}").contains("Leder"));
    }

    #[test]
    fn a_membership_ended_on_the_last_day_a_role_stopped_never_after_today() {
        let h = two_years(); // Kasserer 2024-09-01..2025-09-01, Leder 2025-09-01..2026-09-01
        assert_eq!(ended_on(&h, date(2026, 10, 15)), date(2026, 9, 1));
        // Revoked the same day a running role was cut short: that role now ends today.
        assert_eq!(ended_on(&h, date(2026, 3, 1)), date(2026, 3, 1));
        assert_eq!(
            ended_on(&[], date(2026, 3, 1)),
            date(2026, 3, 1),
            "held nothing"
        );
    }

    #[test]
    fn one_unbroken_run_of_roles_is_one_period_with_nothing_earlier() {
        let p = active_period(&two_years(), date(2026, 3, 1));
        assert_eq!(p.since, date(2024, 9, 1));
        assert!(p.earlier.is_empty());
        assert_eq!(p.label_on(date(2025, 1, 1)), None, "the name, all through");
    }

    #[test]
    fn after_a_gap_the_current_period_starts_again_and_earlier_events_show_role_and_year() {
        // Kasserer 2024-09-01..2025-09-01, then nothing, then a guest from 2026-02-01.
        let mut h = vec![two_years().remove(0)];
        h.push(held(
            "Gjest",
            CapabilityClass::Guest,
            date(2026, 2, 1),
            date(2026, 6, 1),
        ));
        let p = active_period(&h, date(2026, 3, 1));
        assert_eq!(p.since, date(2026, 2, 1));
        assert_eq!(p.earlier.len(), 1);
        assert_eq!(
            p.label_on(date(2025, 3, 1))
                .map(|l| (l.role, l.first_year, l.last_year)),
            Some(("Kasserer".into(), 2024, 2025))
        );
        assert_eq!(
            p.label_on(date(2026, 2, 1)),
            None,
            "the first day of the new period"
        );
        assert_eq!(p.label_on(date(2026, 3, 1)), None);
    }

    #[test]
    fn roles_that_touch_or_overlap_are_one_period_and_a_one_day_gap_splits() {
        let touching = [
            held(
                "A",
                CapabilityClass::Member,
                date(2025, 1, 1),
                date(2025, 6, 1),
            ),
            held(
                "B",
                CapabilityClass::Member,
                date(2025, 6, 1),
                date(2026, 6, 1),
            ),
        ];
        assert_eq!(
            active_period(&touching, date(2026, 1, 1)).since,
            date(2025, 1, 1)
        );
        let overlapping = [
            held(
                "A",
                CapabilityClass::Member,
                date(2025, 1, 1),
                date(2025, 9, 1),
            ),
            held(
                "B",
                CapabilityClass::Member,
                date(2025, 6, 1),
                date(2026, 6, 1),
            ),
        ];
        assert_eq!(
            active_period(&overlapping, date(2026, 1, 1)).since,
            date(2025, 1, 1)
        );
        let gap = [
            held(
                "A",
                CapabilityClass::Member,
                date(2025, 1, 1),
                date(2025, 6, 1),
            ),
            held(
                "B",
                CapabilityClass::Member,
                date(2025, 6, 2),
                date(2026, 6, 1),
            ),
        ];
        let p = active_period(&gap, date(2026, 1, 1));
        assert_eq!(p.since, date(2025, 6, 2));
        assert_eq!(p.earlier.len(), 1);
    }

    #[test]
    fn a_period_still_to_start_counts_from_today() {
        // Returned by invitation today for a role that starts next month: what they do
        // today is theirs, what they did in the old period is not.
        let h = [
            held(
                "A",
                CapabilityClass::Member,
                date(2025, 1, 1),
                date(2025, 6, 1),
            ),
            held(
                "B",
                CapabilityClass::Member,
                date(2026, 4, 1),
                date(2027, 4, 1),
            ),
        ];
        let p = active_period(&h, date(2026, 3, 1));
        assert_eq!(p.since, date(2026, 3, 1));
        assert!(p.label_on(date(2025, 3, 1)).is_some());
        assert_eq!(p.label_on(date(2026, 3, 1)), None);
    }
}
