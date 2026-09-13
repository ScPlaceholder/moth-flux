//! SCRATCH (independent review). All three arms as #[inline(never)] wrappers called BY NAME.
//! Every arm is now a real function with `&Planes` parameters, so noalias/dereferenceable is
//! applied identically to all three and no black_box sits inside any hot loop.
use moth_flux::event::Outcome;
use moth_flux::flux::flux;
use moth_flux::planes::ran_and_not_ok;
use moth_flux::Planes;
use std::hint::black_box;
use std::time::Instant;

fn build(len: usize) -> (Planes, Planes) {
    let dist = [(Outcome::NothingRan,34_197usize),(Outcome::Ok,6_743),(Outcome::Fail,74),
                (Outcome::CannotTell,250),(Outcome::NoMatch,19)];
    let (mut ran, mut verdict) = (Planes::with_len(len), Planes::with_len(len));
    let mut i=0usize;
    'o: while i<len { for &(o,n) in &dist { for _ in 0..n {
        if i>=len {break 'o;} let (r,v,_m)=o.to_trits(); ran.set(i,r); verdict.set(i,v); i+=1; }}}
    (ran, verdict)
}

#[inline(never)]
fn w_lib(ran: &Planes, verdict: &Planes, len: usize) -> usize { ran_and_not_ok(ran, verdict, len) }

#[inline(never)]
fn w_local(ran: &Planes, verdict: &Planes, len: usize) -> usize {
    let mut n = 0u32;
    let words = (len + 63) / 64;
    for w in 0..words {
        let ran_yes = ran.known[w] & ran.value[w];
        let verdict_yes = verdict.known[w] & verdict.value[w];
        let mut hit = ran_yes & !verdict_yes;
        if w == words - 1 && len % 64 != 0 { hit &= (1u64 << (len % 64)) - 1; }
        n += hit.count_ones();
    }
    n as usize
}

#[inline(never)]
fn w_flux(a: &Planes, b: &Planes) -> usize { (flux!{ count(a == YES and b != YES) }).value }

fn med(v:&mut Vec<f64>)->f64{v.sort_by(|a,b|a.partial_cmp(b).unwrap());v[v.len()/2]}

fn main(){
    let sizes=[41_980usize,1_000_000,10_000_000];
    let reps=51;
    println!("OUT-OF-LINE — all three arms #[inline(never)], called by name, rotating order");
    println!("{:>10}  {:>12}  {:>12}  {:>12}  {:>12}  {:>12}  {:>8}",
             "rows","w_lib us","w_local us","w_flux us","flux/lib","flux/local","count");
    for &len in &sizes{
        let (ran,verdict)=build(len);
        let inner:usize=if len<=100_000 {256} else if len<=2_000_000 {16} else {2};
        let f=inner as f64;
        let (mut l,mut lo,mut fx)=(vec![],vec![],vec![]);
        let (mut r1,mut r2)=(vec![],vec![]);
        let mut seen=0usize;
        for rep in 0..reps{
            let (mut t1,mut t2,mut t3)=(0.0,0.0,0.0);
            let (mut a1,mut a2,mut a3)=(0usize,0usize,0usize);
            for slot in 0..3 { match (slot+rep)%3 {
                0=>{let t=Instant::now();
                    for _ in 0..inner {let a=black_box(&ran);let b=black_box(&verdict);let n=black_box(len);
                                       a1=black_box(w_lib(a,b,n));}
                    t1=t.elapsed().as_secs_f64()*1e6/f;}
                1=>{let t=Instant::now();
                    for _ in 0..inner {let a=black_box(&ran);let b=black_box(&verdict);let n=black_box(len);
                                       a2=black_box(w_local(a,b,n));}
                    t2=t.elapsed().as_secs_f64()*1e6/f;}
                _=>{let t=Instant::now();
                    for _ in 0..inner {let a=black_box(&ran);let b=black_box(&verdict);
                                       a3=black_box(w_flux(a,b));}
                    t3=t.elapsed().as_secs_f64()*1e6/f;}
            }}
            assert_eq!(a1,a2); assert_eq!(a1,a3); seen=a1;
            l.push(t1); lo.push(t2); fx.push(t3);
            r1.push(t3/t1.max(1e-9)); r2.push(t3/t2.max(1e-9));
        }
        println!("{:>10}  {:>12.2}  {:>12.2}  {:>12.2}  {:>12.3}  {:>12.3}  {:>8}",
                 len,med(&mut l),med(&mut lo),med(&mut fx),med(&mut r1),med(&mut r2),seen);
    }
    println!("== outline COMPLETE (rc=0) ==");
}
