//! SCRATCH (independent review, 2026-09-13). Isolates WHICH of the three differences between
//! `planes::ran_and_not_ok` and the `flux!` expansion changes the emitted code.
//!
//! Three axes:
//!   len source : `len` PARAMETER            vs `ran.len()` derived inside
//!   accumulator: `u32` (+ `as usize` at end) vs `usize`
//!   tail test  : `w == words - 1`            vs `w + 1 == words`
//!
//! flux! uses: derived len, usize acc, `w + 1 == words`.
//! ran_and_not_ok uses: param len, u32 acc, `w == words - 1`.
//! Everything is `#[no_mangle] #[inline(never)]` in ONE crate so the codegen context is equal.
use moth_flux::flux::flux;
use moth_flux::Planes;

macro_rules! variant {
    ($name:ident, param, $acc:ty, $tail:tt) => {
        #[no_mangle]
        #[inline(never)]
        pub fn $name(ran: &Planes, verdict: &Planes, len: usize) -> usize {
            body!(ran, verdict, len, $acc, $tail)
        }
    };
    ($name:ident, derived, $acc:ty, $tail:tt) => {
        #[no_mangle]
        #[inline(never)]
        pub fn $name(ran: &Planes, verdict: &Planes) -> usize {
            let len = ran.len();
            body!(ran, verdict, len, $acc, $tail)
        }
    };
}

macro_rules! tailtest {
    (minus1, $w:expr, $words:expr) => {
        $w == $words - 1
    };
    (plus1, $w:expr, $words:expr) => {
        $w + 1 == $words
    };
}

macro_rules! body {
    ($ran:expr, $verdict:expr, $len:expr, $acc:ty, $tail:tt) => {{
        let mut n: $acc = 0;
        let words = ($len + 63) / 64;
        for w in 0..words {
            let ran_yes = $ran.known[w] & $ran.value[w];
            let verdict_yes = $verdict.known[w] & $verdict.value[w];
            let mut hit = ran_yes & !verdict_yes;
            if tailtest!($tail, w, words) && $len % 64 != 0 {
                hit &= (1u64 << ($len % 64)) - 1;
            }
            n += hit.count_ones() as $acc;
        }
        n as usize
    }};
}

// param len
variant!(v_p_u32_m1, param, u32, minus1); // == shipped ran_and_not_ok, exactly
variant!(v_p_u32_p1, param, u32, plus1);
variant!(v_p_usz_m1, param, usize, minus1);
variant!(v_p_usz_p1, param, usize, plus1);
// derived len
variant!(v_d_u32_m1, derived, u32, minus1);
variant!(v_d_u32_p1, derived, u32, plus1);
variant!(v_d_usz_m1, derived, usize, minus1);
variant!(v_d_usz_p1, derived, usize, plus1); // == the flux! shape, by hand

#[no_mangle]
#[inline(never)]
pub fn v_flux(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

fn main() {
    let ran = Planes::with_len(1024);
    let verdict = Planes::with_len(1024);
    let n = ran.len();
    println!(
        "{}{}{}{}{}{}{}{}{}",
        v_p_u32_m1(&ran, &verdict, n),
        v_p_u32_p1(&ran, &verdict, n),
        v_p_usz_m1(&ran, &verdict, n),
        v_p_usz_p1(&ran, &verdict, n),
        v_d_u32_m1(&ran, &verdict),
        v_d_u32_p1(&ran, &verdict),
        v_d_usz_m1(&ran, &verdict),
        v_d_usz_p1(&ran, &verdict),
        v_flux(&ran, &verdict)
    );
}
