//! Lossless time series compression codecs, measured against each other on real weather
//! station observations. See `docs/format.md` for the block formats.
//!
//! The codecs allocate nothing beyond the output buffer, so the crate builds without `std`:
//! `default-features = false` leaves everything except the error trait and the benchmark.
//!
//! ```
//! use graupel::{codec::Gorilla, Codec, Point};
//!
//! let points = vec![
//!     Point::new(1_672_531_200, 8.2),
//!     Point::new(1_672_534_800, 8.1),
//!     Point::new(1_672_538_400, 7.9),
//! ];
//! let block = Gorilla.encode(&points).unwrap();
//! assert_eq!(graupel::decode(&block).unwrap(), points);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod bits;
pub mod codec;
pub mod dataset;
mod error;

pub use codec::{decode, Codec};
pub use error::{Error, Result};

/// A single observation: seconds since the Unix epoch, and the measured value.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub timestamp: i64,
    pub value: f64,
}

impl Point {
    pub fn new(timestamp: i64, value: f64) -> Self {
        Point { timestamp, value }
    }
}

/// Bit-pattern equality, because `==` hides NaN payloads and the sign of zero.
impl PartialEq for Point {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp && self.value.to_bits() == other.value.to_bits()
    }
}

/// Bytes a point occupies uncompressed: an `i64` timestamp plus an `f64` value.
pub const RAW_POINT_BYTES: usize = 16;

/// Splits a series into the blocks a real database would store, cutting on fixed epoch-aligned
/// windows rather than on a point count, which is what Prometheus and friends do.
///
/// A window of zero or less yields the whole series as one block.
pub fn chunk_by_window(points: &[Point], window_seconds: i64) -> alloc::vec::Vec<&[Point]> {
    use alloc::vec::Vec;

    if points.is_empty() {
        return Vec::new();
    }
    if window_seconds <= 0 {
        return alloc::vec![points];
    }

    let mut blocks = Vec::new();
    let mut start = 0;
    let mut current = points[0].timestamp.div_euclid(window_seconds);
    for (index, point) in points.iter().enumerate() {
        let window = point.timestamp.div_euclid(window_seconds);
        if window != current {
            blocks.push(&points[start..index]);
            start = index;
            current = window;
        }
    }
    blocks.push(&points[start..]);
    blocks
}
