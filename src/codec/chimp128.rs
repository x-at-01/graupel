//! Chimp128, the windowed variant from the same paper as [`super::chimp`]: XOR against the
//! best of the last 128 values rather than always the previous one.
//!
//! Two values agreeing on their low mantissa bits XOR to something with many trailing zeros,
//! so keying a side table on those bits finds a good reference in one lookup.
//!
//! Bit layout follows <https://github.com/panagiotisl/chimp>.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::bits::{BitReader, BitWriter};
use crate::codec::chimp::{LEADING_INDEX, LEADING_ROUND};
use crate::codec::{finish_block, read_count, start_block, Codec, Dod, TAG_CHIMP128};
use crate::error::{Error, Result};
use crate::Point;

const WINDOW: usize = 128;
const WINDOW_BITS: u32 = 7;

/// Naming a reference costs `WINDOW_BITS` plus 6 for the significant-bit count, so it only
/// wins beyond that many trailing zeros.
const TRAILING_THRESHOLD: u32 = 6 + WINDOW_BITS;

const KEY_BITS: u32 = TRAILING_THRESHOLD + 1;
const KEY_MASK: u64 = (1 << KEY_BITS) - 1;
const TABLE_LEN: usize = 1 << KEY_BITS;

const NO_WINDOW: u32 = u32::MAX;

pub struct Chimp128;

impl Codec for Chimp128 {
    fn name(&self) -> &'static str {
        "chimp128"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        let (head, mut w) = start_block(TAG_CHIMP128, points)?;
        if let Some(first) = points.first() {
            w.write_bits(first.timestamp as u64, 64);
            w.write_bits(first.value.to_bits(), 64);
            let mut timestamps = Dod::new(first.timestamp);
            let mut values = Chimp128Writer::new(first.value.to_bits());
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
    let mut values = Chimp128Reader::new(bits);
    for _ in 1..count {
        let timestamp = timestamps.read(&mut r)?;
        let bits = values.read(&mut r)?;
        points.push(Point::new(timestamp, f64::from_bits(bits)));
    }
    Ok(points)
}

struct Chimp128Writer {
    stored: [u64; WINDOW],
    /// Low `KEY_BITS` of a value to the sequence number that last held it.
    recent: Box<[u32; TABLE_LEN]>,
    sequence: u32,
    current: usize,
    leading: u32,
}

impl Chimp128Writer {
    fn new(first: u64) -> Self {
        let mut writer = Chimp128Writer {
            stored: [0; WINDOW],
            recent: Box::new([0; TABLE_LEN]),
            sequence: 0,
            current: 0,
            leading: NO_WINDOW,
        };
        writer.stored[0] = first;
        writer.recent[(first & KEY_MASK) as usize] = 0;
        writer
    }

