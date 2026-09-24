//! "Today" is the server's calendar date in Europe/Oslo (flow spec §2.1, #3412). Every
//! rule that depends on the date takes a [`Moment`], so one operation sees one instant and
//! one date, and a test can pin both.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::Timestamp;

/// The IANA zone that decides "today". Bundled into the binary (jiff's
/// `tzdb-bundle-always`), so the answer never depends on the runtime image's tzdata.
pub const OSLO: &str = "Europe/Oslo";

/// One instant and the Europe/Oslo calendar date it falls on. Constructed only through
/// [`Moment::at`], so the two can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    now: Timestamp,
    today: Date,
}

impl Moment {
    pub fn at(now: Timestamp) -> Self {
        Self {
            now,
            today: oslo_today(now),
        }
    }

    pub fn now(&self) -> Timestamp {
        self.now
    }

    pub fn today(&self) -> Date {
        self.today
    }
}

/// The Europe/Oslo calendar date of `now`.
pub fn oslo_today(now: Timestamp) -> Date {
    // Cannot fail: the zone is compiled into the binary by `tzdb-bundle-always`, and the
    // unit tests below prove the lookup works.
    let tz = TimeZone::get(OSLO).expect("Europe/Oslo is in the bundled tzdb");
    now.to_zoned(tz).date()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn winter_midnight_is_utc_plus_one() {
        assert_eq!(oslo_today(ts("2026-12-31T22:59:59Z")), date(2026, 12, 31));
        assert_eq!(oslo_today(ts("2026-12-31T23:00:00Z")), date(2027, 1, 1));
    }

    #[test]
    fn summer_midnight_is_utc_plus_two() {
        assert_eq!(oslo_today(ts("2026-07-31T21:59:59Z")), date(2026, 7, 31));
        assert_eq!(oslo_today(ts("2026-07-31T22:00:00Z")), date(2026, 8, 1));
    }

    #[test]
    fn the_night_daylight_saving_starts() {
        // 29 March 2026: clocks go from 02:00 to 03:00. At 23:30 UTC on the 28th Oslo is
        // still on UTC+1, so it is already 00:30 on the 29th.
        assert_eq!(oslo_today(ts("2026-03-28T22:59:59Z")), date(2026, 3, 28));
        assert_eq!(oslo_today(ts("2026-03-28T23:30:00Z")), date(2026, 3, 29));
    }

    #[test]
    fn a_moment_carries_its_own_date() {
        let m = Moment::at(ts("2026-07-31T22:30:00Z"));
        assert_eq!(m.now(), ts("2026-07-31T22:30:00Z"));
        assert_eq!(m.today(), date(2026, 8, 1));
    }
}
