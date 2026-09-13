//! flux_macro — the FLUX compiler, as a single function-like proc-macro.
//!
//! # What compiles and what refuses
//!
//! FLUX has exactly two value kinds and the boundary between them is the whole language:
//!
//! - **Plane** — a named trit-plane input (`ran`, `verdict`, ...), or a Kleene combination of
//!   planes (`a and b`, `not a`, `a xor b`), or `mask(...)` / `select(...)`.
//! - **Mask** — one bit per row, produced ONLY by a comparison (`p == YES`, `p != UNKNOWN`) or a
//!   boolean combination of masks.
//!
//! A comparison is the plane-level mirror of `Trit::resolve`: it is the single door from ternary
//! to binary, and writing it forces the author to say what Unknown means at that crossing
//! (`p == YES` — Unknown is a miss; `p != NO` — Unknown is a hit). There is no other door.
//! Mixing a plane and a mask under `and`/`or`/`xor`, counting a bare plane, or returning a bare
//! mask are all COMPILE ERRORS — never a coercion, never a silent materialization.
//!
//! # The lowering
//!
//! Every accepted expression compiles to ONE fused pass, word-at-a-time — the same loop shape as
//! `planes::ran_and_not_ok`: per 64 rows, a handful of `&`/`|`/`^`/`!` on `u64`s, a tail mask on
//! the final word only, then either a popcount (`count(...)`) or two stores (plane result).
//! No per-element extraction, no intermediate `Planes` allocations for subexpressions, no
//! dispatch at runtime. The witness string embedded in the result is generated from the SAME
//! in-memory op list that generates the code, so the two cannot drift independently.
//!
//! Every semantic formula below (what CMP does with Unknown, what MASK does to a masked-out row)
//! is proven against an independent scalar reference, exhaustively over the trit domain, in
//! `moth_flux/tests/flux_proof.rs`. If you change a formula here and the proof does not fail,
//! the proof has a hole — fix the proof first.

use proc_macro::{Delimiter, TokenStream, TokenTree};

#[proc_macro]
pub fn flux(input: TokenStream) -> TokenStream {
    match compile(input) {
        Ok(code) => code
            .parse()
            .expect("flux: internal error - generated code did not re-parse"),
        // No trailing semicolon: flux! sits in expression position, and `expr;` there adds a
        // spurious "macro expansion ignores `;`" diagnostic on top of the real refusal.
        Err(msg) => format!("compile_error!({:?})", format!("FLUX: {}", msg))
            .parse()
            .expect("flux: internal error - error emission did not parse"),
    }
}

// ------------------------------------------------------------------------------------------
// Lexing: flatten Rust token trees into the six tokens FLUX actually has.
// ------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Tok {
    Ident(String),
    EqEq,
    NotEq,
    Comma,
    Group(Vec<Tok>),
}

