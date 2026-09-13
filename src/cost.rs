//! The measured cost model. Every constant here was EARNED on this machine, on real or
//! distribution-faithful data, with the equality assertions passing. Nothing in this file is a
//! guess dressed as a number; where the model is calibration rather than mechanism, it says so.
//!
//! # The four facts (measured 2026-09-13, `adversarial.rs` + `main.rs`)
//!
//! 1. **Density ceiling ~4x.** Once BOTH arms are bandwidth-bound, packed 2-bit planes beat a
//!    byte column by exactly the bit ratio: measured 3.43–4.08x across 41,980 -> 200,000,000
//!    rows. Every larger number seen that night (23x, 16x, 8.8x) was measuring a weak baseline,
//!    not MOTH. The ceiling is the DENSITY RATIO and cannot be exceeded by cleverness on the
//!    same memory system.
//! 2. **Out of cache, columnar cost = (arrays touched) x (row cost).** Point-lookup penalty
//!    2.48x, multi-field 3.49x — both predicted from counting cache misses BEFORE measuring,
//!    both stable to 1–2% CV.
//! 3. **Conversion costs ~100 scans; break-even ~42 queries of the same shape.** So layout is a
//!    MATERIALISATION decision, made once and amortised — not a per-query one.
//! 4. **Bit-planes beat shift-and-mask by ~74x.** Changing the layout beat optimising the
//!    access. (This is why there is no "fast path" through `Trits::get` anywhere.)

/// Fact 1: the scan-speedup ceiling for 2-bit planes over a byte column. Measured range
/// 3.43–4.08; the ceiling is the bit ratio 8/2.
pub const DENSITY_CEILING: f64 = 4.0;

/// Fact 2, calibration: measured point-lookup penalty, columnar (2 trit columns as 4 plane
/// arrays) vs one row-store cache line. CV 1–2% across five runs.
pub const POINT_LOOKUP_PENALTY: f64 = 2.48;

/// Fact 2, calibration: measured multi-field penalty (4 columnar arrays + kind + target vs one
/// 8-byte row).
pub const MULTI_FIELD_PENALTY: f64 = 3.49;

/// Fact 3: converting a byte column into bit-planes costs about this many plane-scans.
pub const CONVERSION_COST_SCANS: f64 = 100.0;

/// How a workload touches the data. The model deliberately has THREE cases and no more —
/// each one corresponds to a measurement that exists. A case without a measurement behind it
/// would be a guess wearing the same units as the facts around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Access {
    /// Full sequential pass over the queried columns.
    SequentialScan,
    /// Random single-record reads of the queried columns.
    PointLookup,
    /// Random single-record reads touching EVERY field of the record.
    MultiField,
}

/// Which layout holds the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repr {
    /// AoS: one struct per row, one cache line per record.
    RowStore,
    /// SoA with ternary fields as bit-planes.
    ColumnarPlanes,
}

/// Relative cost of a workload on a layout, in arbitrary units comparable ONLY within one
/// (rows, access) pair. Mechanism per fact 2: out of cache, cost is proportional to the number
/// of distinct arrays an access must touch — i.e. to cache misses, not instructions.
///
/// `arrays_touched` is for the COLUMNAR side (the row store touches 1 line by construction).
/// The scan case uses bytes moved instead, because a scan is bandwidth-bound, not miss-bound —
/// that is the regime boundary fact 1 and fact 2 sit on either side of.
///
/// ⚠ SCOPE, stated so this cannot be quietly over-applied: calibrated at 100k probes over
/// populations up to 200M rows on this machine. In-cache workloads (fact 1's small end) show
/// gaps SMALLER than these; the model is an out-of-cache model and says nothing else.
pub fn relative_cost(
    repr: Repr,
    access: Access,
    arrays_touched: usize,
    bytes_per_row: f64,
) -> f64 {
    match (repr, access) {
        // Bandwidth-bound: cost ~ bytes moved. 2-bit planes move bytes/4 per queried byte-field.
        (Repr::RowStore, Access::SequentialScan) => bytes_per_row,
        (Repr::ColumnarPlanes, Access::SequentialScan) => bytes_per_row / DENSITY_CEILING,
        // Miss-bound: cost ~ arrays touched. The row store is one line regardless of field count.
        (Repr::RowStore, Access::PointLookup | Access::MultiField) => 1.0,
        (Repr::ColumnarPlanes, Access::PointLookup | Access::MultiField) => arrays_touched as f64,
    }
}

/// The measured penalties, exposed as the calibration the raw array count over-predicts.
///
/// ★ HONEST GAP IN THE MECHANISM, left visible instead of smoothed over: counting arrays says
/// the point-lookup penalty "should" be 4 (known+value for two columns) and multi-field ~6; the
/// measured values are 2.48 and 3.49. The MECHANISM (more arrays -> more misses -> higher cost,
/// ordering preserved) is confirmed; the CONSTANT of proportionality is not 1.0 per array. Use
/// `relative_cost` for ordering decisions and these calibrations for magnitude claims. Do not
/// re-derive the gap from theory — measure it if it starts to matter.
pub fn measured_penalty(access: Access) -> Option<f64> {
    match access {
        Access::SequentialScan => None, // that regime's number is DENSITY_CEILING, a speedUP
        Access::PointLookup => Some(POINT_LOOKUP_PENALTY),
        Access::MultiField => Some(MULTI_FIELD_PENALTY),
    }
}

