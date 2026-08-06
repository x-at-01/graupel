use crate::bits::{BitReader, BitWriter};
use crate::error::{Error, Result};
use crate::Point;

mod chimp;
mod chimp128;
mod decimal;
mod dod;
mod gorilla;

pub use chimp::Chimp;
pub use chimp128::Chimp128;
pub use decimal::Decimal;
pub use gorilla::Gorilla;

pub const TAG_GORILLA: u8 = 0;
pub const TAG_DECIMAL: u8 = 1;
pub const TAG_CHIMP: u8 = 2;
pub const TAG_CHIMP128: u8 = 3;

pub trait Codec {
    fn name(&self) -> &'static str;

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>>;
}

/// The leading tag byte names the codec, so a stored block stays readable without tracking
/// that separately.
pub fn decode(block: &[u8]) -> Result<Vec<Point>> {
    let (&tag, body) = block.split_first().ok_or(Error::UnexpectedEnd)?;
    match tag {
        TAG_GORILLA => gorilla::decode(body),
        TAG_DECIMAL => decimal::decode(body),
        TAG_CHIMP => chimp::decode(body),
        TAG_CHIMP128 => chimp128::decode(body),
        other => Err(Error::UnknownEncoding(other)),
    }
}

pub fn all() -> Vec<Box<dyn Codec>> {
    vec![
        Box::new(Gorilla),
        Box::new(Decimal),
        Box::new(Chimp),
        Box::new(Chimp128),
    ]
}

pub(crate) struct Dod {
    prev: i64,
    delta: i64,
}

impl Dod {
    pub(crate) fn new(first: i64) -> Self {
        Dod {
            prev: first,
            delta: 0,
        }
    }

    pub(crate) fn write(&mut self, w: &mut BitWriter, value: i64) {
        let delta = value.wrapping_sub(self.prev);
        dod::write(w, delta.wrapping_sub(self.delta));
        self.prev = value;
        self.delta = delta;
    }

    pub(crate) fn read(&mut self, r: &mut BitReader) -> Result<i64> {
        let delta = self.delta.wrapping_add(dod::read(r)?);
        self.prev = self.prev.wrapping_add(delta);
        self.delta = delta;
        Ok(self.prev)
    }
}

pub(crate) fn write_count(w: &mut BitWriter, count: usize) -> Result<()> {
    if count > u32::MAX as usize {
        return Err(Error::TooManyPoints(count));
    }
    w.write_bits(count as u64, 32);
    Ok(())
}

pub(crate) fn start_block(tag: u8, points: &[Point]) -> Result<(Vec<u8>, BitWriter)> {
    let mut writer = BitWriter::with_capacity(points.len() * 3 + 16);
    write_count(&mut writer, points.len())?;
    Ok((vec![tag], writer))
}

pub(crate) fn finish_block(mut head: Vec<u8>, writer: BitWriter) -> Vec<u8> {
    head.extend_from_slice(&writer.finish());
    head
}

pub(crate) fn read_count(r: &mut BitReader) -> Result<usize> {
    Ok(r.read_bits(32)? as usize)
}
