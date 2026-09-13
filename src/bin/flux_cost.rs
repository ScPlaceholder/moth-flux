//! Does the `flux!` boundary COST anything? Compiled macro vs the hand-written hot loop.
//!
//! Fable's own not-covered list said it plainly: "Performance is shape-equivalent by construction,
//! NOT benchmarked. I ran no timing and inspected no optimized assembly." That is the one claim in
//! the whole delivery resting on argument rather than measurement, so it is the one worth measuring
//! — and it is deliberately measured by someone other than the implementer, because a benchmark
//! written by the author of the code under test is grading its own homework.
//!
//! THE QUESTION, narrowly: does
//!     flux! { count(ran == YES and verdict != YES) }
//! run at the same speed as the hand-written
//!     planes::ran_and_not_ok(&ran, &verdict, len)
//! If yes, the semantic boundary is free and the restriction costs nothing at runtime. If no, the
//! gap is the price of the guarantee and we should know its size before building anything on it.
//!
//! ⚠ METHOD, copied from bin/adversarial.rs where it was earned:
//!   * ARMS INTERLEAVED per rep, never run as two sequential loops. Sequential arms drift with
//!     turbo/thermal state and produced a 10.8% CV that looked like a real difference.
//!   * MEDIAN OF PER-REP RATIOS, not a ratio of medians. One slow rep then perturbs one ratio
//!     instead of the whole summary.
//!   * `black_box` on BOTH arms' inputs and outputs, uniformly. Applied to one arm only, it
//!     measures the barrier rather than the code.
//!   * EQUALITY ASSERTED EVERY REP. A benchmark whose arms disagree is timing two different
//!     programs; that assertion is what caught the NoMatch encoding bug in this crate.
use moth_flux::event::Outcome;
use moth_flux::flux::flux;
use moth_flux::planes::ran_and_not_ok;
use moth_flux::Planes;
use std::hint::black_box;
use std::time::Instant;

fn build(len: usize) -> (Planes, Planes) {
    // The real .moth_events.jsonl distribution from event.rs, tiled to `len`.
    let dist = [
        (Outcome::NothingRan, 34_197usize),
        (Outcome::Ok, 6_743),
        (Outcome::Fail, 74),
        (Outcome::CannotTell, 250),
        (Outcome::NoMatch, 19),
    ];
    let cycle: usize = dist.iter().map(|(_, n)| n).sum();
    let mut ran = Planes::with_len(len);
    let mut verdict = Planes::with_len(len);
    let mut i = 0usize;
    while i < len {
        for &(o, n) in &dist {
            for _ in 0..n {
                if i >= len {
                    break;
                }
                let (r, v, _m) = o.to_trits();
                ran.set(i, r);
                verdict.set(i, v);
                i += 1;
            }
        }
        if cycle == 0 {
            break;
        }
    }
    (ran, verdict)
}

// ★★★ THE SYMMETRY CONTROLS, added 2026-09-13 after the bounds-check hypothesis was refuted.
//   The first version timed the macro EXPANDED INLINE between two black_box barriers against
//   `ran_and_not_ok`, a standalone function LLVM optimises whole. Those are not the same context,
//   so the 2.5x could be the lowering OR could be my harness. Two conditions settle it:
//     inline(never) on BOTH  -> both are opaque calls, contexts equalised
//     inline(always) on BOTH -> both are pasted into the caller, contexts equalised the other way
//   Gap persists in both  -> the generated code is responsible.
//   Gap collapses         -> it was a benchmark artefact, which is worth just as much to know.
#[inline(never)]
fn hand_never(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    ran_and_not_ok(ran, verdict, len)
}

