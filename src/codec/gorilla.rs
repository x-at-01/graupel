//! Gorilla, from "Gorilla: A Fast, Scalable, In-Memory Time Series Database" (VLDB 2015).
//!
//! Values are XORed against the previous one and only the window of bits that actually
//! changed is stored. This is the baseline every other codec here is measured against.

use crate::bits::{BitReader, BitWriter};
use crate::codec::{finish_block, read_count, start_block, Codec, Dod, TAG_GORILLA};
use crate::error::{Error, Result};
use crate::Point;

/// The paper spends 5 bits on the leading-zero count, so counts above 31 are clamped.
/// The extra zeros then travel inside the significant-bit window, which stays lossless.
const MAX_LEADING: u32 = 31;

const NO_WINDOW: u32 = u32::MAX;

pub struct Gorilla;

impl Codec for Gorilla {
    fn name(&self) -> &'static str {
        "gorilla"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        let (head, mut w) = start_block(TAG_GORILLA, points)?;
        if let Some(first) = points.first() {
            w.write_bits(first.timestamp as u64, 64);
            w.write_bits(first.value.to_bits(), 64);
            let mut timestamps = Dod::new(first.timestamp);
            let mut values = XorWriter::new(first.value.to_bits());
            for point in &points[1..] {
                timestamps.write(&mut w, point.timestamp);
                values.write(&mut w, point.value.to_bits());
            }
        }
        Ok(finish_block(head, w))
    }
}

pub(crate) fn decode(body: &[u8]) -> Result<Vec<Point>> {
    let mut r = BitReader::new(body);
    let count = read_count(&mut r)?;
    let mut points = Vec::with_capacity(count);
    if count == 0 {
        return Ok(points);
    }
    let timestamp = r.read_bits(64)? as i64;
    let bits = r.read_bits(64)?;
    points.push(Point::new(timestamp, f64::from_bits(bits)));

    let mut timestamps = Dod::new(timestamp);
    let mut values = XorReader::new(bits);
    for _ in 1..count {
        let timestamp = timestamps.read(&mut r)?;
        let bits = values.read(&mut r)?;
        points.push(Point::new(timestamp, f64::from_bits(bits)));
    }
    Ok(points)
}

struct XorWriter {
    prev: u64,
    leading: u32,
    trailing: u32,
}

impl XorWriter {
    fn new(first: u64) -> Self {
        XorWriter {
            prev: first,
            leading: NO_WINDOW,
            trailing: 0,
        }
    }

    fn write(&mut self, w: &mut BitWriter, bits: u64) {
        let xor = self.prev ^ bits;
        self.prev = bits;
        if xor == 0 {
            w.write_bit(false);
            return;
        }
        w.write_bit(true);

        let leading = xor.leading_zeros().min(MAX_LEADING);
        let trailing = xor.trailing_zeros();
        let reusable =
            self.leading != NO_WINDOW && leading >= self.leading && trailing >= self.trailing;

        if reusable {
            w.write_bit(false);
            w.write_bits(xor >> self.trailing, 64 - self.leading - self.trailing);
        } else {
            w.write_bit(true);
            w.write_bits(leading as u64, 5);
            let width = 64 - leading - trailing;
            // The width field is 6 bits wide but the range is 1..=64, so a full 64-bit window
            // is stored as 0. Width can never actually be zero here because xor != 0.
            w.write_bits((width & 0x3F) as u64, 6);
            w.write_bits(xor >> trailing, width);
            self.leading = leading;
            self.trailing = trailing;
        }
    }
}

struct XorReader {
    prev: u64,
    leading: u32,
    trailing: u32,
}

impl XorReader {
    fn new(first: u64) -> Self {
        XorReader {
            prev: first,
            leading: NO_WINDOW,
            trailing: 0,
        }
    }

    fn read(&mut self, r: &mut BitReader) -> Result<u64> {
        if !r.read_bit()? {
            return Ok(self.prev);
        }
        if r.read_bit()? {
            self.leading = r.read_bits(5)? as u32;
            let stored = r.read_bits(6)? as u32;
            let width = if stored == 0 { 64 } else { stored };
            if self.leading + width > 64 {
                return Err(Error::MalformedBlock);
            }
            self.trailing = 64 - self.leading - width;
        } else if self.leading == NO_WINDOW {
            return Err(Error::MalformedBlock);
        }
        let width = 64 - self.leading - self.trailing;
        let xor = r.read_bits(width)? << self.trailing;
        self.prev ^= xor;
        Ok(self.prev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode as decode_block;

    fn roundtrip(points: &[Point]) {
        let block = Gorilla.encode(points).unwrap();
        assert_eq!(decode_block(&block).unwrap(), points);
    }

    #[test]
    fn empty_and_single_point_blocks() {
        roundtrip(&[]);
        roundtrip(&[Point::new(1_700_000_000, 21.5)]);
    }

    #[test]
    fn a_steady_temperature_series() {
        let points: Vec<Point> = (0..500)
            .map(|i| Point::new(1_700_000_000 + i * 3600, 12.0 + (i % 7) as f64 * 0.1))
            .collect();
        roundtrip(&points);
    }

    #[test]
    fn an_unchanging_series_costs_two_bits_per_point() {
        let points: Vec<Point> = (0..1000)
            .map(|i| Point::new(1_700_000_000 + i * 3600, 4.2))
            .collect();
        let block = Gorilla.encode(&points).unwrap();
        let expected_bits: usize = 32   // point count
            + 64                 // first timestamp
            + 64                 // first value
            + 68                 // second timestamp: an hourly step overflows every narrow bucket
            + 1                  // second value, unchanged
            + 998 * 2; // steady interval and unchanged value, one bit each
        assert_eq!(block.len(), 1 + expected_bits.div_ceil(8));
    }

    #[test]
    fn extreme_and_special_values() {
        roundtrip(&[
            Point::new(0, 0.0),
            Point::new(1, -0.0),
            Point::new(2, f64::NAN),
            Point::new(3, f64::INFINITY),
            Point::new(4, f64::NEG_INFINITY),
            Point::new(5, f64::MIN_POSITIVE),
            Point::new(6, f64::MAX),
            Point::new(7, f64::MIN),
            Point::new(i64::MAX, 1.0),
            Point::new(i64::MIN, 1.0),
        ]);
    }

    #[test]
    fn truncated_blocks_are_rejected_without_panicking() {
        let points: Vec<Point> = (0..50)
            .map(|i| Point::new(1_700_000_000 + i * 3600, i as f64 * 1.7))
            .collect();
        let block = Gorilla.encode(&points).unwrap();
        for cut in 1..block.len() {
            let _ = decode_block(&block[..cut]);
        }
    }
}
