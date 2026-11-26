# Reference packets

E1.31 data packets built by an independent implementation (TypeScript, written separately from pixel-cast), one
packet per file as hexadecimal text (whitespace ignored). `tests/packets.rs` builds the same packets with
`pixel_cast::sacn` and compares them byte for byte.

| File | CID | Source name | Priority | Universe | Sequence | Options | Values |
|---|---|---|---|---|---|---|---|
| `full-universe.hex` | zeros | `pixel-cast` | 100 | 1 | 0 | none | 510: `i & 255` |
| `last-universe-terminated.hex` | `01…10` | `pixel-cast test` | 150 | 258 | 200 | Stream_Terminated | 144: `(i × 7) & 255` |
| `utf8-name-max-universe.hex` | `01…10` | `Écran № 1` | 0 | 63999 | 255 | Preview_Data | none |