#[inline(never)]
fn flux_never(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

#[inline(always)]
fn hand_always(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    ran_and_not_ok(ran, verdict, len)
}

#[inline(always)]
fn flux_always(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

fn pair(
    label: &str,
    len: usize,
    reps: usize,
    ran: &Planes,
    verdict: &Planes,
    f_macro: fn(&Planes, &Planes) -> usize,
    f_hand: fn(&Planes, &Planes, usize) -> usize,
) {
    let mut ratios: Vec<f64> = Vec::with_capacity(reps);
    let mut m_us: Vec<f64> = Vec::with_capacity(reps);
    let mut h_us: Vec<f64> = Vec::with_capacity(reps);
    let mut seen = 0usize;
    for _ in 0..reps {
        let a = black_box(ran);
        let b = black_box(verdict);
        let n = black_box(len);

        let t0 = Instant::now();
        let rm = f_macro(a, b);
        let dm = t0.elapsed().as_secs_f64() * 1e6;
        black_box(rm);

        let t1 = Instant::now();
        let rh = f_hand(a, b, n);
        let dh = t1.elapsed().as_secs_f64() * 1e6;
        black_box(rh);

        assert_eq!(rm, rh, "{} arms disagree at len={}", label, len);
        seen = rh;
        m_us.push(dm);
        h_us.push(dh);
        ratios.push(dm / dh.max(1e-9));
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|x, y| x.partial_cmp(y).unwrap());
        v[v.len() / 2]
    };
    println!(
        "{:>14}  {:>12}  {:>12.1}  {:>12.1}  {:>10.3}  {:>8}",
        label,
        len,
        med(&mut m_us),
        med(&mut h_us),
        med(&mut ratios),
        seen
    );
}

fn main() {
    // 41,980 is the live corpus. The larger sizes exist only to show the ratio is not an artefact
    // of everything fitting in L1 — they are NOT a claim about a workload we have.
    let sizes = [41_980usize, 1_000_000, 10_000_000];
    let reps = 25;

    println!("flux! boundary cost — compiled macro vs hand-written ran_and_not_ok");
    println!("interleaved arms, {} reps, median of per-rep ratios\n", reps);
    println!("{:>12}  {:>12}  {:>12}  {:>10}  {:>8}", "rows", "macro us", "hand us", "macro/hand", "count");

    for &len in &sizes {
        let (ran, verdict) = build(len);
        let mut ratios: Vec<f64> = Vec::with_capacity(reps);
        let mut m_us: Vec<f64> = Vec::with_capacity(reps);
        let mut h_us: Vec<f64> = Vec::with_capacity(reps);
        let mut count_seen = 0usize;

        for _ in 0..reps {
            // ⛔ SYMMETRY. The first version bound `black_box(&ran)` OUTSIDE the macro's timer and
            //   called it INSIDE the hand arm's, so the hand arm paid an extra barrier the macro
            //   did not. That flattered the macro and it STILL lost — so the finding survived the
            //   flaw — but a benchmark whose arms differ in two ways cannot attribute anything, and
            //   I have spent this whole night on exactly that error. Both arms now take their
            //   barriers in the same place, outside the clock.
            let a = black_box(&ran);
            let b = black_box(&verdict);
            let n = black_box(len);

            let t0 = Instant::now();
            let r_macro = flux! { count(a == YES and b != YES) };
            let dm = t0.elapsed().as_secs_f64() * 1e6;
            black_box(r_macro.value);

            let t1 = Instant::now();
            let r_hand = ran_and_not_ok(a, b, n);
            let dh = t1.elapsed().as_secs_f64() * 1e6;
            black_box(r_hand);

            // ⛔ If these ever disagree the timing is meaningless — two different programs.
            assert_eq!(r_macro.value, r_hand, "macro and hand loop disagree at len={}", len);
            count_seen = r_hand;

            m_us.push(dm);
            h_us.push(dh);
            ratios.push(dm / dh.max(1e-9));
        }

        let med = |v: &mut Vec<f64>| {
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            v[v.len() / 2]
        };
        println!(
            "{:>12}  {:>12.1}  {:>12.1}  {:>10.3}  {:>8}",
            len,
            med(&mut m_us),
            med(&mut h_us),
            med(&mut ratios),
            count_seen
        );
    }

    // ── THE SYMMETRY CONTROLS ────────────────────────────────────────────────────────────────
    println!();
    println!("SYMMETRY CONTROLS — same harness, contexts equalised both directions");
    println!(
        "{:>14}  {:>12}  {:>12}  {:>12}  {:>10}  {:>8}",
        "condition", "rows", "macro us", "hand us", "macro/hand", "count"
    );
    for &len in &sizes {
        let (ran, verdict) = build(len);
        pair("inline(never)", len, reps, &ran, &verdict, flux_never, hand_never);
        pair("inline(always)", len, reps, &ran, &verdict, flux_always, hand_always);
    }
    println!();
    println!("IF the gap persists under BOTH conditions, the generated code owns it.");
    println!("IF it collapses when the contexts match, the earlier 2.5x was my harness, not FLUX.");

    println!();
    println!("READ IT LIKE THIS: a ratio near 1.00 means the semantic boundary is FREE — the macro");
    println!("emits the same work as the hand-written loop, so the restriction costs nothing at");
    println!("runtime. Above ~1.15 the guarantee has a measurable price and we should know it.");
    println!("⚠ This measures ONE expression. It is not a claim about the compiler in general.");
    println!("== fluxcost COMPLETE (rc=0) ==");
}
