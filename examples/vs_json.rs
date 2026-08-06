use graupel::{codec::Auto, dataset, Codec};
use std::io::Write;

fn gzip(data: &[u8]) -> usize {
    let mut child = std::process::Command::new("gzip")
        .arg("-9")
        .arg("-c")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(data).unwrap();
    child.wait_with_output().unwrap().stdout.len()
}

fn main() {
    let text = std::fs::read_to_string("data/080840-99999-2023.isd").unwrap();
    let series = dataset::isd_lite::parse(&text);
    let s = series
        .iter()
        .find(|s| s.variable == "air_temperature")
        .unwrap();
    let month = &s.points[..720.min(s.points.len())];

    // What a REST API typically sends today.
    let json: String = format!(
        "[{}]",
        month
            .iter()
            .map(|p| format!(r#"{{"t":{},"v":{}}}"#, p.timestamp, p.value))
            .collect::<Vec<_>>()
            .join(",")
    );

    let block = Auto.encode(month).unwrap();

    println!(
        "Serie: Logrono/Agoncillo, temperatura, {} puntos (30 dias horarios)\n",
        month.len()
    );
    println!(
        "{:<28}{:>10}{:>14}{:>12}",
        "formato", "bytes", "bytes/punto", "vs JSON+gz"
    );
    println!("{}", "-".repeat(64));
    let jz = gzip(json.as_bytes());
    let rows: [(&str, usize); 4] = [
        ("JSON", json.len()),
        ("JSON + gzip", jz),
        ("graupel", block.len()),
        ("graupel + gzip", gzip(&block)),
    ];
    for (name, bytes) in rows {
        println!(
            "{:<28}{:>10}{:>14.2}{:>11.1}x",
            name,
            bytes,
            bytes as f64 / month.len() as f64,
            jz as f64 / bytes as f64
        );
    }
    println!("\nmuestra JSON: {}", &json[..90.min(json.len())]);
}
