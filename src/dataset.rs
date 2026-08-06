//! Reader for NOAA ISD-Lite, the hourly surface observation archive used by the benchmark.
//!
//! The format is worth knowing before reading any result: NOAA already stores temperature,
//! dew point, pressure and wind speed as tenths, so the underlying quantities have one
//! decimal digit of precision. That is exactly the shape real station data has, and exactly
//! the shape the decimal codec is built to exploit.
//!
//! <https://www.ncei.noaa.gov/pub/data/noaa/isd-lite/>

use crate::Point;

const MISSING: i64 = -9999;

pub struct Variable {
    pub name: &'static str,
    pub unit: &'static str,
    column: usize,
    divisor: f64,
}

pub const VARIABLES: [Variable; 5] = [
    Variable {
        name: "air_temperature",
        unit: "degC",
        column: 4,
        divisor: 10.0,
    },
    Variable {
        name: "dew_point",
        unit: "degC",
        column: 5,
        divisor: 10.0,
    },
    Variable {
        name: "sea_level_pressure",
        unit: "hPa",
        column: 6,
        divisor: 10.0,
    },
    Variable {
        name: "wind_direction",
        unit: "deg",
        column: 7,
        divisor: 1.0,
    },
    Variable {
        name: "wind_speed",
        unit: "m/s",
        column: 8,
        divisor: 10.0,
    },
];

pub struct Series {
    pub variable: &'static str,
    pub unit: &'static str,
    pub points: Vec<Point>,
}

/// Splits one ISD-Lite file into one series per variable. Rows flagged `-9999` are dropped
/// rather than interpolated, so the gaps a real station leaves behind survive into the
/// benchmark instead of being smoothed away.
pub fn parse(text: &str) -> Vec<Series> {
    let mut series: Vec<Series> = VARIABLES
        .iter()
        .map(|v| Series {
            variable: v.name,
            unit: v.unit,
            points: Vec::new(),
        })
        .collect();

    for line in text.lines() {
        let fields: Vec<i64> = line
            .split_whitespace()
            .filter_map(|f| f.parse::<i64>().ok())
            .collect();
        if fields.len() < 9 {
            continue;
        }
        let timestamp = to_epoch(fields[0], fields[1] as u32, fields[2] as u32, fields[3]);
        for (index, variable) in VARIABLES.iter().enumerate() {
            let raw = fields[variable.column];
            if raw != MISSING {
                series[index]
                    .points
                    .push(Point::new(timestamp, raw as f64 / variable.divisor));
            }
        }
    }

    series.retain(|s| !s.points.is_empty());
    series
}

fn to_epoch(year: i64, month: u32, day: u32, hour: i64) -> i64 {
    days_from_civil(year, month, day) * 86_400 + hour * 3_600
}

/// Howard Hinnant's civil-date algorithm, which avoids pulling in a date crate for what is
/// ultimately a handful of integer operations.
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

    const SAMPLE: &str = "\
2023 01 01 00    82    49 10241   150    10 -9999 -9999     0
2023 01 01 01    78    39 10237    80    10 -9999 -9999 -9999
2023 01 01 02 -9999    36 10236    80    15 -9999 -9999 -9999
";

    #[test]
    fn known_dates_match_their_epoch_seconds() {
        assert_eq!(to_epoch(1970, 1, 1, 0), 0);
        assert_eq!(to_epoch(2000, 3, 1, 0), 951_868_800);
        assert_eq!(to_epoch(2023, 1, 1, 0), 1_672_531_200);
        assert_eq!(to_epoch(2024, 2, 29, 12), 1_709_208_000);
        assert_eq!(to_epoch(1969, 12, 31, 23), -3_600);
    }

    #[test]
    fn tenths_become_the_values_a_station_actually_reports() {
        let series = parse(SAMPLE);
        let temperature = series
            .iter()
            .find(|s| s.variable == "air_temperature")
            .unwrap();
        assert_eq!(temperature.points[0].value, 8.2);
        assert_eq!(temperature.points[1].value, 7.8);
    }

    #[test]
    fn missing_readings_leave_a_gap_instead_of_a_value() {
        let series = parse(SAMPLE);
        let temperature = series
            .iter()
            .find(|s| s.variable == "air_temperature")
            .unwrap();
        let pressure = series
            .iter()
            .find(|s| s.variable == "sea_level_pressure")
            .unwrap();
        assert_eq!(temperature.points.len(), 2);
        assert_eq!(pressure.points.len(), 3);
        assert_eq!(
            pressure.points[2].timestamp - pressure.points[1].timestamp,
            3_600
        );
    }

    #[test]
    fn short_or_junk_lines_are_skipped() {
        assert!(parse("not a row\n\n2023 01\n").is_empty());
    }
}
