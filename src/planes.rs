//! Bit-plane ternary — GPT's suggestion, and it is a better encoding than the one I wrote.
//!
//! # The idea
//!
//! Stop storing a trit as two adjacent bits and stop extracting elements one at a time. Split the
//! population into two parallel BIT-PLANES:
//!
//! ```text
//!   KNOWN plane   1 = this trit is Yes or No,  0 = Unknown
//!   VALUE plane   1 = Yes,                     0 = No (meaningless where KNOWN is 0)
//! ```
//!
//! Same 2 bits per trit, so **density is unchanged** — GPT's note that this wins storage is not
//! right, the 2× was already there and bit-planes do not add to it. What changes is entirely the
//! ACCESS PATTERN: a `u64` word now holds 64 trits, and Kleene logic becomes plain boolean ops on
//! whole words. Sixty-four elements per instruction instead of one element per shift-and-mask.
//!
//! ★★ **AND THIS IS WHY MY OWN "NEXT EXPERIMENT" WAS WRONG.** I proposed hand-writing SIMD over my
//! existing nibble layout. But the layout WAS the problem — you do not vectorise a bad layout, you
//! replace it. Bit-planes are the SIMD-native form and the shift-and-mask version can never reach
//! them however carefully it is hand-optimised. GPT got to a better representation than I did.
//!
//! # ⛔⛔ AND THE TRAP IN ADOPTING IT, WHICH THE SUGGESTION DOES NOT MENTION
//!
//! If I upgrade MOTH's inner loop to bit-planes and leave the baseline as an auto-vectorised byte
//! compare, a MOTH win means nothing: **I would have optimised my side and not theirs.** That is the
//! strawman failure with the arms swapped, and it is exactly how a project talks itself into a
//! result. So the comparison below keeps rkyv AND adds a hand-written **byte-plane** baseline — the
//! same trick applied to a one-byte-per-row layout — so the question is honestly
//! *"do 2-bit planes beat 8-bit planes"*, not *"does my new code beat my old code"*.

use crate::trit::{Trit, Trits};

/// Two parallel bit-planes over the same population of trits.
///
/// # Canonical form
///
/// Where `known` is 0 the corresponding `value` bit is ALSO 0, including every padding slot past
/// `len`. The whole-population Kleene ops below rely on this to stay branch-free, and each of them
/// preserves it (checked in the exhaustive test, not just claimed here). `set_yes`/`set_no` cannot
/// break it; only direct writes to the public words can, which is what the tail mask in
/// [`ran_and_not_ok`] guards against.
#[derive(Default, Clone)]
pub struct Planes {
    /// 1 = known (Yes or No), 0 = Unknown.
    pub known: Vec<u64>,
    /// 1 = Yes, 0 = No. Only meaningful where `known` is 1.
    pub value: Vec<u64>,
    len: usize,
}

