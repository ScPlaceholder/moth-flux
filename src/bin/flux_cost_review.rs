//! SCRATCH (independent review, 2026-09-13). Re-runs the flux!-vs-hand cost question WITHOUT
//! function pointers, so `#[inline(never)]` / `#[inline(always)]` actually take effect.
//!
//! What is different from `flux_cost.rs`, and why each change was needed:
//!
//!  1. NO FUNCTION POINTERS. `flux_cost::pair()` takes both arms as `fn(...)`, which forces an
//!     opaque indirect call on both sides. `#[inline(always)]` cannot apply through one, so the
//!     two "independent conditions" were one condition run twice. Here each condition is a
//!     separate hand-written loop calling the arms BY NAME.
//!
//!  2. ARM ORDER ALTERNATES. `flux_cost.rs` always times the macro FIRST and the hand loop
//!     SECOND, over the same 4 planes. At 10M rows that is 5 MB: the first arm pays the cache
//!     misses and the second reads warm. A fixed order bakes that in. Here even reps run
//!     macro-then-hand and odd reps run hand-then-macro, and the medians are taken per order as
//!     well as pooled, so the size of the order effect is visible instead of assumed.
//!
//!  3. INNER REPETITION. At 41,980 rows one pass is ~1-2 us and `Instant` on Windows is QPC with
//!     ~100 ns granularity, so a single-pass timing carries several percent of quantisation.
//!     Each timed region now runs `inner` passes so it lasts ~1 ms.
//!
//!  4. A THIRD CONDITION: DIRECT. Both arms written straight into the timing loop with no
//!     wrapper at all - what a caller who just writes the code actually gets.
use moth_flux::event::Outcome;
use moth_flux::flux::flux;
use moth_flux::planes::ran_and_not_ok;
use moth_flux::Planes;
use std::hint::black_box;
use std::time::Instant;

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
    'outer: while i < len {
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

struct Acc {
    m: Vec<f64>,
    h: Vec<f64>,
    r_mh: Vec<f64>, // ratios from reps where the MACRO ran first
    r_hm: Vec<f64>, // ratios from reps where the HAND ran first
    seen: usize,
}

impl Acc {
    fn new() -> Acc {
        Acc { m: vec![], h: vec![], r_mh: vec![], r_hm: vec![], seen: 0 }
    }
    fn push(&mut self, dm: f64, dh: f64, rm: usize, rh: usize, macro_first: bool, label: &str) {
        assert_eq!(rm, rh, "{}: arms disagree", label);
        self.seen = rh;
        self.m.push(dm);
        self.h.push(dh);
        if macro_first {
            self.r_mh.push(dm / dh.max(1e-9));
        } else {
            self.r_hm.push(dm / dh.max(1e-9));
        }
    }
    fn report(&mut self, label: &str, len: usize) {
        let med = |v: &mut Vec<f64>| -> f64 {
            if v.is_empty() {
                return f64::NAN;
            }
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            v[v.len() / 2]
        };
        let mut pooled: Vec<f64> = self.r_mh.iter().chain(self.r_hm.iter()).cloned().collect();
        println!(
            "{:>16}  {:>10}  {:>10.2}  {:>10.2}  {:>9.3}  {:>9.3}  {:>9.3}  {:>9}",
            label,
            len,
            med(&mut self.m),
            med(&mut self.h),
            med(&mut pooled),
            med(&mut self.r_mh),
            med(&mut self.r_hm),
            self.seen
        );
    }
}

fn main() {
    let sizes = [41_980usize, 1_000_000, 10_000_000];
    let reps = 51;

    println!("INDEPENDENT RE-RUN — no function pointers, alternating arm order, inner repetition");
    println!("median over {} reps; ratio >1 means the MACRO is slower\n", reps);
    println!(
        "{:>16}  {:>10}  {:>10}  {:>10}  {:>9}  {:>9}  {:>9}  {:>9}",
        "condition", "rows", "macro us", "hand us", "ratio", "r(m1st)", "r(h1st)", "count"
    );

    for &len in &sizes {
        let (ran, verdict) = build(len);
        let inner: usize = match len {
            n if n <= 100_000 => 256,
            n if n <= 2_000_000 => 16,
            _ => 2,
        };
        let f = inner as f64;

        // ── DIRECT: no wrappers at all ──────────────────────────────────────────────────────
        let mut acc = Acc::new();
        for rep in 0..reps {
            let macro_first = rep % 2 == 0;
            let (mut dm, mut dh) = (0.0, 0.0);
            let (mut rm, mut rh) = (0usize, 0usize);
            for phase in 0..2 {
                if (phase == 0) == macro_first {
                    let t = Instant::now();
                    for _ in 0..inner {
                        let a = black_box(&ran);
                        let b = black_box(&verdict);
                        rm = black_box((flux! { count(a == YES and b != YES) }).value);
                    }
                    dm = t.elapsed().as_secs_f64() * 1e6 / f;
                } else {
                    let t = Instant::now();
                    for _ in 0..inner {
                        let a = black_box(&ran);
                        let b = black_box(&verdict);
                        let n = black_box(len);
                        rh = black_box(ran_and_not_ok(a, b, n));
                    }
                    dh = t.elapsed().as_secs_f64() * 1e6 / f;
                }
            }
            acc.push(dm, dh, rm, rh, macro_first, "direct");
        }
        acc.report("direct", len);

        // ── inline(never) wrappers, called BY NAME ──────────────────────────────────────────
        let mut acc = Acc::new();
        for rep in 0..reps {
            let macro_first = rep % 2 == 0;
            let (mut dm, mut dh) = (0.0, 0.0);
            let (mut rm, mut rh) = (0usize, 0usize);
            for phase in 0..2 {
                if (phase == 0) == macro_first {
                    let t = Instant::now();
                    for _ in 0..inner {
                        let a = black_box(&ran);
                        let b = black_box(&verdict);
                        rm = black_box(flux_never(a, b));
                    }
                    dm = t.elapsed().as_secs_f64() * 1e6 / f;
                } else {
                    let t = Instant::now();
                    for _ in 0..inner {
                        let a = black_box(&ran);
                        let b = black_box(&verdict);
                        let n = black_box(len);
                        rh = black_box(hand_never(a, b, n));
                    }
                    dh = t.elapsed().as_secs_f64() * 1e6 / f;
                }
            }
            acc.push(dm, dh, rm, rh, macro_first, "inline(never)");
        }
        acc.report("inline(never)", len);

        // ── inline(always) wrappers, called BY NAME ─────────────────────────────────────────
        let mut acc = Acc::new();
        for rep in 0..reps {
            let macro_first = rep % 2 == 0;
            let (mut dm, mut dh) = (0.0, 0.0);
            let (mut rm, mut rh) = (0usize, 0usize);
            for phase in 0..2 {
                if (phase == 0) == macro_first {
                    let t = Instant::now();
                    for _ in 0..inner {
                        let a = black_box(&ran);
                        let b = black_box(&verdict);
                        rm = black_box(flux_always(a, b));
                    }
                    dm = t.elapsed().as_secs_f64() * 1e6 / f;
                } else {
                    let t = Instant::now();
                    for _ in 0..inner {
                        let a = black_box(&ran);
                        let b = black_box(&verdict);
                        let n = black_box(len);
                        rh = black_box(hand_always(a, b, n));
                    }
                    dh = t.elapsed().as_secs_f64() * 1e6 / f;
                }
            }
            acc.push(dm, dh, rm, rh, macro_first, "inline(always)");
        }
        acc.report("inline(always)", len);
        println!();
    }
    println!("== review bench COMPLETE (rc=0) ==");
}
