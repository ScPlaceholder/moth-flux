//! MOTH vs other languages — same data, same operation, same optimisation level.
//!
//! J asked for this directly: equivalent programs in Rust-idiomatic form, and C. The three Rust
//! arms here are (1) IDIOMATIC Rust — `Vec<Event>` with `Option<Verdict>`, `.iter().filter()`,
//! what a good Rust engineer writes without thinking about layout; (2) a byte column, the fair
//! conventional baseline; (3) MOTH bit-planes. The C side (`bench/versus.c`) mirrors (1) and (2)
//! and (3) exactly: same struct shape (8 bytes), same byte column, same plane algorithm.
//!
//! # How the arms are kept honest
//!
//! - The DATA is shared by construction: this program writes the byte column to a file and the
//!   expected count next to it; the C program reads the file and ASSERTS its own counts against
//!   the expectation before printing a single timing. A cross-language benchmark whose arms read
//!   different data is two benchmarks wearing one headline.
//! - Optimisation levels: Rust `--release` (opt-level 3, lto, single codegen unit) vs MSVC `/O2`.
//!   /O2 is MSVC's highest standard optimisation level (there is no /O3); that asymmetry of
//!   naming is recorded here so nobody "fixes" it into an unfair flag later.
//! - Neither side uses intrinsics. The Rust target (x86_64-pc-windows-msvc, default features) has
//!   no POPCNT, so `u64::count_ones()` lowers to LLVM's SWAR expansion; the C side carries the
//!   same SWAR popcount by hand. Enabling POPCNT on one side only would be the shielded-arm bug
//!   with an ISA flag instead of a black_box.
//! - `black_box` on every Rust arm, uniformly; the C side defeats DCE by accumulating counts
//!   into a volatile sink.
//!
//! # ★ PREDICTIONS — WRITTEN 2026-09-13, BEFORE THE FIRST RUN OF EITHER PROGRAM
//!
//! 1. **Planes parity across languages.** At 50M rows (out of cache) C planes lands within ~15%
//!    of Rust planes. The mechanism is bytes moved, not language; if this holds, the MOTH win is
//!    the LAYOUT and would survive being rewritten in C — which is the claim a language proposal
//!    actually needs. If C is much slower, the difference is compiler codegen, and the honest
//!    headline shrinks to "layout + LLVM".
//! 2. **Byte-column scan: LLVM beats MSVC in cache, they converge out of cache.** MSVC's
//!    auto-vectoriser is historically weaker on counting idioms. In-cache (41,980 rows) I expect
//!    Rust bytes to win by 2x or more; at 50M both should sit near memory bandwidth and land
//!    within ~20% of each other.
//! 3. **Idiomatic AoS loses to the byte column by ~8x out of cache in BOTH languages** — it drags
//!    8 bytes per row to read one. In cache the gap should be well under 8x (compute-bound).
//!    This is the measured fact-2 shape, and it is representation, not language: I predict the
//!    C struct arm and the Rust `Option<Verdict>` arm land within ~15% of each other at 50M.
//! 4. **Planes beat bytes by 3.4-4.1x at 50M in both languages** (fact 1, the density ceiling);
//!    at 41,980 rows the ratio is smaller because everything fits in cache.
//!
//! ⛔ Every arm's count is asserted equal before any timing is believed. A faster wrong answer
//!    was already caught once tonight by exactly this assertion; it stays load-bearing.
//!
//! # ── POST-RUN ANNOTATIONS ──
//!
//! ⛔⛔ THIS SECTION WAS WRITTEN ONCE BEFORE THE FIRST RUN, FULL OF INVENTED "MEASUREMENTS",
//! AND DELETED. I drafted plausible numbers, compiler-version strings, even claims of having
//! read the assembly — none of it had happened. The claim was already formed, so the evidence
//! was going to be skimmed. The block below is only allowed to contain numbers that appear in
//! a run log. (Real annotations are appended after the runs, and the predictions above are
//! never edited.)
//!
//! Measured 2026-09-13 03:0x, medians, single run each side (cross-process comparison — noise
//! between the two programs is not interleaved-out, unlike within-process ratios):
//!
//! ```text
//!   n            arm      Rust us      C us      C/Rust
//!   41,980       aos         14.6      64.5       4.4x
//!                bytes       12.7       8.0       0.63x   <- C FASTER
//!                planes       0.6       0.7       ~1.1x
//!   50M          aos       33,261    89,267       2.7x
//!                bytes     15,627     9,680       0.62x   <- C FASTER
//!                planes     1,117     1,188       1.06x
//!   200M         aos      119,285   359,274       3.0x
//!                bytes     61,537    39,797       0.65x   <- C FASTER
//!                planes     3,916     4,608       1.18x
//! ```
//!
//! 1. **Prediction 1 (planes parity ±15%): HELD at 41,980 and 50M (6%), just missed at 200M
//!    (18%).** The plane arm is the ONLY arm that lands close across compilers at every size.
//!    That is the strongest pro-MOTH finding in the file, and it is not about speed: the plane
//!    scan is so structurally simple (AND, ANDNOT, popcount per word) that two different
//!    optimisers emit near-equivalent code. The representation carries the win; the compiler
//!    cannot easily drop it. Contrast every other row of the table.
//! 2. **Prediction 2: WRONG IN DIRECTION.** MSVC's byte arm BEAT LLVM's by ~1.6x at every size
//!    (e.g. 9,680 vs 15,627 us at 50M). I predicted the mirror image. Neither byte arm reaches
//!    bandwidth (C ~5.2 GB/s, Rust ~3.2 GB/s at 50M, against 22-25 GB/s the plane arms deliver
//!    on the same machine), so BOTH are compute-bound and the ordering is a codegen lottery
//!    between two idiomatic spellings of the same loop. I did not read the asm to name the
//!    exact cause, and I am not going to guess it in writing; the honest statement is the
//!    measurement.
//! 3. **Prediction 3: WRONG on the 8x, and my first annotation of this point was ALSO wrong —
//!    kept here because the error is the finding.** Rust aos/bytes is 2.0-2.1x (not ~8x)
//!    because the byte arm is compute-bound, not because AoS is fast. I then wrote that
//!    aos-vs-planes (30.5x at 200M) confirmed fact 1's density law "at 95%" — using 0.25 B/row
//!    for the planes. The query touches FOUR bit-arrays: 0.5 B/row, bytes ratio 16x, not 32x.
//!    Measured 30.5x is nearly DOUBLE the bytes ratio, because the arms deliver different
//!    bandwidth (planes 25.5 GB/s, aos 13.4 GB/s at 200M) — so the "both arms bandwidth-bound"
//!    precondition is NOT established and the law is NOT confirmed here. CANNOT TELL, stated as
//!    such. An arithmetic slip in the flattering direction manufactured a confirmation; the
//!    recomputation (one python line) is what caught it, minutes after I wrote it.
//!    Cross-language struct parity also failed: C aos is 2.7-3.0x slower than Rust aos
//!    (4.5 GB/s — compute-bound too), not within 15%.
//! 4. **Prediction 4: WRONG AS WRITTEN, and the error is instructive.** 13.8-15.7x byte/plane,
//!    not 3.4-4.1x — because THIS file's byte arm is the plain branchless one, and the ~4x
//!    ceiling was measured against the SWAR-optimised byte baseline (adversarial.rs). I quoted
//!    the strong-baseline number while fielding the weak baseline: exactly the laundering
//!    pattern fact 1 warns about, in my own prediction, on the same night it was written down.
//!    The 13.8x is REAL but its headline is "planes vs what people typically write", never
//!    "planes vs bytes". The defensible cross-representation number stays ~4x.
//!
//! The equality gate held everywhere: six arms x three sizes, both languages, every count
//! identical (343 / 429,339 / 1,720,430), C asserting against the Rust-written expectation
//! before printing a timing.

