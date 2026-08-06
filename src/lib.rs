//! Lossless compression codecs for time series, built to be compared against each other
//! on real weather station observations rather than on synthetic benchmarks.
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

/// Compares values by their bit pattern so that round-trip tests can assert on NaN payloads
/// and on the difference between `0.0` and `-0.0`, which `==` deliberately hides.
impl PartialEq for Point {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp && self.value.to_bits() == other.value.to_bits()
    }
}

/// Bytes a point occupies uncompressed: an `i64` timestamp plus an `f64` value.
pub const RAW_POINT_BYTES: usize = 16;
