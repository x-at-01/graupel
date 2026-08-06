# Block formats

Every block is one byte of tag followed by a big-endian bit stream. Bits are written most
significant first, and the final byte is zero-padded. A decoder never needs the padding
length because the point count tells it when to stop.

| tag | codec |
|---|---|
| `0x00` | Gorilla |
| `0x01` | Decimal scaling |
| `0x02` | Chimp |
| `0x03` | Chimp128 |

The tag is what lets a stored block be decoded without recording which codec produced it. It is
also what makes two things possible without any format of their own: the decimal codec emits a
Gorilla block when scaling would lose precision, and `Auto` encodes with all four and keeps the
smallest, whichever that turns out to be.

## Shared: delta-of-delta

Timestamps use delta-of-delta in all three codecs, and the decimal codec uses the same
encoding for its scaled integer values. Given a value `v[n]`, the encoder stores
`(v[n] - v[n-1]) - (v[n-1] - v[n-2])`, with both previous deltas taken as zero at the start of
a block. That start condition removes the special case the original paper needs for the
second point, at a cost of one wide bucket per block.

| prefix | payload | range |
|---|---|---|
| `0` | none | exactly 0 |
| `10` | 7 bits, two's complement | -64 to 63 |
| `110` | 9 bits, two's complement | -256 to 255 |
| `1110` | 12 bits, two's complement | -2048 to 2047 |
| `1111` | 64 bits, two's complement | anything |

The ranges are the natural two's complement ones rather than the off-by-one ranges printed in
the Gorilla paper. A steady sampling interval therefore costs exactly one bit per point, and
an arbitrary gap always fits, so a series with missing readings needs no special handling.

## Gorilla (`0x00`)

```
32 bits   point count, unsigned
--- if count == 0, the block ends here ---
64 bits   first timestamp, two's complement
64 bits   first value, raw IEEE-754 bit pattern
--- then, for each remaining point ---
          timestamp delta-of-delta
          value, encoded below
```

Each value is XORed against the previous one:

| prefix | meaning | payload |
|---|---|---|
| `0` | XOR is zero, value unchanged | none |
| `10` | reuse the previous window | `64 - leading - trailing` bits |
| `11` | new window | 5 bits leading, 6 bits width, `width` bits |

The previous window is only reusable when the new value's leading and trailing zero counts are
both at least as large as the stored window's, so the significant bits are guaranteed to fit.

Two details are easy to get wrong:

- The leading-zero count is clamped to 31 because the field is 5 bits. Any zeros beyond that
  travel inside the significant-bit window, which stays lossless but wastes space. This is a
  real limit of the original design, not a shortcut taken here.
- The width field is 6 bits and the range is 1 to 64, so a full 64-bit window is stored as 0.
  A width of zero cannot otherwise occur, because that branch is only reached when the XOR is
  non-zero.

## Decimal scaling (`0x01`)

```
32 bits   point count, unsigned
 8 bits   decimal scale s, 0 to 17
--- if count == 0, the block ends here ---
64 bits   first timestamp, two's complement
64 bits   first scaled value, two's complement
--- then, for each remaining point ---
          timestamp delta-of-delta
          scaled value delta-of-delta
```

The encoder picks the smallest `s` in 0 to 17 such that every value `v` in the block satisfies
all of:

- `v * 10^s` is finite and has no fractional part
- `|v * 10^s| <= 2^53`, above which an f64 can no longer hold every integer
- `(v * 10^s) / 10^s == v`, the round trip actually returning the original bit pattern
- `v` is not negative zero, which would come back as positive zero after passing through an
  integer

If no such `s` exists, the encoder emits a Gorilla block instead. One awkward value is enough
to disqualify the whole block, which is the price of the scale being a block-level property.

This is also why the codec is not streaming: it must see every value before it can choose `s`
and write the first byte.

## Chimp (`0x02`)

Same framing as Gorilla, with a different value encoding:

| prefix | meaning | payload |
|---|---|---|
| `00` | XOR is zero | none |
| `01` | enough trailing zeros to store them explicitly | 3 bits leading index, 6 bits width, `width` bits |
| `10` | same leading-zero bucket as the previous value | `64 - leading` bits |
| `11` | new leading-zero bucket | 3 bits leading index, `64 - leading` bits |

Leading-zero counts are rounded down into eight buckets — 0, 8, 12, 16, 18, 20, 22, 24 —
addressed by a 3-bit index, so the count costs three bits instead of Gorilla's five. Rounding
down never overstates the zeros, so no significant bit is ever lost.

The `01` branch is taken when there are more than six trailing zeros, which is the point at
which the nine bits of overhead pay for the window they save. After a `01`, the reference
leading count is invalidated: the window just written is narrower than a `10` would imply, so
the next value must state its bucket explicitly.

## Chimp128 (`0x03`)

Same framing again. The difference is that the XOR reference is no longer always the previous
value but the best of the last 128, so two of the four branches carry a 7-bit reference index:

| prefix | meaning | payload |
|---|---|---|
| `00` | XOR is zero | 7 bits reference |
| `01` | enough trailing zeros to store them explicitly | 7 bits reference, 3 bits leading index, 6 bits width, `width` bits |
| `10` | same leading-zero bucket, previous value as reference | `64 - leading` bits |
| `11` | new leading-zero bucket, previous value as reference | 3 bits leading index, `64 - leading` bits |

Finding the reference is the interesting part. A side table of 2^14 entries maps the low 14
bits of a value to the sequence number that last held a value with those same low bits. Two
values agreeing on their low bits XOR to something with at least 14 trailing zeros, so a
single lookup produces a good candidate in constant time. The candidate is used only when it
is still inside the 128-value window and its XOR has more than 13 trailing zeros — 7 bits to
name the reference plus 6 for the width, which is what it has to earn back.

Both branches carrying a reference index invalidate the stored leading count afterwards,
because neither implies a window the next value could reuse.

The failure mode is worth stating plainly: a whole number has all-zero low mantissa bits, so
every whole number collides on key zero. The lookup then returns the previous value and the
7-bit index buys nothing. This is measurable — it is why wind direction is the one variable in
the benchmark where Chimp128 loses to plain Chimp.

## Robustness

Decoders reject impossible windows rather than reading out of range, and every decode path
returns `Err` instead of panicking on truncated or corrupted input. The test suite flips a bit
in every byte position of every block, two thousand times per codec, and asserts that nothing
panics.
