//! Chimp, from "Chimp: Efficient Lossless Floating Point Compression for Time Series
//! Databases" (VLDB 2022). For the windowed variant see [`super::chimp128`].

use crate::bits::{BitReader, BitWriter};
use crate::codec::{finish_block, read_count, start_block, Codec, Dod, TAG_CHIMP};
use crate::error::{Error, Result};
use crate::Point;

pub(crate) const LEADING_ROUND: [u32; 8] = [0, 8, 12, 16, 18, 20, 22, 24];

#[rustfmt::skip]
pub(crate) const LEADING_INDEX: [u8; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 2, 2,
    3, 3, 4, 4, 5, 5, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
];

/// An explicit trailing-zero count costs 9 extra bits, so it only pays off beyond this many.
const TRAILING_THRESHOLD: u32 = 6;

const NO_WINDOW: u32 = u32::MAX;

pub struct Chimp;

impl Codec for Chimp {
    fn name(&self) -> &'static str {
        "chimp"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        let (head, mut w) = start_block(TAG_CHIMP, points)?;
        if let Some(first) = points.first() {
            w.write_bits(first.timestamp as u64, 64);
            w.write_bits(first.value.to_bits(), 64);
            let mut timestamps = Dod::new(first.timestamp);
            let mut values = ChimpWriter::new(first.value.to_bits());
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
    let mut values = ChimpReader::new(bits);
    for _ in 1..count {
        let timestamp = timestamps.read(&mut r)?;
        let bits = values.read(&mut r)?;
        points.push(Point::new(timestamp, f64::from_bits(bits)));
    }
    Ok(points)
}

struct ChimpWriter {
    prev: u64,
    leading: u32,
}

impl ChimpWriter {
    fn new(first: u64) -> Self {
        ChimpWriter {
            prev: first,
            leading: NO_WINDOW,
        }
    }

    fn write(&mut self, w: &mut BitWriter, bits: u64) {
        let xor = self.prev ^ bits;
        self.prev = bits;
        if xor == 0 {
            w.write_bits(0b00, 2);
            return;
        }

        let index = LEADING_INDEX[xor.leading_zeros() as usize] as usize;
        let leading = LEADING_ROUND[index];
        let trailing = xor.trailing_zeros();

        if trailing > TRAILING_THRESHOLD {
            let width = 64 - leading - trailing;
            w.write_bits(0b01, 2);
            w.write_bits(index as u64, 3);
            w.write_bits(width as u64, 6);
            w.write_bits(xor >> trailing, width);
            // This window is narrower than a '10' would imply, so it cannot be the reference.
            self.leading = NO_WINDOW;
        } else if leading == self.leading {
            w.write_bits(0b10, 2);
            w.write_bits(xor, 64 - leading);
        } else {
            w.write_bits(0b11, 2);
            w.write_bits(index as u64, 3);
            w.write_bits(xor, 64 - leading);
            self.leading = leading;
        }
    }
}

struct ChimpReader {
    prev: u64,
    leading: u32,
}

impl ChimpReader {
    fn new(first: u64) -> Self {
        ChimpReader {
            prev: first,
            leading: NO_WINDOW,
        }
    }

    fn read(&mut self, r: &mut BitReader) -> Result<u64> {
        let xor = match r.read_bits(2)? {
            0b00 => 0,
            0b01 => {
                let index = r.read_bits(3)? as usize;
                let leading = LEADING_ROUND[index];
                let width = r.read_bits(6)? as u32;
                if width == 0 || leading + width > 64 {
                    return Err(Error::MalformedBlock);
                }
                self.leading = NO_WINDOW;
                r.read_bits(width)? << (64 - leading - width)
            }
            0b10 => {
                if self.leading == NO_WINDOW {
                    return Err(Error::MalformedBlock);
                }
                r.read_bits(64 - self.leading)?
            }
            _ => {
                let index = r.read_bits(3)? as usize;
                self.leading = LEADING_ROUND[index];
                r.read_bits(64 - self.leading)?
            }
        };
        self.prev ^= xor;
        Ok(self.prev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode as decode_block;

    fn roundtrip(points: &[Point]) {
        let block = Chimp.encode(points).unwrap();
        assert_eq!(decode_block(&block).unwrap(), points);
    }

    #[test]
    fn empty_and_single_point_blocks() {
        roundtrip(&[]);
        roundtrip(&[Point::new(1_700_000_000, 21.5)]);
    }

    #[test]
    fn every_branch_gets_exercised() {
        roundtrip(&[
            Point::new(0, 1.0),
            Point::new(3600, 1.0),
            Point::new(7200, 1.5),
            Point::new(10800, 1.5000000001),
            Point::new(14400, -273.15),
            Point::new(18000, 1e300),
            Point::new(21600, f64::NAN),
            Point::new(25200, f64::INFINITY),
            Point::new(28800, -0.0),
            Point::new(32400, 0.0),
        ]);
    }

    #[test]
    fn the_leading_table_agrees_with_the_rounding_table() {
        for leading in 0..64u32 {
            let rounded = LEADING_ROUND[LEADING_INDEX[leading as usize] as usize];
            assert!(
                rounded <= leading,
                "rounding must never overstate leading zeros"
            );
            let next = LEADING_ROUND
                .iter()
                .copied()
                .find(|&candidate| candidate > rounded);
            if let Some(next) = next {
                assert!(
                    leading < next,
                    "leading {leading} should have rounded to {next}"
                );
            }
        }
    }

    #[test]
    fn a_long_realistic_series() {
        let points: Vec<Point> = (0..5000)
            .map(|i| {
                let daily = (i as f64 / 24.0 * std::f64::consts::TAU).sin() * 6.0;
                Point::new(
                    1_700_000_000 + i * 3600,
                    ((12.0 + daily) * 10.0).round() / 10.0,
                )
            })
            .collect();
        roundtrip(&points);
    }

    #[test]
    fn truncated_blocks_are_rejected_without_panicking() {
        let points: Vec<Point> = (0..50)
            .map(|i| Point::new(1_700_000_000 + i * 3600, i as f64 * 1.7))
            .collect();
        let block = Chimp.encode(&points).unwrap();
        for cut in 1..block.len() {
            let _ = decode_block(&block[..cut]);
        }
    }
}
