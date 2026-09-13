//! The two workloads DESIGNED TO MAKE MOTH LOSE, plus the scale sweep on the real implementation.
//!
//! # ⛔ WHY THIS FILE EXISTS AND WHY IT IS LATE
//!
//! I proposed these two queries at 01:35, then ran the fun GPU test instead, then wrote a memory
//! about rigour. Two hours later the tests built to hurt were still unwritten. **I will run the
//! experiment that might make me look good and defer the one built to hurt, and I will do it while
//! composing a note about intellectual honesty.** Written down so the next version of me notices
//! the shape earlier than I did.
//!
//! # What was wrong with the GPU benchmark, and the correction
//!
//! `moth_flux_gpu.py` compared **torch-CPU against torch-GPU** and I nearly read that as a MOTH
//! result. torch-CPU takes 91 us for 41,980 rows; the Rust bit-plane scan takes 0.55 us — 165x
//! apart. So the correction is not a tweak: **run the RUST scan at every size and measure it**,
//! rather than scaling one 41,980-row reading linearly, which is what my "crossover near 50M"
//! estimate did. An extrapolation is not a measurement, especially across the L3 boundary where
//! the curve is guaranteed to bend.
//!
//! # ★ PREDICTIONS, WRITTEN BEFORE THE RUN
//!
//! 1. rkyv beats planes on POINT LOOKUP by 5-20x. One offset and one load, against two loads plus
//!    shifts per field with no locality between a record's own fields.
//! 2. rkyv beats planes on MULTI-FIELD READ by a similar margin. Row store wins by construction;
//!    columnar must touch N separate arrays for one record.
//!    ⛔ BUILT ON THE FOURTH ASKING, 01:52. I predicted this at 01:35 and deferred it three times —
//!    every deferral landed on the same test, the one where columnar is weakest BY CONSTRUCTION.
//!    Sharpened prediction now that the lookup data is in: a row store touches ONE cache line per
//!    record; the columnar side touches FOUR separate arrays. Out of cache that should be close to
//!    a 4x penalty, and the point-lookup result says the miss is what dominates, so I expect the
//!    multi-field gap to be WIDER than the single-field one (2.4x) rather than narrower.
//! 3. Planes' scan advantage GROWS with corpus size, because half the bytes matter more once the
//!    working set leaves cache.
//!
//! If 1 and 2 hold, MOTH FLUX is an analytics representation and not a general one — narrower than
//! the proposal claims, and defensible. If 3 fails, the whole memory-movement argument fails with it.
//!
//! ⚠ THE SYNTHETIC CORPUS KEEPS THE REAL DISTRIBUTION: 83% absent, ~16% ok, the rest split across
//! fail / cannot_tell / no_match. A uniform random fill would change the branch behaviour and I
//! would be measuring a different workload than our telemetry while calling it ours.

use moth_flux::event::Outcome;
use moth_flux::planes::Planes;
use moth_flux::Trit;
use std::hint::black_box;
use std::time::Instant;

const SIZES: [usize; 5] = [41_980, 500_000, 5_000_000, 50_000_000, 200_000_000];

/// Deterministic LCG — no Rand dependency, and reproducible across runs by construction.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
}

fn synth_outcome(r: u64) -> Outcome {
    // Real distribution from .moth_events.jsonl, per 10,000 rows.
    match r % 10_000 {
        0..=8_305 => Outcome::NothingRan,   // 83.05%
        8_306..=9_913 => Outcome::Ok,       // 16.08%
        9_914..=9_973 => Outcome::CannotTell,
        9_974..=9_991 => Outcome::Fail,
        _ => Outcome::NoMatch,
    }
}


/// ⛔⛔ THE FAIR BYTE BASELINE, BUILT AFTER FLAGGING IT AS "MEDIOCRE" THREE TIMES WITHOUT FIXING IT.
///
/// Every "scan win" number in this file rests on the byte arm, and that arm ran at 2.9 GB/s flat
/// across five orders of magnitude — which is compute-bound, not bandwidth-bound, i.e. it was
/// never even trying. I kept reporting that as a caveat. **A caveat repeated is a task avoided.**
///
/// ★ AND THE FIRST SUSPECT IS THE PREDICATE, NOT THE LAYOUT. The old arm asked `o != 0 && o != 1`
///   — two comparisons and a branch per byte. The encoding makes that exactly equivalent to
///   `o >= 2`, ONE comparison, which is the shape LLVM auto-vectorises. If that alone closes most
///   of the gap, then "columnar beats row" was partly "my predicate was badly written", and the
///   honest headline shrinks.
///
/// ⚠ No intrinsics and no unsafe. This is what a competent engineer writes without reaching for
///   platform-specific code, which is the right opponent — beating hand-written AVX would be a
///   different and much stronger claim than I am in a position to make.
#[inline(never)]
fn scan_bytes_naive(col: &[u8]) -> usize {
    col.iter().filter(|&&o| o != 0 && o != 1).count()
}

