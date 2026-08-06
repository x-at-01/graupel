# graupel

Lossless time series compression codecs in Rust, measured against each other on public
observation archives instead of on synthetic data.

On 467,550 real readings from three archives — weather stations, tide gauges and river gauges:

| codec | bytes/point | vs uncompressed |
|---|---|---|
| Gorilla (VLDB 2015) | 6.06 | 2.6x |
| Chimp (VLDB 2022) | 4.80 | 3.3x |
| Chimp128 (VLDB 2022) | 2.84 | 5.6x |
| Decimal scaling | 2.01 | 8.0x |
| **Best-of-four per block** | **1.62** | **9.9x** |

The headline is not any single row. It is that **no codec wins everywhere**, and the gap
between the best and worst choice for a given series is often larger than the gap between
codecs on average.

## Why the choice matters more than the codec

Gorilla XORs each value against the previous one and stores only the window of bits that
changed. Its paper reports 1.37 bytes per point. On this data it manages 6.06.

The reason is that a station reporting 12.3 °C is not producing a binary quantity. IEEE-754
stores 12.3 as a long awkward fraction and 12.4 as a different long awkward fraction, so two
readings a tenth of a degree apart differ across most of the mantissa and the window stays
wide. Multiplying a block by the smallest power of ten that makes every value an exact integer
undoes that, which is the trick
[VictoriaMetrics documented](https://faun.pub/victoriametrics-achieving-better-compression-for-time-series-data-than-gorilla-317bc1f95932).

But that only helps when the quantity really is short in decimal. River discharge is reported
as whole cubic feet per second in the tens of thousands, and there decimal scaling is the
**worst** of the four — worse than plain Gorilla. Tide gauges report millimetres, three decimal
digits, and decimal scaling loses to Chimp128 there too.

Chimp attacks the same problem without assuming anything about decimals: it keeps the XOR but
rounds the leading-zero count into eight buckets and pays for an explicit trailing-zero count
only when that earns its keep. Chimp128 goes further and XORs against the best of the last 128
values rather than always the previous one.

Since blocks carry a tag byte naming their codec, an encoder can simply try all four and keep
the smallest. That is the `auto` row, and on this data it beats every fixed choice.

## Results

```
source     variable                 points   raw   gorilla   decimal     chimp  chimp128      auto
--------------------------------------------------------------------------------------------------
co-ops     water_level              29,760    16      7.81      6.30      6.76      4.38      3.66
co-ops     water_level_sigma        29,760    16      6.83      1.18      6.17      1.80      1.18
isd-lite   air_temperature          79,807    16      7.19      1.35      5.56      2.84      1.35
isd-lite   dew_point                79,772    16      7.01      1.35      5.43      2.63      1.35
isd-lite   sea_level_pressure       72,857    16      6.74      1.44      5.46      2.85      1.44
isd-lite   wind_direction           69,384    16      2.38      1.79      2.46      3.34      1.79
isd-lite   wind_speed               71,298    16      6.59      1.31      4.05      2.42      1.31
usgs-nwis  discharge                17,452    16      2.44      4.85      2.15      3.02      1.90
usgs-nwis  gage_height              17,460    16      5.49      5.44      5.09      2.36      2.36

codec        bytes/point    vs raw        encode        decode
--------------------------------------------------------------
gorilla            6.059      2.6x    48 Mpt/s     50 Mpt/s
decimal            2.012      8.0x    55 Mpt/s     74 Mpt/s
chimp              4.804      3.3x    39 Mpt/s     39 Mpt/s
chimp128           2.838      5.6x    40 Mpt/s     39 Mpt/s
auto               1.618      9.9x    11 Mpt/s     72 Mpt/s
```

Three things in that table are worth more than the averages.

**Decimal scaling wins five series and loses three.** It is best on tenths and on small
three-decimal values, and worst of all four on whole-number discharge.

**Chimp128 loses to plain Chimp on exactly one variable, wind direction.** Its lookup table is
keyed on the low mantissa bits, whole numbers have those all zero, so every reading collides on
the same key, the reference degenerates to the previous value, and the 7-bit index buys
nothing. NOAA reports wind direction in whole degrees.

**Trying all four costs 4.5x in encode time and nothing at decode.** Encoding runs at 11 Mpt/s
instead of 55, but decoding is unchanged because the tag byte makes the block self-describing.

### Block size

Everything above stores one block per series, which no real database does. Prometheus uses
two-hour blocks. Chunking the same data on epoch-aligned windows:

```
block window        blocks   gorilla   decimal     chimp  chimp128      auto
----------------------------------------------------------------------------
6 hours             70,003      8.74      5.81      8.39      7.89      5.70
1 day               18,016      6.27      2.85      5.68      4.27      2.64
1 week               2,624      5.79      2.04      4.93      3.05      1.74
1 month                656      5.85      1.99      4.83      2.89      1.64
1 year                 112      6.05      2.01      4.81      2.84      1.62
whole series            64      6.06      2.01      4.80      2.84      1.62
```

The 13-byte block header dominates until roughly a week's worth of points, and small blocks
cost far more than the choice of codec: at six-hour blocks the best codec is worse than the
worst codec at one-week blocks. Gorilla is the odd one out, reaching its minimum at one week
and getting slightly worse with larger blocks, because a longer block gives its single reusable
window more chances to be a bad fit.

Every number here is produced by the benchmark on your machine. Nothing is quoted.

## Reproducing

```sh
./scripts/fetch-data.sh          # ~15 MB, no account or API key anywhere
cargo run --release --bin graupel-bench
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
- **Not new algorithms.** Gorilla is from 2015, Chimp from 2022, decimal scaling is
  VictoriaMetrics' documented approach. The contribution is the comparison and the harness.
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
src/codec/auto.rs       encode with all four, keep the smallest
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
- Valyala, [VictoriaMetrics: achieving better compression than Gorilla](https://faun.pub/victoriametrics-achieving-better-compression-for-time-series-data-than-gorilla-317bc1f95932)

## License

MIT or Apache-2.0, at your option.
