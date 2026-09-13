//! REQUIREMENT TWO — executable semantic proof, per operation, over the FULL trit domain.
//!
//! The scalar reference in `mod scalar` is written from the truth tables, one row at a time,
//! with `match` and `if` — it never touches a bit-plane word, so it cannot inherit a bug from
//! the lowering it is checking. If reference and lowering ever disagree, the lowering is wrong
//! by definition; the tables are the spec.
//!
//! Domain coverage per operation (n = number of plane inputs, domain = 3^n):
//! - comparisons (6 forms): 3 cases each, as pure vectors AND cycling vectors
//! - mask and/or/xor: 9 pairs; mask not: 3
//! - Kleene and/or/xor: 9 pairs; Kleene not: 3 — checked PER ROW on the output plane
//! - mask(): 9 pairs; select(): 27 triples — checked PER ROW
//!
//! Tail cases are first-class: every sweep runs at lengths {0, 1, 5, 63, 64, 65, 128, 130, 200}
//! (length 0, sub-word, exact-word, word+1), and one test corrupts the padding bits directly.

use moth_flux::flux::flux;
use moth_flux::{Planes, Trit};

const ALL: [Trit; 3] = [Trit::No, Trit::Unknown, Trit::Yes];
const LENS: [usize; 9] = [0, 1, 5, 63, 64, 65, 128, 130, 200];

/// The dumb, obviously-correct reference. One row at a time. No words, no planes.
mod scalar {
    use moth_flux::Trit;

    pub fn cmp_eq(t: Trit, lit: Trit) -> bool {
        t == lit
    }
    pub fn cmp_ne(t: Trit, lit: Trit) -> bool {
        t != lit
    }
    pub fn k_and(a: Trit, b: Trit) -> Trit {
        match (a, b) {
            (Trit::No, _) | (_, Trit::No) => Trit::No,
            (Trit::Yes, Trit::Yes) => Trit::Yes,
            _ => Trit::Unknown,
        }
    }
    pub fn k_or(a: Trit, b: Trit) -> Trit {
        match (a, b) {
            (Trit::Yes, _) | (_, Trit::Yes) => Trit::Yes,
            (Trit::No, Trit::No) => Trit::No,
            _ => Trit::Unknown,
        }
    }
    pub fn k_not(a: Trit) -> Trit {
        match a {
            Trit::Yes => Trit::No,
            Trit::No => Trit::Yes,
            Trit::Unknown => Trit::Unknown,
        }
    }
    pub fn k_xor(a: Trit, b: Trit) -> Trit {
        match (a, b) {
            (Trit::Unknown, _) | (_, Trit::Unknown) => Trit::Unknown,
            _ => {
                if (a == Trit::Yes) != (b == Trit::Yes) {
                    Trit::Yes
                } else {
                    Trit::No
                }
            }
        }
    }
    pub fn mask_op(t: Trit, m: bool) -> Trit {
        if m {
            t
        } else {
            Trit::Unknown // a masked-out row is UNASKED, not No
        }
    }
    pub fn select_op(m: bool, a: Trit, b: Trit) -> Trit {
        if m {
            a
        } else {
            b
        }
    }
}

fn planes_from(v: &[Trit]) -> Planes {
    let mut p = Planes::with_len(v.len());
    for (i, &t) in v.iter().enumerate() {
        p.set(i, t);
    }
    p
}

// Cycling inputs: (va, vb) covers all 9 pairs with period 9; (vm, va, vb) all 27 with period 27.
fn va(len: usize) -> Vec<Trit> {
    (0..len).map(|i| ALL[i % 3]).collect()
}
fn vb(len: usize) -> Vec<Trit> {
    (0..len).map(|i| ALL[(i / 3) % 3]).collect()
}
fn vc(len: usize) -> Vec<Trit> {
    (0..len).map(|i| ALL[(i / 9) % 3]).collect()
}

fn canonical_check(p: &Planes, len: usize, what: &str) {
    for w in 0..p.known.len() {
        assert_eq!(p.value[w] & !p.known[w], 0, "{}: value bit outside known", what);
    }
    if len % 64 != 0 && !p.known.is_empty() {
        let tail = !((1u64 << (len % 64)) - 1);
        assert_eq!(
            p.known[p.known.len() - 1] & tail,
            0,
            "{}: padding leaked into known",
            what
        );
    }
}

// ------------------------------------------------------------------------------------------
// Comparisons: the six doors. Domain 3 each.
// ------------------------------------------------------------------------------------------

