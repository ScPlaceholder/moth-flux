//! SCRATCH (independent review, 2026-09-13). Emits three symbols so their hot loops can be
//! diffed instruction-by-instruction. Delete when done.
//!
//! Three arms, not two, deliberately:
//!   asm_hand         - planes::ran_and_not_ok exactly as shipped (len is a PARAMETER)
//!   asm_hand_selflen - the same loop but len derived from ran.len(), matching what the macro
//!                      does. Isolates "is any difference just the length source?"
//!   asm_flux         - the flux! expansion
use moth_flux::flux::flux;
use moth_flux::planes::ran_and_not_ok;
use moth_flux::Planes;

#[no_mangle]
#[inline(never)]
pub fn asm_hand(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    ran_and_not_ok(ran, verdict, len)
}

#[no_mangle]
#[inline(never)]
pub fn asm_hand_selflen(ran: &Planes, verdict: &Planes) -> usize {
    // byte-for-byte the body of ran_and_not_ok, with len taken from the plane like flux! does
    let len = ran.len();
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

/// Same as above but with a usize accumulator, to isolate the u32-vs-usize accumulator question.
#[no_mangle]
#[inline(never)]
pub fn asm_hand_usize_acc(ran: &Planes, verdict: &Planes) -> usize {
    let len = ran.len();
    let mut n = 0usize;
    let words = (len + 63) / 64;
    for w in 0..words {
        let ran_yes = ran.known[w] & ran.value[w];
        let verdict_yes = verdict.known[w] & verdict.value[w];
        let mut hit = ran_yes & !verdict_yes;
        if w + 1 == words && len % 64 != 0 {
            hit &= (1u64 << (len % 64)) - 1;
        }
        n += hit.count_ones() as usize;
    }
    n
}

#[no_mangle]
#[inline(never)]
pub fn asm_flux(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

fn main() {
    let ran = Planes::with_len(1024);
    let verdict = Planes::with_len(1024);
    let n = ran.len();
    println!(
        "{} {} {} {}",
        asm_hand(&ran, &verdict, n),
        asm_hand_selflen(&ran, &verdict),
        asm_hand_usize_acc(&ran, &verdict),
        asm_flux(&ran, &verdict)
    );
}
