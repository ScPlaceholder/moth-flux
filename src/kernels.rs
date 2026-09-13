//! Library-side kernels — an experiment, not yet a compiler target.
//!
//! ⛔ WHY THIS FILE EXISTS. An independent asm review on 2026-09-13 established:
//!     `planes::ran_and_not_ok` compiled in THIS crate      -> vectorised, 4 words/iter, movdqu
//!     the same body written in the CONSUMER crate          -> scalar, 1 word/iter
//!     a `flux!` expansion (always in the consumer crate)   -> scalar, identical to the above
//!   flux/localhand was 1.00 and flux/libhand was 2.34. So the lowering is innocent and the gap is
//!   compilation CONTEXT: LLVM finds the higher-level optimisation for library code reached by
//!   inlining, and not for the same source written at the call site.
//!
//! ★ THE HYPOTHESIS UNDER TEST, and it is falsifiable: if location is what matters, then moving the
//!   FLUX-SHAPED body into the library should recover the vectorisation, and `flux!` should
//!   eventually emit a call to something like this instead of pasting a loop into the caller.
//!
//! ⚠ THE RIVAL HYPOTHESIS THIS IS BUILT TO SEPARATE, which I would otherwise have missed: maybe the
//!   flux loop SHAPE (usize accumulator, self-derived len, `w+1 == words` tail test, one word per
//!   iteration) is what blocks vectorisation, and location is irrelevant. The reviewer's asm probe
//!   found that shape identical register-for-register to a hand-written equivalent — but every
//!   variant it tried was in the consumer crate, so shape and location were never varied
//!   independently. This file varies location while holding shape fixed.
//!
//!   library + flux shape   -> vectorises?   then LOCATION is the cause, and the fix is a call
//!   library + flux shape   -> still scalar? then SHAPE is the cause, and a call fixes nothing
//!
//! ⚠ NOT WIRED INTO THE MACRO. Nothing emits these. `flux_macro` is untouched. This is a
//!   measurement, and until it has an answer it must not become an architecture.
use crate::Planes;

/// `count(ran == YES and verdict != YES)` written in the EXACT shape `flux!` emits, but living in
/// the library. Same arithmetic and same tail handling as `planes::ran_and_not_ok`; deliberately
/// NOT the same loop form, because the loop form is the variable being held fixed.
pub fn count_and_not_yes_fluxshape(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    let words = (len + 63) / 64;
    let mut count: usize = 0;
    for w in 0..words {
        let ik0 = ran.known[w];
        let iv0 = ran.value[w];
        let ik1 = verdict.known[w];
        let iv1 = verdict.value[w];
        let m0 = ik0 & iv0;
        let m1 = !(ik1 & iv1);
        let mut hit = m0 & m1;
        if w + 1 == words && (len % 64) != 0 {
            hit &= (1u64 << (len % 64)) - 1;
        }
        count += hit.count_ones() as usize;
    }
    count
}
