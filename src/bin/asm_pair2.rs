//! SCRATCH (independent review). Same two symbols, but the hand arm is a LOCAL copy of
//! ran_and_not_ok's body rather than a cross-crate call. Isolates "is the vectorisation a
//! property of the loop, or of the LTO/inlining path?"
use moth_flux::flux::flux;
use moth_flux::Planes;

#[no_mangle]
#[inline(never)]
pub fn q_hand(ran: &Planes, verdict: &Planes, len: usize) -> usize {
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

#[no_mangle]
#[inline(never)]
pub fn q_flux(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

fn main() {
    let ran = Planes::with_len(1024);
    let verdict = Planes::with_len(1024);
    let n = ran.len();
    println!("{} {}", q_hand(&ran, &verdict, n), q_flux(&ran, &verdict));
}
