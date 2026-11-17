//! Dates and times as the app shows them: moments in milliseconds since the epoch, shown in the
//! system's time zone or in UTC, with `strftime` patterns.

use jiff::civil::{Date, DateTime};
use jiff::fmt::temporal::Pieces;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ZonedDateTime};

/// The moment `millis` in the system's time zone; `None` outside the representable range.
pub fn local(millis: f64) -> Option<ZonedDateTime> {
    Timestamp::from_millisecond(millis as i64)
        .ok()
        .map(|moment| moment.to_zoned_datetime(TimeZone::system()))
}

/// The moment `millis` in the system's time zone, written by `pattern`; empty when out of range.
pub fn local_format(millis: f64, pattern: &str) -> String {
    local(millis)
        .map(|moment| moment.strftime(pattern).to_string())
        .unwrap_or_default()
}

/// The moment `millis` in UTC, written by `pattern`; `None` when out of range.
pub fn utc_format(millis: f64, pattern: &str) -> Option<String> {
    Timestamp::from_millisecond(millis as i64)
        .ok()
        .map(|moment| moment.strftime(pattern).to_string())
}

/// Now in the system's time zone, written by `pattern`: `today("%Y-%m-%d")`.
pub fn today(pattern: &str) -> String {
    ZonedDateTime::now().strftime(pattern).to_string()
}

/// Now, in milliseconds since the epoch.
pub fn now_millis() -> i64 {
    Timestamp::now().as_millisecond()
}

/// `moment` as RFC 3339 in UTC to the millisecond: `2026-11-17T09:30:00.123Z`.
pub fn rfc3339_millis(moment: Timestamp) -> String {
    format!("{moment:.3}")
}

/// The calendar day an RFC 3339 time is written on, in its own offset: `2026-11-04` from
/// `2026-11-04T23:30:00-05:00`.
pub fn written_date(rfc3339: &str) -> Option<Date> {
    rfc3339.parse::<Timestamp>().ok()?;
    Pieces::parse(rfc3339).ok().map(|pieces| pieces.date())
}

/// A wall-clock time in the system's time zone, as milliseconds; `None` for a time the zone
/// skips or repeats (a daylight-saving change).
pub fn local_millis(time: DateTime) -> Option<f64> {
    TimeZone::system()
        .to_ambiguous_zoned_datetime(time)
        .unambiguous()
        .ok()
        .map(|moment| moment.timestamp().as_millisecond() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formats_and_rejects_out_of_range() {
        assert_eq!(
            utc_format(1_762_300_800_000., "%Y-%m-%d %b %-d").as_deref(),
            Some("2025-11-05 Nov 5")
        );
        assert_eq!(utc_format(1e300, "%Y"), None);
        assert_eq!(local_format(1e300, "%Y"), "");
        assert!(local(1e300).is_none());
    }

    #[test]
    fn local_round_trips_a_wall_clock_time() {
        let noon = jiff::civil::date(2026, 7, 14).at(12, 30, 0, 0);
        let millis = local_millis(noon).unwrap();
        assert_eq!(local(millis).unwrap().datetime(), noon);
        assert_eq!(local_format(millis, "%Y-%m-%d %H:%M"), "2026-07-14 12:30");
    }

    #[test]
    fn rfc3339_is_utc_to_the_millisecond() {
        let moment: Timestamp = "2026-11-17T09:30:00.123456Z".parse().unwrap();
        assert_eq!(rfc3339_millis(moment), "2026-11-17T09:30:00.123Z");
        let whole: Timestamp = "2026-11-17T09:30:00Z".parse().unwrap();
        assert_eq!(rfc3339_millis(whole), "2026-11-17T09:30:00.000Z");
    }

    #[test]
    fn written_dates_keep_their_offset() {
        assert_eq!(
            written_date("2026-11-04T23:30:00-05:00"),
            Some(jiff::civil::date(2026, 11, 4))
        );
        assert_eq!(
            written_date("2026-11-04T01:00:00Z"),
            Some(jiff::civil::date(2026, 11, 4))
        );
        assert_eq!(written_date("2026-11-04"), None);
        assert_eq!(written_date("soon"), None);
    }

    #[test]
    fn now_is_after_this_was_written() {
        assert!(now_millis() > 1_762_000_000_000);
        assert_eq!(today("%Y").len(), 4);
    }
}