    fn write(&mut self, w: &mut BitWriter, bits: u64) {
        let key = (bits & KEY_MASK) as usize;
        let candidate = self.recent[key];

        // `current` is `sequence` modulo the window, so a stored sequence number maps straight
        // onto a slot.
        let (reference, xor) = if self.sequence - candidate < WINDOW as u32 {
            let slot = candidate as usize % WINDOW;
            let xor = bits ^ self.stored[slot];
            if xor.trailing_zeros() > TRAILING_THRESHOLD {
                (slot, xor)
            } else {
                (self.current, bits ^ self.stored[self.current])
            }
        } else {
            (self.current, bits ^ self.stored[self.current])
        };

        if xor == 0 {
            w.write_bits(0b00, 2);
            w.write_bits(reference as u64, WINDOW_BITS);
            self.leading = NO_WINDOW;
        } else {
            let index = LEADING_INDEX[xor.leading_zeros() as usize] as usize;
            let leading = LEADING_ROUND[index];
            let trailing = xor.trailing_zeros();

            if trailing > TRAILING_THRESHOLD {
                let width = 64 - leading - trailing;
                w.write_bits(0b01, 2);
                w.write_bits(reference as u64, WINDOW_BITS);
                w.write_bits(index as u64, 3);
                w.write_bits(width as u64, 6);
                w.write_bits(xor >> trailing, width);
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

        self.current = (self.current + 1) % WINDOW;
        self.stored[self.current] = bits;
        self.sequence += 1;
        self.recent[key] = self.sequence;
    }
}

struct Chimp128Reader {
    stored: [u64; WINDOW],
    current: usize,
    leading: u32,
}

impl Chimp128Reader {
    fn new(first: u64) -> Self {
        let mut reader = Chimp128Reader {
            stored: [0; WINDOW],
            current: 0,
            leading: NO_WINDOW,
        };
        reader.stored[0] = first;
        reader
    }

    fn read(&mut self, r: &mut BitReader) -> Result<u64> {
        let value = match r.read_bits(2)? {
            0b00 => {
                let reference = r.read_bits(WINDOW_BITS)? as usize;
                self.leading = NO_WINDOW;
                self.stored[reference]
            }
            0b01 => {
                let reference = r.read_bits(WINDOW_BITS)? as usize;
                let leading = LEADING_ROUND[r.read_bits(3)? as usize];
                let width = r.read_bits(6)? as u32;
                if width == 0 || leading + width > 64 {
                    return Err(Error::MalformedBlock);
                }
                let xor = r.read_bits(width)? << (64 - leading - width);
                self.leading = NO_WINDOW;
                self.stored[reference] ^ xor
            }
            0b10 => {
                if self.leading == NO_WINDOW {
                    return Err(Error::MalformedBlock);
                }
                self.stored[self.current] ^ r.read_bits(64 - self.leading)?
            }
            _ => {
                self.leading = LEADING_ROUND[r.read_bits(3)? as usize];
                self.stored[self.current] ^ r.read_bits(64 - self.leading)?
            }
        };
        self.current = (self.current + 1) % WINDOW;
        self.stored[self.current] = value;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::chimp::Chimp;
    use crate::codec::decode as decode_block;

    fn roundtrip(points: &[Point]) {
        let block = Chimp128.encode(points).unwrap();
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
    fn a_value_repeating_outside_the_window_still_decodes() {
        let mut points: Vec<Point> = (0..400)
            .map(|i| Point::new(1_700_000_000 + i * 3600, i as f64 * 0.25))
            .collect();
        points[399].value = points[0].value;
        roundtrip(&points);
    }

    #[test]
    fn beats_plain_chimp_when_a_recent_value_repeats_exactly() {
        use std::f64::consts::{E, LN_2, PI, SQRT_2, TAU};
        let cycle = [PI, E, SQRT_2, LN_2, TAU];
        let points: Vec<Point> = (0..4000)
            .map(|i| Point::new(1_700_000_000 + i as i64 * 3600, cycle[i % cycle.len()]))
            .collect();
        let windowed = Chimp128.encode(&points).unwrap().len();
        let plain = Chimp.encode(&points).unwrap().len();
        assert!(
            windowed < plain,
            "chimp128 {windowed} bytes should beat chimp {plain} bytes"
        );
    }

    /// Whole numbers all collide on key zero, so the candidate degenerates to the previous
    /// value while still costing 7 bits to name. That is why wind direction, reported in whole
    /// degrees, is the one variable where Chimp128 loses to plain Chimp in the benchmark.
    #[test]
    fn whole_numbers_collide_on_key_zero_but_tenths_do_not() {
        for whole in [1013.0f64, 250.0, 0.0, -40.0] {
            assert_eq!(whole.to_bits() & KEY_MASK, 0, "{whole} should collide");
        }
        for tenth in [8.2f64, 7.9, -3.1, 12.4] {
            assert_ne!(
                tenth.to_bits() & KEY_MASK,
                0,
                "{tenth} should have discriminating low bits"
            );
        }
    }

    #[test]
    fn truncated_blocks_are_rejected_without_panicking() {
        let points: Vec<Point> = (0..300)
            .map(|i| Point::new(1_700_000_000 + i * 3600, i as f64 * 1.7))
            .collect();
        let block = Chimp128.encode(&points).unwrap();
        for cut in 1..block.len() {
            let _ = decode_block(&block[..cut]);
        }
    }
}