#[test]
fn proof_comparisons_exhaustive() {
    let mut cases = 0usize;
    let mut run = |v: &[Trit]| {
        let p = planes_from(v);
        let got = [
            flux! { count(p == YES) }.value,
            flux! { count(p == NO) }.value,
            flux! { count(p == UNKNOWN) }.value,
            flux! { count(p != YES) }.value,
            flux! { count(p != NO) }.value,
            flux! { count(p != UNKNOWN) }.value,
        ];
        let want = [
            v.iter().filter(|&&t| scalar::cmp_eq(t, Trit::Yes)).count(),
            v.iter().filter(|&&t| scalar::cmp_eq(t, Trit::No)).count(),
            v.iter().filter(|&&t| scalar::cmp_eq(t, Trit::Unknown)).count(),
            v.iter().filter(|&&t| scalar::cmp_ne(t, Trit::Yes)).count(),
            v.iter().filter(|&&t| scalar::cmp_ne(t, Trit::No)).count(),
            v.iter().filter(|&&t| scalar::cmp_ne(t, Trit::Unknown)).count(),
        ];
        assert_eq!(got, want, "comparisons diverged at len {}", v.len());
        cases += 6 * v.len();
    };
    for &len in &LENS {
        run(&va(len)); // cycling: every trit at every position class, including the tail word
    }
    for &t in &ALL {
        run(&vec![t; 70]); // pure: each domain point alone, crossing a word boundary
    }
    eprintln!(
        "proof_comparisons_exhaustive: 6 forms x 3-trit domain, {} row-level checks",
        cases
    );
}

// ------------------------------------------------------------------------------------------
// Boolean ops over masks. Domain 3^2 = 9 per binary op (through comparison doors), 3 for not.
// ------------------------------------------------------------------------------------------

#[test]
fn proof_mask_boolean_ops_exhaustive() {
    let mut cases = 0usize;
    let mut run = |a_: &[Trit], b_: &[Trit]| {
        let a = planes_from(a_);
        let b = planes_from(b_);
        let pairs = a_.iter().zip(b_.iter());
        let and = flux! { count((a == YES) and (b == YES)) }.value;
        let or = flux! { count((a == YES) or (b == YES)) }.value;
        let xor = flux! { count((a == YES) xor (b == YES)) }.value;
        let not = flux! { count(not (a == YES)) }.value;
        // same ops through DIFFERENT doors, so the door choice is exercised too
        let and2 = flux! { count((a != NO) and (b == UNKNOWN)) }.value;
        let or2 = flux! { count((a == NO) or (b != UNKNOWN)) }.value;
        assert_eq!(
            and,
            pairs.clone().filter(|(&x, &y)| scalar::cmp_eq(x, Trit::Yes) && scalar::cmp_eq(y, Trit::Yes)).count()
        );
        assert_eq!(
            or,
            pairs.clone().filter(|(&x, &y)| scalar::cmp_eq(x, Trit::Yes) || scalar::cmp_eq(y, Trit::Yes)).count()
        );
        assert_eq!(
            xor,
            pairs.clone().filter(|(&x, &y)| scalar::cmp_eq(x, Trit::Yes) != scalar::cmp_eq(y, Trit::Yes)).count()
        );
        assert_eq!(not, a_.iter().filter(|&&x| !scalar::cmp_eq(x, Trit::Yes)).count());
        assert_eq!(
            and2,
            pairs.clone().filter(|(&x, &y)| scalar::cmp_ne(x, Trit::No) && scalar::cmp_eq(y, Trit::Unknown)).count()
        );
        assert_eq!(
            or2,
            pairs.clone().filter(|(&x, &y)| scalar::cmp_eq(x, Trit::No) || scalar::cmp_ne(y, Trit::Unknown)).count()
        );
        cases += 6 * a_.len();
    };
    for &len in &LENS {
        run(&va(len), &vb(len));
    }
    for &x in &ALL {
        for &y in &ALL {
            run(&vec![x; 70], &vec![y; 70]); // all 9 domain points, pure, across a word boundary
        }
    }
    eprintln!(
        "proof_mask_boolean_ops_exhaustive: and/or/xor/not over masks, 9-pair domain, {} row-level checks",
        cases
    );
}

