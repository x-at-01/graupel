use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use graupel::codec::all;
use graupel::{chunk_by_window, dataset, decode, Codec, Point, RAW_POINT_BYTES};

/// Keyed on (source, variable) so two archives reporting the same quantity stay apart.
type VariableTable = BTreeMap<(&'static str, &'static str), Vec<Tally>>;

const WINDOWS: [(&str, i64); 6] = [
    ("6 hours", 6 * 3_600),
    ("1 day", 86_400),
    ("1 week", 7 * 86_400),
    ("1 month", 30 * 86_400),
    ("1 year", 365 * 86_400),
    ("whole series", 0),
];

fn main() -> ExitCode {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));

    let series = match load(&dir) {
        Ok(series) if !series.is_empty() => series,
        _ => {
            eprintln!("no .isd, .csv or .rdb files in {}", dir.display());
            eprintln!("run ./scripts/fetch-data.sh first");
            return ExitCode::FAILURE;
        }
    };

    let total_points: usize = series.iter().map(|s| s.points.len()).sum();
    let sources: BTreeSet<&str> = series.iter().map(|s| s.source).collect();

    println!("graupel benchmark — public time series archives");
    println!(
        "{} sources, {} series, {} points\n",
        sources.len(),
        series.len(),
        thousands(total_points)
    );

    match measure_whole_series(&series) {
        Ok(by_variable) => {
            print_per_variable(&by_variable);
            print_overall(&by_variable);
        }
        Err(reason) => {
            eprintln!("{reason}");
            return ExitCode::FAILURE;
        }
    }

    match measure_windows(&series) {
        Ok(rows) => print_windows(&rows),
        Err(reason) => {
            eprintln!("{reason}");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

fn load(dir: &Path) -> std::io::Result<Vec<dataset::Series>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    files.sort();

    let mut series = Vec::new();
    for file in files {
        let parse = match file.extension().and_then(|e| e.to_str()) {
            Some("isd") => dataset::isd_lite::parse,
            Some("csv") => dataset::coops::parse,
            Some("rdb") => dataset::usgs::parse,
            _ => continue,
        };
        match fs::read_to_string(&file) {
            Ok(text) => series.extend(parse(&text)),
            Err(err) => eprintln!("skipping {}: {err}", file.display()),
        }
    }
    Ok(series)
}

fn measure_whole_series(series: &[dataset::Series]) -> Result<VariableTable, String> {
    let codecs = all();
    let mut by_variable: VariableTable = BTreeMap::new();

    for one in series {
        let tallies = by_variable
            .entry((one.source, one.variable))
            .or_insert_with(|| codecs.iter().map(|c| Tally::new(c.name())).collect());
        for (codec, tally) in codecs.iter().zip(tallies.iter_mut()) {
            let measurement = measure(&**codec, &one.points)
                .map_err(|e| format!("{} failed on {}: {e}", codec.name(), one.variable))?;
            tally.add(one.points.len(), measurement);
        }
    }
    Ok(by_variable)
}

/// Chunking is where a real database differs from this benchmark's default of one block per
/// series, so it gets measured rather than assumed.
fn measure_windows(series: &[dataset::Series]) -> Result<Vec<(&'static str, Vec<Tally>)>, String> {
    let codecs = all();
    let mut rows = Vec::new();

    for (label, window) in WINDOWS {
        let mut tallies: Vec<Tally> = codecs.iter().map(|c| Tally::new(c.name())).collect();
        for one in series {
            for block in chunk_by_window(&one.points, window) {
                for (codec, tally) in codecs.iter().zip(tallies.iter_mut()) {
                    let measurement = measure(&**codec, block)
                        .map_err(|e| format!("{} failed at window {label}: {e}", codec.name()))?;
                    tally.add(block.len(), measurement);
                }
            }
        }
        rows.push((label, tallies));
    }
    Ok(rows)
}

struct Measurement {
    bytes: usize,
    encode: Duration,
    decode: Duration,
}

fn measure(codec: &dyn Codec, points: &[Point]) -> Result<Measurement, String> {
    let started = Instant::now();
    let block = codec.encode(points).map_err(|e| e.to_string())?;
    let encode = started.elapsed();

    let started = Instant::now();
    let restored = decode(&block).map_err(|e| e.to_string())?;
    let decode_time = started.elapsed();

    if restored != points {
        return Err("round trip changed the data".to_string());
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
    blocks: usize,
    encode: Duration,
    decode: Duration,
}

impl Tally {
    fn new(codec: &'static str) -> Self {
        Tally {
            codec,
            points: 0,
            bytes: 0,
            blocks: 0,
            encode: Duration::ZERO,
            decode: Duration::ZERO,
        }
    }

    fn add(&mut self, points: usize, measurement: Measurement) {
        self.points += points;
        self.bytes += measurement.bytes;
        self.blocks += 1;
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

fn print_per_variable(by_variable: &VariableTable) {
    let Some(first) = by_variable.values().next() else {
        return;
    };
    print!(
        "{:<11}{:<20}{:>11}{:>6}",
        "source", "variable", "points", "raw"
    );
    for tally in first {
        print!("{:>10}", tally.codec);
    }
    println!("\n{}", "-".repeat(48 + 10 * first.len()));

    for ((source, variable), tallies) in by_variable {
        print!(
            "{:<11}{:<20}{:>11}{:>6}",
            source,
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

fn print_overall(by_variable: &VariableTable) {
    let Some(first) = by_variable.values().next() else {
        return;
    };
    let mut totals: Vec<Tally> = first.iter().map(|t| Tally::new(t.codec)).collect();
    for tallies in by_variable.values() {
        for (total, tally) in totals.iter_mut().zip(tallies) {
            total.points += tally.points;
            total.bytes += tally.bytes;
            total.blocks += tally.blocks;
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
    println!();
}

fn print_windows(rows: &[(&'static str, Vec<Tally>)]) {
    let Some((_, first)) = rows.first() else {
        return;
    };
    print!("{:<16}{:>10}", "block window", "blocks");
    for tally in first {
        print!("{:>10}", tally.codec);
    }
    println!("\n{}", "-".repeat(26 + 10 * first.len()));

    for (label, tallies) in rows {
        print!("{:<16}{:>10}", label, thousands(tallies[0].blocks));
        for tally in tallies {
            print!("{:>10.2}", tally.bytes_per_point());
        }
        println!();
    }
    println!("\nbytes per point by block size, lower is better");
}

fn throughput(points: usize, elapsed: Duration) -> String {
    if elapsed.is_zero() {
        return "-".to_string();
    }
    format!("{:.0} Mpt/s", points as f64 / elapsed.as_secs_f64() / 1e6)
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
