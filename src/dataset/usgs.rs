//! USGS instantaneous river values in RDB format, fifteen-minute cadence.
//!
//! A third shape again: discharge is reported as whole cubic feet per second and runs into the
//! tens of thousands, while gage height alongside it carries two decimals. The two live in the
//! same file, which makes it a clean test of how much the codec ranking depends on how a
//! number was rounded before anyone stored it.
//!
//! Timestamps are read as the wall clock the gauge reported, without applying the `tz_cd`
//! offset, so the daylight-saving jumps stay in the series where the delta-of-delta encoding
//! has to deal with them.
//!
//! <https://waterservices.usgs.gov/nwis/iv/>

use alloc::vec::Vec;

use super::{parse_datetime, Series};
use crate::Point;

const SOURCE: &str = "usgs-nwis";

const VARIABLES: [(&str, &str, &str); 2] = [
    ("discharge", "ft3/s", "_00060"),
    ("gage_height", "ft", "_00065"),
];

pub fn parse(text: &str) -> Vec<Series> {
    let mut rows = text.lines().filter(|line| !line.starts_with('#'));
    let Some(header) = rows.next() else {
        return Vec::new();
    };
    let columns: Vec<&str> = header.split('\t').collect();
    let datetime = columns.iter().position(|&c| c == "datetime");

    let mut series = Vec::new();
    let mut sources = Vec::new();
    for (name, unit, suffix) in VARIABLES {
        if let Some(column) = columns
            .iter()
            .position(|c| c.ends_with(suffix) && !c.ends_with("_cd"))
        {
            series.push(Series {
                source: SOURCE,
                variable: name,
                unit,
                points: Vec::new(),
            });
            sources.push(column);
        }
    }

    let Some(datetime) = datetime else {
        return Vec::new();
    };

    // The row after the header declares column widths, not data.
    for line in rows.skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let Some(field) = fields.get(datetime) else {
            continue;
        };
        let Some(timestamp) = parse_datetime(field) else {
            continue;
        };
        for (index, &column) in sources.iter().enumerate() {
            if let Some(Ok(value)) = fields.get(column).map(|f| f.trim().parse::<f64>()) {
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
# comment line
# another
agency_cd\tsite_no\tdatetime\ttz_cd\t69928_00060\t69928_00060_cd\t69929_00065\t69929_00065_cd
5s\t15s\t20d\t6s\t14n\t10s\t14n\t10s
USGS\t01646500\t2023-01-01 00:00\tEST\t9760\tA\t3.95\tA
USGS\t01646500\t2023-01-01 00:15\tEST\t9760\tA\t3.95\tA
USGS\t01646500\t2023-01-01 00:30\tEST\t9660\tA\t3.94\tA
";

    #[test]
    fn both_parameters_come_out_of_the_same_file() {
        let series = parse(SAMPLE);
        assert_eq!(series.len(), 2);
        let discharge = series.iter().find(|s| s.variable == "discharge").unwrap();
        let height = series.iter().find(|s| s.variable == "gage_height").unwrap();
        assert_eq!(discharge.points[0].value, 9760.0);
        assert_eq!(height.points[0].value, 3.95);
        assert_eq!(discharge.points.len(), 3);
    }

    #[test]
    fn the_width_declaration_row_is_not_data() {
        let series = parse(SAMPLE);
        let discharge = series.iter().find(|s| s.variable == "discharge").unwrap();
        assert_eq!(
            discharge.points[1].timestamp - discharge.points[0].timestamp,
            900
        );
    }

    #[test]
    fn a_file_with_only_comments_yields_nothing() {
        assert!(parse("# just\n# comments\n").is_empty());
        assert!(parse("").is_empty());
    }
}
