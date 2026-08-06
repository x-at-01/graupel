# graupel

Four lossless time series compression codecs in Rust, measured against each other on real
weather station observations.

The interesting result is not that any of them is new — none is — but by how much the choice
matters. On 373,118 hourly readings from ten NOAA stations:

| codec | bytes/point | vs uncompressed |
|---|---|---|
| Gorilla (VLDB 2015) | 6.05 | 2.6x |
| Chimp (VLDB 2022) | 4.65 | 3.4x |
| Chimp128 (VLDB 2022) | 2.81 | 5.7x |
| Decimal scaling | **1.44** | **11.1x** |

Gorilla is the algorithm most time series databases still ship, and the one whose paper
reports 1.37 bytes per point. On this data it manages 6.05. That gap is the whole point of
the repository.

## Why the gap

Gorilla XORs each value against the previous one and stores only the window of bits that
changed. It assumes consecutive values share most of their binary representation.

A station reporting 12.3 °C is not producing a binary quantity. IEEE-754 stores 12.3 as a
long, awkward fraction, and 12.4 as a different long awkward fraction. Two readings a tenth
of a degree apart can differ across most of the mantissa, so the window Gorilla has to store
stays wide.

Multiplying the whole block by the smallest power of ten that turns every value into an exact
integer undoes the damage. 12.3 and 12.4 become 123 and 124, and integers respond to
delta-of-delta the way the paper always promised. This is the trick
[VictoriaMetrics documented](https://faun.pub/victoriametrics-achieving-better-compression-for-time-series-data-than-gorilla-317bc1f95932),
and the numbers above are an independent check of their claim on data they did not choose.

Chimp attacks the same problem from the other side: it keeps the XOR but spends its bits more
carefully, rounding the leading-zero count into eight buckets and paying for an explicit
trailing-zero count only when that earns its keep. Chimp128 goes further and XORs against the
best of the last 128 values instead of always the previous one. Both get there without
assuming anything about decimal precision, which matters for values that genuinely have no
short decimal form.

## Results

```
variable                    points     raw   gorilla   decimal     chimp  chimp128
----------------------------------------------------------------------------------
air_temperature             79,807      16      7.19      1.35      5.56      2.84
dew_point                   79,772      16      7.01      1.35      5.43      2.63
sea_level_pressure          72,857      16      6.74      1.44      5.46      2.85
wind_direction              69,384      16      2.38      1.79      2.46      3.34
wind_speed                  71,298      16      6.59      1.31      4.05      2.42

codec        bytes/point    vs raw        encode        decode
--------------------------------------------------------------
gorilla            6.054      2.6x    34 Mpt/s     37 Mpt/s
decimal            1.443     11.1x    35 Mpt/s     44 Mpt/s
chimp              4.649      3.4x    26 Mpt/s     31 Mpt/s
chimp128           2.811      5.7x    27 Mpt/s     30 Mpt/s
```

Wind direction is the one variable that inverts every ranking, and the reason is the same in
each case: NOAA reports it in whole degrees. Decimal scaling has nothing to undo, so its lead
mostly evaporates. The series jumps discontinuously, so Chimp loses to Gorilla. And a whole
number has all-zero low mantissa bits, so every reading collides on the same key in Chimp128's
lookup table, the reference degenerates to the previous value, and it pays 7 bits a point to
name it — the only variable where Chimp128 comes out worse than plain Chimp.

Two takeaways survive that. The trick that wins depends on how the numbers were rounded before
they were ever stored, and an average over variables would have hidden all of it.

Every number above is produced by the benchmark on your own machine. Nothing is quoted.

## Reproducing

```sh
./scripts/fetch-data.sh          # ~10 MB from NOAA, no account or API key
cargo run --release --bin graupel-bench
```

`fetch-data.sh` pulls a year of hourly observations from NOAA's ISD-Lite archive for ten
stations chosen to span climates, from Singapore's near-constant equatorial temperatures to
Albuquerque's high desert swings. `YEAR=2019 ./scripts/fetch-data.sh` picks a different year.

The benchmark verifies a lossless round trip for every series before reporting anything, so a
result you see is a result that survived decoding.

## Using it as a library

```rust
use graupel::{codec::Decimal, Codec, Point};

let readings = vec![
    Point::new(1_672_531_200, 8.2),
    Point::new(1_672_534_800, 8.1),
    Point::new(1_672_538_400, 7.9),
];

let block = Decimal.encode(&readings)?;
let restored = graupel::decode(&block)?;

assert_eq!(restored, readings);
```

Blocks carry a one-byte tag naming the codec that wrote them, so `decode` handles any block
without being told which one it was. That also lets the decimal codec fall back to Gorilla
transparently when a block contains a value with no exact decimal form — a NaN, an infinity,
or something like π — rather than losing precision to force the scaling through.

No dependencies, `Cargo.toml` included.

## What this is not

- **Not a time series database.** There is no storage engine, no index, no query layer. If
  you want one in Rust, [tsink](https://github.com/cantrepro/tsink) is active and well ahead.
- **Not new algorithms.** Gorilla is from 2015, Chimp from 2022, decimal scaling is
  VictoriaMetrics' documented approach. The contribution here is the comparison and the
  measurement harness, not the ideas.
- **Not production-hardened.** It is correct as far as the test suite reaches, which includes
  randomised round trips, adjacent float bit patterns, and bit-flip corruption of every block,
  but it has not run anywhere real.

## Layout

```
src/bits.rs           bit-level reader and writer
src/codec/dod.rs      delta-of-delta, shared by all three codecs
src/codec/gorilla.rs  XOR with a reusable significant-bit window
src/codec/decimal.rs  decimal scaling with a Gorilla fallback
src/codec/chimp.rs    bucketed leading zeros and explicit trailing zeros
src/codec/chimp128.rs the same, XORed against the best of the last 128 values
src/dataset.rs        NOAA ISD-Lite parser
src/bin/bench.rs      the harness that produces the tables above
docs/format.md        bit-level specification of all three block formats
```

## Contributing

Contributions are welcome, and several of the most interesting pieces are deliberately
unfinished. See [CONTRIBUTING.md](CONTRIBUTING.md) for what needs doing and how results are
expected to be reported — briefly: a change that claims to compress better has to show the
benchmark output before and after.

## References

- Pelkonen et al., [Gorilla: A Fast, Scalable, In-Memory Time Series Database](https://www.vldb.org/pvldb/vol8/p1816-teller.pdf), VLDB 2015
- Liakos et al., [Chimp: Efficient Lossless Floating Point Compression for Time Series Databases](https://www.vldb.org/pvldb/vol15/p3058-liakos.pdf), VLDB 2022, and its [reference implementation](https://github.com/panagiotisl/chimp)
- Valyala, [VictoriaMetrics: achieving better compression than Gorilla](https://faun.pub/victoriametrics-achieving-better-compression-for-time-series-data-than-gorilla-317bc1f95932)
- NOAA, [Integrated Surface Database (ISD-Lite)](https://www.ncei.noaa.gov/products/land-based-station/integrated-surface-database)

## License

MIT or Apache-2.0, at your option.
