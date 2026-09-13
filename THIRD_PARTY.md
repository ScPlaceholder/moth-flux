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

## `flux_py` — the Python sidecar (its OWN workspace)

⚠ ADDED 2026-09-13, AND THIS SECTION PREVIOUSLY SAID IT WAS EXCLUDED. The stated reason was
that it is "one day old". So is most of this repository. The real content of that judgement was
caution rather than a criterion, and it was asked about within the hour — kept here rather than
rewritten, because a reason that does not survive one question is worth seeing.

| crate | version | why it is here | licence |
|---|---|---|---|
| `pyo3` | 0.22 | the Python extension boundary, `extension-module` feature | MIT OR Apache-2.0 |
| `moth_flux` | path `..` | the FLUX compiler; part of this repository | Apache-2.0 |

`flux_py` declares its own `[workspace]` deliberately: adding it to the parent members list
would make every `cargo build` at the root try to link a Python extension, which changes the
frozen reference implementation's build for the sake of an experiment beside it.

⚠ It is NOT a second implementation of FLUX. There is no parser and no lowering in it — the
`flux!` blocks in `src/lib.rs` are compiled by the same proc-macro as everything else here.
Python hands it a bulk buffer and receives counts.

★ MEASURED, so nobody has to guess: FLUX beats a Python loop ~7x when the caller already holds
the buffer, and LOSES at 0.80x when Python must build that buffer row by row. Its own computation
is ~0.7% of the bulk-path runtime. The interesting number is not the speed — it is that the
three-valued enforcement costs almost nothing when the representation is already right.

## Licence compatibility

All of the above are MIT or MIT/Apache-2.0 dual-licensed, which are compatible with this
repository's Apache-2.0 licence. No copyleft components are present.

⚠ Verified by reading each crate's stated licence field, not by running a licence scanner.
That is a weaker check than a scanner and is recorded as such.