fn lex(ts: TokenStream) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut it = ts.into_iter();
    while let Some(tt) = it.next() {
        match tt {
            TokenTree::Ident(id) => out.push(Tok::Ident(id.to_string())),
            TokenTree::Group(g) => match g.delimiter() {
                Delimiter::Parenthesis => out.push(Tok::Group(lex(g.stream())?)),
                // `None` groups appear when another macro forwards tokens; they are transparent.
                Delimiter::None => out.extend(lex(g.stream())?),
                d => {
                    return Err(format!(
                        "unexpected {:?} delimiter - FLUX uses parentheses only",
                        d
                    ))
                }
            },
            TokenTree::Punct(p) => match p.as_char() {
                ',' => out.push(Tok::Comma),
                '=' => match it.next() {
                    Some(TokenTree::Punct(q)) if q.as_char() == '=' => out.push(Tok::EqEq),
                    _ => return Err("stray `=` - did you mean `==`?".into()),
                },
                '!' => match it.next() {
                    Some(TokenTree::Punct(q)) if q.as_char() == '=' => out.push(Tok::NotEq),
                    _ => return Err("`!` is not a FLUX operator - write `not`".into()),
                },
                '&' => return Err("`&`/`&&` is not a FLUX operator - write `and`".into()),
                '|' => return Err("`|`/`||` is not a FLUX operator - write `or`".into()),
                '^' => return Err("`^` is not a FLUX operator - write `xor`".into()),
                c => return Err(format!("unexpected `{}`", c)),
            },
            TokenTree::Literal(l) => {
                return Err(format!(
                    "unexpected literal `{}` - FLUX has named planes and the trit literals \
                     YES / NO / UNKNOWN, nothing else",
                    l
                ))
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------------
// Parsing. Precedence, loosest to tightest: or, xor, and, not, primary.
// ------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Lit {
    Yes,
    No,
    Unknown,
}

#[derive(Clone, Copy, PartialEq)]
enum CmpOp {
    Eq,
    Ne,
}

#[derive(Clone, Copy, PartialEq)]
enum Bop {
    And,
    Or,
    Xor,
}

enum Ast {
    Plane(String),
    Cmp(String, CmpOp, Lit),
    Not(Box<Ast>),
    Bin(Bop, Box<Ast>, Box<Ast>),
    Mask(Box<Ast>, Box<Ast>),
    Select(Box<Ast>, Box<Ast>, Box<Ast>),
    Count(Box<Ast>),
}

struct P<'a> {
    t: &'a [Tok],
    i: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.t.get(self.i)
    }
    fn bump(&mut self) -> Option<&'a Tok> {
        let t = self.t.get(self.i);
        self.i += 1;
        t
    }
    fn eat_kw(&mut self, kw: &str) -> bool {
        if let Some(Tok::Ident(s)) = self.peek() {
            if s == kw {
                self.i += 1;
                return true;
            }
        }
        false
    }
}

fn parse_all(toks: &[Tok]) -> Result<Ast, String> {
    let mut p = P { t: toks, i: 0 };
    let a = parse_or(&mut p)?;
    if p.i != toks.len() {
        return Err("unexpected trailing tokens after the expression".into());
    }
    Ok(a)
}

fn parse_or(p: &mut P) -> Result<Ast, String> {
    let mut a = parse_xor(p)?;
    while p.eat_kw("or") {
        a = Ast::Bin(Bop::Or, Box::new(a), Box::new(parse_xor(p)?));
    }
    Ok(a)
}

fn parse_xor(p: &mut P) -> Result<Ast, String> {
    let mut a = parse_and(p)?;
    while p.eat_kw("xor") {
        a = Ast::Bin(Bop::Xor, Box::new(a), Box::new(parse_and(p)?));
    }
    Ok(a)
}

fn parse_and(p: &mut P) -> Result<Ast, String> {
    let mut a = parse_unary(p)?;
    while p.eat_kw("and") {
        a = Ast::Bin(Bop::And, Box::new(a), Box::new(parse_unary(p)?));
    }
    Ok(a)
}

fn parse_unary(p: &mut P) -> Result<Ast, String> {
    if p.eat_kw("not") {
        Ok(Ast::Not(Box::new(parse_unary(p)?)))
    } else {
        parse_primary(p)
    }
}

fn split_args(toks: &[Tok]) -> Vec<&[Tok]> {
    if toks.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (i, t) in toks.iter().enumerate() {
        if matches!(t, Tok::Comma) {
            out.push(&toks[start..i]);
            start = i + 1;
        }
    }
    out.push(&toks[start..]);
    if out.len() > 1 && out.last().map(|s| s.is_empty()) == Some(true) {
        out.pop(); // tolerate one trailing comma
    }
    out
}

