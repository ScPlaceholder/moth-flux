//! The row store (AoS) — the columnar store's equal partner, not its fallback.
//!
//! # Why this file exists as a first-class citizen
//!
//! The measured cost model (see `cost.rs`) says neither layout wins in general:
//!
//! ```text
//! scan          : columnar wins, ceiling 4.08x (density ratio, measured 41,980 -> 200,000,000 rows)
//! point lookup  : row wins, 2.48x  (columnar touches N arrays; the row is one cache line)
//! multi-field   : row wins, 3.49x  (same mechanism, more arrays)
//! ```
//!
//! A representation-aware language therefore needs BOTH layouts as real types with real
//! conversions, so the choice is a visible materialisation decision (~100 scans to convert,
//! break-even ~42 queries) instead of an accident of which struct someone reached for first.
//!
//! ⚠ `EventRow` is 8 bytes with padding (3x Trit + u8 + u32, align 4). The rkyv comparator in
//! `main.rs` stores 6 bytes-ish per row archived; the honest row-store footprint is the in-memory
//! one, `size_of::<EventRow>() * rows`, and `footprint()` reports exactly that — not a
//! hand-flattered packed figure.

use crate::event::EventColumns;
use crate::trit::{Trit, Trits};

/// One event, laid out the way a struct-of-fields programmer would write it. `Copy`, fixed-size,
/// no pointers — the same "a trit is a value, never a thunk" invariant the packed forms carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventRow {
    pub ran: Trit,
    pub verdict: Trit,
    pub matched: Trit,
    pub kind: u8,
    pub target: u32,
}

/// The AoS store: one `Vec` of rows plus the shared target-string table.
#[derive(Default)]
pub struct EventRows {
    pub rows: Vec<EventRow>,
    /// Interned target strings; `EventRow::target` indexes here. Shared vocabulary with
    /// `EventColumns` so conversion is index-preserving and needs no re-hashing.
    pub targets: Vec<String>,
}

impl EventRows {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// In-memory bytes, padding included — see the module header for why padding is counted.
    pub fn footprint(&self) -> usize {
        self.rows.len() * std::mem::size_of::<EventRow>()
            + self.targets.iter().map(|s| s.len() + 24).sum::<usize>()
    }

    /// THE SAME QUERY the columnar stores answer, on rows: ran a check, verdict not Ok.
    /// Exists so every layout can be asked the same question and the answers cross-asserted —
    /// the assertion that caught the NoMatch encoding bug lives on exactly this seam.
    pub fn ran_and_not_ok(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.ran == Trit::Yes && r.verdict != Trit::Yes)
            .count()
    }
}

// ---- Interconversion: AoS <-> SoA, both directions, index-preserving --------------------------
//
// Conversion is a full pass over the data (measured ~100 plane-scans of cost for the packed
// forms; see cost.rs). These are deliberately plain loops: the cost model says conversion happens
// once per layout decision, so the clarity of an obvious loop is worth more than a clever one.

impl From<&EventColumns> for EventRows {
    fn from(c: &EventColumns) -> EventRows {
        let mut rows = Vec::with_capacity(c.len());
        for i in 0..c.len() {
            rows.push(EventRow {
                ran: c.ran.get(i),
                verdict: c.verdict.get(i),
                matched: c.matched.get(i),
                kind: c.kind[i],
                target: c.target[i],
            });
        }
        EventRows { rows, targets: c.targets.clone() }
    }
}

impl From<&EventRows> for EventColumns {
    fn from(r: &EventRows) -> EventColumns {
        let n = r.rows.len();
        let mut ran = Trits::with_len(n);
        let mut verdict = Trits::with_len(n);
        let mut matched = Trits::with_len(n);
        let mut kind = Vec::with_capacity(n);
        let mut target = Vec::with_capacity(n);
        for (i, row) in r.rows.iter().enumerate() {
            ran.set(i, row.ran);
            verdict.set(i, row.verdict);
            matched.set(i, row.matched);
            kind.push(row.kind);
            target.push(row.target);
        }
        EventColumns::from_parts(ran, verdict, matched, kind, target, r.targets.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::RawEvent;

    fn corpus() -> Vec<RawEvent> {
        // All five outcome states present, so the round-trip is exercised over the full
        // cardinality — a fixture missing a state would let a merged encoding pass unseen.
        let mk = |o: Option<&str>, k: Option<&str>, t: Option<&str>| RawEvent {
            kind: k.map(String::from),
            target: t.map(String::from),
            outcome: o.map(String::from),
            host: None,
            seq: None,
        };
        vec![
            mk(None, Some("stage_enter"), Some("a")),
            mk(Some("ok"), Some("report"), Some("b")),
            mk(Some("fail"), Some("report"), Some("a")),
            mk(Some("cannot_tell"), Some("stage_exit"), None),
            mk(Some("no_match"), None, Some("c")),
            mk(None, None, None),
            mk(Some("ok"), Some("report_origin"), Some("b")),
        ]
    }

    #[test]
    fn aos_and_soa_answer_the_query_identically() {
        let cols = EventColumns::build(&corpus());
        let rows = EventRows::from(&cols);
        assert_eq!(
            rows.ran_and_not_ok(),
            cols.ran_and_not_ok(),
            "the two layouts disagree on the same data — one of them is lying"
        );
    }

    #[test]
    fn conversion_round_trips_every_field() {
        let cols = EventColumns::build(&corpus());
        let rows = EventRows::from(&cols);
        let back = EventColumns::from(&rows);
        assert_eq!(back.len(), cols.len());
        for i in 0..cols.len() {
            assert_eq!(back.ran.get(i), cols.ran.get(i), "ran diverged at {}", i);
            assert_eq!(back.verdict.get(i), cols.verdict.get(i), "verdict diverged at {}", i);
            assert_eq!(back.matched.get(i), cols.matched.get(i), "matched diverged at {}", i);
            assert_eq!(back.kind[i], cols.kind[i], "kind diverged at {}", i);
            assert_eq!(back.target[i], cols.target[i], "target diverged at {}", i);
        }
        assert_eq!(back.targets, cols.targets, "target vocabulary must survive the round trip");
    }

    #[test]
    fn row_footprint_counts_padding_honestly() {
        // If EventRow ever grows and the padding changes, this stops being 8 and the footprint
        // arithmetic in every benchmark shifts with it — better a test fails than a benchmark lies.
        assert_eq!(std::mem::size_of::<EventRow>(), 8);
    }
}
