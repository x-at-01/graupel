//! Every codec has to be lossless for every input, so the interesting test is not one shape
//! of data but many: whatever the generator produces, all three must agree with each other
//! and with what went in.

use graupel::codec::all;
use graupel::{decode, Point};

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn assert_all_codecs_roundtrip(points: &[Point], label: &str) {
    for codec in all() {
        let block = codec
            .encode(points)
            .unwrap_or_else(|e| panic!("{label}: {} failed to encode: {e}", codec.name()));
        let restored = decode(&block)
            .unwrap_or_else(|e| panic!("{label}: {} failed to decode: {e}", codec.name()));
        assert_eq!(restored, points, "{label}: {} lost data", codec.name());
    }
}

#[test]
fn tenth_precision_series_with_gaps() {
    let mut rng = Xorshift(0x2026_0806);
    for series in 0..200 {
        let length = rng.below(400) as usize + 1;
        let mut timestamp = 1_600_000_000i64;
        let mut tenths = rng.below(600) as i64 - 300;
        let points: Vec<Point> = (0..length)
            .map(|_| {
                timestamp += if rng.below(20) == 0 {
                    3_600 * (rng.below(48) as i64 + 1)
                } else {
                    3_600
                };
                tenths += rng.below(9) as i64 - 4;
                Point::new(timestamp, tenths as f64 / 10.0)
            })
            .collect();
        assert_all_codecs_roundtrip(&points, &format!("tenths series {series}"));
    }
}

#[test]
fn arbitrary_bit_patterns() {
    let mut rng = Xorshift(0xDEAD_BEEF);
    for series in 0..200 {
        let length = rng.below(200) as usize + 1;
        let points: Vec<Point> = (0..length)
            .map(|_| Point::new(rng.next() as i64, f64::from_bits(rng.next())))
            .collect();
        assert_all_codecs_roundtrip(&points, &format!("random bits series {series}"));
    }
}

#[test]
fn values_that_only_differ_in_the_last_mantissa_bit() {
    let mut value = 1.0f64;
    let points: Vec<Point> = (0..500)
        .map(|i| {
            value = f64::from_bits(value.to_bits() + 1);
            Point::new(1_600_000_000 + i * 60, value)
        })
        .collect();
    assert_all_codecs_roundtrip(&points, "adjacent floats");
}

#[test]
fn a_constant_series_of_every_length_up_to_a_hundred() {
    for length in 0..100 {
        let points: Vec<Point> = (0..length)
            .map(|i| Point::new(1_600_000_000 + i as i64 * 3_600, 15.5))
            .collect();
        assert_all_codecs_roundtrip(&points, &format!("constant length {length}"));
    }
}

#[test]
fn timestamps_going_backwards_are_still_lossless() {
    let points: Vec<Point> = (0..200)
        .map(|i| Point::new(1_600_000_000 - i * 900, 3.3))
        .collect();
    assert_all_codecs_roundtrip(&points, "descending timestamps");
}

#[test]
fn truncating_a_block_anywhere_never_panics() {
    let mut rng = Xorshift(0x0BAD_C0DE);
    let points: Vec<Point> = (0..300)
        .map(|i| Point::new(1_600_000_000 + i * 3_600, rng.below(1000) as f64 / 10.0))
        .collect();
    for codec in all() {
        let block = codec.encode(&points).unwrap();
        for cut in 0..block.len() {
            let _ = decode(&block[..cut]);
        }
    }
}

#[test]
fn corrupted_blocks_never_panic() {
    let mut rng = Xorshift(0xFEED_FACE);
    let points: Vec<Point> = (0..300)
        .map(|i| Point::new(1_600_000_000 + i * 3_600, rng.below(1000) as f64 / 10.0))
        .collect();
    for codec in all() {
        let block = codec.encode(&points).unwrap();
        for _ in 0..2_000 {
            let mut corrupted = block.clone();
            let index = rng.below(corrupted.len() as u64) as usize;
            corrupted[index] ^= 1 << rng.below(8);
            let _ = decode(&corrupted);
        }
    }
}