fn parse_primary(p: &mut P) -> Result<Ast, String> {
    match p.bump() {
        Some(Tok::Group(inner)) => parse_all(inner),
        Some(Tok::Ident(s)) => match s.as_str() {
            "count" | "mask" | "select" => {
                let name = s.clone();
                let args_toks = match p.bump() {
                    Some(Tok::Group(inner)) => split_args(inner),
                    _ => {
                        return Err(format!(
                            "`{}` needs parenthesized arguments: {}",
                            name,
                            usage(&name)
                        ))
                    }
                };
                let mut args = Vec::new();
                for a in &args_toks {
                    args.push(parse_all(a)?);
                }
                match (name.as_str(), args.len()) {
                    ("count", 1) => Ok(Ast::Count(Box::new(args.remove(0)))),
                    ("mask", 2) => {
                        let m = args.remove(1);
                        let pl = args.remove(0);
                        Ok(Ast::Mask(Box::new(pl), Box::new(m)))
                    }
                    ("select", 3) => {
                        let b = args.remove(2);
                        let a = args.remove(1);
                        let m = args.remove(0);
                        Ok(Ast::Select(Box::new(m), Box::new(a), Box::new(b)))
                    }
                    (_, n) => Err(format!(
                        "`{}` takes {} - got {} argument(s)",
                        name,
                        usage(&name),
                        n
                    )),
                }
            }
            "YES" | "NO" | "UNKNOWN" => Err(format!(
                "the trit literal `{}` can only appear on the right of `==` / `!=`",
                s
            )),
            "and" | "or" | "xor" => Err(format!(
                "`{}` is an operator; an operand is missing before it",
                s
            )),
            name => match p.peek() {
                Some(Tok::EqEq) | Some(Tok::NotEq) => {
                    let op = match p.bump() {
                        Some(Tok::EqEq) => CmpOp::Eq,
                        _ => CmpOp::Ne,
                    };
                    let lit = match p.bump() {
                        Some(Tok::Ident(l)) => match l.as_str() {
                            "YES" => Lit::Yes,
                            "NO" => Lit::No,
                            "UNKNOWN" => Lit::Unknown,
                            other => {
                                return Err(format!(
                                    "the right side of a comparison must be YES, NO or UNKNOWN, \
                                     got `{}`. Comparing two planes is not in FLUX - it would be \
                                     Kleene equality, a different operation this prototype does \
                                     not lower.",
                                    other
                                ))
                            }
                        },
                        _ => {
                            return Err(
                                "the right side of a comparison must be YES, NO or UNKNOWN".into()
                            )
                        }
                    };
                    Ok(Ast::Cmp(name.to_string(), op, lit))
                }
                _ => Ok(Ast::Plane(name.to_string())),
            },
        },
        Some(Tok::Comma) => Err("unexpected `,`".into()),
        Some(Tok::EqEq) | Some(Tok::NotEq) => Err("comparison without a left-hand plane".into()),
        None => Err("expected an expression, found nothing".into()),
    }
}

fn usage(name: &str) -> &'static str {
    match name {
        "count" => "exactly one argument: count(<mask>)",
        "mask" => "exactly two arguments: mask(<plane>, <mask>)",
        _ => "exactly three arguments: select(<mask>, <plane-if-set>, <plane-if-clear>)",
    }
}

// ------------------------------------------------------------------------------------------
// Typed lowering. This is where the boundary refuses. Output: word-level op list, which is
// BOTH the code and the witness - one source, two renderings.
// ------------------------------------------------------------------------------------------

#[derive(Clone)]
enum Val {
    /// One `u64` per word; 1 = row selected. `c` is the generated local, `w` the witness name.
    Mask { c: String, w: String },
    /// A (known, value) pair of `u64`s per word, canonical form (value subset of known).
    Plane { k: String, v: String, w: String },
}

#[derive(Default)]
struct Gen {
    planes: Vec<String>, // named inputs, first-use order; planes[0] supplies the length
    loads: Vec<String>,  // per-word input loads
    ops: Vec<String>,    // per-word op lines
    wit: Vec<String>,    // witness lines, same order the ops were chosen
    nm: usize,           // mask temp counter
    nt: usize,           // plane temp counter
}

fn wline(op: &str, ins: &str, out: &str, note: &str) -> String {
    format!(
        "operation: {:<14} inputs: {:<28} output: {:<7} ;; {}",
        op, ins, out, note
    )
}

impl Gen {
    fn input(&mut self, name: &str) -> (String, String, String) {
        if let Some(i) = self.planes.iter().position(|p| p == name) {
            return (
                format!("__fx_ik{}", i),
                format!("__fx_iv{}", i),
                format!("p{}", i),
            );
        }
        let i = self.planes.len();
        self.planes.push(name.to_string());
        // Loads read from the generated fn's parameters (`__fx_p{i}`), not the caller's variable
        // names — the loop body lives inside a private fn now (see emit_count), and a nested fn
        // cannot capture its environment. The caller's variables are passed at the call site.
        self.loads
            .push(format!("let __fx_ik{i}: u64 = __fx_p{i}.known[__fx_w];"));
        self.loads
            .push(format!("let __fx_iv{i}: u64 = __fx_p{i}.value[__fx_w];"));
        self.wit.push(wline(
            "LOAD",
            name,
            &format!("p{}", i),
            "known+value words, 64 rows each",
        ));
        (
            format!("__fx_ik{}", i),
            format!("__fx_iv{}", i),
            format!("p{}", i),
        )
    }

