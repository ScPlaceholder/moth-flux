//! One MOTH City telemetry event, in the three representations under test.
//!
//! # ★★★★★ THE FINDING THAT CAME OUT OF MODELLING THE REAL DATA
//!
//! `outcome` in `.moth_events.jsonl` looks like a textbook trit:
//!
//! ```text
//! ok(6743)  fail(74)  cannot_tell(250)  no_match(19)   — on 7,087 of 41,283 rows
//! ```
//!
//! Map it the obvious way — `ok`→Yes, `fail`→No, `cannot_tell`→Unknown — and you have a clean
//! ternary column. **And it is wrong**, because 34,196 rows have no `outcome` key at all, and
//! *absent* is not *cannot_tell*.
//!
//! - **cannot_tell** = a check RAN and could not reach a verdict. An answer.
//! - **absent** = no check ran. Not an answer.
//!
//! A single trit has exactly one slot for "not Yes and not No", so it must merge them — and merging
//! them is the precise error my instruments spend every night refusing to make. A ternary type is
//! not automatically a licence to stop counting states: **the question is never "how many states
//! does this field have", it is "how many DISTINCT things can I be told".** Here it is five.
//!
//! So the record carries two trits: `ran` (did anything look?) and `verdict` (what did it find?).
//! Four bits total, and no state is quietly folded into another.
//!
//! ⚠ THIS IS A RESULT FOR THE PROPOSAL, NOT A DETAIL. §8 argues ternary because "value + valid flag"
//! is a workaround. True — but the fix is not "one trit instead of two fields". It is that the
//! compiler should force you to enumerate the distinct things the world can tell you, and then give
//! you exactly that many states. Sometimes that is three. Here it is five, and a language that made
//! three feel like the natural answer would have caused this bug rather than prevented it.

use crate::trit::{Trit, Trits};
use serde::Deserialize;

/// What the JSON actually contains. Only the fields the benchmark scan touches.
#[derive(Debug, Deserialize, Clone)]
pub struct RawEvent {
    pub kind: Option<String>,
    pub target: Option<String>,
    pub outcome: Option<String>,
    pub host: Option<String>,
    pub seq: Option<u64>,
}

/// The five distinct things the record can tell us about an outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// No `outcome` key. Nothing looked. 34,196 of 41,283 rows.
    NothingRan,
    Ok,
    Fail,
    /// A check ran and reached no verdict — an answer, and NOT the same as `NothingRan`.
    CannotTell,
    NoMatch,
}

impl Outcome {
    pub fn parse(s: Option<&str>) -> Outcome {
        match s {
            None => Outcome::NothingRan,
            Some("ok") => Outcome::Ok,
            Some("fail") => Outcome::Fail,
            Some("cannot_tell") => Outcome::CannotTell,
            Some("no_match") => Outcome::NoMatch,
            // ⚠ An unrecognised string is NOT NothingRan. Something ran and said a word we do not
            //   know; folding that into "nothing ran" would be the same merge this file exists to
            //   refuse, committed one level down.
            Some(_) => Outcome::CannotTell,
        }
    }

    /// (ran, verdict, matched) as THREE trits.
    ///
    /// ⛔⛔ IT WAS TWO, AND TWO WAS WRONG. Measured 2026-09-13 01:20: the benchmark's equality
    ///   assertion fired — rkyv counted 348, MOTH counted 329, and the 19-row gap was exactly the
    ///   `no_match` population. I had encoded NoMatch as `(ran=Unknown, verdict=No)` and my own
    ///   comment admitted why: *"we needed a fifth state and had nine."* **I picked a FREE slot
    ///   instead of a MEANINGFUL one.** `no_match` means a check RAN and matched nothing, so its
    ///   `ran` is Yes. The query asked `ran == Yes` and correctly excluded it.
    ///   ★ An encoding chosen for capacity, read correctly, returning the wrong answer. Ternary was
    ///     supposed to carry the semantics; I assigned a state arbitrarily and the semantics broke
    ///     silently. Only the cross-representation assertion caught it.
    ///
    /// ★★★ AND THE FIX EXPOSES THE REAL SHAPE, WHICH IS THE RESULT FOR THE PROPOSAL. `outcome` is
    ///   not 3 states, and it is not 5-states-in-9-slots. It is **two dimensions**: did-anything-run
    ///   (BINARY — its Unknown never occurs in 41,797 rows) times what-it-said (FOUR values). Two
    ///   trits give 9 slots but the shape required is 2x4, so the capacity was spent on the wrong
    ///   axis. A third column is needed: 6 bits against rkyv's 8, and the footprint advantage
    ///   nearly evaporates.
    ///   **A ternary type did not fit this data better than a binary one. It failed differently.**
    pub fn to_trits(self) -> (Trit, Trit, Trit) {
        match self {
            //                      ran          verdict        matched
            Outcome::NothingRan => (Trit::No,    Trit::Unknown, Trit::Unknown),
            Outcome::Ok         => (Trit::Yes,   Trit::Yes,     Trit::Yes),
            Outcome::Fail       => (Trit::Yes,   Trit::No,      Trit::Yes),
            Outcome::CannotTell => (Trit::Yes,   Trit::Unknown, Trit::Yes),
            Outcome::NoMatch    => (Trit::Yes,   Trit::Unknown, Trit::No),
        }
    }

