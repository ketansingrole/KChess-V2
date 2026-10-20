//! Calendar dates and UTC timestamps, through `jiff`: day counts since 1970-01-01, `YYYY-MM-DD`
//! days, ISO 8601 text and millisecond timestamps. Dates are proleptic Gregorian and lie in
//! years -9999..=9999; anything outside that range is unreadable.

use jiff::civil::{Date, DateTime};
use jiff::tz::Offset;
use jiff::{Span, Timestamp};

const EPOCH: Date = jiff::civil::date(1970, 1, 1);

/// The date `days` days after 1970-01-01, as (year, month 1-12, day 1-31).
pub fn civil_from_days(days: i64) -> Option<(i64, i64, i64)> {
    let date = EPOCH.checked_add(Span::new().try_days(days).ok()?).ok()?;
    Some((
        i64::from(date.year()),
        i64::from(date.month()),
        i64::from(date.day()),
    ))
}

/// Days since 1970-01-01 of a real calendar date.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    let date = Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    Some(i64::from(EPOCH.until(date).ok()?.get_days()))
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` calendar day (ASCII digits, exactly that shape).
pub fn day_number(day: &str) -> Option<i64> {
    let bytes = day.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    let date: Date = day.parse().ok()?;
    Some(i64::from(EPOCH.until(date).ok()?.get_days()))
}

/// Milliseconds since the epoch of ISO 8601 text: a date, optionally with a time (`T` or space),
/// seconds, fraction and offset. A date or time without an offset is UTC.
pub fn parse_iso_millis(text: &str) -> Option<i64> {
    let timestamp = text
        .parse::<Timestamp>()
        .or_else(|_| {
            text.parse::<DateTime>()
                .and_then(|t| Offset::UTC.to_timestamp(t))
        })
        .or_else(|_| {
            text.parse::<Date>()
                .and_then(|d| Offset::UTC.to_timestamp(d.into()))
        })
        .ok()?;
    Some(timestamp.as_millisecond())
}

/// `new Date(ms).toISOString()` (`1970-01-01T00:00:00.000Z`) for a whole millisecond count.
pub fn iso_millis(ms: i64) -> Option<String> {
    let timestamp = Timestamp::from_millisecond(ms).ok()?;
    Some(timestamp.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_counts_follow_the_gregorian_calendar() {
        assert_eq!(civil_from_days(0), Some((1970, 1, 1)));
        assert_eq!(civil_from_days(-1), Some((1969, 12, 31)));
        assert_eq!(civil_from_days(11_016), Some((2000, 2, 29)));
        assert_eq!(civil_from_days(19_782), Some((2024, 2, 29)));
        assert_eq!(days_from_civil(2024, 3, 1), Some(19_783));
        assert_eq!(days_from_civil(2023, 2, 29), None);
        assert_eq!(civil_from_days(i64::MAX), None);
    }

    #[test]
    fn days_must_be_written_exactly() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2024-02-29"), Some(19_782));
        assert_eq!(day_number("2023-02-29"), None);
        assert_eq!(day_number("2026-13-01"), None);
        assert_eq!(day_number("2026-1-01"), None);
        assert_eq!(day_number("20260101"), None);
        assert_eq!(day_number("２０２６-01-01"), None);
    }

    #[test]
    fn iso_text_reads_with_or_without_time_and_offset() {
        assert_eq!(parse_iso_millis("1970-01-01"), Some(0));
        assert_eq!(parse_iso_millis("1970-01-01T00:00:01Z"), Some(1_000));
        assert_eq!(parse_iso_millis("1970-01-01 00:01"), Some(60_000));
        assert_eq!(parse_iso_millis("1970-01-01T00:00:00.5Z"), Some(500));
        assert_eq!(parse_iso_millis("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(parse_iso_millis("2024-02-30"), None);
        assert_eq!(parse_iso_millis("soon"), None);
    }

    #[test]
    fn timestamps_write_as_javascript_writes_them() {
        assert_eq!(iso_millis(0).as_deref(), Some("1970-01-01T00:00:00.000Z"));
        assert_eq!(
            iso_millis(1_700_000_000_123).as_deref(),
            Some("2023-11-14T22:13:20.123Z")
        );
        assert_eq!(iso_millis(-1).as_deref(), Some("1969-12-31T23:59:59.999Z"));
    }
}
