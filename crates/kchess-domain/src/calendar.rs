//! Calendar dates and UTC timestamps: day counts since 1970-01-01, `YYYY-MM-DD` days, ISO 8601
//! text and millisecond timestamps. Dates are proleptic Gregorian and lie in years
//! -9999..=9999; anything outside that range is unreadable. Text follows ECMAScript's date-time
//! string format, the ISO 8601 subset `Date.parse` and `toISOString` use.

const MIN_YEAR: i64 = -9999;
const MAX_YEAR: i64 = 9999;
const DAY_MS: i64 = 86_400_000;

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The date `days` days after 1970-01-01, as (year, month 1-12, day 1-31)
/// (Howard Hinnant's `civil_from_days`).
pub fn civil_from_days(days: i64) -> Option<(i64, i64, i64)> {
    // Bounded first so the arithmetic below cannot overflow.
    if !(days_from_civil(MIN_YEAR, 1, 1)?..=days_from_civil(MAX_YEAR, 12, 31)?).contains(&days) {
        return None;
    }
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Some((year, month, day))
}

/// Days since 1970-01-01 of a real calendar date (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
    {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// Reads ASCII digits from the front of the text, keeping the rest.
struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn digits(&mut self, count: usize) -> Option<i64> {
        let (head, rest) = self.0.split_at_checked(count)?;
        if !head.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.0 = rest;
        Some(head.iter().fold(0, |n, b| n * 10 + i64::from(b - b'0')))
    }

    fn eat(&mut self, accepted: &[u8]) -> Option<u8> {
        let (&first, rest) = self.0.split_first()?;
        accepted.contains(&first).then(|| {
            self.0 = rest;
            first
        })
    }

    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

/// `YYYY-MM-DD` at the cursor, as days since 1970-01-01. With `expanded`, the year may also be
/// a sign and six digits (`-000001`, never `-000000`), as `toISOString` writes years outside
/// 0..=9999.
fn date(cursor: &mut Cursor, expanded: bool) -> Option<i64> {
    let year = match expanded.then(|| cursor.eat(b"+-")).flatten() {
        Some(b'-') => -cursor.digits(6).filter(|year| *year != 0)?,
        Some(_) => cursor.digits(6)?,
        None => cursor.digits(4)?,
    };
    cursor.eat(b"-")?;
    let month = cursor.digits(2)?;
    cursor.eat(b"-")?;
    let day = cursor.digits(2)?;
    days_from_civil(year, month, day)
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` calendar day (ASCII digits, exactly that shape).
pub fn day_number(day: &str) -> Option<i64> {
    let mut cursor = Cursor(day.as_bytes());
    let days = date(&mut cursor, false)?;
    cursor.done().then_some(days)
}

/// Milliseconds since the epoch of ISO 8601 text: a date, optionally with a time (`T` or space),
/// seconds, fraction and offset (`Z`, `±HH:MM` or `±HHMM`). A date or time without an offset is UTC.
pub fn parse_iso_millis(text: &str) -> Option<i64> {
    let mut cursor = Cursor(text.as_bytes());
    let days = date(&mut cursor, true)?;
    let mut ms = days * DAY_MS;
    if cursor.eat(b"Tt ").is_some() {
        let hour = cursor.digits(2).filter(|h| *h < 24)?;
        cursor.eat(b":")?;
        let minute = cursor.digits(2).filter(|m| *m < 60)?;
        let mut second = 0;
        if cursor.eat(b":").is_some() {
            second = cursor.digits(2).filter(|s| *s < 60)?;
        }
        let mut fraction_ms = 0;
        if cursor.eat(b".,").is_some() {
            // Up to nanoseconds; digits past milliseconds are truncated, as `Date.parse` does.
            let mut count = 0;
            while count < 9 && cursor.0.first().is_some_and(u8::is_ascii_digit) {
                let digit = cursor.digits(1)?;
                if count < 3 {
                    fraction_ms = fraction_ms * 10 + digit;
                }
                count += 1;
            }
            if count == 0 {
                return None;
            }
            fraction_ms *= 10_i64.pow(3_u32.saturating_sub(count));
        }
        ms += ((hour * 60 + minute) * 60 + second) * 1000 + fraction_ms;
        if let Some(sign) = cursor.eat(b"+-") {
            let hours = cursor.digits(2).filter(|h| *h < 24)?;
            cursor.eat(b":");
            let minutes = cursor.digits(2).filter(|m| *m < 60)?;
            let offset = (hours * 60 + minutes) * 60_000;
            ms += if sign == b'+' { -offset } else { offset };
        } else {
            cursor.eat(b"Zz");
        }
    }
    cursor.done().then_some(ms)
}

/// `new Date(ms).toISOString()` (`1970-01-01T00:00:00.000Z`) for a whole millisecond count.
pub fn iso_millis(ms: i64) -> Option<String> {
    let (year, month, day) = civil_from_days(ms.div_euclid(DAY_MS))?;
    let time = ms.rem_euclid(DAY_MS);
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else {
        // Years outside 0..=9999 are written with a sign and six digits, as JavaScript does.
        format!("{}{:06}", if year < 0 { '-' } else { '+' }, year.abs())
    };
    Some(format!(
        "{year}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        time / 3_600_000,
        time / 60_000 % 60,
        time / 1000 % 60,
        time % 1000
    ))
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
        assert_eq!(days_from_civil(1900, 2, 29), None);
        assert_eq!(days_from_civil(2000, 2, 29), Some(11_016));
        assert_eq!(civil_from_days(i64::MAX), None);
        assert_eq!(civil_from_days(i64::MIN), None);
    }

    #[test]
    fn day_counts_round_trip_across_the_whole_range() {
        let first = days_from_civil(MIN_YEAR, 1, 1).expect("first day");
        let last = days_from_civil(MAX_YEAR, 12, 31).expect("last day");
        let mut expected = (MIN_YEAR, 1, 1);
        for days in first..=last {
            assert_eq!(civil_from_days(days), Some(expected), "day {days}");
            assert_eq!(
                days_from_civil(expected.0, expected.1, expected.2),
                Some(days)
            );
            let (year, month, day) = expected;
            expected = if day < days_in_month(year, month) {
                (year, month, day + 1)
            } else if month < 12 {
                (year, month + 1, 1)
            } else {
                (year + 1, 1, 1)
            };
        }
        assert_eq!(civil_from_days(first - 1), None);
        assert_eq!(civil_from_days(last + 1), None);
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
        assert_eq!(day_number("2026-01-01 "), None);
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
    fn iso_text_edge_cases() {
        assert_eq!(
            parse_iso_millis("1970-01-01T00:00:00.123456789Z"),
            Some(123)
        );
        assert_eq!(parse_iso_millis("1970-01-01T00:00:00.1234567890Z"), None);
        assert_eq!(parse_iso_millis("1970-01-01T00:00:00.Z"), None);
        assert_eq!(parse_iso_millis("1970-01-01T00:00-0130"), Some(5_400_000));
        assert_eq!(parse_iso_millis("1969-12-31T23:59:59.999Z"), Some(-1));
        assert_eq!(parse_iso_millis("2026-10-21T24:00:00Z"), None);
        assert_eq!(parse_iso_millis("2026-10-21T10:60Z"), None);
        assert_eq!(parse_iso_millis("2026-10-21T10Z"), None);
        assert_eq!(parse_iso_millis("2026-10-21T10:00:00ZZ"), None);
        assert_eq!(parse_iso_millis("2026-10-21Z"), None);
        assert_eq!(
            parse_iso_millis("2026-10-21T08:30:00.000Z"),
            Some(1_792_571_400_000)
        );
    }

    #[test]
    fn timestamps_write_as_javascript_writes_them() {
        assert_eq!(iso_millis(0).as_deref(), Some("1970-01-01T00:00:00.000Z"));
        assert_eq!(
            iso_millis(1_700_000_000_123).as_deref(),
            Some("2023-11-14T22:13:20.123Z")
        );
        assert_eq!(iso_millis(-1).as_deref(), Some("1969-12-31T23:59:59.999Z"));
        assert_eq!(
            iso_millis(-62_198_755_200_000).as_deref(),
            Some("-000001-01-01T00:00:00.000Z")
        );
        assert_eq!(iso_millis(i64::MAX), None);
    }

    #[test]
    fn written_timestamps_read_back() {
        for ms in [
            0,
            -1,
            1,
            951_782_400_000,
            1_700_000_000_123,
            253_402_300_799_999,
            -62_198_755_200_000,
            -377_705_116_800_000,
        ] {
            let text = iso_millis(ms).expect("in range");
            assert_eq!(parse_iso_millis(&text), Some(ms), "{text}");
        }
        assert_eq!(
            parse_iso_millis("+002000-01-01"),
            parse_iso_millis("2000-01-01")
        );
        for text in [
            "-000000-01-01",
            "+010000-01-01",
            "-2000-01-01",
            "-0000010-01-01",
        ] {
            assert_eq!(parse_iso_millis(text), None, "{text}");
        }
        assert_eq!(day_number("-000001-01-01"), None);
    }
}
