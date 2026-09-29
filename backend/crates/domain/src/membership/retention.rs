//! How long a former member is remembered (#3511, docs/member-retention-design.md, Erik's
//! M2 of 29 September 2026). When a membership ends, its display name and contact address
//! are kept hidden for the member's chosen period and then destroyed. Pure: persistence
//! reads the account's `retention_months` and the roles the membership held, and asks
//! [`keep_until`] whether to retain or clear.

use jiff::civil::Date;
use jiff::Span;

/// `accounts.retention_months`: the allowed values and nothing else (migration 0009's
/// `accounts_retention_months_is_allowed`). `None` clears at once, #3502's D3 behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionMonths {
    None,
    Three,
    Six,
    Twelve,
    TwentyFour,
}

impl RetentionMonths {
    pub const DEFAULT: Self = Self::Three;
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Three,
        Self::Six,
        Self::Twelve,
        Self::TwentyFour,
    ];

    pub fn months(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Three => 3,
            Self::Six => 6,
            Self::Twelve => 12,
            Self::TwentyFour => 24,
        }
    }

    pub fn from_months(n: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.months() == n)
    }
}

/// The first day the fields are no longer kept: `ended_on` plus the period, a month end
/// clamped (jiff's rule). `None` for a period of none.
pub fn retained_until(ended_on: Date, months: RetentionMonths) -> Option<Date> {
    if months == RetentionMonths::None {
        return None;
    }
    ended_on
        .checked_add(Span::new().months(i64::from(months.months())))
        .ok()
}

/// Whether a membership that ended on `ended_on` still keeps its fields on `today`: the
/// date they go, or `None` to clear them now.
pub fn keep_until(ended_on: Date, months: RetentionMonths, today: Date) -> Option<Date> {
    retained_until(ended_on, months).filter(|until| today < *until)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn only_the_five_allowed_values_parse() {
        for (n, v) in [
            (0, RetentionMonths::None),
            (3, RetentionMonths::Three),
            (6, RetentionMonths::Six),
            (12, RetentionMonths::Twelve),
            (24, RetentionMonths::TwentyFour),
        ] {
            assert_eq!(RetentionMonths::from_months(n), Some(v));
            assert_eq!(v.months(), n);
        }
        for n in [-1, 1, 2, 4, 5, 7, 11, 13, 18, 25, 36] {
            assert_eq!(RetentionMonths::from_months(n), None, "{n}");
        }
        assert_eq!(RetentionMonths::DEFAULT, RetentionMonths::Three);
        assert_eq!(RetentionMonths::ALL.len(), 5);
    }

    #[test]
    fn the_period_counts_calendar_months_from_the_day_it_ended() {
        let ended = date(2026, 10, 1);
        assert_eq!(retained_until(ended, RetentionMonths::None), None);
        assert_eq!(
            retained_until(ended, RetentionMonths::Three),
            Some(date(2027, 1, 1))
        );
        assert_eq!(
            retained_until(ended, RetentionMonths::TwentyFour),
            Some(date(2028, 10, 1))
        );
        // A month end clamps rather than spilling into the next month.
        assert_eq!(
            retained_until(date(2026, 11, 30), RetentionMonths::Three),
            Some(date(2027, 2, 28))
        );
    }

    #[test]
    fn keep_until_is_the_date_while_it_is_still_ahead_and_none_after() {
        let ended = date(2026, 10, 1);
        let m = RetentionMonths::Three;
        assert_eq!(
            keep_until(ended, m, date(2026, 12, 31)),
            Some(date(2027, 1, 1))
        );
        assert_eq!(
            keep_until(ended, m, date(2027, 1, 1)),
            None,
            "the last day is exclusive"
        );
        assert_eq!(keep_until(ended, RetentionMonths::None, ended), None);
    }
}
