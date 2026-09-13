# Third-party components

This repository bundles no third-party source code and no model weights. It does declare
Cargo dependencies, which a build fetches from crates.io. They are listed here so the
terms are visible without running `cargo tree`.

⚠ This file is written because MOTH's other repository states that it bundles nothing and
depends on nothing. That is true there and NOT true here, and copying the reassuring
sentence across would have been a false claim inherited by resemblance.

## Runtime dependencies (`Cargo.toml`)

| crate | version | why it is here | licence |
|---|---|---|---|
| `rkyv` | 0.7 | zero-copy archive format; the comparator the benchmarks measure against | MIT |
| `serde` | 1 | derive support for the event types | MIT OR Apache-2.0 |
| `serde_json` | 1 | a LABELLED FLOOR, not a serious comparator — see the note in `Cargo.toml` | MIT OR Apache-2.0 |
| `bytecheck` | 0.6 | validation for the rkyv path | MIT |
| `flux_macro` | path | the FLUX compiler itself; part of this repository, not third-party | Apache-2.0 |

`flux_macro` has **no dependencies of its own** — it is written directly against
`proc_macro`, deliberately, so the compiler carries no supply chain.

## Not included here

`flux_py`, the pyo3 sidecar, is not in this repository. It is one day old, lives in its
own Cargo workspace, and would add `pyo3` to the dependency set. It will be published, if
at all, only after it has run long enough against real telemetry to be worth someone's
time.

## Licence compatibility

All of the above are MIT or MIT/Apache-2.0 dual-licensed, which are compatible with this
repository's Apache-2.0 licence. No copyleft components are present.

⚠ Verified by reading each crate's stated licence field, not by running a licence scanner.
That is a weaker check than a scanner and is recorded as such.
