use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use graupel::codec::all;
use graupel::{dataset, decode, Point, RAW_POINT_BYTES};

fn main() -> ExitCode {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));

    let files = match collect_files(&dir) {
        Ok(files) if !files.is_empty() => files,
        Ok(_) => {
            eprintln!("no .txt files in {}", dir.display());
            eprintln!("run ./scripts/fetch-data.sh first");
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("cannot read {}: {err}", dir.display());
            eprintln!("run ./scripts/fetch-data.sh first");
            return ExitCode::FAILURE;
        }
    };

    let codecs = all();
    let mut by_variable: BTreeMap<&'static str, Vec<Tally>> = BTreeMap::new();
    let mut stations = 0;
    let mut series_count = 0;

    for file in &files {
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("skipping {}: {err}", file.display());
                continue;
            }
        };
        stations += 1;
        for series in dataset::parse(&text) {
            series_count += 1;
            let tallies = by_variable
                .entry(series.variable)
                .or_insert_with(|| codecs.iter().map(|c| Tally::new(c.name())).collect());
            for (codec, tally) in codecs.iter().zip(tallies.iter_mut()) {
                match measure(&**codec, &series.points) {
                    Ok(measurement) => tally.add(&series.points, measurement),
                    Err(reason) => {
                        eprintln!(
                            "{}/{} failed on {}: {reason}",
                            file.display(),
                            series.variable,
                            codec.name()
                        );
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
    }

    let total_points: usize = by_variable
        .values()
        .flat_map(|t| t.first())
        .map(|t| t.points)
        .sum();

    println!("graupel benchmark — NOAA ISD-Lite hourly observations");
    println!(
        "{stations} stations, {series_count} series, {} points\n",
        thousands(total_points)
    );

    print_per_variable(&by_variable);
    print_overall(&by_variable);

    ExitCode::SUCCESS
}

struct Measurement {
    bytes: usize,
    encode: Duration,
    decode: Duration,
}

fn measure(codec: &dyn graupel::Codec, points: &[Point]) -> Result<Measurement, String> {
    let started = Instant::now();
    let block = codec.encode(points).map_err(|e| e.to_string())?;
    let encode = started.elapsed();

    let started = Instant::now();
    let restored = decode(&block).map_err(|e| e.to_string())?;
    let decode_time = started.elapsed();

    if restored != points {
        return Err(format!(
            "round trip changed the data ({} in, {} out)",
            points.len(),
            restored.len()
        ));
    }
    Ok(Measurement {
        bytes: block.len(),
        encode,
        decode: decode_time,
    })
}

struct Tally {
    codec: &'static str,
    points: usize,
    bytes: usize,
    encode: Duration,
    decode: Duration,
}

impl Tally {
    fn new(codec: &'static str) -> Self {
        Tally {
            codec,
            points: 0,
            bytes: 0,
            encode: Duration::ZERO,
            decode: Duration::ZERO,
        }
    }

    fn add(&mut self, points: &[Point], measurement: Measurement) {
        self.points += points.len();
        self.bytes += measurement.bytes;
        self.encode += measurement.encode;
        self.decode += measurement.decode;
    }

    fn bytes_per_point(&self) -> f64 {
        if self.points == 0 {
            0.0
        } else {
            self.bytes as f64 / self.points as f64
        }
    }
}

fn print_per_variable(by_variable: &BTreeMap<&'static str, Vec<Tally>>) {
    let Some(first) = by_variable.values().next() else {
        return;
    };
    print!("{:<22}{:>12}{:>8}", "variable", "points", "raw");
    for tally in first {
        print!("{:>10}", tally.codec);
    }
    println!("\n{}", "-".repeat(42 + 10 * first.len()));

    for (variable, tallies) in by_variable {
        print!(
            "{:<22}{:>12}{:>8}",
            variable,
            thousands(tallies[0].points),
            RAW_POINT_BYTES
        );
        for tally in tallies {
            print!("{:>10.2}", tally.bytes_per_point());
        }
        println!();
    }
    println!("\nbytes per point, lower is better\n");
}

fn print_overall(by_variable: &BTreeMap<&'static str, Vec<Tally>>) {
    let Some(first) = by_variable.values().next() else {
        return;
    };
    let mut totals: Vec<Tally> = first.iter().map(|t| Tally::new(t.codec)).collect();
    for tallies in by_variable.values() {
        for (total, tally) in totals.iter_mut().zip(tallies) {
            total.points += tally.points;
            total.bytes += tally.bytes;
            total.encode += tally.encode;
            total.decode += tally.decode;
        }
    }

    println!(
        "{:<10}{:>14}{:>10}{:>14}{:>14}",
        "codec", "bytes/point", "vs raw", "encode", "decode"
    );
    println!("{}", "-".repeat(62));
    for total in &totals {
        let per_point = total.bytes_per_point();
        println!(
            "{:<10}{:>14.3}{:>9.1}x{:>12} {:>12} ",
            total.codec,
            per_point,
            RAW_POINT_BYTES as f64 / per_point,
            throughput(total.points, total.encode),
            throughput(total.points, total.decode),
        );
    }
}

fn throughput(points: usize, elapsed: Duration) -> String {
    if elapsed.is_zero() {
        return "-".to_string();
    }
    format!("{:.0} Mpt/s", points as f64 / elapsed.as_secs_f64() / 1e6)
}

fn collect_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    Ok(files)
}

fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