/// Fact 3 arithmetic: after how many same-shape queries does converting to planes pay for
/// itself? Conversion costs ~100 plane-scans; each converted query saves (speedup - 1)
/// plane-scans relative to running on bytes.
///
/// At the measured conservative speedup 3.43x this gives 41.2 — the "~42 queries" figure.
/// At the top of the measured range (4.08x) it gives 32.5. Returns `None` when the speedup is
/// <= 1: a conversion that buys nothing has no break-even, and returning infinity would invite
/// arithmetic on a fiction.
pub fn break_even_queries(speedup: f64) -> Option<f64> {
    if speedup <= 1.0 {
        return None;
    }
    Some(CONVERSION_COST_SCANS / (speedup - 1.0))
}

// ---- The representation selector that DOES NOT EXIST yet --------------------------------------

/// ⛔ DO NOT BUILD THE SELECTOR. This trait is a placeholder with one hardcoded policy, and that
/// is deliberate, not unfinished.
///
/// An automatic selector needs to know the QUERY MIX — how many scans vs lookups vs multi-field
/// reads this data will actually serve — and that workload census does not exist yet. A selector
/// built now would be this crate hardcoding its own benchmark as everyone's workload, which is
/// exactly the "weak baseline laundered into a win" failure with a policy engine wrapped around
/// it. Fact 3 already tells us the decision is a materialisation decision made ~once; a human
/// reading the cost model makes it better than a heuristic guessing the census.
///
/// When a real census exists, implement this trait against it and delete this paragraph.
pub trait ReprPolicy {
    fn choose(&self, rows: usize, access: Access) -> Repr;
}

/// The one policy that exists: what the measurements justify for MOTH telemetry today —
/// analytics-shaped data, scan-dominated, converted once at ingest.
pub struct ScanDominatedIngest;

impl ReprPolicy for ScanDominatedIngest {
    fn choose(&self, _rows: usize, access: Access) -> Repr {
        match access {
            Access::SequentialScan => Repr::ColumnarPlanes,
            // The measured 2.48x / 3.49x say random access belongs to the row store.
            Access::PointLookup | Access::MultiField => Repr::RowStore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_preserves_the_measured_ordering() {
        // The model's job is ORDERING, and the ordering must match what was measured:
        // planes win scans, rows win random access, multi-field is worse for columnar than
        // point lookup (more arrays).
        let scan_row = relative_cost(Repr::RowStore, Access::SequentialScan, 1, 1.0);
        let scan_col = relative_cost(Repr::ColumnarPlanes, Access::SequentialScan, 4, 1.0);
        assert!(scan_col < scan_row, "planes must win the scan regime");

        let lk_row = relative_cost(Repr::RowStore, Access::PointLookup, 1, 1.0);
        let lk_col = relative_cost(Repr::ColumnarPlanes, Access::PointLookup, 4, 1.0);
        assert!(lk_row < lk_col, "rows must win point lookup");

        let mf_col = relative_cost(Repr::ColumnarPlanes, Access::MultiField, 6, 1.0);
        assert!(mf_col > lk_col, "more arrays touched must never cost less");
    }

    #[test]
    fn scan_speedup_is_capped_at_the_density_ratio() {
        let row = relative_cost(Repr::RowStore, Access::SequentialScan, 1, 1.0);
        let col = relative_cost(Repr::ColumnarPlanes, Access::SequentialScan, 4, 1.0);
        let speedup = row / col;
        assert!((speedup - DENSITY_CEILING).abs() < 1e-9,
            "the model must not promise more than the bit ratio; measured max was 4.08");
    }

    #[test]
    fn break_even_reproduces_the_measured_figure() {
        // Fact 3: ~42 queries at the conservative measured speedup.
        let be = break_even_queries(3.43).unwrap();
        assert!((be - 41.2).abs() < 0.1, "3.43x should break even near 41 queries, got {}", be);
        let be_top = break_even_queries(4.08).unwrap();
        assert!(be_top > 30.0 && be_top < 34.0);
        assert_eq!(break_even_queries(1.0), None, "no gain, no break-even — not infinity");
        assert_eq!(break_even_queries(0.5), None, "a slowdown must not produce a break-even");
    }

    #[test]
    fn calibration_constants_stay_below_the_raw_array_count() {
        // The documented honest gap: if someone "fixes" the calibration to equal the array
        // count, they are re-deriving from theory what was settled by measurement. Fail them.
        assert!(POINT_LOOKUP_PENALTY < 4.0);
        assert!(MULTI_FIELD_PENALTY < 6.0);
        assert!(MULTI_FIELD_PENALTY > POINT_LOOKUP_PENALTY);
    }

    #[test]
    fn the_one_policy_matches_the_measurements() {
        let p = ScanDominatedIngest;
        assert_eq!(p.choose(1_000_000, Access::SequentialScan), Repr::ColumnarPlanes);
        assert_eq!(p.choose(1_000_000, Access::PointLookup), Repr::RowStore);
        assert_eq!(p.choose(1_000_000, Access::MultiField), Repr::RowStore);
    }
}
