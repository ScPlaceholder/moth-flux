//! WHY IS THE MACRO SLOW? Five arms, one variable at a time. ANSWERED — see the results below.
//!
//! The reviewer established flux/localhand ≈ 1.00, flux/libhand ≈ 2.34, and concluded the cause was
//! compilation CONTEXT. But every variant it timed lived in the consumer crate AND was inlined, so
//! LOCATION and the FUNCTION BOUNDARY were confounded with each other and with SHAPE.
//!
//!   A  lib_vec       planes::ran_and_not_ok                library  + vectorising shape + fn
//!   B  lib_flux      kernels::count_and_not_yes_fluxshape  library  + FLUX shape + fn
//!   C  local_flux    the same body written here            consumer + FLUX shape + fn
//!   D  macro         flux! { ... } expanded inline         consumer + FLUX shape + INLINE
//!   E  macro_wrapped the same flux! inside a plain fn      consumer + FLUX shape + fn
//!
//! ⛔ MEASURED 2026-09-13, and both of my earlier hypotheses are dead:
//!        rows        A       B       C       D        E      D/A     E/A
//!      41,980     0.60    0.60    0.50    1.10     0.60    1.833   1.000
//!   1,000,000     9.50    8.80    8.80   26.60     8.70    2.800   0.916
//!  10,000,000    97.40   91.20   91.20  276.20    91.30    2.836   0.937
//!
//! NOT location — B and C are equal, and both slightly beat A.
//! NOT the loop shape — B, C and E all use the FLUX shape and all are fast.
//! It is the FUNCTION BOUNDARY. D and E are the SAME generated expansion; D is pasted into the
//! caller's loop body and E sits in a plain `fn`. 2.8x apart.
//!
//! ★ SO THE COMPILER FIX IS: emit a private fn and call it, instead of pasting the body. E proves
//!   that works WITHOUT touching flux_macro — the generated code was never the problem.
//! ⚠ AND IT MUST BE A PLAIN `fn`. The reviewer's `#[inline(never)]` wrappers left flux at 19.86 vs
//!   lib 10.13 — still slow. Forcing never-inline is as bad as pasting. The precise claim is:
//!   **a boundary LLVM is ALLOWED TO INLINE AS A UNIT.** Not "a call", not "the library".
//!
//! ⚠ UNEXPLAINED, flagged not theorised: B, C and E all beat A by 4-8%. The FLUX shape is slightly
//!   faster than `ran_and_not_ok` whenever both are behind a boundary. Small, consistent across
//!   three sizes, unchased — I have burned three theories on this question in one day.
//!
//! ⚠ Method as before: interleaved arms, median of per-rep ratios, black_box outside the clock on
//!   every arm equally, equality asserted every rep. And the arms are called through the SAME
//!   shape — a plain direct call — because the last time I got this wrong it was because one arm
//!   was reachable for optimisation and another was not.
use moth_flux::event::Outcome;
use moth_flux::flux::flux;
use moth_flux::kernels::count_and_not_yes_fluxshape;
use moth_flux::planes::ran_and_not_ok;
use moth_flux::Planes;
use std::hint::black_box;
use std::time::Instant;

/// Arm C: byte-for-byte the body of the library kernel, but compiled HERE.
fn local_flux(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    let words = (len + 63) / 64;
    let mut count: usize = 0;
    for w in 0..words {
        let ik0 = ran.known[w];
        let iv0 = ran.value[w];
        let ik1 = verdict.known[w];
        let iv1 = verdict.value[w];
        let m0 = ik0 & iv0;
        let m1 = !(ik1 & iv1);
        let mut hit = m0 & m1;
        if w + 1 == words && (len % 64) != 0 {
            hit &= (1u64 << (len % 64)) - 1;
        }
        count += hit.count_ones() as usize;
    }
    count
}

