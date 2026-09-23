//! The flow spec's numbers and date arithmetic, in one place so every caller computes
//! them the same way (#3412: "compute and store the boundary explicitly, so the same rule
//! is used everywhere").

use jiff::civil::Date;
use jiff::{SignedDuration, Timestamp, ToSpan};

use super::period::Period;

/// Calendar months a handover grant runs from the admin role's exclusive end (§6.2).
pub const HANDOVER_MONTHS: i32 = 6;
/// A new admin's end date must lie between these many months from today (§3.1.5).
pub const ADMIN_END_MIN_MONTHS: i32 = 1;
pub const ADMIN_END_MAX_MONTHS: i32 = 24;
/// The default end date is the first 1 October at least this many months away (§3.1.5).
pub const ADMIN_END_DEFAULT_LEAD_MONTHS: i32 = 3;
/// A pending FAU not verified within this many days expires (§3.3).
pub const PENDING_SIGNUP_DAYS: i64 = 7;
/// One address may hold at most this many pending FAU-er at a time (§3.3).
pub const MAX_PENDING_SIGNUPS_PER_ADDRESS: i64 = 3;
/// Every invitation is valid for this many days (§5.1).
pub const INVITATION_DAYS: i64 = 14;
/// Recovery notices reach everyone who held a role in this many past months when no
/// member remains (§6.4, ADR-003 decision 10).
pub const RECENT_HOLDER_MONTHS: i32 = 24;

/// The name the registrant's first role carries (§3.4.2). Stored as data, like any role
/// name an admin types; it is identical in Bokmål and English.
pub const BOOTSTRAP_ADMIN_ROLE_NAME: &str = "Administrator";
/// Erik's oversight address: signup collisions (§3.2), activations (§3.4.5) and every
/// recovery (ADR-003 decisions 8 and 10) are copied here.
pub const EWB_OVERSIGHT_ADDRESS: &str = "fau@ewb-solutions.as";

/// Adds calendar months, truncating to the last valid day of the target month (jiff's
/// behaviour for civil dates), saturating at the end of the representable range.
fn add_months(day: Date, months: i32) -> Date {
    day.checked_add(months.months()).unwrap_or(Date::MAX)
}

/// The exclusive end of the handover window for an admin role ending (exclusively) on
/// `admin_ends_on_exclusive`: six calendar months later, truncated to the last valid day
/// of the month. 2027-08-31 gives 2028-02-29.
pub fn handover_boundary(admin_ends_on_exclusive: Date) -> Date {
    add_months(admin_ends_on_exclusive, HANDOVER_MONTHS)
}

/// The handover window itself, `[admin_ends_on_exclusive, boundary)`. `None` only at the
/// very end of the calendar, where there is no later day to end on.
pub fn handover_period(admin_ends_on_exclusive: Date) -> Option<Period> {
    Period::new(
        admin_ends_on_exclusive,
        handover_boundary(admin_ends_on_exclusive),
    )
    .ok()
}