    /// The compile-checked mirror of [`Outcome::to_trits`], as raw trit bits (No=0, Unknown=1,
    /// Yes=2), in `Outcome` declaration order.
    ///
    /// ★ WHAT THE CONST ASSERT BELOW BUYS AND WHAT IT DOES NOT. `injective` refuses at COMPILE
    /// time the merge that produced the five-state finding — two states sharing one encoding
    /// slot. It does NOT catch the NoMatch bug itself (a state parked in the WRONG slot): that
    /// encoding was injective and still wrong, and only the cross-representation assertion in
    /// the benchmark caught it. Both guards stay; they cover different failures.
    ///
    /// ⚠ The table duplicates the match in `to_trits` on purpose: the match carries the earned
    /// history in its comments, the table is the form a const fn can check, and the
    /// `table_and_match_agree` test is what keeps the two from drifting apart silently.
    pub const TRIT_ENCODING: [[u8; 3]; 5] = [
        [0, 1, 1], // NothingRan  (ran=No,  verdict=Unknown, matched=Unknown)
        [2, 2, 2], // Ok
        [2, 0, 2], // Fail
        [2, 1, 2], // CannotTell
        [2, 1, 0], // NoMatch     (ran=Yes — the slot the bug got wrong)
    ];

    pub fn from_trits(ran: Trit, verdict: Trit, matched: Trit) -> Outcome {
        match (ran, verdict, matched) {
            (Trit::No, _, _) => Outcome::NothingRan,
            (Trit::Yes, Trit::Yes, _) => Outcome::Ok,
            (Trit::Yes, Trit::No, _) => Outcome::Fail,
            (Trit::Yes, Trit::Unknown, Trit::No) => Outcome::NoMatch,
            (Trit::Yes, Trit::Unknown, _) => Outcome::CannotTell,
            _ => Outcome::CannotTell,
        }
    }
}

// ⛔ COMPILE-TIME: no two outcome states may share an encoding slot. If a sixth state is added
//   and lands on an existing tuple, the build fails HERE, at the table, not as a wrong count in
//   production. (`cardinality::injective` — the trap this cannot catch is documented on the table.)
const _: () = assert!(crate::cardinality::injective(&Outcome::TRIT_ENCODING));

/// The packed MOTH column store: parallel arrays, ternary fields in `Trits`.
///
/// Columnar rather than row-wise on purpose. The benchmark scan reads two fields out of five, and
/// a row store drags the other three through cache to get there. If packed ternary is going to win
/// anywhere it is here, and if it loses even here the language does not earn itself.
#[derive(Default)]
pub struct EventColumns {
    pub ran: Trits,
    pub verdict: Trits,
    pub matched: Trits,
    pub kind: Vec<u8>,
    pub target: Vec<u32>,
    pub targets: Vec<String>,
    len: usize,
}

impl EventColumns {
    pub fn build(rows: &[RawEvent]) -> EventColumns {
        let n = rows.len();
        let mut c = EventColumns {
            ran: Trits::with_len(n),
            verdict: Trits::with_len(n),
            matched: Trits::with_len(n),
            kind: Vec::with_capacity(n),
            target: Vec::with_capacity(n),
            targets: Vec::new(),
            len: n,
        };
        let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for (i, r) in rows.iter().enumerate() {
            let o = Outcome::parse(r.outcome.as_deref());
            let (ran, verdict, matched) = o.to_trits();
            c.ran.set(i, ran);
            c.verdict.set(i, verdict);
            c.matched.set(i, matched);
            c.kind.push(match r.kind.as_deref() {
                Some("stage_enter") => 0,
                Some("report_origin") => 1,
                Some("report") => 2,
                Some("stage_exit") => 3,
                _ => 4,
            });
            let t = r.target.as_deref().unwrap_or("");
            let id = match seen.get(t) {
                Some(&id) => id,
                None => {
                    let id = c.targets.len() as u32;
                    c.targets.push(t.to_string());
                    seen.insert(t.to_string(), id);
                    id
                }
            };
            c.target.push(id);
        }
        c
    }

