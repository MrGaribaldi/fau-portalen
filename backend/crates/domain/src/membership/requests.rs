//! Access requests and replacement proposals (§5.2–5.4): one model, one set of limits.
//! The limits are the spec's starting values for #3417 and #3418 to tune (§12).

use jiff::civil::Date;
use jiff::ToSpan;

use super::period::Period;

/// One open request per address per FAU (§5.4).
pub const MAX_OPEN_REQUESTS_PER_ADDRESS: i64 = 1;
/// Five requests per FAU per day (§5.4).
pub const MAX_REQUESTS_PER_TENANT_PER_DAY: i64 = 5;
/// An unhandled request lapses after this many days (§5.2.4).
pub const REQUEST_LAPSE_DAYS: i32 = 30;
/// The optional message is plain text of at most this many characters (§5.2).
pub const MESSAGE_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestLimit {
    /// This address already has an open request in this FAU.
    OpenRequestExists,
    /// The FAU has received its daily maximum.
    TenantDailyLimit,
}

/// `open_by_address`: pending requests from this address in this FAU, where a
/// replacement proposal counts against its proposer. `created_today`: requests created in
/// this FAU on today's Europe/Oslo date, any status.
pub fn check_request_limits(open_by_address: i64, created_today: i64) -> Result<(), RequestLimit> {
    if open_by_address >= MAX_OPEN_REQUESTS_PER_ADDRESS {
        return Err(RequestLimit::OpenRequestExists);
    }
    if created_today >= MAX_REQUESTS_PER_TENANT_PER_DAY {
        return Err(RequestLimit::TenantDailyLimit);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageError {
    /// A control character other than a line break (e.g. NUL, ESC).
    ControlCharacter,
    TooLong,
}

/// Trims the optional message; blank becomes `None`. `\r\n` and lone `\r` are normalised
/// to `\n` before anything else is checked, so the 500-character bound and the control
/// character check both see the normalised text. `\n` is kept; any other control
/// character is refused. Escaping is the renderer's job ("always escaped", §5.2), so the
/// text is otherwise stored as typed.
pub fn normalise_message(raw: Option<&str>) -> Result<Option<String>, MessageError> {
    let trimmed = match raw.map(str::trim) {
        None | Some("") => return Ok(None),
        Some(s) => s,
    };
    let normalised = trimmed.replace("\r\n", "\n").replace('\r', "\n");
    if normalised.chars().any(|c| c != '\n' && c.is_control()) {
        return Err(MessageError::ControlCharacter);
    }
    if normalised.chars().count() > MESSAGE_MAX_CHARS {
        return Err(MessageError::TooLong);
    }
    Ok(Some(normalised))
}

/// Requests created on or before this date have lapsed by `today`: a request created on
/// 1 September lapses on 1 October.
pub fn lapse_cutoff(today: Date) -> Date {
    today
        .checked_sub(REQUEST_LAPSE_DAYS.days())
        .unwrap_or(Date::MIN)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementDateError {
    /// The successor's role would start before today (§5.3).
    StartsInPast,
    Empty,
}

/// Validates a replacement proposal's dates: starting no earlier than today, non-empty.
pub fn check_replacement_dates(
    today: Date,
    starts_on: Date,
    ends_on_exclusive: Date,
) -> Result<Period, ReplacementDateError> {
    if starts_on < today {
        return Err(ReplacementDateError::StartsInPast);
    }
    Period::new(starts_on, ends_on_exclusive).map_err(|_| ReplacementDateError::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn one_open_request_per_address() {
        assert_eq!(check_request_limits(0, 0), Ok(()));
        assert_eq!(
            check_request_limits(1, 0),
            Err(RequestLimit::OpenRequestExists)
        );
    }

    #[test]
    fn five_requests_per_fau_per_day() {
        assert_eq!(check_request_limits(0, 4), Ok(()));
        assert_eq!(
            check_request_limits(0, 5),
            Err(RequestLimit::TenantDailyLimit)
        );
    }

    #[test]
    fn messages_are_trimmed_optional_and_bounded_in_characters() {
        assert_eq!(normalise_message(None), Ok(None));
        assert_eq!(normalise_message(Some("   ")), Ok(None));
        assert_eq!(
            normalise_message(Some(" Hei! ")),
            Ok(Some("Hei!".to_owned()))
        );
        assert!(normalise_message(Some(&"ø".repeat(500))).is_ok());
        assert_eq!(
            normalise_message(Some(&"ø".repeat(501))),
            Err(MessageError::TooLong)
        );
    }

    #[test]
    fn interior_control_characters_are_rejected() {
        assert_eq!(
            normalise_message(Some("Hei\u{0}der")),
            Err(MessageError::ControlCharacter),
            "a NUL is rejected"
        );
        assert_eq!(
            normalise_message(Some("Hei\u{1b}der")),
            Err(MessageError::ControlCharacter),
            "an ESC is rejected"
        );
    }

    #[test]
    fn line_breaks_are_kept_and_carriage_returns_are_normalised() {
        assert_eq!(
            normalise_message(Some("Hei\ndu")),
            Ok(Some("Hei\ndu".to_owned())),
            "a bare line feed is kept"
        );
        assert_eq!(
            normalise_message(Some("Hei\r\ndu")),
            Ok(Some("Hei\ndu".to_owned())),
            "CRLF becomes LF"
        );
        assert_eq!(
            normalise_message(Some("Hei\rdu")),
            Ok(Some("Hei\ndu".to_owned())),
            "a lone CR becomes LF"
        );
    }

    #[test]
    fn the_length_bound_is_counted_after_normalisation() {
        // 300 CRLF pairs is 602 raw characters (over the 500 bound) but normalises to
        // 302 characters (under it), since each two-character CRLF collapses to one LF.
        let raw = format!("a{}a", "\r\n".repeat(300));
        assert!(raw.chars().count() > MESSAGE_MAX_CHARS);
        let normalised = normalise_message(Some(&raw)).unwrap().unwrap();
        assert_eq!(normalised.chars().count(), 302);
    }

    #[test]
    fn requests_lapse_after_thirty_days() {
        assert_eq!(lapse_cutoff(date(2026, 10, 1)), date(2026, 9, 1));
        assert_eq!(lapse_cutoff(date(2026, 3, 1)), date(2026, 1, 30));
    }

    #[test]
    fn replacement_dates_start_today_or_later() {
        let today = date(2026, 9, 23);
        assert_eq!(
            check_replacement_dates(today, date(2026, 9, 22), date(2027, 10, 1)),
            Err(ReplacementDateError::StartsInPast)
        );
        assert_eq!(
            check_replacement_dates(today, date(2027, 10, 1), date(2027, 10, 1)),
            Err(ReplacementDateError::Empty)
        );
        let p = check_replacement_dates(today, today, date(2027, 10, 1)).unwrap();
        assert_eq!(p.starts_on(), today);
    }
}
