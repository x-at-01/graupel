//! Delta-of-delta, shared by every codec for timestamps and by the decimal codec for values.
//!
//! The widest bucket is 64 bits so an arbitrary gap never needs a special case.

use crate::bits::{fits_in, sign_extend, BitReader, BitWriter};
use crate::error::Result;

const BUCKETS: [u32; 3] = [7, 9, 12];

pub fn write(w: &mut BitWriter, dod: i64) {
    if dod == 0 {
        w.write_bit(false);
        return;
    }
    for (index, &width) in BUCKETS.iter().enumerate() {
        if fits_in(dod, width) {
            w.write_bits(u64::MAX, index as u32 + 1);
            w.write_bit(false);
            w.write_bits(dod as u64, width);
            return;
        }
    }
    w.write_bits(u64::MAX, 4);
    w.write_bits(dod as u64, 64);
}

pub fn read(r: &mut BitReader) -> Result<i64> {
    let mut leading_ones = 0u32;
    while leading_ones < 4 && r.read_bit()? {
        leading_ones += 1;
    }
    match leading_ones {
        0 => Ok(0),
        4 => Ok(r.read_bits(64)? as i64),
        n => {
            let width = BUCKETS[n as usize - 1];
            Ok(sign_extend(r.read_bits(width)?, width))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bucket_boundary_survives_a_roundtrip() {
        let values = [
            0,
            1,
            -1,
            63,
            -64,
            64,
            -65,
            255,
            -256,
            256,
            -257,
            2047,
            -2048,
            2048,
            -2049,
            i64::MAX,
            i64::MIN,
            86_400,
            -86_400,
        ];
        let mut w = BitWriter::new();
        for v in values {
            write(&mut w, v);
        }
        let buf = w.finish();
        let mut r = BitReader::new(&buf);
        for v in values {
            assert_eq!(read(&mut r).unwrap(), v);
        }
    }

    #[test]
    fn a_steady_interval_costs_one_bit_per_point() {
        let mut w = BitWriter::new();
        for _ in 0..1000 {
            write(&mut w, 0);
        }
        assert_eq!(w.bit_len(), 1000);
    }
}