/// ⛔ HYPOTHESIS 2, AFTER HYPOTHESIS 1 FAILED. The predicate was NOT the cause — naive and fair
/// came within 1% of each other, so LLVM treated `o != 0 && o != 1` and `o >= 2` identically and
/// my "badly written predicate" theory was wrong.
///
/// Next suspect, and this one is testable rather than plausible: `n += ...` is a single
/// LOOP-CARRIED DEPENDENCY. Every iteration must wait for the previous add. Four independent
/// accumulators break the chain and let the pipeline run four adds in flight.
///
/// ⚠ I HAVE NOW GUESSED AT THIS MECHANISM TWICE WITHOUT LOOKING. First "it never vectorised",
/// then "the predicate". Both were narrated, neither was measured. This one is at least a
/// difference I can observe: if four accumulators do not move it, the cause is elsewhere again and
/// I should stop guessing and read the assembly.
#[inline(never)]
fn scan_bytes_4acc(col: &[u8]) -> usize {
    let (mut a, mut b, mut c, mut d) = (0usize, 0usize, 0usize, 0usize);
    let chunks = col.chunks_exact(4);
    let rem = chunks.remainder();
    for ch in chunks {
        a += (ch[0] >= 2) as usize;
        b += (ch[1] >= 2) as usize;
        c += (ch[2] >= 2) as usize;
        d += (ch[3] >= 2) as usize;
    }
    a + b + c + d + rem.iter().filter(|&&o| o >= 2).count()
}

/// ★★★ THE OPTIMISED CONVENTIONAL BASELINE — SWAR, 8 bytes per iteration, still no unsafe.
///
/// This exists to test a prediction I wrote down BEFORE running it: as the byte arm gets faster,
/// **the gap should converge toward ~4x — the density ratio, 2 bits against 8 — and stop there.**
///
///   converges to ~4x   -> density IS the mechanism. It was invisible until now only because the
///                         two arms were limited by DIFFERENT things: planes already at ~25 GB/s
///                         and memory-bound, bytes at 4.3 GB/s and still compute-bound.
///   converges below    -> density is not the mechanism, and that claim dies with evidence
///                         rather than by absence.
///
/// The trick: values are 0..4, so `byte >= 2` is exactly `byte & 0xFE != 0`. Mask a whole u64,
/// fold each byte's surviving bits down to its own bit 0, keep only the low bit of each byte, and
/// popcount. Eight bytes per ~6 operations.
/// ⚠ The right-shifts cross byte boundaries — byte n+1's low bits land in byte n's high bits — but
///   the `& 0x0101..` discards every position except bit 0, and bit 0 of byte n can only have come
///   from byte n's own bits. Correctness is asserted against the other arms every run regardless;
///   I am not trusting this reasoning on its own.
#[inline(never)]
fn scan_bytes_swar(col: &[u8]) -> usize {
    const HI: u64 = 0xFEFE_FEFE_FEFE_FEFE;
    const LO: u64 = 0x0101_0101_0101_0101;
    let mut n = 0u32;
    let chunks = col.chunks_exact(8);
    let rem = chunks.remainder();
    for ch in chunks {
        let x = u64::from_le_bytes([ch[0], ch[1], ch[2], ch[3], ch[4], ch[5], ch[6], ch[7]]);
        let mut y = x & HI;
        y |= y >> 1;
        y |= y >> 2;
        y |= y >> 4;
        n += (y & LO).count_ones();
    }
    n as usize + rem.iter().filter(|&&o| o >= 2).count()
}

#[inline(never)]
fn scan_bytes_fair(col: &[u8]) -> usize {
    // Branchless, single comparison, accumulator LLVM can widen into vector lanes.
    let mut n = 0usize;
    for &o in col {
        n += (o >= 2) as usize;
    }
    n
}