/// The default admin end date offered at signup: the first 1 October at least three
/// months after `today`. The value is an exclusive end, like every stored period end.
pub fn default_admin_end(today: Date) -> Date {
    let earliest = add_months(today, ADMIN_END_DEFAULT_LEAD_MONTHS);
    let this_year = Date::new(earliest.year(), 10, 1).unwrap_or(Date::MAX);
    if this_year >= earliest {
        this_year
    } else {
        Date::new(earliest.year() + 1, 10, 1).unwrap_or(Date::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminEndError {
    /// Earlier than one month after today.
    TooSoon,
    /// Later than 24 months after today.
    TooLate,
}

/// Validates a chosen admin end date (exclusive) against the 1–24 month range, both
/// bounds inclusive.
pub fn validate_admin_end(today: Date, ends_on_exclusive: Date) -> Result<(), AdminEndError> {
    if ends_on_exclusive < add_months(today, ADMIN_END_MIN_MONTHS) {
        return Err(AdminEndError::TooSoon);
    }
    if ends_on_exclusive > add_months(today, ADMIN_END_MAX_MONTHS) {
        return Err(AdminEndError::TooLate);
    }
    Ok(())
}

/// When a pending signup created at `now` expires. An exact duration, not calendar days:
/// a daylight-saving change inside the week does not move the deadline by an hour.
pub fn pending_signup_expiry(now: Timestamp) -> Timestamp {
    now.checked_add(SignedDuration::from_hours(24 * PENDING_SIGNUP_DAYS))
        .unwrap_or(Timestamp::MAX)
}

/// When an invitation issued (or re-sent) at `now` expires.
pub fn invitation_expiry(now: Timestamp) -> Timestamp {
    now.checked_add(SignedDuration::from_hours(24 * INVITATION_DAYS))
        .unwrap_or(Timestamp::MAX)
}

/// The first day of the "held a role in the past 24 months" window.
pub fn recent_holder_window_start(today: Date) -> Date {
    today
        .checked_sub(RECENT_HOLDER_MONTHS.months())
        .unwrap_or(Date::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn handover_boundary_is_six_calendar_months_later() {
        assert_eq!(handover_boundary(date(2027, 8, 1)), date(2028, 2, 1));
        assert_eq!(handover_boundary(date(2027, 10, 1)), date(2028, 4, 1));
    }

    #[test]
    fn handover_boundary_truncates_to_month_end_in_a_leap_year() {
        // The worked example in #3412.
        assert_eq!(handover_boundary(date(2027, 8, 31)), date(2028, 2, 29));
    }

    #[test]
    fn handover_boundary_truncates_to_month_end_outside_a_leap_year() {
        assert_eq!(handover_boundary(date(2026, 8, 31)), date(2027, 2, 28));
        assert_eq!(handover_boundary(date(2027, 12, 31)), date(2028, 6, 30));
        assert_eq!(handover_boundary(date(2028, 2, 29)), date(2028, 8, 29));
    }

    #[test]
    fn handover_period_starts_on_the_exclusive_end() {
        let p = handover_period(date(2027, 10, 1)).unwrap();
        assert_eq!(p.starts_on(), date(2027, 10, 1));
        assert_eq!(p.ends_on_exclusive(), date(2028, 4, 1));
        assert!(handover_period(Date::MAX).is_none());
    }

    #[test]
    fn default_admin_end_is_the_first_october_at_least_three_months_away() {
        assert_eq!(default_admin_end(date(2026, 9, 23)), date(2027, 10, 1));
        assert_eq!(default_admin_end(date(2026, 1, 15)), date(2026, 10, 1));
        assert_eq!(default_admin_end(date(2026, 6, 30)), date(2026, 10, 1));
        // Exactly three months away still counts as "at least three months".
        assert_eq!(default_admin_end(date(2026, 7, 1)), date(2026, 10, 1));
        assert_eq!(default_admin_end(date(2026, 7, 2)), date(2027, 10, 1));
        assert_eq!(default_admin_end(date(2026, 10, 1)), date(2027, 10, 1));
    }

    #[test]
    fn the_default_always_passes_validation() {
        let mut day = date(2026, 1, 1);
        while day < date(2029, 1, 1) {
            assert_eq!(
                validate_admin_end(day, default_admin_end(day)),
                Ok(()),
                "{day}"
            );
            day = day.tomorrow().unwrap();
        }
    }

    #[test]
    fn admin_end_range_is_one_to_twenty_four_months_inclusive() {
        let today = date(2026, 9, 23);
        assert_eq!(
            validate_admin_end(today, date(2026, 10, 22)),
            Err(AdminEndError::TooSoon)
        );
        assert_eq!(validate_admin_end(today, date(2026, 10, 23)), Ok(()));
        assert_eq!(validate_admin_end(today, date(2028, 9, 23)), Ok(()));
        assert_eq!(
            validate_admin_end(today, date(2028, 9, 24)),
            Err(AdminEndError::TooLate)
        );
    }

    #[test]
    fn admin_end_range_truncates_at_month_end() {
        // One month after 31 January is 28 February, not 3 March.
        let today = date(2026, 1, 31);
        assert_eq!(
            validate_admin_end(today, date(2026, 2, 27)),
            Err(AdminEndError::TooSoon)
        );
        assert_eq!(validate_admin_end(today, date(2026, 2, 28)), Ok(()));
        // 24 months after 29 February 2028 is 28 February 2030.
        let leap = date(2028, 2, 29);
        assert_eq!(validate_admin_end(leap, date(2030, 2, 28)), Ok(()));
        assert_eq!(
            validate_admin_end(leap, date(2030, 3, 1)),
            Err(AdminEndError::TooLate)
        );
    }

    #[test]
    fn pending_signups_expire_after_seven_days() {
        assert_eq!(
            pending_signup_expiry(ts("2026-09-23T10:00:00Z")),
            ts("2026-09-30T10:00:00Z")
        );
    }

    #[test]
    fn invitations_expire_after_fourteen_days_even_across_a_clock_change() {
        assert_eq!(
            invitation_expiry(ts("2026-09-23T10:00:00Z")),
            ts("2026-10-07T10:00:00Z")
        );
        // Oslo leaves daylight saving on 25 October 2026; the lifetime is exact.
        assert_eq!(
            invitation_expiry(ts("2026-10-20T10:00:00Z")),
            ts("2026-11-03T10:00:00Z")
        );
    }

    #[test]
    fn recent_holder_window_is_twenty_four_months() {
        assert_eq!(
            recent_holder_window_start(date(2026, 9, 23)),
            date(2024, 9, 23)
        );
        assert_eq!(
            recent_holder_window_start(date(2028, 2, 29)),
            date(2026, 2, 28)
        );
    }
}