/// Arm E: the MACRO, wrapped in a plain `fn`. This is the whole scheduled test — if a function
/// boundary is what the macro is missing, E should land on C, not on D, and the compiler change
/// (emit a private fn, call it) is proven without touching flux_macro at all.
///
/// ⚠ PLAIN `fn`, NOT `#[inline(never)]`, and that distinction turns out to matter. The independent
///   reviewer wrapped both arms in `#[inline(never)]` and got flux 19.86 vs lib 10.13 — still slow.
///   Arm C here is a plain `fn` and is fast. So the hypothesis is narrower than "a function
///   boundary helps": it is "a boundary LLVM may inline AS A UNIT helps". Forcing never-inline is
///   as bad as pasting the body in, which is exactly why the reviewer's control could not see this.
fn macro_wrapped(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

fn build(len: usize) -> (Planes, Planes) {
    let dist = [
        (Outcome::NothingRan, 34_197usize),
        (Outcome::Ok, 6_743),
        (Outcome::Fail, 74),
        (Outcome::CannotTell, 250),
        (Outcome::NoMatch, 19),
    ];
    let mut ran = Planes::with_len(len);
    let mut verdict = Planes::with_len(len);
    let mut i = 0usize;
    'outer: loop {
        for &(o, n) in &dist {
            for _ in 0..n {
                if i >= len {
                    break 'outer;
                }
                let (r, v, _m) = o.to_trits();
                ran.set(i, r);
                verdict.set(i, v);
                i += 1;
            }
        }
    }
    (ran, verdict)
}

fn med(v: &mut Vec<f64>) -> f64 {
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    v[v.len() / 2]
}

fn main() {
    let sizes = [41_980usize, 1_000_000, 10_000_000];
    let reps = 31;

    println!("LOCATION vs SHAPE — B is the experiment; A is the target it must reach");
    println!(
        "{:>10}  {:>9}  {:>9}  {:>10}  {:>9}  {:>10}  {:>7}  {:>7}  {:>7}",
        "rows", "A libvec", "B libflux", "C localflx", "D macro", "E wrapped", "D/A", "E/A", "count"
    );

    for &len in &sizes {
        let (ran, verdict) = build(len);
        let (mut a, mut b, mut c, mut d, mut e) = (vec![], vec![], vec![], vec![], vec![]);
        let mut seen = 0usize;
        for _ in 0..reps {
            let (pr, pv, n) = (black_box(&ran), black_box(&verdict), black_box(len));

            let t = Instant::now();
            let ra = ran_and_not_ok(pr, pv, n);
            a.push(t.elapsed().as_secs_f64() * 1e6);
            black_box(ra);

            let t = Instant::now();
            let rb = count_and_not_yes_fluxshape(pr, pv, n);
            b.push(t.elapsed().as_secs_f64() * 1e6);
            black_box(rb);

            let t = Instant::now();
            let rc = local_flux(pr, pv, n);
            c.push(t.elapsed().as_secs_f64() * 1e6);
            black_box(rc);

            let t = Instant::now();
            let rd = (flux! { count(pr == YES and pv != YES) }).value;
            d.push(t.elapsed().as_secs_f64() * 1e6);
            black_box(rd);

            assert_eq!(ra, rb, "A/B disagree");
            assert_eq!(ra, rc, "A/C disagree");
            let t = Instant::now();
            let re = macro_wrapped(pr, pv);
            e.push(t.elapsed().as_secs_f64() * 1e6);
            black_box(re);

            assert_eq!(ra, rd, "A/D disagree");
            assert_eq!(ra, re, "A/E disagree");
            seen = ra;
        }
        let (ma, mb, mc, md, me) = (med(&mut a), med(&mut b), med(&mut c), med(&mut d), med(&mut e));
        println!(
            "{:>10}  {:>9.2}  {:>9.2}  {:>10.2}  {:>9.2}  {:>10.2}  {:>7.3}  {:>7.3}  {:>7}",
            len, ma, mb, mc, md, me, md / ma, me / ma, seen
        );
    }

    println!();
    println!("B/A near 1.00  -> LOCATION is the cause. The macro should emit a library CALL.");
    println!("B/A near C/A   -> SHAPE is the cause. A call fixes nothing; my prediction was wrong.");
    println!("== fluxlocate COMPLETE (rc=0) ==");
}