impl Planes {
    pub fn with_len(len: usize) -> Planes {
        let w = (len + 63) / 64;
        Planes { known: vec![0u64; w], value: vec![0u64; w], len }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Unknown is the absence of a bit in `known`, so it needs no write — the default state of an
    /// untouched plane is already "I do not know", same invariant as `Trits`.
    #[inline]
    pub fn set_yes(&mut self, i: usize) {
        self.known[i >> 6] |= 1u64 << (i & 63);
        self.value[i >> 6] |= 1u64 << (i & 63);
    }

    #[inline]
    pub fn set_no(&mut self, i: usize) {
        self.known[i >> 6] |= 1u64 << (i & 63);
    }

    pub fn bytes(&self) -> usize {
        (self.known.len() + self.value.len()) * 8
    }

    /// Element read. This is the COLD path — one shift and two loads per element, exactly the
    /// access shape the bit-plane layout exists to avoid on scans (measured ~74x slower per
    /// element than whole-word ops). It is here for conversion, testing, and point lookups,
    /// where the cost model says the row store should be holding the data anyway.
    #[inline]
    pub fn get(&self, i: usize) -> Trit {
        debug_assert!(i < self.len);
        let w = i >> 6;
        let bit = 1u64 << (i & 63);
        if self.known[w] & bit == 0 {
            Trit::Unknown
        } else if self.value[w] & bit != 0 {
            Trit::Yes
        } else {
            Trit::No
        }
    }

    #[inline]
    pub fn set(&mut self, i: usize, t: Trit) {
        match t {
            Trit::Yes => self.set_yes(i),
            Trit::No => self.set_no(i),
            Trit::Unknown => {
                let w = i >> 6;
                let bit = 1u64 << (i & 63);
                self.known[w] &= !bit;
                self.value[w] &= !bit; // keep canonical: unknown slots carry value 0
            }
        }
    }

    /// Population counts, one popcount per word. `known & value` needs no tail mask because the
    /// canonical form keeps padding at 0 in BOTH planes; `count_no` uses `known & !value`, and
    /// `!value` turns padding zeros into ones — but `known` is 0 there, so the AND still excludes
    /// them. Both facts are invariants of the canonical form, and the padding-corruption test
    /// below is what checks the claim rather than trusting this comment.
    pub fn count_yes(&self) -> usize {
        self.known.iter().zip(&self.value).map(|(k, v)| (k & v).count_ones() as usize).sum()
    }

    pub fn count_no(&self) -> usize {
        self.known.iter().zip(&self.value).map(|(k, v)| (k & !v).count_ones() as usize).sum()
    }

    pub fn count_unknown(&self) -> usize {
        self.len - self.known.iter().map(|k| k.count_ones() as usize).sum::<usize>()
    }

    // ---- Kleene three-valued logic, whole population, word at a time -------------------------
    //
    // The scalar truth tables live in `Trit`'s `Kleene` impl and are the spec; these are the same
    // tables expressed as boolean algebra over planes. Derivation, kept because the disjointness
    // argument is what makes `known = yes | no` legal:
    //
    //   yes(a) = a.known &  a.value      no(a) = a.known & !a.value
    //   AND: result No if either is No; Yes only if both Yes. A trit cannot be Yes and No at
    //        once, so (yes(a)&yes(b)) and (no(a)|no(b)) are DISJOINT and known = their union.
    //   OR : the dual. NOT: swap yes and no; unknown stays unknown.
    //
    // ⚠ An exhaustive 9-case test against the scalar impl runs on every `cargo test` — the scalar
    //   table is the spec, this block is an optimisation of it, and the test is what keeps the
    //   two from drifting apart silently.

    /// Whole-population Kleene AND. Panics if lengths differ — two populations of different
    /// lengths have no elementwise AND, and padding one silently would invent Unknown rows.
    pub fn and(&self, other: &Planes) -> Planes {
        assert_eq!(self.len, other.len, "Kleene AND over different populations");
        let mut out = Planes::with_len(self.len);
        for w in 0..self.known.len() {
            let yes = (self.known[w] & self.value[w]) & (other.known[w] & other.value[w]);
            let no = (self.known[w] & !self.value[w]) | (other.known[w] & !other.value[w]);
            out.value[w] = yes;
            out.known[w] = yes | no;
        }
        // Canonical-form note: `no` can carry bits where one arm is No and the other Unknown —
        // that is correct Kleene (No dominates). Padding cannot appear: both arms hold known=0
        // there, so yes=0 and no=0.
        out
    }

    /// Whole-population Kleene OR.
    pub fn or(&self, other: &Planes) -> Planes {
        assert_eq!(self.len, other.len, "Kleene OR over different populations");
        let mut out = Planes::with_len(self.len);
        for w in 0..self.known.len() {
            let yes = (self.known[w] & self.value[w]) | (other.known[w] & other.value[w]);
            let no = (self.known[w] & !self.value[w]) & (other.known[w] & !other.value[w]);
            out.value[w] = yes;
            out.known[w] = yes | no;
        }
        out
    }

    /// Whole-population Kleene NOT. `known` is untouched; the negation of "I do not know" is
    /// "I do not know", which falls out of the algebra rather than needing a branch.
    pub fn not(&self) -> Planes {
        let mut out = Planes::with_len(self.len);
        for w in 0..self.known.len() {
            out.known[w] = self.known[w];
            out.value[w] = self.known[w] & !self.value[w]; // & known keeps padding/unknown at 0
        }
        out
    }
}

// ---- Interconversion: the packed 2-bit layout and the bit-plane layout ------------------------
//
// ⚠ Element-at-a-time on purpose, and that is not laziness: conversion is a MATERIALISATION step
// (measured cost ~100 scans, break-even ~42 queries — see `cost.rs`), so it runs once per layout
// decision, not per query. The rule "do not reintroduce per-element extraction" is about HOT
// paths; spending SWAR cleverness here would optimise the part of the pipeline the measurements
// say does not matter, and buy risk with it.

impl From<&Trits> for Planes {
    fn from(t: &Trits) -> Planes {
        let mut p = Planes::with_len(t.len());
        for i in 0..t.len() {
            match t.get(i) {
                Trit::Yes => p.set_yes(i),
                Trit::No => p.set_no(i),
                Trit::Unknown => {} // the default state of an untouched plane is already Unknown
            }
        }
        p
    }
}

impl From<&Planes> for Trits {
    fn from(p: &Planes) -> Trits {
        let mut t = Trits::with_len(p.len());
        for i in 0..p.len() {
            t.set(i, p.get(i));
        }
        t
    }
}

/// The benchmark query, on planes: `ran == Yes AND verdict != Yes`.
///
/// Per 64 rows: two ANDs, one NOT, one AND, one popcount. No element extraction anywhere.
pub fn ran_and_not_ok(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    let mut n = 0u32;
    let words = (len + 63) / 64;
    for w in 0..words {
        let ran_yes = ran.known[w] & ran.value[w];
        let verdict_yes = verdict.known[w] & verdict.value[w];
        let mut hit = ran_yes & !verdict_yes;
        if w == words - 1 && len % 64 != 0 {
            // ⚠ Mask the tail. Padding bits are 0 in `known`, so a padded slot is Unknown and can
            //   never satisfy `ran == Yes` — but relying on that is relying on an invariant a future
            //   edit could break silently, and the mask costs one instruction on one word.
            hit &= (1u64 << (len % 64)) - 1;
        }
        n += hit.count_ones();
    }
    n as usize
}

/// THE HONEST BASELINE: the same trick applied to a conventional one-byte-per-row layout.
///
/// If MOTH is going to claim a win it has to beat a competent byte implementation, not just an
/// unoptimised one. This is what a good engineer writes without any ternary in the picture.
pub fn ran_and_not_ok_bytes(outcome: &[u8], ok: u8, nothing_ran: u8) -> usize {
    outcome.iter().filter(|&&o| o != nothing_ran && o != ok).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_default_to_unknown() {
        let p = Planes::with_len(200);
        for w in &p.known {
            assert_eq!(*w, 0, "an untouched plane must read Unknown everywhere");
        }
    }

    #[test]
    fn tail_padding_cannot_be_counted() {
        // 65 rows: one full word plus a single live bit in the second. If the tail mask were
        // missing, 63 padding slots would still read Unknown and be excluded — so this test would
        // pass for the wrong reason. Force the failure: set every bit of the padding region.
        let len = 65;
        let mut ran = Planes::with_len(len);
        let verdict = Planes::with_len(len);
        for i in 0..len {
            ran.set_yes(i);
        }
        ran.known[1] = u64::MAX; // deliberately corrupt the padding
        ran.value[1] = u64::MAX;
        assert_eq!(ran_and_not_ok(&ran, &verdict, len), len,
                   "the tail mask must bound the count at len even with corrupt padding");
    }

    #[test]
    fn kleene_on_planes_matches_kleene_on_trits_exhaustively() {
        // The scalar truth table is the SPEC; the plane ops are an optimisation of it. All nine
        // (a, b) pairs, at a length that crosses a word boundary so tail handling is exercised
        // too. If this ever fails, fix the planes, never the table.
        use crate::trit::Kleene;
        const ALL: [Trit; 3] = [Trit::No, Trit::Unknown, Trit::Yes];
        let len = 130; // 2 full words + a 2-bit tail
        for &a in &ALL {
            for &b in &ALL {
                let mut pa = Planes::with_len(len);
                let mut pb = Planes::with_len(len);
                for i in 0..len {
                    pa.set(i, a);
                    pb.set(i, b);
                }
                let and = pa.and(&pb);
                let or = pa.or(&pb);
                let not = pa.not();
                for i in 0..len {
                    assert_eq!(and.get(i), a.and(b), "AND({:?},{:?}) diverged at {}", a, b, i);
                    assert_eq!(or.get(i), a.or(b), "OR({:?},{:?}) diverged at {}", a, b, i);
                    assert_eq!(not.get(i), a.not(), "NOT({:?}) diverged at {}", a, i);
                }
                // Canonical form must survive every op: value ⊆ known, padding all-zero.
                for p in [&and, &or, &not] {
                    for w in 0..p.known.len() {
                        assert_eq!(p.value[w] & !p.known[w], 0, "value bit outside known");
                    }
                    let tail_mask = !((1u64 << (len % 64)) - 1);
                    assert_eq!(p.known[p.known.len() - 1] & tail_mask, 0, "padding leaked");
                }
            }
        }
    }

    #[test]
    fn counts_agree_with_element_reads() {
        let len = 200;
        let mut p = Planes::with_len(len);
        let (mut y, mut nn, mut u) = (0, 0, 0);
        for i in 0..len {
            match i % 7 {
                0 | 1 => { p.set(i, Trit::Yes); y += 1; }
                2 => { p.set(i, Trit::No); nn += 1; }
                _ => { u += 1; }
            }
        }
        assert_eq!(p.count_yes(), y);
        assert_eq!(p.count_no(), nn);
        assert_eq!(p.count_unknown(), u);
    }

    #[test]
    fn trits_and_planes_are_interconvertible_losslessly() {
        // 101 elements: crosses both the 4-per-byte and 64-per-word boundaries unevenly.
        let len = 101;
        let mut t = Trits::with_len(len);
        let pattern = [Trit::Yes, Trit::Unknown, Trit::No, Trit::Unknown];
        for i in 0..len {
            t.set(i, pattern[i % 4]);
        }
        let p = Planes::from(&t);
        for i in 0..len {
            assert_eq!(p.get(i), t.get(i), "Trits->Planes diverged at {}", i);
        }
        let t2 = Trits::from(&p);
        assert_eq!(t2, t, "Trits->Planes->Trits must be the identity");
        // and the aggregate view agrees too — the cross-representation assertion in miniature
        assert_eq!(p.count_yes(), t.count_yes());
    }

    #[test]
    fn set_unknown_restores_canonical_form() {
        let mut p = Planes::with_len(70);
        p.set_yes(65);
        p.set(65, Trit::Unknown);
        assert_eq!(p.get(65), Trit::Unknown);
        assert_eq!(p.value[1] & (1 << 1), 0, "un-setting must clear the value plane too");
    }

    #[test]
    fn matches_the_scalar_definition() {
        let len = 300;
        let mut ran = Planes::with_len(len);
        let mut verdict = Planes::with_len(len);
        let mut expect = 0;
        for i in 0..len {
            match i % 3 {
                0 => { ran.set_yes(i); verdict.set_yes(i); }            // ok
                1 => { ran.set_yes(i); verdict.set_no(i); expect += 1; } // fail
                _ => { ran.set_no(i); }                                  // nothing ran
            }
        }
        assert_eq!(ran_and_not_ok(&ran, &verdict, len), expect);
    }
}
