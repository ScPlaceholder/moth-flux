//! FLUX — the smallest possible compiler for one constrained expression form over trit planes.
//!
//! # The thesis this module implements (settled with J and GPT, 2026-09-13 — not relitigated here)
//!
//! > FLUX is a constrained semantic language in which unresolved state and its physical
//! > representation are first-class, and every valid expression must compile directly to a
//! > semantics-preserving representation without implicit coercion, hidden materialization, or
//! > interpreter overhead.
//!
//! If an expression cannot be lowered semantics-preservingly, it is a COMPILE ERROR. Never a
//! slow path, never a silent materialization, never a warning beside a usable output.
//!
//! # Why a proc-macro
//!
//! One line, as promised: the refusals must be compile errors the caller cannot catch and route
//! around, and rustc-time expansion makes the fused loop a property of the emitted artifact
//! rather than a promise about an interpreter's behavior.
//!
//! # The language, all of it
//!
//! Two value kinds, and the boundary between them is the point:
//!
//! | kind  | produced by                                                     |
//! |-------|-----------------------------------------------------------------|
//! | Plane | a named input; `and`/`or`/`xor`/`not` over planes (Kleene); `mask(p, m)`; `select(m, a, b)` |
//! | Mask  | a comparison `p == YES` / `p != UNKNOWN` (any of the 6 forms); `and`/`or`/`xor`/`not` over masks (boolean) |
//!
//! A comparison is the plane-level mirror of [`Trit::resolve`](crate::Trit::resolve): the ONLY
//! door from ternary to binary, and writing it states what Unknown means at that crossing
//! (`p == YES`: Unknown is a miss; `p != NO`: Unknown is a hit). Top level must be `count(<mask>)`
//! (→ [`Fluxed<usize>`]) or a plane expression (→ `Fluxed<Planes>`).
//!
//! Semantics decisions, stated because they are decisions:
//! - Comparisons are STRUCTURAL: they ask what the record says, so `p == UNKNOWN` is true exactly
//!   where the record is Unknown. This is not SQL's `NULL = NULL` trap — the trap is a comparison
//!   that returns the third value and then gets silently coerced; here the comparison returns a
//!   definite bit and there is no coercion anywhere to lose it in.
//! - `mask(p, m)`: a masked-out row becomes **Unknown** (unasked), never No.
//! - Kleene ops on planes follow the [`Kleene`](crate::trit::Kleene) truth tables; XOR of an
//!   Unknown with anything is Unknown.
//!
//! # Requirement one — the lowering witness
//!
//! Every compilation embeds the lowering it actually chose as [`Fluxed::witness`]: one line per
//! word-level operation, generated from the same op list that generated the code, so witness and
//! code cannot drift independently. Every invariant in the thesis is a negative (no interpreter,
//! no coercion, no materialization) and a negative claim verified by nothing is how a broken
//! instrument reports a clean board — the witness is the positive artifact a human can read.
//!
//! # Requirement two — executable semantic proof
//!
//! Every operation is proven against an independent scalar reference, exhaustively over its trit
//! domain (3^n for n plane inputs), at lengths including 0 and non-multiples of 64, in
//! `tests/flux_proof.rs`. No operation joins the language without passing it.
//!
//! # A worked example
//!
//! ```
//! use moth_flux::flux::flux;
//! use moth_flux::{Planes, Trit};
//!
//! let mut ran = Planes::with_len(5);
//! let mut verdict = Planes::with_len(5);
//! // row 0: nothing ran; row 1: ok; row 2: fail; row 3: cannot_tell; row 4: no_match
//! ran.set(0, Trit::No);
//! for i in 1..5 { ran.set(i, Trit::Yes); }
//! verdict.set(1, Trit::Yes);
//! verdict.set(2, Trit::No);
//! // rows 3 and 4 stay Unknown - an answer ("cannot tell"), not an absence
//!
//! let r = flux! { count(ran == YES and verdict != YES) };
//! assert_eq!(r.value, 3); // fail + cannot_tell + no_match; absent and ok excluded
//! assert!(r.witness.contains("POPCOUNT"));
//! ```
//!
//! # Refusals — the boundary saying no
//!
//! Each block below MUST fail to compile; `cargo test` runs them as `compile_fail` doctests.
//! (Honest limit: `compile_fail` proves *a* compile error fired, not which one — the blocks are
//! kept minimal so the FLUX refusal is the only candidate. trybuild with blessed .stderr files
//! is the upgrade path if that ever stops being convincing.)
//!
//! A plane does not coerce to a mask — the flagship refusal:
//!
//! ```compile_fail
//! use moth_flux::flux::flux;
//! use moth_flux::Planes;
//! let ran = Planes::with_len(4);
//! let verdict = Planes::with_len(4);
//! let _ = flux! { count((ran == YES) and verdict) };
//! ```
//!
//! `count` of a bare plane is ambiguous — say what Unknown means:
//!
//! ```compile_fail
//! use moth_flux::flux::flux;
//! use moth_flux::Planes;
//! let ran = Planes::with_len(4);
//! let _ = flux! { count(ran) };
//! ```
//!
//! A bare mask cannot be the result — materializing it would invent known-ness:
//!
//! ```compile_fail
//! use moth_flux::flux::flux;
//! use moth_flux::Planes;
//! let ran = Planes::with_len(4);
//! let _ = flux! { ran == YES };
//! ```
//!
//! Comparing two planes is not in the language:
//!
//! ```compile_fail
//! use moth_flux::flux::flux;
//! use moth_flux::Planes;
//! let ran = Planes::with_len(4);
//! let verdict = Planes::with_len(4);
//! let _ = flux! { count(ran == verdict) };
//! ```
//!
//! `count` cannot appear inside a row-wise expression:
//!
//! ```compile_fail
//! use moth_flux::flux::flux;
//! use moth_flux::Planes;
//! let ran = Planes::with_len(4);
//! let verdict = Planes::with_len(4);
//! let _ = flux! { count((ran == YES) and (count(verdict == YES) == YES)) };
//! ```
//!
//! Rust's boolean operators are refused by name, with the FLUX spelling in the error:
//!
//! ```compile_fail
//! use moth_flux::flux::flux;
//! use moth_flux::Planes;
//! let ran = Planes::with_len(4);
//! let verdict = Planes::with_len(4);
//! let _ = flux! { count((ran == YES) && (verdict == YES)) };
//! ```

pub use flux_macro::flux;

/// A compiled-and-executed FLUX expression: the value plus the lowering witness.
///
/// The witness is requirement one, not decoration: it is the positive, inspectable record of the
/// lowering the compiler actually chose — one line per word-level operation, in the order chosen,
/// with the exact word formula after `;;`. It is a `&'static str` baked in at compile time from
/// the same op list the code was generated from.
#[derive(Debug, Clone)]
pub struct Fluxed<T> {
    /// The result: `usize` for `count(...)`, [`crate::Planes`] for a plane expression.
    pub value: T,
    /// The lowering, as chosen at compile time. Read it; that is what it is for.
    pub witness: &'static str,
}
