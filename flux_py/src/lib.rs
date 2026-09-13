//! FLUX, callable from Python. The B-shaped sidecar from J's question, not the A-shaped one.
//!
//! # What this is NOT
//!
//! There is no FLUX parser here, no IR, no second lowering. Python cannot write `flux!` expressions
//! through this module and it never will — that would be a second implementation of the semantics,
//! which is precisely what G warned against and what I argued against before contradicting myself
//! in the same message. The `flux!` blocks below are compiled by the SAME proc-macro that compiles
//! `moth_cards.rs`, in this same workspace, from the same source of meaning.
//!
//! # The one design decision that decides whether the benchmark is honest
//!
//! `building_cards.py` classifies each row in a Python loop. If this extension made Python build
//! the planes and only then called across, the Python loop would sit in BOTH arms and FLUX could
//! not win no matter how fast it was — I would be benchmarking `for` against `for`.
//!
//! So Rust takes a BULK BUFFER of pre-encoded codes and does the classification itself:
//!
//! ```text
//!   0  outcome absent      -> present = No , verdict = Unknown
//!   1  outcome "ok"        -> present = Yes, verdict = Yes
//!   2  outcome cannot_tell -> present = Yes, verdict = Unknown
//!   3  anything else       -> present = Yes, verdict = No
//! ```
//!
//! ⚠ AND THAT ENCODING IS THE HONEST CATCH, SO IT IS STATED IN THE OPEN: somebody has to produce
//!   those codes. If the caller builds them one at a time in Python, the cost simply moved and this
//!   whole extension is theatre. `bench_flux_py.py` therefore measures the marshalling arm
//!   SEPARATELY rather than folding it into the win.
use moth_flux::flux::flux;
use moth_flux::{Planes, Trit};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// Two planes, because `absent` and `cannot_tell` are different facts and one trit cannot hold
/// both — the same conclusion `moth_cards.rs` reached from `building_cards.py`'s shape.
fn planes_from_codes(codes: &[u8]) -> (Planes, Planes) {
    let n = codes.len();
    let mut present = Planes::with_len(n);
    let mut verdict = Planes::with_len(n);
    for (i, c) in codes.iter().enumerate() {
        let (p, v) = match c {
            0 => (Trit::No, Trit::Unknown),
            1 => (Trit::Yes, Trit::Yes),
            2 => (Trit::Yes, Trit::Unknown),
            _ => (Trit::Yes, Trit::No),
        };
        present.set(i, p);
        verdict.set(i, v);
    }
    (present, verdict)
}

/// `(unk_n, fail_n)` for one target's rows — the exact pair `building_cards._scan_events` produces.
///
/// `unk_n` counts ONLY the explicit third state. Writing `verdict == UNKNOWN` alone would also
/// catch rows that carried no outcome at all, which is the merge `building_cards` deliberately
/// does not do.
#[pyfunction]
fn count_unk_fail(codes: &Bound<'_, PyBytes>) -> (usize, usize) {
    let (present, verdict) = planes_from_codes(codes.as_bytes());
    let unk = flux! { count(present == YES and verdict == UNKNOWN) };
    let fail = flux! { count(present == YES and verdict == NO) };
    (unk.value, fail.value)
}

/// Plane construction WITHOUT the counting, so the benchmark can attribute time to marshalling
/// versus computation instead of reporting one number and calling it "FLUX".
#[pyfunction]
fn build_only(codes: &Bound<'_, PyBytes>) -> usize {
    let (present, _verdict) = planes_from_codes(codes.as_bytes());
    present.len()
}

/// ⚠ A DELIBERATE REFUSAL, EXPOSED SO THE BENCHMARK CANNOT QUIETLY CHEAT.
///
/// This is the rule `city_time_bench.py` mislabelled this morning: `outcome != "ok"`. It counts a
/// `cannot_tell` and an absent row as failures. It is here ONLY so the harness can demonstrate that
/// the two rules give different numbers on the same data — the reason the distinction is not
/// pedantry. Nothing in MOTH-OS should call it.
#[pyfunction]
fn count_not_ok_WRONG(codes: &Bound<'_, PyBytes>) -> usize {
    codes.as_bytes().iter().filter(|c| **c != 1).count()
}

#[pymodule]
fn flux_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(count_unk_fail, m)?)?;
    m.add_function(wrap_pyfunction!(build_only, m)?)?;
    m.add_function(wrap_pyfunction!(count_not_ok_WRONG, m)?)?;
    Ok(())
}
