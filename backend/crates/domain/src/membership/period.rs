//! Half-open date ranges `[starts_on, ends_on_exclusive)` with a mandatory end (#3412;
//! the same rule 0002's `role_period_is_non_empty` check enforces in the database).

use jiff::civil::Date;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    starts_on: Date,
    ends_on_exclusive: Date,
}

/// `starts_on` is not before `ends_on_exclusive`: the period would contain no day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyPeriod;

impl Period {
    pub fn new(starts_on: Date, ends_on_exclusive: Date) -> Result<Self, EmptyPeriod> {
        if starts_on < ends_on_exclusive {
            Ok(Self {
                starts_on,
                ends_on_exclusive,
            })
        } else {
            Err(EmptyPeriod)
        }
    }

    pub fn starts_on(&self) -> Date {
        self.starts_on
    }

    pub fn ends_on_exclusive(&self) -> Date {
        self.ends_on_exclusive
    }

    /// Whether `day` falls inside the period. The end date itself is outside it: a role
    /// ending 2027-08-01 grants nothing on 2027-08-01.
    pub fn contains(&self, day: Date) -> bool {
        self.starts_on <= day && day < self.ends_on_exclusive
    }

    /// Whether the period is over by `day`, so offering it would grant nothing ever.
    pub fn has_ended_by(&self, day: Date) -> bool {
        self.ends_on_exclusive <= day
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn contains_the_start_and_excludes_the_end() {
        let p = Period::new(date(2026, 8, 1), date(2027, 8, 1)).unwrap();
        assert!(!p.contains(date(2026, 7, 31)));
        assert!(p.contains(date(2026, 8, 1)));
        assert!(p.contains(date(2027, 7, 31)));
        assert!(!p.contains(date(2027, 8, 1)));
    }

    #[test]
    fn rejects_empty_and_reversed_periods() {
        assert_eq!(
            Period::new(date(2026, 8, 1), date(2026, 8, 1)),
            Err(EmptyPeriod)
        );
        assert_eq!(
            Period::new(date(2027, 8, 1), date(2026, 8, 1)),
            Err(EmptyPeriod)
        );
    }

    #[test]
    fn has_ended_by_the_exclusive_end() {
        let p = Period::new(date(2026, 8, 1), date(2027, 8, 1)).unwrap();
        assert!(!p.has_ended_by(date(2027, 7, 31)));
        assert!(p.has_ended_by(date(2027, 8, 1)));
    }
}