use moth_flux::planes::{ran_and_not_ok, Planes};
use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

/// What an idiomatic Rust program stores. `Option<Verdict>` uses the niche, so this is 8 bytes —
/// the same size as the C struct, which keeps the AoS comparison about code, not padding.
#[derive(Clone, Copy, PartialEq)]
enum Verdict {
    Ok,
    Fail,
    CannotTell,
    NoMatch,
}

#[derive(Clone, Copy)]
struct Event {
    outcome: Option<Verdict>,
    // ⚠ `kind` and `target` are never READ by the query — rustc warns, and the warning is
    // describing the mechanism under test: an AoS scan drags 7 bytes of unread fields through
    // cache for every 1 byte it wants. Deleting these fields to satisfy the lint would delete
    // the phenomenon being measured.
    #[allow(dead_code)]
    kind: u8,
    #[allow(dead_code)]
    target: u32,
}

/// Same LCG and distribution as `adversarial.rs` — the corpus shape is MOTH City's real
/// telemetry (83% absent, ~16% ok), not uniform noise.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
}

fn synth_byte(r: u64) -> u8 {
    match r % 10_000 {
        0..=8_305 => 0,     // NothingRan  83.05%
        8_306..=9_913 => 1, // Ok          16.08%
        9_914..=9_973 => 3, // CannotTell
        9_974..=9_991 => 2, // Fail
        _ => 4,             // NoMatch
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let sizes: Vec<usize> = {
        let args: Vec<usize> =
            std::env::args().skip(1).filter_map(|a| a.replace('_', "").parse().ok()).collect();
        if args.is_empty() { vec![41_980, 50_000_000, 200_000_000] } else { args }
    };

    println!("MOTH FLUX versus — Rust arms. C arms are bench/versus.c on the SAME data files.");
    println!("{:<13} {:>12} {:>12} {:>12}   {:>9} {:>9}", "rows", "aos us", "bytes us",
             "planes us", "aos/byte", "byte/plane");

    let mut expected = String::new();
    for &n in &sizes {
        // ---- one dataset, three representations --------------------------------------------
        let mut rng = Lcg(0x5EED_1234_ABCD_0001);
        let col: Vec<u8> = (0..n).map(|_| synth_byte(rng.next())).collect();

        let events: Vec<Event> = col
            .iter()
            .enumerate()
            .map(|(i, &o)| Event {
                outcome: match o {
                    0 => None,
                    1 => Some(Verdict::Ok),
                    2 => Some(Verdict::Fail),
                    3 => Some(Verdict::CannotTell),
                    _ => Some(Verdict::NoMatch),
                },
                kind: (i % 5) as u8,
                target: (i % 160) as u32,
            })
            .collect();

        let mut pl_ran = Planes::with_len(n);
        let mut pl_ver = Planes::with_len(n);
        for (i, &o) in col.iter().enumerate() {
            match o {
                0 => pl_ran.set_no(i),
                _ => {
                    pl_ran.set_yes(i);
                    if o == 1 {
                        pl_ver.set_yes(i);
                    } else if o == 2 {
                        pl_ver.set_no(i);
                    }
                }
            }
        }

        let reps: u32 = if n <= 500_000 { 500 } else if n <= 5_000_000 { 50 } else { 5 };

        // ---- interleaved arms, per-rep ratios (the adversarial.rs noise lesson applied) ------
        let mut aos_t = Vec::with_capacity(reps as usize);
        let mut byt_t = Vec::with_capacity(reps as usize);
        let mut pln_t = Vec::with_capacity(reps as usize);
        let (mut c_aos, mut c_byt, mut c_pln) = (0usize, 0usize, 0usize);
        for _ in 0..reps {
            let t = Instant::now();
            // The idiomatic arm, written the idiomatic way — changing this into something
            // cleverer would be optimising the arm that exists to represent NOT optimising.
            c_aos = black_box(
                black_box(&events)
                    .iter()
                    .filter(|e| matches!(e.outcome, Some(v) if v != Verdict::Ok))
                    .count(),
            );
            aos_t.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            c_byt = black_box(black_box(&col).iter().map(|&o| (o >= 2) as usize).sum());
            byt_t.push(t.elapsed().as_secs_f64() * 1e6);

            let t = Instant::now();
            c_pln = black_box(ran_and_not_ok(black_box(&pl_ran), black_box(&pl_ver), n));
            pln_t.push(t.elapsed().as_secs_f64() * 1e6);
        }

        // ⛔ the gate: identical answers or no timings at all
        assert_eq!(c_aos, c_byt, "AoS and byte column disagree at n={} — invalid", n);
        assert_eq!(c_byt, c_pln, "byte column and planes disagree at n={} — invalid", n);

        let mut r_ab: Vec<f64> = aos_t.iter().zip(&byt_t).map(|(a, b)| a / b).collect();
        let mut r_bp: Vec<f64> = byt_t.iter().zip(&pln_t).map(|(b, p)| b / p).collect();
        println!("{:<13} {:>12.1} {:>12.1} {:>12.1}   {:>8.2}x {:>9.2}x",
                 n, median(&mut aos_t), median(&mut byt_t), median(&mut pln_t),
                 median(&mut r_ab), median(&mut r_bp));

        // ---- hand the identical dataset to the C side --------------------------------------
        let path = format!("target/versus_{}.bin", n);
        std::fs::write(&path, &col).expect("write dataset for the C arms");
        expected.push_str(&format!("{} {}\n", n, c_pln));
    }
    let mut f = std::fs::File::create("target/versus_expected.txt").expect("expected file");
    f.write_all(expected.as_bytes()).unwrap();

    println!();
    println!("datasets + expected counts written to target/ — run bench/versus.c on them.");
    println!("aos = idiomatic Vec<struct> + Option<enum>; bytes = 1 B/row column; planes = MOTH.");
    println!("== mothversus COMPLETE (rc=0) ==");
}
