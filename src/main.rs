//! The benchmark. Three representations of the same 41,283 real MOTH City events, one query.
//!
//! # ⛔ HOW THIS BENCHMARK IS RIGGED, AND WHICH WAY
//!
//! The comparator is **rkyv with a proper `#[repr(u8)]` enum**, which is what a competent Rust
//! engineer would actually write. It is NOT rkyv with a `String` outcome — that would be a
//! strawman, and comparing a string compare against a 2-bit mask would let me publish a win I did
//! not earn. serde_json is present only as a labelled floor so the absolute numbers have an anchor;
//! **beating it means nothing.**
//!
//! ★ AND THE HONEST PREDICTION, WRITTEN BEFORE THE RUN so it cannot be adjusted afterwards:
//! at 41,283 rows the whole outcome column is ~41 KB as bytes and ~21 KB as packed trits. **Both
//! fit in L2 on this machine.** The cache-locality argument that motivates the entire proposal may
//! therefore not bite at all at this scale, and if so the correct report is "no difference, and the
//! corpus is too small to test the claim" — not a hunt for a configuration where MOTH wins.
//!
//! ⛔⛔ **ALL THREE MUST RETURN THE SAME COUNT, ASSERTED.** A faster wrong answer is the classic way
//! a benchmark like this goes wrong, and three implementations that disagree are not comparable at
//! all. The equality check below is the most important line in the file; if it ever fails, every
//! timing printed alongside it is meaningless.

use moth_flux::event::{EventColumns, Outcome, RawEvent};
use moth_flux::planes::{ran_and_not_ok, ran_and_not_ok_bytes, Planes};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use std::hint::black_box;
use std::time::Instant;

// ⛔⛔ black_box ON EVERY ARM, ADDED AFTER THE PLANES SCAN READ 0.000 ms. That is not a speed, it is
//   a measurement fault: the inputs are loop-invariant and the result is read once after the loop,
//   so LLVM is free to compute it once and delete 199 of the 200 reps. A timing of zero is the
//   benchmark equivalent of a zero-byte output file — it means the instrument did not run, never
//   that there was nothing to do. Applied UNIFORMLY, because shielding only the arm I want to win
//   is how a baseline gets quietly handicapped.

/// What rkyv stores. Same five states, one byte, laid out row-wise — the fair opponent.
#[derive(Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone, Copy, PartialEq)]
#[archive(check_bytes)]
#[repr(u8)]
pub enum ArchOutcome {
    NothingRan,
    Ok,
    Fail,
    CannotTell,
    NoMatch,
}

#[derive(Archive, RkyvSerialize, RkyvDeserialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ArchEvent {
    pub outcome: ArchOutcome,
    pub kind: u8,
    pub target: u32,
}

