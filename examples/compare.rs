//! Compares graupel against implementations and compressors it does not control: the `tsz`
//! crate (the most-downloaded Rust Gorilla), and gzip, zstd and xz run over the raw bytes.
//!
//! Comparing a codec only against your own implementation of its rivals proves nothing, since
//! a weak rival might just be a weak implementation.
//!
//! Run with: cargo run --release --example compare

use std::io::Write;
use std::process::{Command, Stdio};

use graupel::codec::{all, Auto};
use graupel::{dataset, decode, Codec, Point, RAW_POINT_BYTES};

use tsz::stream::{BufferedReader, BufferedWriter};
use tsz::{Decode, Encode, StdDecoder, StdEncoder};

fn main() {
    let series = load();
    if series.is_empty() {
        eprintln!("no data; run ./scripts/fetch-data.sh first");
        return;
    }
    let points: usize = series.iter().map(|s| s.points.len()).sum();
    println!("{} series, {} points\n", series.len(), points);

    let mut rows: Vec<(String, usize)> = Vec::new();

    for codec in all() {
        let bytes: usize = series
            .iter()
            .map(|s| codec.encode(&s.points).unwrap().len())
            .sum();
        rows.push((format!("graupel::{}", codec.name()), bytes));
    }

    rows.push((
        "tsz (Gorilla crate)".into(),
        series.iter().map(tsz_size).sum(),
    ));

    for (name, argv) in [
        ("gzip -9", vec!["gzip", "-9", "-c"]),
        ("zstd -19", vec!["zstd", "-19", "-c", "-q"]),
        ("xz -9", vec!["xz", "-9", "-c"]),
    ] {
        let bytes: usize = series
            .iter()
            .map(|s| external(&argv, &raw_bytes(&s.points)))
            .sum();
        rows.push((name.into(), bytes));
    }

    // Generic compressors on the text a REST API would actually send.
    for (name, argv) in [
        ("JSON + gzip -9", vec!["gzip", "-9", "-c"]),
        ("JSON + zstd -19", vec!["zstd", "-19", "-c", "-q"]),
    ] {
        let bytes: usize = series
            .iter()
            .map(|s| external(&argv, json(&s.points).as_bytes()))
            .sum();
        rows.push((name.into(), bytes));
    }

    rows.sort_by_key(|&(_, bytes)| bytes);
    let best = rows[0].1 as f64;

    println!(
        "{:<24}{:>12}{:>14}{:>12}",
        "format", "bytes", "bytes/point", "vs best"
    );
    println!("{}", "-".repeat(62));
    println!(
        "{:<24}{:>12}{:>14.2}{:>11.1}x",
        "uncompressed",
        points * RAW_POINT_BYTES,
        RAW_POINT_BYTES as f64,
        (points * RAW_POINT_BYTES) as f64 / best
    );
    for (name, bytes) in &rows {
        println!(
            "{:<24}{:>12}{:>14.3}{:>11.2}x",
            name,
            bytes,
            *bytes as f64 / points as f64,
            *bytes as f64 / best
        );
    }

    verify_tsz_roundtrip(&series[0].points);
    println!("\ntsz round trip verified, so its size is a fair comparison");
}

fn load() -> Vec<dataset::Series> {
    let mut series = Vec::new();
    let Ok(entries) = std::fs::read_dir("data") else {
        return series;
    };
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let parse = match path.extension().and_then(|e| e.to_str()) {
            Some("isd") => dataset::isd_lite::parse,
            Some("csv") => dataset::coops::parse,
            Some("rdb") => dataset::usgs::parse,
            _ => continue,
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            series.extend(parse(&text));
        }
    }
    series
}

fn raw_bytes(points: &[Point]) -> Vec<u8> {
    let mut out = Vec::with_capacity(points.len() * RAW_POINT_BYTES);
    for point in points {
        out.extend_from_slice(&point.timestamp.to_le_bytes());
        out.extend_from_slice(&point.value.to_bits().to_le_bytes());
    }
    out
}

fn json(points: &[Point]) -> String {
    let body: Vec<String> = points
        .iter()
        .map(|p| format!(r#"{{"t":{},"v":{}}}"#, p.timestamp, p.value))
        .collect();
    format!("[{}]", body.join(","))
}

fn external(argv: &[&str], data: &[u8]) -> usize {
    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", argv[0]));
    child.stdin.take().unwrap().write_all(data).unwrap();
    child.wait_with_output().unwrap().stdout.len()
}

fn tsz_size(series: &dataset::Series) -> usize {
    let Some(first) = series.points.first() else {
        return 0;
    };
    let mut encoder = StdEncoder::new(first.timestamp as u64, BufferedWriter::new());
    for point in &series.points {
        encoder.encode(tsz::DataPoint::new(point.timestamp as u64, point.value));
    }
    encoder.close().len()
}

/// tsz is only a fair comparison if it is also lossless on this data.
fn verify_tsz_roundtrip(points: &[Point]) {
    let mut encoder = StdEncoder::new(points[0].timestamp as u64, BufferedWriter::new());
    for point in points {
        encoder.encode(tsz::DataPoint::new(point.timestamp as u64, point.value));
    }
    let bytes = encoder.close();

    let mut decoder = StdDecoder::new(BufferedReader::new(bytes));
    for point in points {
        let decoded = decoder.next().expect("tsz stream ended early");
        assert_eq!(decoded.get_time(), point.timestamp as u64);
        assert_eq!(decoded.get_value().to_bits(), point.value.to_bits());
    }

    // And that graupel is too, on the same series.
    let block = Auto.encode(points).unwrap();
    assert_eq!(decode(&block).unwrap(), points);
}
