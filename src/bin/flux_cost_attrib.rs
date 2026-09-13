//! SCRATCH (independent review). THE ATTRIBUTION TEST.
//!
//! The re-run shows the macro 1.7-2.4x slower than `planes::ran_and_not_ok` whenever both are
//! inlined into the caller. Before that is charged to the FLUX lowering, one alternative has to
//! be killed: `ran_and_not_ok` lives in ANOTHER CRATE and arrives through LTO, while a `flux!`
//! expansion is always textually in the consumer crate. If a hand-written LOCAL copy of the exact
//! same loop is also slow, the gap belongs to the compilation path, not to FLUX.
//!
//! Third arm = `local_hand`: character-for-character `ran_and_not_ok`'s body, defined here.
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
                if i >= len { break 'outer; }
                let (r, v, _m) = o.to_trits();
                ran.set(i, r);
                verdict.set(i, v);
                i += 1;
            }
        }
    }
    (ran, verdict)
}

/// Byte-for-byte the body of `moth_flux::planes::ran_and_not_ok`, but LOCAL to this crate.
#[inline(always)]
fn local_hand(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    let mut n = 0u32;
    let words = (len + 63) / 64;
    for w in 0..words {
        let ran_yes = ran.known[w] & ran.value[w];
        let verdict_yes = verdict.known[w] & verdict.value[w];
        let mut hit = ran_yes & !verdict_yes;
        if w == words - 1 && len % 64 != 0 {
            hit &= (1u64 << (len % 64)) - 1;
        }
        n += hit.count_ones();
    }
    n as usize
}

fn med(v: &mut Vec<f64>) -> f64 { v.sort_by(|a,b| a.partial_cmp(b).unwrap()); v[v.len()/2] }

fn main() {
    let sizes = [41_980usize, 1_000_000, 10_000_000];
    let reps = 51;
    println!("ATTRIBUTION — all three arms inlined DIRECTLY into the timing loop, rotating order");
    println!("{:>10}  {:>12}  {:>12}  {:>12}  {:>12}  {:>12}  {:>8}",
             "rows", "libhand us", "localhand us", "flux us", "flux/libhand", "flux/local", "count");
    for &len in &sizes {
        let (ran, verdict) = build(len);
        let inner: usize = if len <= 100_000 { 256 } else if len <= 2_000_000 { 16 } else { 2 };
        let f = inner as f64;
        let (mut lh, mut loh, mut fx) = (vec![], vec![], vec![]);
        let (mut r1, mut r2) = (vec![], vec![]);
        let mut seen = 0usize;
        for rep in 0..reps {
            let mut t_lib = 0.0; let mut t_loc = 0.0; let mut t_flx = 0.0;
            let (mut a1, mut a2, mut a3) = (0usize, 0usize, 0usize);
            // rotate which arm goes first so no arm is permanently cache-advantaged
            for slot in 0..3 {
                match (slot + rep) % 3 {
                    0 => { let t = Instant::now();
                           for _ in 0..inner { let a=black_box(&ran); let b=black_box(&verdict); let n=black_box(len);
                                               a1 = black_box(ran_and_not_ok(a,b,n)); }
                           t_lib = t.elapsed().as_secs_f64()*1e6/f; }
                    1 => { let t = Instant::now();
                           for _ in 0..inner { let a=black_box(&ran); let b=black_box(&verdict); let n=black_box(len);
                                               a2 = black_box(local_hand(a,b,n)); }
                           t_loc = t.elapsed().as_secs_f64()*1e6/f; }
                    _ => { let t = Instant::now();
                           for _ in 0..inner { let a=black_box(&ran); let b=black_box(&verdict);
                                               a3 = black_box((flux!{ count(a == YES and b != YES) }).value); }
                           t_flx = t.elapsed().as_secs_f64()*1e6/f; }
                }
            }
            assert_eq!(a1, a2, "lib vs local disagree");
            assert_eq!(a1, a3, "lib vs flux disagree");
            seen = a1;
            lh.push(t_lib); loh.push(t_loc); fx.push(t_flx);
            r1.push(t_flx/t_lib.max(1e-9)); r2.push(t_flx/t_loc.max(1e-9));
        }
        println!("{:>10}  {:>12.2}  {:>12.2}  {:>12.2}  {:>12.3}  {:>12.3}  {:>8}",
                 len, med(&mut lh), med(&mut loh), med(&mut fx), med(&mut r1), med(&mut r2), seen);
    }
    println!("== attribution COMPLETE (rc=0) ==");
}
