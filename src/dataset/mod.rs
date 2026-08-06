//! Parsers for the public archives the benchmark runs on. All three are downloadable without
//! an account, which is what makes the results reproducible.

use alloc::vec::Vec;

use crate::Point;

pub mod coops;
pub mod isd_lite;
pub mod usgs;

pub struct Series {
    pub source: &'static str,
    pub variable: &'static str,
    pub unit: &'static str,
    pub points: Vec<Point>,
}

/// Parses `YYYY-MM-DD HH:MM`, the timestamp both water archives use.
pub(crate) fn parse_datetime(field: &str) -> Option<i64> {
    let (date, time) = field.trim().split_once(' ')?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;

    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(to_epoch(year, month, day, hour) + minute * 60)
}

pub(crate) fn to_epoch(year: i64, month: u32, day: u32, hour: i64) -> i64 {
    days_from_civil(year, month, day) * 86_400 + hour * 3_600
}

/// Howard Hinnant's civil-date algorithm, so the crate needs no date dependency.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 {
        month as i64 - 3
    } else {
        month as i64 + 9
    };
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates_match_their_epoch_seconds() {
        assert_eq!(to_epoch(1970, 1, 1, 0), 0);
        assert_eq!(to_epoch(2000, 3, 1, 0), 951_868_800);
        assert_eq!(to_epoch(2023, 1, 1, 0), 1_672_531_200);
        assert_eq!(to_epoch(2024, 2, 29, 12), 1_709_208_000);
        assert_eq!(to_epoch(1969, 12, 31, 23), -3_600);
    }

    #[test]
    fn datetimes_parse_to_the_same_instants() {
        assert_eq!(parse_datetime("2023-01-01 00:00"), Some(1_672_531_200));
        assert_eq!(parse_datetime("2023-01-01 00:06"), Some(1_672_531_560));
        assert_eq!(parse_datetime(" 2024-02-29 12:00 "), Some(1_709_208_000));
    }

    #[test]
    fn malformed_datetimes_are_rejected() {
        for bad in [
            "",
            "2023-01-01",
            "not a date",
            "2023-13-01 00:00",
            "2023-01-01 xx:00",
        ] {
            assert_eq!(parse_datetime(bad), None, "{bad:?} should not parse");
        }
    }
}
