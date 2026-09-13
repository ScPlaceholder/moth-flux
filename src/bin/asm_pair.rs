//! SCRATCH (independent review). Minimal two-symbol file: the SHIPPED library function reached
//! through a wrapper, next to the flux! expansion. Nothing else in the crate to perturb inlining.
use moth_flux::flux::flux;
use moth_flux::planes::ran_and_not_ok;
use moth_flux::Planes;

#[no_mangle]
#[inline(never)]
pub fn p_hand(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    ran_and_not_ok(ran, verdict, len)
}

#[no_mangle]
#[inline(never)]
pub fn p_flux(a: &Planes, b: &Planes) -> usize {
    (flux! { count(a == YES and b != YES) }).value
}

fn main() {
    let ran = Planes::with_len(1024);
    let verdict = Planes::with_len(1024);
    let n = ran.len();
    println!("{} {}", p_hand(&ran, &verdict, n), p_flux(&ran, &verdict));
}
