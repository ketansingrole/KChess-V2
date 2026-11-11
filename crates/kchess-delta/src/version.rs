//! Release versions: `YEAR.MONTH.COUNTER` in the UTC release month (2026.10.0, 2026.10.1, …
//! 2026.11.0), without zero padding, so the version stays valid semver for Cargo and the update
//! manifests. Tags are the version after a `v`.

use std::fmt;

/// A release version, ordered as releases are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub year: u32,
    pub month: u32,
    pub counter: u64,
}

impl Version {
    /// `text` as a release version; none for anything else (`1.2`, `2026.10.1-beta`, `2026.01.0`).
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let (year, month, counter) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let plain = |part: &str| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'))
        };
        if !(plain(year) && plain(month) && plain(counter))
            || year.len() != 4
            || !year.starts_with("20")
        {
            return None;
        }
        let version = Version {
            year: year.parse().ok()?,
            month: month.parse().ok()?,
            counter: counter.parse().ok()?,
        };
        (1..=12).contains(&version.month).then_some(version)
    }

    /// A release tag (`v2026.10.12`) or version, surrounding space ignored.
    pub fn of_tag(tag: &str) -> Option<Version> {
        let text = tag.trim();
        Version::parse(text.strip_prefix('v').unwrap_or(text))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.year, self.month, self.counter)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn versions_read_and_order_as_releases() {
        let version = |year, month, counter| Version {
            year,
            month,
            counter,
        };
        assert_eq!(Version::parse("2026.10.0"), Some(version(2026, 10, 0)));
        assert_eq!(
            Version::of_tag(" v2026.10.12\n"),
            Some(version(2026, 10, 12))
        );
        assert_eq!(Version::of_tag("2026.9.30"), Some(version(2026, 9, 30)));
        assert!(Version::of_tag("v2026.11.0") > Version::of_tag("v2026.10.12"));
        assert!(Version::of_tag("v2026.10.12") > Version::of_tag("2026.9.30"));
        assert_eq!(version(2026, 11, 3).to_string(), "2026.11.3");
        for refused in [
            "",
            "v1.2",
            "2026.10.1-beta",
            "2026.10",
            "2026.10.1.2",
            "2026.01.0",
            "2026.10.01",
            "2026.0.1",
            "2026.13.0",
            "1999.1.0",
            "20260.1.0",
            "2026..1",
            "x2026.1.0",
            "vv2026.1.0",
        ] {
            assert_eq!(Version::of_tag(refused), None, "{refused}");
        }
        assert_eq!(Version::parse("v2026.10.1"), None, "a tag is no version");
    }
}
