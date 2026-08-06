//! Gorilla, from "Gorilla: A Fast, Scalable, In-Memory Time Series Database" (VLDB 2015).
//! The baseline every other codec here is measured against.

use alloc::vec::Vec;

use crate::bits::{BitReader, BitWriter};
use crate::codec::{
    decode_block, encode_block, Codec, ValueCoding, ValueDecoder, ValueEncoder, TAG_GORILLA,
};
use crate::error::{Error, Result};
use crate::Point;

/// The leading-zero field is 5 bits, so higher counts clamp and the extra zeros ride along
/// inside the significant-bit window.
const MAX_LEADING: u32 = 31;

const NO_WINDOW: u32 = u32::MAX;

pub struct Gorilla;

impl Codec for Gorilla {
    fn name(&self) -> &'static str {
        "gorilla"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        encode_block(TAG_GORILLA, &RawXor, points)
    }
}

struct RawXor;

impl ValueCoding for RawXor {
    type Encoder = XorWriter;
    type Decoder = XorReader;

    fn pack(&self, value: f64) -> u64 {
        value.to_bits()
    }

    fn unpack(&self, bits: u64) -> f64 {
        f64::from_bits(bits)
    }

    fn encoder(&self, first: u64) -> XorWriter {
        XorWriter::new(first)
    }

    fn decoder(&self, first: u64) -> XorReader {
        XorReader::new(first)
    }
}

pub(crate) fn decode(body: &[u8]) -> Result<Vec<Point>> {
    decode_block(|_| Ok(RawXor), body)
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
}

impl ValueEncoder for XorWriter {
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
            // Width ranges 1..=64 in a 6-bit field, so 64 wraps to 0. Zero cannot occur
            // because xor != 0 here.
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
}

impl ValueDecoder for XorReader {
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
        let expected_bits: usize = 16   // point count, varint, two groups
            + 40                 // first timestamp, zigzag varint, five groups
            + 64                 // first value, raw bit pattern
            + 21                 // second timestamp: an hourly step needs the 16-bit bucket
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
