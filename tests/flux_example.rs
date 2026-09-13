//! The worked example from the brief: source expression in, witness out, result value, and the
//! same result from the scalar reference — on the real corpus distribution from event.rs.

use moth_flux::event::Outcome;
use moth_flux::flux::flux;
use moth_flux::{Planes, Trit};

#[test]
fn worked_example_witness_and_result() {
    // The .moth_events.jsonl distribution documented in event.rs.
    let dist = [
        (Outcome::NothingRan, 34_197usize),
        (Outcome::Ok, 6_743),
        (Outcome::Fail, 74),
        (Outcome::CannotTell, 250),
        (Outcome::NoMatch, 19),
    ];
    let len: usize = dist.iter().map(|(_, n)| n).sum();
    let mut ran = Planes::with_len(len);
    let mut verdict = Planes::with_len(len);
    let mut scalar_count = 0usize;
    let mut i = 0;
    for &(o, n) in &dist {
        for _ in 0..n {
            let (r, v, _m) = o.to_trits();
            ran.set(i, r);
            verdict.set(i, v);
            // the scalar reference, row at a time, straight from the trits
            if r == Trit::Yes && v != Trit::Yes {
                scalar_count += 1;
            }
            i += 1;
        }
    }

    let r = flux! { count(ran == YES and verdict != YES) };

    // fail(74) + cannot_tell(250) + no_match(19) = 343; ok and absent excluded.
    assert_eq!(r.value, 343, "the compiled path");
    assert_eq!(r.value, scalar_count, "compiled path vs scalar reference");
    assert_eq!(
        r.value,
        moth_flux::planes::ran_and_not_ok(&ran, &verdict, len),
        "compiled path vs the hand-written reference hot loop"
    );

    eprintln!("source:  count(ran == YES and verdict != YES)");
    eprintln!("{}", r.witness);
    eprintln!("result:  {}   (scalar reference: {})", r.value, scalar_count);

    // Requirement one: the witness is a positive artifact with one line per chosen operation,
    // in the order they were chosen. Header line first, then the ops.
    let lines: Vec<&str> = r.witness.lines().collect();
    let expect: [(&str, &str); 7] = [
        ("LOAD", "p0"),
        ("CMP_EQ_YES", "m0"),
        ("LOAD", "p1"),
        ("CMP_NE_YES", "m1"),
        ("AND", "m2"),
        ("TAILMASK", "m2"),
        ("POPCOUNT", "count"),
    ];
    assert_eq!(
        lines.len(),
        expect.len() + 1,
        "witness must have exactly one line per operation plus the header:\n{}",
        r.witness
    );
    for (line, (op, out)) in lines[1..].iter().zip(&expect) {
        assert!(
            line.starts_with(&format!("operation: {}", op)) && line.contains(&format!("output: {}", out)),
            "witness line {:?} does not describe ({}, {})",
            line,
            op,
            out
        );
    }
}

#[test]
fn plane_result_carries_a_witness_too() {
    let mut a = Planes::with_len(3);
    let mut b = Planes::with_len(3);
    a.set(0, Trit::Yes);
    a.set(1, Trit::No); // a[2] stays Unknown
    b.set(0, Trit::No);
    b.set(1, Trit::Yes);
    b.set(2, Trit::Yes);

    let r = flux! { select(a == UNKNOWN, b, a) };
    // row 0: a known -> keep a = Yes; row 1: keep a = No; row 2: a Unknown -> take b = Yes
    assert_eq!(r.value.get(0), Trit::Yes);
    assert_eq!(r.value.get(1), Trit::No);
    assert_eq!(r.value.get(2), Trit::Yes);
    assert!(r.witness.contains("CMP_EQ_UNKNOWN"));
    assert!(r.witness.contains("SELECT"));
    assert!(r.witness.contains("STORE"));
    eprintln!("source:  select(a == UNKNOWN, b, a)");
    eprintln!("{}", r.witness);
}

#[test]
fn runtime_boundary_different_populations_panic() {
    // Length agreement is a RUNTIME check (plane lengths are values, not types) - same policy
    // as planes.rs's assert. Honest limit, stated in the module docs.
    let a = Planes::with_len(64);
    let b = Planes::with_len(65);
    let r = std::panic::catch_unwind(|| flux! { count((a == YES) and (b == YES)) }.value);
    assert!(r.is_err(), "combining different populations must refuse loudly, not truncate");
}