    fn mask(&mut self, code: String, op: &str, ins: &str, formula: String) -> Val {
        let j = self.nm;
        self.nm += 1;
        self.ops.push(format!("let __fx_m{j}: u64 = {code};"));
        self.wit.push(wline(
            op,
            ins,
            &format!("m{}", j),
            &format!("m{} = {}", j, formula),
        ));
        Val::Mask {
            c: format!("__fx_m{}", j),
            w: format!("m{}", j),
        }
    }

    fn plane(&mut self, kcode: String, vcode: String, op: &str, ins: &str, formula: String) -> Val {
        let j = self.nt;
        self.nt += 1;
        self.ops.push(format!("let __fx_tk{j}: u64 = {kcode};"));
        self.ops.push(format!("let __fx_tv{j}: u64 = {vcode};"));
        self.wit.push(wline(op, ins, &format!("t{}", j), &formula));
        Val::Plane {
            k: format!("__fx_tk{}", j),
            v: format!("__fx_tv{}", j),
            w: format!("t{}", j),
        }
    }

    fn lower(&mut self, a: &Ast) -> Result<Val, String> {
        match a {
            Ast::Plane(n) => {
                let (k, v, w) = self.input(n);
                Ok(Val::Plane { k, v, w })
            }

            Ast::Cmp(n, op, lit) => {
                let (k, v, w) = self.input(n);
                // The six doors from ternary to binary. Each is total over {No, Unknown, Yes}
                // and each states explicitly which side of the mask Unknown lands on.
                let (code, wop, formula) = match (op, lit) {
                    (CmpOp::Eq, Lit::Yes) => (
                        format!("{k} & {v}"),
                        "CMP_EQ_YES",
                        format!("{w}.known & {w}.value"),
                    ),
                    (CmpOp::Eq, Lit::No) => (
                        format!("{k} & !{v}"),
                        "CMP_EQ_NO",
                        format!("{w}.known & !{w}.value"),
                    ),
                    (CmpOp::Eq, Lit::Unknown) => {
                        (format!("!{k}"), "CMP_EQ_UNKNOWN", format!("!{w}.known"))
                    }
                    (CmpOp::Ne, Lit::Yes) => (
                        format!("!({k} & {v})"),
                        "CMP_NE_YES",
                        format!("!({w}.known & {w}.value)"),
                    ),
                    (CmpOp::Ne, Lit::No) => (
                        format!("!({k} & !{v})"),
                        "CMP_NE_NO",
                        format!("!({w}.known & !{w}.value)"),
                    ),
                    (CmpOp::Ne, Lit::Unknown) => {
                        (format!("{k}"), "CMP_NE_UNKNOWN", format!("{w}.known"))
                    }
                };
                Ok(self.mask(code, wop, &w.clone(), formula))
            }

            Ast::Not(x) => match self.lower(x)? {
                Val::Mask { c, w } => {
                    Ok(self.mask(format!("!{c}"), "NOT", &w.clone(), format!("!{w}")))
                }
                Val::Plane { k, v, w } => Ok(self.plane(
                    k.clone(),
                    format!("{k} & !{v}"),
                    "KLEENE_NOT",
                    &w.clone(),
                    format!("known = {w}.known; value = {w}.known & !{w}.value"),
                )),
            },

            Ast::Bin(op, x, y) => {
                let a = self.lower(x)?;
                let b = self.lower(y)?;
                let opname = match op {
                    Bop::And => "and",
                    Bop::Or => "or",
                    Bop::Xor => "xor",
                };
                match (a, b) {
                    (Val::Mask { c: ca, w: wa }, Val::Mask { c: cb, w: wb }) => {
                        let (sym, name) = match op {
                            Bop::And => ("&", "AND"),
                            Bop::Or => ("|", "OR"),
                            Bop::Xor => ("^", "XOR"),
                        };
                        Ok(self.mask(
                            format!("{ca} {sym} {cb}"),
                            name,
                            &format!("{wa}, {wb}"),
                            format!("{wa} {sym} {wb}"),
                        ))
                    }
                    (
                        Val::Plane { k: ak, v: av, w: wa },
                        Val::Plane { k: bk, v: bv, w: wb },
                    ) => {
                        let ins = format!("{wa}, {wb}");
                        match op {
                            // The formulas are planes.rs's Kleene derivation, verbatim; the
                            // exhaustive proof in tests/flux_proof.rs checks them against the
                            // scalar truth tables, not against planes.rs.
                            Bop::And => {
                                let j = self.nt;
                                self.nt += 1;
                                self.ops.push(format!(
                                    "let __fx_y{j}: u64 = ({ak} & {av}) & ({bk} & {bv});"
                                ));
                                self.ops.push(format!(
                                    "let __fx_n{j}: u64 = ({ak} & !{av}) | ({bk} & !{bv});"
                                ));
                                self.ops
                                    .push(format!("let __fx_tk{j}: u64 = __fx_y{j} | __fx_n{j};"));
                                self.ops.push(format!("let __fx_tv{j}: u64 = __fx_y{j};"));
                                self.wit.push(wline(
                                    "KLEENE_AND",
                                    &ins,
                                    &format!("t{}", j),
                                    "yes=(a.k&a.v)&(b.k&b.v); no=(a.k&!a.v)|(b.k&!b.v); known=yes|no; value=yes",
                                ));
                                Ok(Val::Plane {
                                    k: format!("__fx_tk{}", j),
                                    v: format!("__fx_tv{}", j),
                                    w: format!("t{}", j),
                                })
                            }
                            Bop::Or => {
                                let j = self.nt;
                                self.nt += 1;
                                self.ops.push(format!(
                                    "let __fx_y{j}: u64 = ({ak} & {av}) | ({bk} & {bv});"
                                ));
                                self.ops.push(format!(
                                    "let __fx_n{j}: u64 = ({ak} & !{av}) & ({bk} & !{bv});"
                                ));
                                self.ops
                                    .push(format!("let __fx_tk{j}: u64 = __fx_y{j} | __fx_n{j};"));
                                self.ops.push(format!("let __fx_tv{j}: u64 = __fx_y{j};"));
                                self.wit.push(wline(
                                    "KLEENE_OR",
                                    &ins,
                                    &format!("t{}", j),
                                    "yes=(a.k&a.v)|(b.k&b.v); no=(a.k&!a.v)&(b.k&!b.v); known=yes|no; value=yes",
                                ));
                                Ok(Val::Plane {
                                    k: format!("__fx_tk{}", j),
                                    v: format!("__fx_tv{}", j),
                                    w: format!("t{}", j),
                                })
                            }
                            Bop::Xor => Ok(self.plane(
                                format!("{ak} & {bk}"),
                                format!("({ak} & {bk}) & ({av} ^ {bv})"),
                                "KLEENE_XOR",
                                &ins,
                                "known = a.k & b.k; value = known & (a.v ^ b.v) - Unknown if either side is"
                                    .to_string(),
                            )),
                        }
                    }
                    _ => Err(format!(
                        "`{opname}` over a trit plane and a bit-mask. A plane does not coerce to \
                         a mask, because Unknown has to land on one side of the bit and FLUX will \
                         not pick for you. Compare the plane explicitly - `x == YES` (Unknown is \
                         a miss) or `x != NO` (Unknown is a hit) - then combine."
                    )),
                }
            }

            Ast::Mask(pl, m) => {
                let pv = self.lower(pl)?;
                let mv = self.lower(m)?;
                match (pv, mv) {
                    (Val::Plane { k, v, w: wp }, Val::Mask { c, w: wm }) => Ok(self.plane(
                        format!("{k} & {c}"),
                        format!("{v} & {c}"),
                        "MASK",
                        &format!("{wp}, {wm}"),
                        format!(
                            "known = {wp}.known & {wm}; value = {wp}.value & {wm} - a masked-out \
                             row becomes UNKNOWN (unasked), never NO"
                        ),
                    )),
                    _ => Err("mask(<plane>, <mask>): the first argument must be a trit plane and \
                              the second a mask (a comparison, or a boolean combination of \
                              comparisons). Passing them the other way round would ask FLUX to \
                              invent a plane from a mask - use select(...) if that is what you \
                              mean."
                        .into()),
                }
            }

            Ast::Select(m, x, y) => {
                let mv = self.lower(m)?;
                let av = self.lower(x)?;
                let bv = self.lower(y)?;
                match (mv, av, bv) {
                    (
                        Val::Mask { c, w: wm },
                        Val::Plane { k: ak, v: av_, w: wa },
                        Val::Plane { k: bk, v: bv_, w: wb },
                    ) => Ok(self.plane(
                        format!("({c} & {ak}) | (!{c} & {bk})"),
                        format!("({c} & {av_}) | (!{c} & {bv_})"),
                        "SELECT",
                        &format!("{wm}, {wa}, {wb}"),
                        "known = (m & a.k) | (!m & b.k); value = (m & a.v) | (!m & b.v)"
                            .to_string(),
                    )),
                    _ => Err("select(<mask>, <plane-if-set>, <plane-if-clear>): the first \
                              argument must be a mask and the other two trit planes."
                        .into()),
                }
            }

            Ast::Count(_) => Err(
                "count(...) produces one number for the whole population; it cannot appear \
                 inside a row-wise expression. Move it to the top level."
                    .into(),
            ),
        }
    }

