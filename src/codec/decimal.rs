//! Decimal scaling, the trick VictoriaMetrics uses to beat Gorilla on real metrics.
//!
//! XOR compression assumes consecutive values share most of their binary representation, but
//! a reading like 12.3 °C is a decimal quantity that IEEE-754 stores as an awkward binary
//! fraction. Multiplying the block by the smallest 10^s that turns every value into an exact
//! integer recovers the structure, and integers respond far better to delta-of-delta.
//!
//! The scale is a property of the whole block, so unlike Gorilla and Chimp this codec has to
//! see every value before it can emit anything.

use crate::bits::BitReader;
use crate::codec::{finish_block, gorilla, read_count, start_block, Codec, Dod, TAG_DECIMAL};
use crate::error::{Error, Result};
use crate::Point;

const MAX_SCALE: u8 = 17;

const POW10: [f64; 18] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17,
];

/// Above 2^53 an f64 can no longer represent every integer, so a scaled value beyond it could
/// not be reconstructed.
const MAX_EXACT: f64 = 9_007_199_254_740_992.0;

pub struct Decimal;

impl Codec for Decimal {
    fn name(&self) -> &'static str {
        "decimal"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        let Some(scale) = block_scale(points) else {
            return gorilla::Gorilla.encode(points);
        };
        let factor = POW10[scale as usize];

        let (head, mut w) = start_block(TAG_DECIMAL, points)?;
        w.write_bits(scale as u64, 8);
        if let Some(first) = points.first() {
            let first_value = scaled(first.value, factor).expect("scale was validated");
            w.write_bits(first.timestamp as u64, 64);
            w.write_bits(first_value as u64, 64);
            let mut timestamps = Dod::new(first.timestamp);
            let mut values = Dod::new(first_value);
            for point in &points[1..] {
                timestamps.write(&mut w, point.timestamp);
                values.write(
                    &mut w,
                    scaled(point.value, factor).expect("scale was validated"),
                );
            }
        }
        Ok(finish_block(head, w))
    }
}

pub(crate) fn decode(body: &[u8]) -> Result<Vec<Point>> {
    let mut r = BitReader::new(body);
    let count = read_count(&mut r)?;
    let scale = r.read_bits(8)? as usize;
    if scale >= POW10.len() {
        return Err(Error::MalformedBlock);
    }
    let factor = POW10[scale];

    let mut points = Vec::with_capacity(count);
    if count == 0 {
        return Ok(points);
    }
    let timestamp = r.read_bits(64)? as i64;
    let value = r.read_bits(64)? as i64;
    points.push(Point::new(timestamp, value as f64 / factor));

    let mut timestamps = Dod::new(timestamp);
    let mut values = Dod::new(value);
    for _ in 1..count {
        let timestamp = timestamps.read(&mut r)?;
        let value = values.read(&mut r)?;
        points.push(Point::new(timestamp, value as f64 / factor));
    }
    Ok(points)
}

/// Smallest scale that makes every value in the block an exactly representable integer, or
/// `None` when the block has to fall back to Gorilla.
fn block_scale(points: &[Point]) -> Option<u8> {
    (0..=MAX_SCALE).find(|&scale| {
        let factor = POW10[scale as usize];
        points.iter().all(|p| scaled(p.value, factor).is_some())
    })
}

fn scaled(value: f64, factor: f64) -> Option<i64> {
    // Negative zero would come back as +0.0 once it has been through an integer, and callers
    // that compare bit patterns would see the difference.
    if value == 0.0 && value.is_sign_negative() {
        return None;
    }
    let scaled = value * factor;
    if !scaled.is_finite() || scaled.fract() != 0.0 || scaled.abs() > MAX_EXACT {
        return None;
    }
    if scaled / factor != value {
        return None;
    }
    Some(scaled as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode as decode_block, TAG_GORILLA};

    fn roundtrip(points: &[Point]) -> Vec<u8> {
        let block = Decimal.encode(points).unwrap();
        assert_eq!(decode_block(&block).unwrap(), points);
        block
    }

    #[test]
    fn empty_and_single_point_blocks() {
        roundtrip(&[]);
        roundtrip(&[Point::new(1_700_000_000, 21.5)]);
    }

    #[test]
    fn tenths_of_a_degree_pick_scale_one() {
        let points: Vec<Point> = (0..200)
            .map(|i| Point::new(1_700_000_000 + i * 3600, 8.0 + (i % 30) as f64 / 10.0))
            .collect();
        let block = roundtrip(&points);
        assert_eq!(block[0], TAG_DECIMAL);
        assert_eq!(block[5], 1);
    }

    #[test]
    fn integers_pick_scale_zero() {
        let points: Vec<Point> = (0..100)
            .map(|i| Point::new(1_700_000_000 + i * 3600, (950 + i % 40) as f64))
            .collect();
        let block = roundtrip(&points);
        assert_eq!(block[5], 0);
    }

    #[test]
    fn a_single_awkward_value_forces_the_whole_block_to_gorilla() {
        let mut points: Vec<Point> = (0..100)
            .map(|i| Point::new(1_700_000_000 + i * 3600, 8.0 + (i % 30) as f64 / 10.0))
            .collect();
        points[50].value = std::f64::consts::PI;
        let block = roundtrip(&points);
        assert_eq!(block[0], TAG_GORILLA);
    }

    #[test]
    fn special_values_fall_back_rather_than_losing_data() {
        for odd in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 1e300] {
            let points = vec![Point::new(0, 1.5), Point::new(3600, odd)];
            let block = roundtrip(&points);
            assert_eq!(block[0], TAG_GORILLA, "value {odd:?} should not scale");
        }
    }

    #[test]
    fn beats_gorilla_on_station_shaped_data() {
        let points: Vec<Point> = (0..2000)
            .map(|i| {
                let daily = (i as f64 / 24.0 * std::f64::consts::TAU).sin() * 6.0;
                Point::new(
                    1_700_000_000 + i * 3600,
                    ((12.0 + daily) * 10.0).round() / 10.0,
                )
            })
            .collect();
        let decimal = Decimal.encode(&points).unwrap().len();
        let gorilla = gorilla::Gorilla.encode(&points).unwrap().len();
        assert!(
            decimal < gorilla,
            "decimal {decimal} bytes should beat gorilla {gorilla} bytes"
        );
    }
}