/// Counts alone can hide a positional bug (a shifted mask can keep its popcount). Single-hot
/// vectors make position observable through the count.
#[test]
fn proof_mask_ops_are_positional_not_just_counted() {
    let len = 130;
    for &pos in &[0usize, 1, 63, 64, 65, 127, 129] {
        for &pos2 in &[0usize, 64, 129] {
            let mut a_ = vec![Trit::No; len];
            a_[pos] = Trit::Yes;
            let mut b_ = vec![Trit::No; len];
            b_[pos2] = Trit::Yes;
            let a = planes_from(&a_);
            let b = planes_from(&b_);
            let got = flux! { count((a == YES) and (b == YES)) }.value;
            assert_eq!(
                got,
                if pos == pos2 { 1 } else { 0 },
                "single-hot AND at {} vs {}",
                pos,
                pos2
            );
        }
    }
}

// ------------------------------------------------------------------------------------------
// Kleene ops over planes. Domain 9 (binary) / 3 (not). Checked PER ROW on the output plane,
// so position, value and canonical form are all observed.
// ------------------------------------------------------------------------------------------

#[test]
fn proof_kleene_plane_ops_exhaustive() {
    let mut cases = 0usize;
    let mut run = |a_: &[Trit], b_: &[Trit]| {
        let len = a_.len();
        let a = planes_from(a_);
        let b = planes_from(b_);
        let and = flux! { a and b };
        let or = flux! { a or b };
        let xor = flux! { a xor b };
        let not = flux! { not a };
        for i in 0..len {
            assert_eq!(and.value.get(i), scalar::k_and(a_[i], b_[i]), "KLEENE_AND row {}", i);
            assert_eq!(or.value.get(i), scalar::k_or(a_[i], b_[i]), "KLEENE_OR row {}", i);
            assert_eq!(xor.value.get(i), scalar::k_xor(a_[i], b_[i]), "KLEENE_XOR row {}", i);
            assert_eq!(not.value.get(i), scalar::k_not(a_[i]), "KLEENE_NOT row {}", i);
        }
        for (out, what) in [(&and, "and"), (&or, "or"), (&xor, "xor"), (&not, "not")] {
            canonical_check(&out.value, len, what);
        }
        cases += 4 * len;
    };
    for &len in &LENS {
        run(&va(len), &vb(len));
    }
    for &x in &ALL {
        for &y in &ALL {
            run(&vec![x; 70], &vec![y; 70]);
        }
    }
    eprintln!(
        "proof_kleene_plane_ops_exhaustive: and/or/xor/not over planes, 9-pair domain, {} per-row checks",
        cases
    );
}

// ------------------------------------------------------------------------------------------
// MASK: (plane, mask) -> plane. Domain 3 x 3 through a comparison door (both bit values hit).
// ------------------------------------------------------------------------------------------

#[test]
fn proof_mask_operation_exhaustive() {
    let mut cases = 0usize;
    let mut run = |a_: &[Trit], b_: &[Trit]| {
        let len = a_.len();
        let a = planes_from(a_);
        let b = planes_from(b_);
        let m1 = flux! { mask(a, b == YES) };
        let m2 = flux! { mask(a, b != UNKNOWN) };
        for i in 0..len {
            assert_eq!(
                m1.value.get(i),
                scalar::mask_op(a_[i], scalar::cmp_eq(b_[i], Trit::Yes)),
                "MASK(== YES) row {}",
                i
            );
            assert_eq!(
                m2.value.get(i),
                scalar::mask_op(a_[i], scalar::cmp_ne(b_[i], Trit::Unknown)),
                "MASK(!= UNKNOWN) row {}",
                i
            );
        }
        canonical_check(&m1.value, len, "mask1");
        canonical_check(&m2.value, len, "mask2");
        cases += 2 * len;
    };
    for &len in &LENS {
        run(&va(len), &vb(len));
    }
    for &x in &ALL {
        for &y in &ALL {
            run(&vec![x; 70], &vec![y; 70]);
        }
    }
    eprintln!(
        "proof_mask_operation_exhaustive: mask() over 9-pair domain, {} per-row checks",
        cases
    );
}

// ------------------------------------------------------------------------------------------
// SELECT: (mask, plane, plane) -> plane. Domain 3^3 = 27 (cycling period 27 + pure triples).
// ------------------------------------------------------------------------------------------