    fn witness(&self) -> String {
        let mut w = String::from(
            "FLUX lowering witness - one fused pass, u64 word = 64 rows per iteration\n",
        );
        w.push_str(&self.wit.join("\n"));
        w
    }

    fn prologue(&self, witness: &str) -> String {
        let mut s = String::from("{\n");
        s.push_str(&format!(
            "    const __FLUX_WITNESS: &str = {:?};\n",
            witness
        ));
        let first = &self.planes[0];
        s.push_str(&format!("    let __fx_len: usize = {}.len();\n", first));
        for p in &self.planes[1..] {
            s.push_str(&format!(
                "    assert!({p}.len() == __fx_len, \"flux: plane `{p}` has {{}} rows but plane \
                 `{first}` has {{}} - different populations have no elementwise combination\", \
                 {p}.len(), __fx_len);\n"
            ));
        }
        s
    }

    /// Parameter list of the generated private fn: one `&Planes` per referenced plane, in
    /// first-use order (the same order `planes` was built in), plus the length. `__fx_words` is
    /// derived INSIDE the fn from the length, matching the measured-fast arms C and E, which both
    /// derive it locally.
    fn fn_params(&self) -> String {
        let mut ps: Vec<String> = (0..self.planes.len())
            .map(|i| format!("__fx_p{i}: &::moth_flux::Planes"))
            .collect();
        ps.push("__fx_len: usize".to_string());
        ps.join(", ")
    }