fn main() {
    println!("MOTH FLUX — adversarial + scale. Rust on both sides; no extrapolation.\n");
    println!("  {:<14} {:>12} {:>12} {:>10}   {:>12} {:>12}   {:>10} {:>10} {:>7}",
             "rows", "planes us", "bytes us", "scan win", "lookup pl", "lookup by",
             "mfield col", "mfield row", "penalty");

    for &n in SIZES.iter() {
        let mut rng = Lcg(0x5EED_1234_ABCD_0001);
        let mut pl_ran = Planes::with_len(n);
        let mut pl_ver = Planes::with_len(n);
        let mut col: Vec<u8> = Vec::with_capacity(n);
        for i in 0..n {
            let o = synth_outcome(rng.next());
            let (ran, verdict, _m) = o.to_trits();
            match ran { Trit::Yes => pl_ran.set_yes(i), Trit::No => pl_ran.set_no(i), _ => {} }
            match verdict { Trit::Yes => pl_ver.set_yes(i), Trit::No => pl_ver.set_no(i), _ => {} }
            col.push(match o {
                Outcome::NothingRan => 0, Outcome::Ok => 1, Outcome::Fail => 2,
                Outcome::CannotTell => 3, Outcome::NoMatch => 4,
            });
        }

        // Fewer reps as n grows — the point is per-row cost, and 2000 reps of 200M rows is a night.
        let reps: u32 = if n <= 500_000 { 500 } else if n <= 5_000_000 { 50 } else { 5 };

        // ---- SCAN, INTERLEAVED -----------------------------------------------------------
        // ⛔⛔ THIS USED TO BE TWO SEQUENTIAL LOOPS AND THAT WAS THE WHOLE NOISE PROBLEM.
        //   Measured across five identical runs at 200M: the scan win ranged 13.23x to 17.37x
        //   (10.8% CV) while the lookup and multi-field penalties held to 1.1% and 2.4%. The
        //   difference was not luck — those two are ratios taken inside ONE loop over the SAME
        //   probe list microseconds apart, so whatever perturbs the machine perturbs both arms
        //   together and divides out. The scan had a warm-up and a full pass sitting BETWEEN the
        //   two things it was comparing, so its arms drifted independently (7.7% and 3.5%) and the
        //   quotient compounded them.
        // ★ So: alternate the arms inside one loop and take the MEDIAN OF PER-REP RATIOS. The
        //   ratio is the quantity I actually report, so it is the quantity that should be measured
        //   directly rather than reconstructed from two separately-drifting averages.
        // ⚠ Fixing noise structurally, not by averaging more runs. More samples of a badly-formed
        //   comparison buys precision about the wrong number.
        let mut ratios: Vec<f64> = Vec::with_capacity(reps as usize);
        let mut planes_tot = 0f64;
        let mut bytes_tot = 0f64;
        let mut a = 0usize;
        let mut b = 0usize;
        for _ in 0..reps {
            let t = Instant::now();
            a = black_box(moth_flux::planes::ran_and_not_ok(
                black_box(&pl_ran), black_box(&pl_ver), n));
            let p = t.elapsed().as_secs_f64() * 1e6;

            let t = Instant::now();
            b = black_box(scan_bytes_fair(black_box(&col)));
            let q = t.elapsed().as_secs_f64() * 1e6;

            planes_tot += p;
            bytes_tot += q;
            ratios.push(q / p);
        }
        // one timed pass of the OLD arm, so the cost of my own bad predicate is on the record
        let t = Instant::now();
        let naive = black_box(scan_bytes_naive(black_box(&col)));
        let naive_us = t.elapsed().as_secs_f64() * 1e6;
        assert_eq!(naive, a, "naive byte arm disagrees at n={}", n);
        let t = Instant::now();
        let four = black_box(scan_bytes_4acc(black_box(&col)));
        let four_us = t.elapsed().as_secs_f64() * 1e6;
        assert_eq!(four, a, "4-accumulator byte arm disagrees at n={}", n);
        let t = Instant::now();
        let swar = black_box(scan_bytes_swar(black_box(&col)));
        let swar_us = t.elapsed().as_secs_f64() * 1e6;
        assert_eq!(swar, a, "SWAR byte arm disagrees at n={} — the bit trick is wrong", n);
        ratios.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let scan_win = ratios[ratios.len() / 2];   // median, not mean — one stall must not move it
        let planes_us = planes_tot / reps as f64;
        let bytes_us = bytes_tot / reps as f64;
        assert_eq!(a, b, "scan arms disagree at n={} — invalid", n);

        // ---- POINT LOOKUP: the query planes should LOSE ----------------------------------
        // 100k pseudo-random single-record reads. This is the adversarial case: no locality,
        // and planes pay two loads plus shifts where a byte column pays one indexed load.
        let probes: Vec<usize> = {
            let mut r = Lcg(0xC0FFEE);
            (0..100_000).map(|_| (r.next() as usize) % n).collect()
        };
        let t = Instant::now();
        let mut acc = 0usize;
        for &i in &probes {
            let w = i >> 6;
            let bit = 1u64 << (i & 63);
            let ran_yes = (pl_ran.known[w] & pl_ran.value[w] & bit) != 0;
            let ver_yes = (pl_ver.known[w] & pl_ver.value[w] & bit) != 0;
            acc += (ran_yes && !ver_yes) as usize;
        }
        let lk_planes_us = t.elapsed().as_secs_f64() * 1e6;
        black_box(acc);

        let t = Instant::now();
        let mut acc2 = 0usize;
        for &i in &probes {
            let o = col[i];
            acc2 += (o != 0 && o != 1) as usize;
        }
        let lk_bytes_us = t.elapsed().as_secs_f64() * 1e6;
        black_box(acc2);
        assert_eq!(acc, acc2, "lookup arms disagree at n={} — invalid", n);

        // ---- MULTI-FIELD READ: every field of one record. Row store wins by construction. ----
        // Columnar must touch FOUR arrays (2 planes + kind + target); the row store touches one
        // struct, i.e. one cache line. This is the weakest point of the whole columnar bet.
        #[derive(Clone, Copy)]
        struct Row { outcome: u8, kind: u8, target: u32 }
        let rows_soa: Vec<Row> = (0..n).map(|i| Row {
            outcome: col[i], kind: (i % 5) as u8, target: (i % 160) as u32 }).collect();
        let kind_col: Vec<u8> = (0..n).map(|i| (i % 5) as u8).collect();
        let target_col: Vec<u32> = (0..n).map(|i| (i % 160) as u32).collect();

        let t = Instant::now();
        let mut m1 = 0u64;
        for &i in &probes {
            let w = i >> 6; let bit = 1u64 << (i & 63);
            let ran_yes = (pl_ran.known[w] & pl_ran.value[w] & bit) != 0;
            let ver_yes = (pl_ver.known[w] & pl_ver.value[w] & bit) != 0;
            m1 = m1.wrapping_add(ran_yes as u64) .wrapping_add(ver_yes as u64)
                   .wrapping_add(kind_col[i] as u64).wrapping_add(target_col[i] as u64);
        }
        let mf_col_us = t.elapsed().as_secs_f64() * 1e6;
        black_box(m1);

        let t = Instant::now();
        let mut m2 = 0u64;
        for &i in &probes {
            let r = rows_soa[i];
            m2 = m2.wrapping_add((r.outcome != 0 && r.outcome != 1) as u64)
                   .wrapping_add((r.outcome == 1) as u64)
                   .wrapping_add(r.kind as u64).wrapping_add(r.target as u64);
        }
        let mf_row_us = t.elapsed().as_secs_f64() * 1e6;
        black_box(m2);

        println!("  {:<14} {:>12.2} {:>12.2} {:>9.2}x   {:>12.0} {:>12.0}   {:>10.0} {:>10.0} {:>7.2}x",
                 fmt(n), planes_us, bytes_us, scan_win, lk_planes_us, lk_bytes_us,
                 mf_col_us, mf_row_us, mf_col_us / mf_row_us);
        // ---- CONVERSION COST: can adaptive layout selection ever pay for itself? ------------
        // ⛔ THE OBJECTION NOBODY IN THE THREAD HAS RAISED. "Let the compiler pick the layout per
        //   workload" assumes the choice is free. It is not: a real system runs MANY queries
        //   against the SAME data and they want DIFFERENT layouts. You either store both — which
        //   doubles memory and destroys the 4x density that is the only measured win — or you
        //   convert, and conversion is a full pass over both representations.
        // ★ So measure it rather than argue it. If converting costs more than the query it
        //   accelerates, adaptive selection is not a compiler decision at all; it is a SCHEMA
        //   decision made once and amortised, which is a completely different (and much less
        //   novel) claim.
        let t = Instant::now();
        let mut conv_ran = Planes::with_len(n);
        let mut conv_ver = Planes::with_len(n);
        for (i, &o) in col.iter().enumerate() {
            match o { 0 => conv_ran.set_no(i), _ => { conv_ran.set_yes(i);
                      if o == 1 { conv_ver.set_yes(i) } else if o == 2 { conv_ver.set_no(i) } } }
        }
        let conv_us = t.elapsed().as_secs_f64() * 1e6;
        black_box(&conv_ran); black_box(&conv_ver);

        let best = four_us.min(swar_us).min(bytes_us);
        println!("  {:<14}   bytes: naive {:.0} | 4-acc {:.0} | SWAR {:.0} us   best-vs-planes {:.2}x  (density ratio is 4.00x)",
                 "", naive_us, four_us, swar_us, best / planes_us);
        println!("  {:<14}   CONVERT byte->planes {:.0} us = {:.1} plane-scans; breaks even after {:.0} queries",
                 "", conv_us, conv_us / planes_us, conv_us / (best - planes_us).max(1e-9));
    }

    println!();
    println!("  scan win  = MEDIAN of per-rep ratios, arms interleaved (was: quotient of two separate means)");
    println!("  lookup    = total us for 100,000 random single-record reads (LOWER is better)");
    println!("  mfield    = same 100,000 probes but reading ALL FOUR fields; penalty = col/row");
    println!("== mothadversarial COMPLETE (rc=0) ==");
}

fn fmt(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); }
        out.push(c);
    }
    out
}