#[test]
fn proof_select_exhaustive() {
    let mut cases = 0usize;
    let mut run = |m_: &[Trit], a_: &[Trit], b_: &[Trit]| {
        let len = m_.len();
        let m = planes_from(m_);
        let a = planes_from(a_);
        let b = planes_from(b_);
        let s = flux! { select(m == YES, a, b) };
        for i in 0..len {
            assert_eq!(
                s.value.get(i),
                scalar::select_op(scalar::cmp_eq(m_[i], Trit::Yes), a_[i], b_[i]),
                "SELECT row {}",
                i
            );
        }
        canonical_check(&s.value, len, "select");
        cases += len;
    };
    for &len in &LENS {
        run(&va(len), &vb(len), &vc(len)); // all 27 triples once len >= 27
    }
    for &x in &ALL {
        for &y in &ALL {
            for &z in &ALL {
                run(&vec![x; 70], &vec![y; 70], &vec![z; 70]); // all 27, pure
            }
        }
    }
    eprintln!("proof_select_exhaustive: select() over 27-triple domain, {} per-row checks", cases);
}

// ------------------------------------------------------------------------------------------
// Tails, zero length, corrupt padding, and equivalence with the reference hot loop.
// ------------------------------------------------------------------------------------------

#[test]
fn proof_zero_length_and_corrupt_padding() {
    // Length 0: no words, count 0, empty plane out.
    let a = Planes::with_len(0);
    assert_eq!(flux! { count(a == YES) }.value, 0);
    assert_eq!(flux! { not a }.value.len(), 0);

    // Corrupt the padding of the last word directly (the tail test from planes.rs, aimed at the
    // compiler): 65 rows, padding forced to garbage known=value=1.
    let mut a = planes_from(&vec![Trit::Yes; 65]);
    a.known[1] |= !0u64 << 1;
    a.value[1] |= !0u64 << 1;
    assert_eq!(
        flux! { count(a == YES) }.value,
        65,
        "TAILMASK must bound the count at len even with corrupt padding"
    );
    // != YES turns padding zeros into candidate hits in the mask; only the tail mask stops them.
    assert_eq!(
        flux! { count(a != YES) }.value,
        0,
        "a lowering that forgets the tail mask fails HERE"
    );
    // A plane result computed from a corrupt input must still come out canonical.
    let n = flux! { not a };
    canonical_check(&n.value, 65, "not-over-corrupt-padding");
}

#[test]
fn proof_flux_count_equals_reference_hot_loop_and_scalar() {
    // The compiled path against planes::ran_and_not_ok (the reference loop shape) AND against
    // the scalar row-at-a-time definition, on deterministic pseudo-random data.
    let mut state = 0x243F_6A88_85A3_08D3u64;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    for &len in &[0usize, 1, 63, 64, 65, 1000, 41283] {
        let mut ran = Planes::with_len(len);
        let mut verdict = Planes::with_len(len);
        let mut vr = Vec::with_capacity(len);
        let mut vv = Vec::with_capacity(len);
        for i in 0..len {
            let r = ALL[next() % 3];
            let v = ALL[next() % 3];
            ran.set(i, r);
            verdict.set(i, v);
            vr.push(r);
            vv.push(v);
        }
        let f = flux! { count(ran == YES and verdict != YES) };
        let hot = moth_flux::planes::ran_and_not_ok(&ran, &verdict, len);
        let scalar_ref = vr
            .iter()
            .zip(vv.iter())
            .filter(|(r, v)| **r == Trit::Yes && **v != Trit::Yes)
            .count();
        assert_eq!(f.value, hot, "flux vs ran_and_not_ok at len {}", len);
        assert_eq!(f.value, scalar_ref, "flux vs scalar at len {}", len);
    }
}

// A composite expression using most of the language at once, proven per-row against the scalar
// reference composed the same way - operations must agree not only alone but in combination.
#[test]
fn proof_composite_expression() {
    for &len in &LENS {
        let a_ = va(len);
        let b_ = vb(len);
        let c_ = vc(len);
        let a = planes_from(&a_);
        let b = planes_from(&b_);
        let c = planes_from(&c_);
        let out = flux! { select((a == YES) or (b == NO), mask(c, a != UNKNOWN), not c) };
        for i in 0..len {
            let m = scalar::cmp_eq(a_[i], Trit::Yes) || scalar::cmp_eq(b_[i], Trit::No);
            let lhs = scalar::mask_op(c_[i], scalar::cmp_ne(a_[i], Trit::Unknown));
            let rhs = scalar::k_not(c_[i]);
            assert_eq!(out.value.get(i), scalar::select_op(m, lhs, rhs), "composite row {}", i);
        }
        canonical_check(&out.value, len, "composite");
    }
}