    /// Call-site arguments matching `fn_params`: the caller's actual variables. `&{name}` is
    /// correct whether the caller holds `Planes` or `&Planes` — `&&Planes` deref-coerces at the
    /// call boundary.
    fn fn_args(&self) -> String {
        let mut args: Vec<String> = self.planes.iter().map(|p| format!("&{p}")).collect();
        args.push("__fx_len".to_string());
        args.join(", ")
    }

    fn body(&self) -> String {
        let mut s = String::new();
        for l in self.loads.iter().chain(self.ops.iter()) {
            s.push_str("        ");
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    fn emit_count(&self, final_mask: &str) -> String {
        let mut s = self.prologue(&self.witness());
        // ⚠⚠ THIS WAS A `while` WITH A MANUAL COUNTER. I CHANGED IT TO A RANGE LOOP TO CLOSE A
        //    2.6x PERFORMANCE GAP, AND IT DID NOT CLOSE IT. Recording that honestly, because a
        //    comment claiming a fix that did not happen is worse than no comment.
        // MEASURED 2026-09-13 vs the hand-written `ran_and_not_ok`, interleaved arms, 25 reps:
        //        41,980 rows   1.83x  ->  1.67x
        //     1,000,000 rows   2.54x  ->  2.53x
        //    10,000,000 rows   2.60x  ->  2.47x
        // ★ HYPOTHESIS (refuted): the hand loop is `for w in 0..words`, which LLVM can prove
        //   in-bounds so the checks on `known[w]` / `value[w]` are elided, while a manually
        //   incremented counter defeats that — a per-word tax, which fits the ratio GROWING with
        //   size. Plausible, testable, and wrong: making the loop forms identical moved almost
        //   nothing. Bounds-check elision is not where the 2.5x lives.
        // ⛔⛔⛔ AND THEN THE GAP TURNED OUT NOT TO EXIST. Settled 2026-09-13, same day.
        //   The 2.5x was MY BENCHMARK. The macro was expanding INLINE between two black_box
        //   barriers while `ran_and_not_ok` was a standalone function LLVM could optimise whole —
        //   two different codegen contexts, and I was attributing the difference to the lowering.
        //   Equalise the contexts (both arms behind the same call shape) and:
        //        41,980 rows    1.000
        //     1,000,000 rows    1.005  /  1.000
        //    10,000,000 rows    1.008  /  1.001
        //   The generated code costs what the hand-written loop costs. Fable's original
        //   "shape-equivalent by construction" claim was RIGHT, and my refutation of it was wrong.
        // ★ THE TELL I MISSED: under the controls the HAND arm slowed from 9.2us to 19.9us at 1M
        //   while the macro barely moved (23.3 -> 20.0). The hand side had been getting an
        //   optimisation the macro could not, purely from how each was invoked. I had already
        //   "fixed the asymmetry" once by equalising black_box placement — fixed the visible half,
        //   declared the arms matched, and the inlining context was still different.
        // ⚠ Caveat on the control itself, stated because it limits the claim: the harness passes
        //   both arms as function POINTERS, so both are opaque indirect calls and inline(always)
        //   cannot apply through one. The two "conditions" are therefore one condition run twice.
        //   What is established is: THROUGH IDENTICAL CALL SHAPES the two are equal. That answers
        //   the question, but it is not two independent confirmations.
        // ★ The range loop is KEPT anyway: identical semantics, all 51 tests pass, clearer form.
        //   It fixed nothing because there was nothing to fix.
        // ★★ RESOLVED 2026-09-13 (five-arm isolation, src/bin/flux_locate.rs): the cost was the
        //   INLINE PASTE ITSELF. Same expansion inside a plain `fn` (arm E) ran at 0.92-1.00x the
        //   hand-written library kernel; pasted inline (arm D) it ran at 2.8x. So the loop now
        //   lives in a private fn, generated below, and is called immediately with the caller's
        //   variables. ⚠ A PLAIN fn, deliberately: `#[inline(never)]` was measured as slow as
        //   pasting (19.86 vs 10.13), and `#[inline(always)]` was NOT measured — the property that
        //   works is "a boundary LLVM is allowed to inline AS A UNIT", so no attribute at all.
        s.push_str(&format!(
            "    fn __fx_run({}) -> usize {{\n",
            self.fn_params()
        ));
        s.push_str("        let __fx_words: usize = (__fx_len + 63) / 64;\n");
        s.push_str("        let mut __fx_count: usize = 0;\n");
        s.push_str("        for __fx_w in 0..__fx_words {\n");
        s.push_str(&self.body());
        s.push_str(&format!("        let mut __fx_hit: u64 = {final_mask};\n"));
        s.push_str("        if __fx_w + 1 == __fx_words && (__fx_len % 64) != 0 {\n");
        s.push_str("            __fx_hit &= (1u64 << (__fx_len % 64)) - 1;\n");
        s.push_str("        }\n");
        s.push_str("        __fx_count += __fx_hit.count_ones() as usize;\n");
        s.push_str("        }\n");
        s.push_str("        __fx_count\n");
        s.push_str("    }\n");
        s.push_str(&format!(
            "    let __fx_count: usize = __fx_run({});\n",
            self.fn_args()
        ));
        s.push_str(
            "    ::moth_flux::flux::Fluxed { value: __fx_count, witness: __FLUX_WITNESS }\n",
        );
        s.push_str("}\n");
        s
    }

    fn emit_plane(&self, k: &str, v: &str) -> String {
        let mut s = self.prologue(&self.witness());
        // Same boundary as emit_count above, same evidence: the loop body lives in a private
        // plain `fn` (no inline attribute — see the note there) and is called immediately.
        s.push_str(&format!(
            "    fn __fx_run({}) -> ::moth_flux::Planes {{\n",
            self.fn_params()
        ));
        s.push_str("        let __fx_words: usize = (__fx_len + 63) / 64;\n");
        s.push_str("        let mut __fx_out = ::moth_flux::Planes::with_len(__fx_len);\n");
        s.push_str("        for __fx_w in 0..__fx_words {\n");
        s.push_str(&self.body());
        s.push_str(&format!("        let mut __fx_ok: u64 = {k};\n"));
        s.push_str(&format!("        let mut __fx_ov: u64 = {v};\n"));
        s.push_str("        if __fx_w + 1 == __fx_words && (__fx_len % 64) != 0 {\n");
        s.push_str("            let __fx_tm: u64 = (1u64 << (__fx_len % 64)) - 1;\n");
        s.push_str("            __fx_ok &= __fx_tm;\n");
        s.push_str("            __fx_ov &= __fx_tm;\n");
        s.push_str("        }\n");
        s.push_str("        __fx_out.known[__fx_w] = __fx_ok;\n");
        s.push_str("        __fx_out.value[__fx_w] = __fx_ov;\n");
        s.push_str("        }\n");
        s.push_str("        __fx_out\n");
        s.push_str("    }\n");
        s.push_str(&format!(
            "    let __fx_out: ::moth_flux::Planes = __fx_run({});\n",
            self.fn_args()
        ));
        s.push_str(
            "    ::moth_flux::flux::Fluxed { value: __fx_out, witness: __FLUX_WITNESS }\n",
        );
        s.push_str("}\n");
        s
    }
}

// ------------------------------------------------------------------------------------------
// Top level: count(<mask>) -> Fluxed<usize>, plane expression -> Fluxed<Planes>.
// A bare mask at top level is REFUSED - materializing it as a plane would invent known-ness.
// ------------------------------------------------------------------------------------------

fn compile(input: TokenStream) -> Result<String, String> {
    let toks = lex(input)?;
    let ast = parse_all(&toks)?;
    let mut g = Gen::default();
    // Lower first (refusals live there), then check we saw at least one plane BEFORE emitting:
    // the emitters index planes[0] for the population length.
    let out = match ast {
        Ast::Count(inner) => match g.lower(&inner)? {
            Val::Mask { c, w } => {
                if g.planes.is_empty() {
                    return Err("the expression references no trit planes".into());
                }
                g.wit.push(wline(
                    "TAILMASK",
                    &format!("{}, len", w),
                    &w,
                    "last word only, when len % 64 != 0 - padding bits can never be counted",
                ));
                g.wit
                    .push(wline("POPCOUNT", &w, "count", "one count_ones per 64 rows"));
                g.emit_count(&c)
            }
            Val::Plane { .. } => {
                return Err(
                    "count(<plane>) is ambiguous - count what? Yes rows? Known rows? Write the \
                     comparison so Unknown has an explicit meaning: count(p == YES), \
                     count(p != UNKNOWN), ..."
                        .into(),
                )
            }
        },
        other => match g.lower(&other)? {
            Val::Plane { k, v, w } => {
                if g.planes.is_empty() {
                    return Err("the expression references no trit planes".into());
                }
                g.wit.push(wline(
                    "TAILMASK",
                    &format!("{}, len", w),
                    &w,
                    "keeps the padding of the result canonical (known = value = 0 past len)",
                ));
                g.wit.push(wline(
                    "STORE",
                    &w,
                    "result",
                    "written word-by-word into a fresh Planes",
                ));
                g.emit_plane(&k, &v)
            }
            Val::Mask { .. } => {
                return Err(
                    "a bare mask cannot be the result of a FLUX expression: turning it into a \
                     plane would MATERIALIZE invented known-ness for every row, and FLUX refuses \
                     implicit materialization. Say what you mean: count(<mask>) to count it, or \
                     select(<mask>, a, b) / mask(p, <mask>) to apply it."
                        .into(),
                )
            }
        },
    };
    Ok(out)
}
