# Contributing

This is a benchmark as much as a library, so the bar for a change is a measurement, not an
argument.

## Getting set up

```sh
./scripts/fetch-data.sh
cargo test
cargo run --release --bin graupel-bench
```

There are no dependencies to install beyond a Rust toolchain, `curl` and `gunzip`.

## Claiming a change compresses better

Post the benchmark output from before and after your change, on the same machine and the same
data. A ratio without the table it came from is not reviewable, and neither is a table from a
dataset only you have.

If a change helps one variable and hurts another — which is common, wind direction and
temperature reward opposite tricks — say so rather than quoting the average.

## Correctness comes first

Every codec must be lossless for every input, including NaN payloads, negative zero,
subnormals, infinities, non-monotonic timestamps and blocks of a single point. A codec that
compresses better by losing a bit pattern is not a smaller codec, it is a different one.

New codecs need to pass the shared suite in `tests/roundtrip.rs`, which covers randomised
series, adjacent float bit patterns, truncation at every byte offset and random bit flips.
Adding a codec to `codec::all()` puts it under all of that automatically.

Decoders must return `Err` on malformed input. Panicking on a corrupted block counts as a bug.

## Style

Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` before opening a pull request.
CI runs both.

Comments explain why, not what. If a line needs a comment to say what it does, the line is the
problem. The comments that earn their place here are the ones recording a constraint you
cannot see from the code — why a field is six bits wide, why a branch invalidates state, why a
value has to be rejected.

## Good first issues

- **More datasets.** Three archives is not many, and every one added so far changed a
  conclusion. Grid frequency, solar irradiance and air quality all behave differently again.
  Keep the no-account rule: if reproducing the benchmark needs an API key, it is not
  reproducible.
- **A cheaper `auto`.** Trying all four costs 4.5x in encode time. A classifier that looks at
  the first few points — decimal digits, integrality, magnitude — should pick the right codec
  most of the time for almost nothing.
- **Per-variable block sizing.** The block-size table is aggregated across sources with very
  different cadences. Six-minute tide data and hourly weather almost certainly want different
  windows.

## Scope

Storage engines, indexes and query layers are out of scope. This repository is about how many
bits a point costs and how to find out.
