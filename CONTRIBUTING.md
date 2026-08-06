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

- **Per-block codec selection.** The encoder could try all four and keep the smallest, since
  the tag byte already makes blocks self-describing. Worth knowing what the ceiling is.
- **More datasets.** ISD-Lite is one shape of data. Solar irradiance, tide gauges, air
  quality and grid frequency all behave differently, and a codec that only wins on one of them
  is worth knowing about.
- **Elf or Elf+.** More recent floating-point codecs that erase trailing mantissa bits before
  XORing. They are lossless in the sense that matters here only under specific conditions —
  establishing which is part of the work.
- **Block sizing.** Everything is currently one block per series. Real databases chunk by
  time window, which changes the amortisation of the per-block header and the reachability of
  a single decimal scale.
- **`no_std` support.** The codecs need no allocation beyond the output buffer.

## Scope

Storage engines, indexes and query layers are out of scope. This repository is about how many
bits a point costs and how to find out.