fn to_arch(o: Outcome) -> ArchOutcome {
    match o {
        Outcome::NothingRan => ArchOutcome::NothingRan,
        Outcome::Ok => ArchOutcome::Ok,
        Outcome::Fail => ArchOutcome::Fail,
        Outcome::CannotTell => ArchOutcome::CannotTell,
        Outcome::NoMatch => ArchOutcome::NoMatch,
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../.moth_events.jsonl".to_string());
    let raw = std::fs::read_to_string(&path).expect("corpus not found");

    // ---- parse once, outside every timed section -------------------------------------------
    let mut rows: Vec<RawEvent> = Vec::new();
    let mut json_lines: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(r) = serde_json::from_str::<RawEvent>(line) {
            rows.push(r);
            json_lines.push(line);
        }
    }
    println!("corpus     : {} rows from {}", rows.len(), path);

    // ---- build the three representations ----------------------------------------------------
    let cols = EventColumns::build(&rows);

    let arch: Vec<ArchEvent> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| ArchEvent {
            outcome: to_arch(Outcome::parse(r.outcome.as_deref())),
            kind: cols.kind[i],
            target: cols.target[i],
        })
        .collect();
    let arch_bytes = rkyv::to_bytes::<_, 4096>(&arch).expect("rkyv serialise");

    let json_bytes: usize = json_lines.iter().map(|l| l.len()).sum();

    println!();
    println!("FOOTPRINT (the query fields only, strings excluded from MOTH's ternary columns)");
    println!("  json   (raw lines, whole records) : {:>9} B", json_bytes);
    println!("  rkyv   (archived, whole records)  : {:>9} B", arch_bytes.len());
    println!("  moth   (2 trit cols + kind+target): {:>9} B", cols.footprint());
    println!("  moth ternary columns alone        : {:>9} B",
             cols.ran.as_bytes().len() + cols.verdict.as_bytes().len());

    // ---- the query: events where a check RAN and did not come back Ok ------------------------
    // Chosen because it needs the unresolved state KEPT: rows where nothing ran must not count as
    // failures, and cannot_tell must not count as ok. A binary encoding cannot express the query.
    const REPS: u32 = 2000;

    let mut n_json = 0usize;
    let t = Instant::now();
    for _ in 0..REPS {
        n_json = 0;
        for l in &json_lines {
            let r: RawEvent = serde_json::from_str(l).unwrap();
            let o = Outcome::parse(r.outcome.as_deref());
            if o != Outcome::NothingRan && o != Outcome::Ok {
                n_json += 1;
            }
        }
    }
    let json_ms = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

    let archived = rkyv::check_archived_root::<Vec<ArchEvent>>(&arch_bytes).expect("validate");
    let mut n_rkyv = 0usize;
    let t = Instant::now();
    for _ in 0..REPS {
        n_rkyv = 0;
        for e in black_box(archived).iter() {
            // Zero-copy: `archived` is a typed view straight over `arch_bytes`, no deserialise.
            if !matches!(e.outcome, ArchivedArchOutcome::NothingRan | ArchivedArchOutcome::Ok) {
                n_rkyv += 1;
            }
        }
    }
    black_box(n_rkyv);
    let rkyv_ms = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

    let mut n_moth = 0usize;
    let t = Instant::now();
    for _ in 0..REPS {
        n_moth = black_box(black_box(&cols).ran_and_not_ok());
    }
    let moth_ms = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

    // ---- bit-planes, and an honest byte baseline for them --------------------------------
    let mut pl_ran = Planes::with_len(rows.len());
    let mut pl_ver = Planes::with_len(rows.len());
    let mut bytes_col: Vec<u8> = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        let o = Outcome::parse(r.outcome.as_deref());
        let (ran, verdict, _m) = o.to_trits();
        match ran { moth_flux::Trit::Yes => pl_ran.set_yes(i),
                    moth_flux::Trit::No => pl_ran.set_no(i), _ => {} }
        match verdict { moth_flux::Trit::Yes => pl_ver.set_yes(i),
                        moth_flux::Trit::No => pl_ver.set_no(i), _ => {} }
        bytes_col.push(match o { Outcome::NothingRan => 0, Outcome::Ok => 1, _ => 2 });
    }
    println!("  moth bit-planes (2 cols)          : {:>9} B", pl_ran.bytes() + pl_ver.bytes());
    println!("  byte baseline (1 col, 1 B/row)    : {:>9} B", bytes_col.len());

    let mut n_planes = 0usize;
    let t = Instant::now();
    for _ in 0..REPS { n_planes = black_box(ran_and_not_ok(black_box(&pl_ran), black_box(&pl_ver), rows.len())); }
    let planes_ms = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

    let mut n_bytes = 0usize;
    let t = Instant::now();
    for _ in 0..REPS { n_bytes = black_box(ran_and_not_ok_bytes(black_box(&bytes_col), 1, 0)); }
    let bytes_ms = t.elapsed().as_secs_f64() * 1000.0 / REPS as f64;

    println!();
    println!("SCAN: count events where a check RAN and the verdict was not Ok  ({} reps)", REPS);
    println!("  json   : {:>9.2} us   -> {}", json_ms*1000.0, n_json);
    println!("  rkyv   : {:>9.2} us   -> {}", rkyv_ms*1000.0, n_rkyv);
    println!("  moth   : {:>9.2} us   -> {}", moth_ms*1000.0, n_moth);

    println!("  planes : {:>9.2} us   -> {}   (2-bit planes, 64 trits/word)", planes_ms*1000.0, n_planes);
    println!("  bytes  : {:>9.2} us   -> {}   (honest byte baseline, same trick at 8 bits)", bytes_ms*1000.0, n_bytes);

    assert_eq!(n_json, n_planes, "planes disagree — the benchmark is invalid");
    assert_eq!(n_json, n_bytes, "byte baseline disagrees — the benchmark is invalid");
    assert_eq!(n_json, n_rkyv, "json and rkyv disagree — the benchmark is invalid");
    assert_eq!(n_rkyv, n_moth, "rkyv and moth disagree — the benchmark is invalid");
    println!("  ✓ all three agree on {} — the comparison is valid", n_moth);

    // ---- the row store answers too, on the REAL corpus -------------------------------------
    // The AoS<->SoA conversion has unit tests on synthetic rows; this is the same assertion on
    // the data that actually matters, plus the round trip. The NoMatch encoding bug was caught
    // by exactly this seam — an assertion that lives only in a test file guards only test data.
    let row_store = moth_flux::store::EventRows::from(&cols);
    assert_eq!(row_store.ran_and_not_ok(), n_moth,
               "row store disagrees with columnar on the real corpus");
    let back = moth_flux::event::EventColumns::from(&row_store);
    assert_eq!(back.ran_and_not_ok(), n_moth,
               "SoA->AoS->SoA round trip changed the answer on the real corpus");
    println!("  ✓ row store (AoS) and its round trip agree on {}  (footprint {} B vs {} B columnar)",
             n_moth, row_store.footprint(), cols.footprint());

    println!();
    println!("== mothfluxbench COMPLETE (rc=0) ==");
}
