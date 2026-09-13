//! MOTH FLUX — Phase 0. The smallest thing that can REFUTE the hypothesis.
//!
//! J's proposal (2026-09-13 00:43) is a hybrid binary/ternary systems language. This crate is
//! deliberately NOT that. It is the experiment that decides whether the language is worth writing:
//! a `trit`, a packed record, and one real workload measured against a comparator that can win.
//!
//! # The two invariants, and why they are types rather than style rules
//!
//! **1. A trit may record THAT a value is unknown. It may never hold the computation that would
//! find out.** This is the whole answer to "isn't this just laziness, and doesn't laziness wreck
//! predictable memory?" Haskell's space leaks come from THUNKS — a deferred computation retains
//! references to its inputs, so one unevaluated chain can hold an arbitrarily large graph alive.
//! A `Trit` is a value: two bits, `Copy`, structurally incapable of containing a pointer. A million
//! unresolved trits cost 250 KB and can retain nothing. `Unknown` is not a promise to compute
//! later; it is a fact known now — that we do not know.
//!
//! **2. Crossing from ternary to binary must be explicit and total.** SQL is the largest deployment
//! of three-valued logic in the world and its most notorious footgun is exactly this seam: `NULL`
//! comparisons yield `NULL`, `WHERE` treats `NULL` as not-true, and rows silently vanish. Correct
//! logic, silent coercion, wrong answer. So there is no `impl From<Trit> for bool` here, and there
//! never should be. You get [`Trit::resolve`], which makes you say what unknown means at that call
//! site, and it is the only door.
//!
//! # What this crate is measuring
//!
//! Real corpus: `.moth_events.jsonl`, 41,283 events from MOTH City's own telemetry. The field that
//! makes it the right corpus is `outcome`:
//!
//! ```text
//! ok(6743)  cannot_tell(250)  fail(74)  no_match(19)   — present on 7,087 of 41,283 rows
//! ```
//!
//! 83% of events carry **no outcome at all**. In JSON that is a missing key; in a Rust struct an
//! `Option<Outcome>` and a branch; here it is a trit resting in its `Unknown` state. That is not a
//! storage trick — the absence is the most common state in the data, and it is the one binary
//! representations have no room for.
//!
//! ⚠ The comparator is **rkyv**, not serde_json. Beating JSON would prove nothing; nobody serious
//! uses JSON where this matters, and a baseline built to lose is how a null result gets laundered
//! into a win. serde_json stays only as a labelled floor.

#![forbid(unsafe_code)]

pub mod cardinality;
pub mod cost;
pub mod event;
pub mod flux;
pub mod kernels; // EXPERIMENT ONLY — does library location recover vectorisation? see file header
pub mod planes;
pub mod store;
pub mod trit;

pub use planes::Planes;
pub use trit::{Kleene, Trit, Trits};
