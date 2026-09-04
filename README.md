# graupel

[![crates.io](https://img.shields.io/crates/v/graupel.svg)](https://crates.io/crates/graupel)
[![docs.rs](https://img.shields.io/docsrs/graupel)](https://docs.rs/graupel)
[![CI](https://github.com/jocarrd/graupel/actions/workflows/ci.yml/badge.svg)](https://github.com/jocarrd/graupel/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/graupel.svg)](#license)

Lossless time series compression codecs in Rust, measured against each other on public
observation archives instead of on synthetic data.

```sh
cargo add graupel
```

On 467,550 real readings from three archives — weather stations, tide gauges and river gauges:

| codec | bytes/point | vs uncompressed |
|---|---|---|
| Gorilla (VLDB 2015) | 5.92 | 2.7x |
| Chimp (VLDB 2022) | 4.66 | 3.4x |
| Chimp128 (VLDB 2022) | 2.70 | 5.9x |
| Elf (VLDB 2023) | 2.05 | 7.8x |
| Decimal scaling | 1.28 | 12.5x |
| fastalp (SIGMOD 2024) | 1.06 | 15.1x |
| **Best-of-six per block** | **1.05** | **15.2x** |

The headline is not any single row. It is that **no codec wins everywhere**, and the gap
between the best and worst choice for a given series is often larger than the gap between
codecs on average.

## Why the choice matters more than the codec

Gorilla XORs each value against the previous one and stores only the window of bits that
changed. Its paper reports 1.37 bytes per point. On this data it manages 5.92.

The reason is that a station reporting 12.3 °C is not producing a binary quantity. IEEE-754
stores 12.3 as a long awkward fraction and 12.4 as a different long awkward fraction, so two
readings a tenth of a degree apart differ across most of the mantissa and the window stays
wide. Multiplying a block by the smallest power of ten that turns every value back into an
integer undoes that, which is the trick
[VictoriaMetrics documented](https://faun.pub/victoriametrics-achieving-better-compression-for-time-series-data-than-gorilla-317bc1f95932).

Chimp attacks the same problem without assuming anything about decimals: it keeps the XOR but
rounds the leading-zero count into eight buckets and pays for an explicit trailing-zero count
only when that earns its keep. Chimp128 goes further and XORs against the best of the last 128
values rather than always the previous one. Elf splits the difference, zeroing the mantissa bits
that carry no decimal information before handing the result to Chimp. fastalp (ALP, SIGMOD 2024)
dynamically discovers the best decimal exponent and factor per vector, combining Frame-of-Reference
bitpacking with a bit-exact patch dictionary, exact decimal division reconstruction (eliminating
multiplication truncation false exceptions), adaptive delta encoding, and outlier smoothing.

Since blocks carry a tag byte naming their codec, an encoder can try all six and keep the
smallest. That is the `auto` row.

### Two details that mattered more than the algorithms

**How you decide a value's decimal scale.** The obvious test — does `value * 10^s` land on an
integer — is wrong. `8.55 * 100` is `855.0000000000001`, so that test rejects a scale of 2 and
keeps climbing to 15, where the arithmetic has crossed 2^53 and stopped being exact at all. The
right test is whether dividing back reproduces the original bit pattern, and it does at 2.
Fixing this moved tide gauges from 6.30 to 1.20 bytes per point and river stage from 5.44 to
1.02.

**How wide the delta-of-delta buckets are.** Gorilla's paper stops at 12 bits and then jumps
straight to a 64-bit escape. Any series whose second derivative routinely needs 13 to 32 bits
falls off that cliff on almost every point — river discharge in whole cubic feet per second,
for instance. Adding buckets at 16, 20, 24 and 32 bits costs one bit of prefix to the rare wide
values and improved **every codec here**, discharge under decimal scaling most of all, from
4.85 to 2.19.

## Results

```
source     variable                 points   raw   gorilla   decimal     chimp  chimp128       elf   fastalp      auto
----------------------------------------------------------------------------------------------------------------------
co-ops     water_level              29,760    16      7.81      1.20      6.76      4.38      2.38      1.07      1.07
co-ops     water_level_sigma        29,760    16      6.83      1.18      6.17      1.80      1.91      0.99      0.99
isd-lite   air_temperature          79,807    16      7.07      1.23      5.44      2.72      2.08      1.15      1.15
isd-lite   dew_point                79,772    16      6.89      1.22      5.31      2.51      1.98      1.14      1.11
isd-lite   sea_level_pressure       72,857    16      6.54      1.24      5.26      2.65      2.01      1.08      1.08
isd-lite   wind_direction           69,384    16      2.05      1.46      2.13      3.01      2.13      1.05      1.05
isd-lite   wind_speed               71,298    16      6.47      1.20      3.94      2.31      2.06      0.86      0.86
usgs-nwis  discharge                17,452    16      2.42      2.18      2.12      3.00      2.12      1.46      1.41
usgs-nwis  gage_height              17,460    16      5.46      1.02      5.07      2.33      1.69      0.82      0.82

codec        bytes/point    vs raw        encode        decode
--------------------------------------------------------------
gorilla            5.918      2.7x    78 Mpt/s     52 Mpt/s 
decimal            1.283     12.5x    81 Mpt/s     93 Mpt/s 
chimp              4.663      3.4x    48 Mpt/s     42 Mpt/s 
chimp128           2.697      5.9x    53 Mpt/s     45 Mpt/s 
elf                2.052      7.8x    21 Mpt/s     43 Mpt/s 
fastalp            1.061     15.1x   122 Mpt/s    275 Mpt/s 
auto               1.055     15.2x     8 Mpt/s    234 Mpt/s 
```

Three things in that table are worth more than the averages.

**Decimal scaling wins eight of nine series.** The one it loses is instructive: river discharge
in whole cubic feet per second, where there is no decimal structure left to recover and plain
Chimp edges past it.

**Chimp128 loses to plain Chimp on exactly one variable, wind direction.** Its lookup table is
keyed on the low mantissa bits, whole numbers have those all zero, so every reading collides on
the same key, the reference degenerates to the previous value, and the 7-bit index buys
nothing. NOAA reports wind direction in whole degrees. The same collision is why Elf sits on
Chimp rather than Chimp128: erasing zeroes precisely the bits that table needs.

**Trying all five costs 9x in encode time and nothing at decode.** Encoding runs at 6 Mpt/s
instead of 53, but decoding is unchanged because the tag byte makes the block self-describing.
It is worth 0.9% over always using decimal scaling, which says the interesting work is in the
codecs, not in the selection.

### Block size

Everything above stores one block per series, which no real database does. Prometheus uses
two-hour blocks. Chunking the same data on epoch-aligned windows:

```
block window        blocks   gorilla   decimal     chimp  chimp128       elf   fastalp      auto
------------------------------------------------------------------------------------------------
6 hours             70,003      6.98      2.92      6.63      6.12      4.53      4.50      2.90
1 day               18,016      5.74      1.69      5.15      3.74      2.67      1.91      1.65
1 week               2,624      5.59      1.33      4.74      2.86      2.15      1.20      1.18
1 month                656      5.69      1.28      4.68      2.73      2.07      1.11      1.10
1 year                 112      5.91      1.28      4.66      2.70      2.05      1.06      1.06
whole series            64      5.92      1.28      4.66      2.70      2.05      1.06      1.05
```

The header still dominates below about a day's worth of points, so block size remains a bigger
lever than codec choice at the small end. It used to be far worse: storing the point count in a
fixed 32 bits and the first timestamp in a fixed 64 wasted most of both fields, since a Unix
timestamp needs 5 varint groups and a block count usually needs 1 or 2. Making the header
variable-length took six-hour blocks from 4.70 to 2.91 bytes per point and one-day blocks from
2.14 to 1.68, and cost nothing at any size.

Gorilla is the odd one out, reaching its minimum at one week and getting slightly worse with
larger blocks, because a longer block gives its single reusable window more chances to be a bad
fit.

### Against things it does not control

Comparing a codec only against your own implementation of its rivals proves nothing — a weak
rival might just be a weak implementation. So the same data, against the `tsz` crate (the
most-downloaded Rust Gorilla) and against general-purpose compressors run over the raw bytes:

```
format                         bytes   bytes/point     vs best
--------------------------------------------------------------
uncompressed                 7480800         16.00       15.2x
graupel::auto                 493234         1.055       1.00x
graupel::fastalp              496234         1.061       1.01x
graupel::decimal              599642         1.283       1.22x
graupel::elf                  959575         2.052       1.95x
xz -9                        1231516         2.634       2.50x
graupel::chimp128            1260925         2.697       2.56x
JSON + zstd -19              1335206         2.856       2.71x
zstd -19                     1826879         3.907       3.70x
JSON + gzip -9               1951620         4.174       3.96x
gzip -9                      2004038         4.286       4.06x
graupel::chimp               2180113         4.663       4.42x
graupel::gorilla             2767149         5.918       5.61x
tsz (Gorilla crate)          2788513         5.964       5.65x
```

The two rows that matter most are the last two. This crate's Gorilla and the reference crate
land within 0.8% of each other, which is the evidence that the baseline is implemented right
and that everything measured against it is measured fairly.

The rest: the best general-purpose compressor here, `xz -9`, needs twice the space, and `zstd
-19` over three times. Both are also far slower. Knowing the data is a series of timestamps and
floats is worth more than any amount of generic entropy coding.

Reproduce with `cargo run --release --example compare` — it verifies a lossless round trip
through `tsz` as well, so the sizes are comparable.

Every number here is produced on your machine. Nothing is quoted.

## Reproducing

```sh
./scripts/fetch-data.sh                        # ~15 MB, no account or API key anywhere
cargo run --release --bin graupel-bench        # the tables above
cargo run --release --example compare          # against tsz, gzip, zstd, xz
```

The script pulls from three public archives:

| source | cadence | precision | shape |
|---|---|---|---|
| [NOAA ISD-Lite](https://www.ncei.noaa.gov/pub/data/noaa/isd-lite/) | hourly | tenths | ten stations spanning climates |
| [NOAA CO-OPS](https://api.tidesandcurrents.noaa.gov/api/prod/datagetter) | 6 minutes | thousandths | four tide gauges, smooth and periodic |
| [USGS NWIS](https://waterservices.usgs.gov/nwis/iv/) | 15 minutes | whole and hundredths | four river gauges, spiky |

`YEAR=2019 ./scripts/fetch-data.sh` picks a different year.

The benchmark verifies a lossless round trip for every series and every block before reporting
anything, so a number you see is a number that survived decoding.

## Using it as a library

```toml
[dependencies]
graupel = "0.1"
```

```rust
use graupel::{codec::Auto, Codec, Point};

let readings = vec![
    Point::new(1_672_531_200, 8.2),
    Point::new(1_672_534_800, 8.1),
    Point::new(1_672_538_400, 7.9),
];

let block = Auto.encode(&readings)?;
let restored = graupel::decode(&block)?;

assert_eq!(restored, readings);
```

`decode` handles any block without being told which codec wrote it. That is also what lets the
decimal codec fall back to Gorilla when a block holds a value with no exact decimal form — a
NaN, an infinity, or something like π — instead of losing precision to force the scaling
through.

No dependencies. Builds without `std`:

```toml
graupel = { version = "0.1", default-features = false }
```

## What this is not

- **Not a time series database.** No storage engine, no index, no query layer. If you want one
  in Rust, [tsink](https://github.com/cantrepro/tsink) is active and well ahead.
- **Not new algorithms.** Gorilla is from 2015, Chimp from 2022, Elf from 2023, decimal scaling
  is VictoriaMetrics' documented approach. The contribution is the comparison, the harness, and
  two implementation details the papers gloss over: how to decide a value's decimal scale
  without wrecking it, and how wide the delta-of-delta buckets should be.
- **Not production-hardened.** Correct as far as the test suite reaches — randomised round
  trips, adjacent float bit patterns, truncation at every byte offset, bit-flip corruption of
  every block — but it has not run anywhere real.

## Layout

```
src/bits.rs             bit-level reader and writer
src/codec/dod.rs        delta-of-delta, shared by every codec
src/codec/gorilla.rs    XOR with a reusable significant-bit window
src/codec/decimal.rs    decimal scaling with a Gorilla fallback
src/codec/chimp.rs      bucketed leading zeros, explicit trailing zeros
src/codec/chimp128.rs   the same, XORed against the best of the last 128 values
src/codec/elf.rs        erase decimal-irrelevant mantissa bits, then Chimp
src/codec/auto.rs       encode with all five, keep the smallest
src/dataset/            parsers for the three archives
src/bin/bench.rs        the harness that produces the tables above
docs/format.md          bit-level specification of every block format
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Briefly: a change that claims to compress better has to
show the benchmark output before and after, and a codec that compresses better by losing a bit
pattern is not a smaller codec, it is a different one.

## References

- Pelkonen et al., [Gorilla: A Fast, Scalable, In-Memory Time Series Database](https://www.vldb.org/pvldb/vol8/p1816-teller.pdf), VLDB 2015
- Liakos et al., [Chimp: Efficient Lossless Floating Point Compression for Time Series Databases](https://www.vldb.org/pvldb/vol15/p3058-liakos.pdf), VLDB 2022, and its [reference implementation](https://github.com/panagiotisl/chimp)
- Li et al., [Elf: Erasing-Based Lossless Floating-Point Compression](https://www.vldb.org/pvldb/vol16/p1763-li.pdf), VLDB 2023
- Valyala, [VictoriaMetrics: achieving better compression than Gorilla](https://faun.pub/victoriametrics-achieving-better-compression-for-time-series-data-than-gorilla-317bc1f95932)

## License

MIT or Apache-2.0, at your option.
