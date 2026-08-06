//! Decimal scaling, the approach VictoriaMetrics documented for beating Gorilla on real
//! metrics: rescale the block to exact integers, then delta-of-delta those.
//!
//! The scale is a block-level property, so this codec cannot stream — it has to see every
//! value before it can emit anything.

use alloc::vec::Vec;

use crate::bits::BitReader;
use crate::codec::{
    decode_block, encode_block, gorilla, Codec, Dod, ValueCoding, ValueDecoder, ValueEncoder,
    TAG_DECIMAL,
};
use crate::error::{Error, Result};
use crate::Point;

pub(crate) const MAX_SCALE: u8 = 17;

pub(crate) const POW10: [f64; 18] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17,
];

/// Above 2^53 an f64 stops representing every integer, so a scaled value beyond it could not
/// be reconstructed.
const MAX_EXACT: f64 = 9_007_199_254_740_992.0;

pub struct Decimal;

impl Codec for Decimal {
    fn name(&self) -> &'static str {
        "decimal"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        match block_scale(points) {
            Some(scale) => encode_block(TAG_DECIMAL, &ScaledInteger::new(scale), points),
            None => gorilla::Gorilla.encode(points),
        }
    }
}

/// Values travel as integers scaled by a power of ten, then through delta-of-delta.
struct ScaledInteger {
    scale: u8,
    factor: f64,
}

impl ScaledInteger {
    fn new(scale: u8) -> Self {
        ScaledInteger {
            scale,
            factor: POW10[scale as usize],
        }
    }
}

impl ValueCoding for ScaledInteger {
    type Encoder = IntegerDelta;
    type Decoder = IntegerDelta;

    fn pack(&self, value: f64) -> u64 {
        scaled(value, self.factor).expect("block scale was validated over every value") as u64
    }

    fn unpack(&self, bits: u64) -> f64 {
        bits as i64 as f64 / self.factor
    }

    fn encoder(&self, first: u64) -> IntegerDelta {
        IntegerDelta(Dod::new(first as i64))
    }

    fn decoder(&self, first: u64) -> IntegerDelta {
        IntegerDelta(Dod::new(first as i64))
    }

    fn write_header(&self, w: &mut crate::bits::BitWriter) {
        w.write_bits(self.scale as u64, 8);
    }
}

struct IntegerDelta(Dod);

impl ValueEncoder for IntegerDelta {
    fn write(&mut self, w: &mut crate::bits::BitWriter, bits: u64) {
        self.0.write(w, bits as i64);
    }
}

impl ValueDecoder for IntegerDelta {
    fn read(&mut self, r: &mut BitReader) -> Result<u64> {
        Ok(self.0.read(r)? as u64)
    }
}

pub(crate) fn decode(body: &[u8]) -> Result<Vec<Point>> {
    decode_block(
        |r| {
            let scale = r.read_bits(8)? as usize;
            if scale >= POW10.len() {
                return Err(Error::MalformedBlock);
            }
            Ok(ScaledInteger::new(scale as u8))
        },
        body,
    )
}

/// Rounds to the nearest integer without `f64::round`, which lives in std. Taking the
/// difference against the truncation stays exact right up to 2^53, where adding 0.5 first would
/// already have been swallowed by the gap between representable values.
pub(crate) fn round_nearest(value: f64) -> i64 {
    let truncated = value as i64;
    let fraction = value - truncated as f64;
    if fraction >= 0.5 {
        truncated + 1
    } else if fraction <= -0.5 {
        truncated - 1
    } else {
        truncated
    }
}

/// `None` means the block has to fall back to Gorilla.
pub(crate) fn block_scale(points: &[Point]) -> Option<u8> {
    (0..=MAX_SCALE).find(|&scale| {
        let factor = POW10[scale as usize];
        points.iter().all(|p| scaled(p.value, factor).is_some())
    })
}

pub(crate) fn scaled(value: f64, factor: f64) -> Option<i64> {
    // Negative zero comes back as +0.0 once it has been through an integer.
    if value == 0.0 && value.is_sign_negative() {
        return None;
    }
    let scaled = value * factor;
    if !scaled.is_finite() || scaled.abs() > MAX_EXACT {
        return None;
    }
    // The test that matters is whether the round trip reproduces the exact bit pattern, not
    // whether the multiplication landed on an integer. 8.55 * 100 is 855.0000000000001, but 855
    // divided by 100 is 8.55 to the bit, so the reading needs a scale of 2 rather than the 15 an
    // integrality test would demand — and scales that large push the arithmetic into the range
    // above 2^53 where it stops being exact at all.
    let rounded = round_nearest(scaled);
    if (rounded as f64 / factor).to_bits() != value.to_bits() {
        return None;
    }
    Some(rounded)
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
