//! NOAA CO-OPS verified water level, six-minute cadence.
//!
//! The opposite shape to a weather station: a smooth, strongly periodic signal sampled ten
//! times an hour and reported to the millimetre, so values carry three decimal digits instead
//! of one.
//!
//! <https://api.tidesandcurrents.noaa.gov/api/prod/datagetter>

use alloc::vec::Vec;

use super::{parse_datetime, Series};
use crate::Point;

const SOURCE: &str = "co-ops";

/// Column 2 is the standard deviation of the samples that produced the reading, which is a
/// genuinely different signal — small, noisy, and worth measuring separately.
const VARIABLES: [(&str, &str, usize); 2] =
    [("water_level", "m", 1), ("water_level_sigma", "m", 2)];

pub fn parse(text: &str) -> Vec<Series> {
    let mut series: Vec<Series> = VARIABLES
        .iter()
        .map(|&(name, unit, _)| Series {
            source: SOURCE,
            variable: name,
            unit,
            points: Vec::new(),
        })
        .collect();

    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 3 {
            continue;
        }
        let Some(timestamp) = parse_datetime(fields[0]) else {
            continue;
        };
        for (index, &(_, _, column)) in VARIABLES.iter().enumerate() {
            if let Ok(value) = fields[column].trim().parse::<f64>() {
                series[index].points.push(Point::new(timestamp, value));
            }
        }
    }

    series.retain(|s| !s.points.is_empty());
    series
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Date Time, Water Level, Sigma, O or I (for verified), F, R, L, Quality
2023-01-01 00:00,0.492,0.006,0,0,0,0,v
2023-01-01 00:06,0.481,0.005,0,0,0,0,v
2023-01-01 00:12,,,0,0,0,0,v
";

    #[test]
    fn millimetre_readings_survive_parsing() {
        let series = parse(SAMPLE);
        let level = series.iter().find(|s| s.variable == "water_level").unwrap();
        assert_eq!(level.points[0].value, 0.492);
        assert_eq!(level.points[1].timestamp - level.points[0].timestamp, 360);
    }

    #[test]
    fn blank_readings_are_dropped() {
        let series = parse(SAMPLE);
        assert!(series.iter().all(|s| s.points.len() == 2));
    }

    #[test]
    fn the_header_is_not_read_as_data() {
        assert!(parse("Date Time, Water Level, Sigma\n").is_empty());
    }
}
