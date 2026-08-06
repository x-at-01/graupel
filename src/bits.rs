use alloc::vec::Vec;

use crate::error::{Error, Result};

pub struct BitWriter {
    buf: Vec<u8>,
    free: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        BitWriter {
            buf: Vec::new(),
            free: 0,
        }
    }

    pub fn with_capacity(bytes: usize) -> Self {
        BitWriter {
            buf: Vec::with_capacity(bytes),
            free: 0,
        }
    }

    pub fn write_bit(&mut self, bit: bool) {
        if self.free == 0 {
            self.buf.push(0);
            self.free = 8;
        }
        if bit {
            let last = self.buf.len() - 1;
            self.buf[last] |= 1 << (self.free - 1);
        }
        self.free -= 1;
    }

    pub fn write_bits(&mut self, value: u64, count: u32) {
        debug_assert!(count <= 64);
        let mut left = count;
        while left > 0 {
            if self.free == 0 {
                self.buf.push(0);
                self.free = 8;
            }
            let take = left.min(self.free);
            let chunk = ((value >> (left - take)) & mask(take)) as u8;
            let last = self.buf.len() - 1;
            self.buf[last] |= chunk << (self.free - take);
            self.free -= take;
            left -= take;
        }
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }

    pub fn bit_len(&self) -> usize {
        self.buf.len() * 8 - self.free as usize
    }
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        BitReader { buf, pos: 0 }
    }

    pub fn read_bit(&mut self) -> Result<bool> {
        let byte = self.buf.get(self.pos / 8).ok_or(Error::UnexpectedEnd)?;
        let bit = (byte >> (7 - self.pos % 8)) & 1 == 1;
        self.pos += 1;
        Ok(bit)
    }

    pub fn read_bits(&mut self, count: u32) -> Result<u64> {
        debug_assert!(count <= 64);
        if self.pos + count as usize > self.buf.len() * 8 {
            return Err(Error::UnexpectedEnd);
        }
        let mut out: u64 = 0;
        let mut left = count;
        while left > 0 {
            let byte = self.buf[self.pos / 8];
            let offset = (self.pos % 8) as u32;
            let available = 8 - offset;
            let take = left.min(available);
            let chunk = (byte as u64 >> (available - take)) & mask(take);
            out = (out << take) | chunk;
            self.pos += take as usize;
            left -= take;
        }
        Ok(out)
    }
}

fn mask(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// Reinterprets the low `bits` of a two's complement value as a signed integer.
pub fn sign_extend(value: u64, bits: u32) -> i64 {
    if bits == 0 || bits >= 64 {
        return value as i64;
    }
    let shift = 64 - bits;
    ((value << shift) as i64) >> shift
}

pub fn fits_in(value: i64, bits: u32) -> bool {
    if bits >= 64 {
        return true;
    }
    let limit = 1i64 << (bits - 1);
    value >= -limit && value < limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_bits_survive_a_roundtrip() {
        let pattern = [true, false, true, true, false, false, false, true, true];
        let mut w = BitWriter::new();
        for &bit in &pattern {
            w.write_bit(bit);
        }
        let buf = w.finish();
        let mut r = BitReader::new(&buf);
        for &bit in &pattern {
            assert_eq!(r.read_bit().unwrap(), bit);
        }
    }

    #[test]
    fn values_survive_a_roundtrip_across_byte_boundaries() {
        let cases: [(u64, u32); 6] = [
            (0, 1),
            (1, 1),
            (0b101_1010, 7),
            (0xDEAD_BEEF, 32),
            (u64::MAX, 64),
            (0x1234_5678_9ABC_DEF0, 64),
        ];
        for skew in 0..8 {
            let mut w = BitWriter::new();
            w.write_bits(0, skew);
            for (value, count) in cases {
                w.write_bits(value, count);
            }
            let buf = w.finish();
            let mut r = BitReader::new(&buf);
            r.read_bits(skew).unwrap();
            for (value, count) in cases {
                assert_eq!(r.read_bits(count).unwrap(), value, "skew {skew}");
            }
        }
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let buf = [0xFFu8];
        let mut r = BitReader::new(&buf);
        assert_eq!(r.read_bits(8).unwrap(), 0xFF);
        assert_eq!(r.read_bit(), Err(Error::UnexpectedEnd));
    }

    #[test]
    fn bit_len_tracks_partial_bytes() {
        let mut w = BitWriter::new();
        assert_eq!(w.bit_len(), 0);
        w.write_bits(0, 3);
        assert_eq!(w.bit_len(), 3);
        w.write_bits(0, 8);
        assert_eq!(w.bit_len(), 11);
    }

    #[test]
    fn sign_extension_recovers_negative_values() {
        assert_eq!(sign_extend(0b111_1111, 7), -1);
        assert_eq!(sign_extend(0b100_0000, 7), -64);
        assert_eq!(sign_extend(0b011_1111, 7), 63);
        assert_eq!(sign_extend(u64::MAX, 64), -1);
    }

    #[test]
    fn range_check_matches_two_complement_limits() {
        assert!(fits_in(63, 7));
        assert!(!fits_in(64, 7));
        assert!(fits_in(-64, 7));
        assert!(!fits_in(-65, 7));
        assert!(fits_in(i64::MIN, 64));
    }
}
