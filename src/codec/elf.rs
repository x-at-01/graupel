//! Elf-style erasing, from "Elf: Erasing-Based Lossless Floating-Point Compression"
//! (VLDB 2023), layered on the Chimp engine.
//!
//! A value with a short decimal form has mantissa bits that carry no decimal information. Elf
//! zeroes them before the XOR, which widens the trailing-zero runs the XOR encoder is looking
//! for, and recovers the original by rounding back to the block's decimal places. It stays
//! bit-exact: erasing is only accepted when the round trip reproduces the original bit pattern.
//!
//! This is Elf's idea with a block-level number of decimals rather than the paper's per-value
//! α and β. It sits on Chimp rather than Chimp128 for a measured reason: Chimp128 keys its
//! reference table on the low mantissa bits, which are exactly the bits erasing zeroes, so the
//! two cancel each other out. See `docs/format.md`.

use alloc::vec::Vec;

use crate::bits::BitWriter;
use crate::codec::chimp::{Chimp, ChimpReader, ChimpWriter};
use crate::codec::decimal::{block_scale, round_nearest, POW10};
use crate::codec::{decode_block, encode_block, Codec, ValueCoding, TAG_ELF};
use crate::error::{Error, Result};
use crate::Point;

const MANTISSA_BITS: u32 = 52;

pub struct Elf;

impl Codec for Elf {
    fn name(&self) -> &'static str {
        "elf"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        match block_scale(points) {
            Some(decimals) => encode_block(TAG_ELF, &Erased::new(decimals), points),
            None => Chimp.encode(points),
        }
    }
}

struct Erased {
    decimals: u8,
}

impl Erased {
    fn new(decimals: u8) -> Self {
        Erased { decimals }
    }
}

impl ValueCoding for Erased {
    type Encoder = ChimpWriter;
    type Decoder = ChimpReader;

    fn pack(&self, value: f64) -> u64 {
        erase(value, self.decimals)
    }

    fn unpack(&self, bits: u64) -> f64 {
        restore(bits, self.decimals)
    }

    fn encoder(&self, first: u64) -> ChimpWriter {
        ChimpWriter::new(first)
    }

    fn decoder(&self, first: u64) -> ChimpReader {
        ChimpReader::new(first)
    }

    fn write_header(&self, w: &mut BitWriter) {
        w.write_bits(self.decimals as u64, 8);
    }
}

pub(crate) fn decode(body: &[u8]) -> Result<Vec<Point>> {
    decode_block(
        |r| {
            let decimals = r.read_bits(8)? as usize;
            if decimals >= POW10.len() {
                return Err(Error::MalformedBlock);
            }
            Ok(Erased::new(decimals as u8))
        },
        body,
    )
}

/// Zeroes as many low mantissa bits as survive the round trip. Searching downwards returns the
/// widest run directly, and falling through to the original bits is always safe.
fn erase(value: f64, decimals: u8) -> u64 {
    let bits = value.to_bits();
    for width in (1..=MANTISSA_BITS).rev() {
        let candidate = bits & !((1u64 << width) - 1);
        if restore(candidate, decimals).to_bits() == bits {
            return candidate;
        }
    }
    bits
}

fn restore(erased: u64, decimals: u8) -> f64 {
    let value = f64::from_bits(erased);
    let factor = POW10[decimals as usize];
    let scaled = value * factor;
    if !scaled.is_finite() || scaled.abs() > 9_007_199_254_740_992.0 {
        return value;
    }
    round_nearest(scaled) as f64 / factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode as decode_block, Chimp128, TAG_CHIMP};

    fn roundtrip(points: &[Point]) -> Vec<u8> {
        let block = Elf.encode(points).unwrap();
        assert_eq!(decode_block(&block).unwrap(), points);
        block
    }

    #[test]
    fn empty_and_single_point_blocks() {
        roundtrip(&[]);
        roundtrip(&[Point::new(1_700_000_000, 21.5)]);
    }

    #[test]
    fn erasing_widens_the_trailing_zero_run() {
        let bits = 8.2f64.to_bits();
        let erased = erase(8.2, 1);
        assert!(
            erased.trailing_zeros() > bits.trailing_zeros(),
            "erased {} zeros vs original {}",
            erased.trailing_zeros(),
            bits.trailing_zeros()
        );
        assert_eq!(restore(erased, 1), 8.2);
    }

    #[test]
    fn erasing_is_bit_exact_across_a_range_of_tenths() {
        for tenths in -3000..3000i64 {
            let value = tenths as f64 / 10.0;
            assert_eq!(
                restore(erase(value, 1), 1).to_bits(),
                value.to_bits(),
                "value {value} did not survive erasing"
            );
        }
    }

    #[test]
    fn values_with_no_decimal_form_fall_back() {
        // 1e300 cannot be scaled into an exact integer at any power of ten, unlike π, which
        // reaches one at 10^15.
        let points = vec![Point::new(0, 1.5), Point::new(3600, 1e300)];
        let block = roundtrip(&points);
        assert_eq!(block[0], TAG_CHIMP);
    }

    #[test]
    fn special_values_fall_back_rather_than_rounding() {
        for odd in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0] {
            let points = vec![Point::new(0, 1.5), Point::new(3600, odd)];
            let block = roundtrip(&points);
            assert_eq!(block[0], TAG_CHIMP, "{odd:?} should not be erased");
        }
    }

    #[test]
    fn beats_chimp128_on_tenths() {
        let points: Vec<Point> = (0..4000)
            .map(|i| {
                let daily = (i as f64 / 24.0 * core::f64::consts::TAU).sin() * 6.0;
                Point::new(
                    1_700_000_000 + i * 3600,
                    ((12.0 + daily) * 10.0) as i64 as f64 / 10.0,
                )
            })
            .collect();
        let elf = Elf.encode(&points).unwrap().len();
        let chimp = Chimp128.encode(&points).unwrap().len();
        assert!(elf < chimp, "elf {elf} should beat chimp128 {chimp}");
    }
}