    /// Assemble from already-built columns (the AoS->SoA conversion in `store.rs` lands here).
    /// Asserts every column agrees on the row count — five arrays of four different lengths is
    /// not a shorter table, it is a corrupt one, and refusing loudly beats truncating quietly.
    pub fn from_parts(
        ran: Trits,
        verdict: Trits,
        matched: Trits,
        kind: Vec<u8>,
        target: Vec<u32>,
        targets: Vec<String>,
    ) -> EventColumns {
        let n = kind.len();
        assert_eq!(ran.len(), n, "ran column length mismatch");
        assert_eq!(verdict.len(), n, "verdict column length mismatch");
        assert_eq!(matched.len(), n, "matched column length mismatch");
        assert_eq!(target.len(), n, "target column length mismatch");
        EventColumns { ran, verdict, matched, kind, target, targets, len: n }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bytes actually resident for the two ternary columns plus kind and target ids.
    pub fn footprint(&self) -> usize {
        self.ran.as_bytes().len()
            + self.verdict.as_bytes().len()
            + self.matched.as_bytes().len()
            + self.kind.len()
            + self.target.len() * 4
            + self.targets.iter().map(|s| s.len() + 24).sum::<usize>()
    }

    /// THE BENCHMARK QUERY: how many events ran a check that did not come back Ok?
    ///
    /// Deliberately a question that needs the unresolved state kept, not collapsed: rows where
    /// nothing ran must not count as failures, and `cannot_tell` must not count as `ok`.
    pub fn ran_and_not_ok(&self) -> usize {
        let mut n = 0;
        for i in 0..self.len {
            if self.ran.get(i) == Trit::Yes && self.verdict.get(i) != Trit::Yes {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_and_cannot_tell_survive_the_round_trip_separately() {
        // The whole reason this record carries two trits. If these ever compare equal, the merge
        // this file was written to prevent has come back.
        for o in [
            Outcome::NothingRan,
            Outcome::Ok,
            Outcome::Fail,
            Outcome::CannotTell,
            Outcome::NoMatch,
        ] {
            let (r, v, m) = o.to_trits();
            assert_eq!(Outcome::from_trits(r, v, m), o, "{:?} did not survive packing", o);
        }
        assert_ne!(
            Outcome::NothingRan.to_trits(),
            Outcome::CannotTell.to_trits(),
            "absent and cannot_tell MUST NOT share an encoding"
        );
    }

    #[test]
    fn table_and_match_agree() {
        // The const-checked table and the commented match are two copies of one truth. Two copies
        // of one stale rule is a measured failure mode in this house; this test is the tripwire.
        let all = [
            Outcome::NothingRan,
            Outcome::Ok,
            Outcome::Fail,
            Outcome::CannotTell,
            Outcome::NoMatch,
        ];
        for (i, o) in all.iter().enumerate() {
            let (r, v, m) = o.to_trits();
            assert_eq!(
                [r.bits(), v.bits(), m.bits()],
                Outcome::TRIT_ENCODING[i],
                "{:?}: to_trits and TRIT_ENCODING have drifted apart",
                o
            );
        }
    }

    #[test]
    fn an_unknown_outcome_word_is_not_nothing_ran() {
        assert_eq!(Outcome::parse(Some("weird_new_verdict")), Outcome::CannotTell);
        assert_eq!(Outcome::parse(None), Outcome::NothingRan);
    }

    #[test]
    fn the_query_counts_neither_absent_nor_ok() {
        let rows = vec![
            RawEvent { kind: None, target: None, outcome: None, host: None, seq: None },
            RawEvent { kind: None, target: None, outcome: Some("ok".into()), host: None, seq: None },
            RawEvent { kind: None, target: None, outcome: Some("fail".into()), host: None, seq: None },
            RawEvent { kind: None, target: None, outcome: Some("cannot_tell".into()), host: None, seq: None },
        ];
        let c = EventColumns::build(&rows);
        // fail and cannot_tell count; absent and ok do not.
        assert_eq!(c.ran_and_not_ok(), 2);
    }
}
