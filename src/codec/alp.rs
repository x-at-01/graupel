//! ALP (Adaptive Lossless Floating-Point Compression, SIGMOD 2024), combining dynamic
//! decimal scaling with Frame-of-Reference (FOR) integer bitpacking and an exception patch dictionary.

use alloc::vec::Vec;

use crate::bits::{unzigzag, zigzag, BitReader, BitWriter};
use crate::codec::{Codec, Dod, TAG_ALP};
use crate::error::{Error, Result};
use crate::Point;

pub struct Alp;

impl Codec for Alp {
    fn name(&self) -> &'static str {
        "alp"
    }

    fn encode(&self, points: &[Point]) -> Result<Vec<u8>> {
        if points.len() > u32::MAX as usize {
            return Err(Error::TooManyPoints(points.len()));
        }

        let mut w = BitWriter::with_capacity(points.len() * 3 + 16);
        w.write_varint(points.len() as u64);

        if let Some(first) = points.first() {
            w.write_varint(zigzag(first.timestamp));
            let mut timestamps = Dod::new(first.timestamp);
            for point in &points[1..] {
                timestamps.write(&mut w, point.timestamp);
            }
        }

        let ts_bytes = w.finish();

        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        let val_bytes = fastalp::compress(&values);

        let mut block = Vec::with_capacity(1 + 5 + ts_bytes.len() + val_bytes.len());
        block.push(TAG_ALP);
        write_varint_bytes(ts_bytes.len() as u64, &mut block);
        block.extend_from_slice(&ts_bytes);
        block.extend_from_slice(&val_bytes);

        Ok(block)
    }
}

pub(crate) fn decode(body: &[u8]) -> Result<Vec<Point>> {
    let (ts_len, offset) = read_varint_bytes(body)?;
    let ts_len = ts_len as usize;

    if body.len() < offset + ts_len {
        return Err(Error::MalformedBlock);
    }

    let ts_bytes = &body[offset..offset + ts_len];
    let val_bytes = &body[offset + ts_len..];

    let mut r = BitReader::new(ts_bytes);
    let count = r.read_varint().map_err(|_| Error::MalformedBlock)? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }

    let first_ts = unzigzag(r.read_varint().map_err(|_| Error::MalformedBlock)?);
    let mut timestamps_stream = Dod::new(first_ts);
    let mut timestamps = Vec::with_capacity(count);
    timestamps.push(first_ts);

    for _ in 1..count {
        timestamps.push(
            timestamps_stream
                .read(&mut r)
                .map_err(|_| Error::MalformedBlock)?,
        );
    }

    let values: Vec<f64> = fastalp::decompress(val_bytes).map_err(|_| Error::MalformedBlock)?;
    if values.len() != count {
        return Err(Error::MalformedBlock);
    }

    let points = timestamps
        .into_iter()
        .zip(values)
        .map(|(timestamp, value)| Point::new(timestamp, value))
        .collect();

    Ok(points)
}

fn write_varint_bytes(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let group = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(group);
            return;
        }
        out.push(group | 0x80);
    }
}

fn read_varint_bytes(slice: &[u8]) -> Result<(u64, usize)> {
    let mut value = 0u64;
    for (i, &byte) in slice.iter().enumerate().take(10) {
        value |= ((byte & 0x7F) as u64) << (i * 7);
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    Err(Error::MalformedBlock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode as decode_block;

    fn roundtrip(points: &[Point]) -> Vec<u8> {
        let block = Alp.encode(points).unwrap();
        assert_eq!(decode_block(&block).unwrap(), points);
        block
    }

    #[test]
    fn empty_and_single_point_blocks() {
        roundtrip(&[]);
        roundtrip(&[Point::new(1_700_000_000, 21.5)]);
    }

    #[test]
    fn tenths_of_a_degree_roundtrip() {
        let points: Vec<Point> = (0..200)
            .map(|i| Point::new(1_700_000_000 + i * 3600, 8.0 + (i % 30) as f64 / 10.0))
            .collect();
        let block = roundtrip(&points);
        assert_eq!(block[0], TAG_ALP);
    }

    #[test]
    fn arbitrary_floats_and_special_values() {
        for odd in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -0.0,
            0.0,
            1e300,
            core::f64::consts::PI,
        ] {
            let points = vec![Point::new(0, 1.5), Point::new(3600, odd)];
            let block = roundtrip(&points);
            assert_eq!(block[0], TAG_ALP);
        }
    }
}
