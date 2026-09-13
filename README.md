# MOTH FLUX

A constrained semantic language in which **unresolved state and its physical representation are
first-class**, and every valid expression compiles directly to a semantics-preserving
representation — with no implicit coercion, no hidden materialization, and no interpreter.

The restriction is not a limitation added later. **The restriction is what FLUX is.**

---

## The problem it exists for

Most systems flatten "I don't know" into "no" at the first convenient moment, and the flattening is
invisible afterwards. A check that never ran and a check that ran and failed both end up as
`false`, and every downstream count silently merges them.

FLUX's core type is a **trit** — `Yes` / `No` / `Unknown` — and its central rule is enforced by the
type system rather than by discipline:

```rust
// there is DELIBERATELY no `From<Trit> for bool`
pub fn resolve(self, unknown_as: bool) -> bool
```

You cannot get a `bool` out of a `Trit` without saying, at that exact point, what Unknown means.
A test named `resolve_is_the_only_door_and_it_asks` guards this; if the type ever grows a `Default`
or a `From`, that test is meant to be deleted only after re-reading why it exists.

## The representation

Trits are stored as **two bit-planes** — a KNOWN plane and a VALUE plane, 64 rows per `u64` word:

```
row:      0  1  2  3  4  ...
known:    1  1  0  1  0        0 = Unknown
value:    1  0  ?  1  ?        meaningful only where known = 1
```

A whole predicate over 64 rows becomes a handful of bitwise ops and one popcount. `ran == YES and
verdict != YES` over the live 41,980-event corpus resolves in **0.6 µs**.

## The language

Two value kinds, and the boundary between them is the entire design:

| kind | what it is | how you get one |
|---|---|---|
| **Plane** | a trit column | a named input, or a Kleene combination (`and` `or` `not` `xor`), `mask`, `select` |
| **Mask** | one bit per row | **only** from a comparison (`p == YES`, `p != UNKNOWN`, six forms) |

**A comparison is the plane-level mirror of `resolve`.** It is the single door from ternary to
binary, and writing it forces you to state what Unknown means at that crossing:

- `p == YES` — Unknown is a miss
- `p != NO`  — Unknown is a hit

Those are different questions, and FLUX refuses to pretend they are the same.

```rust
let hits = flux! { count(ran == YES and verdict != YES) };
```

### What it refuses

Six classes, each a compile error with a message that names the two ways to say what you meant:

```
error: FLUX: `and` over a trit plane and a bit-mask. A plane does not coerce to a mask,
because Unknown has to land on one side of the bit and FLUX will not pick for you. Compare
the plane explicitly - `x == YES` (Unknown is a miss) or `x != NO` (Unknown is a hit).

error: FLUX: count(<plane>) is ambiguous - count what? Yes rows? Known rows? Write the
comparison so Unknown has an explicit meaning: count(p == YES), count(p != UNKNOWN), ...
```

Also refused: a bare-mask result, `plane == plane`, nested `count`, and Rust's own `&&`.

## How it compiles

```
FLUX source  ->  semantic validation  ->  plane IR  ->  lowering witness  ->  native code
                         |
                         +-- cannot preserve semantics? --> COMPILE ERROR
```

There is **no interpreter in the hot path**. `flux!` is a proc-macro with zero dependencies; the
grammar is seven operations and a hand-rolled parser. Every accepted expression becomes one fused
word-at-a-time pass — loads, `& | ^ !` on `u64`s, a tail mask on the final word only, then a
popcount or two stores.

### The lowering witness

Every compilation emits the lowering it actually chose, as an inspectable artifact:

```
operation: LOAD           inputs: ran        output: p0    ;; known+value words, 64 rows each
operation: CMP_EQ_YES     inputs: p0         output: m0    ;; m0 = p0.known & p0.value
operation: LOAD           inputs: verdict    output: p1
operation: CMP_NE_YES     inputs: p1         output: m1    ;; m1 = !(p1.known & p1.value)
operation: AND            inputs: m0, m1     output: m2
operation: TAILMASK       inputs: m2, len    output: m2    ;; last word only
operation: POPCOUNT       inputs: m2         output: count
```

This exists because **every invariant in the thesis is a negative** — no interpreter, no coercion,
no hidden materialization — and you cannot observe a materialization that did not happen. The
witness turns unfalsifiable promises into one artifact a human can read. Witness and code are
generated from the same in-memory op list, so they cannot drift apart.

## How it is verified

**Exhaustively.** For an operation over *n* trits the input domain is 3^n and therefore finite, so
CI checks the whole domain rather than sampling:

```
for every trit combination:
    assert scalar_reference(input) == plane_lowering(input)
```

The scalar reference is written **independently**, from truth tables, one row at a time, and never
touches a plane word. Derived from the plane path it would only agree with itself — which is
exactly how an earlier encoding bug survived: `Outcome` needed three trits, not two, and every
benchmark agreed with itself while being wrong.

Case counts, all at lengths `{0, 1, 5, 63, 64, 65, 128, 130, 200}` plus a corrupt-padding probe
aimed at the tail mask:

| operation | row checks |
|---|---|
| comparisons (6 forms) | 5,196 |
| mask and/or/xor/not | 7,716 |
| Kleene plane ops | 5,144 |
| `mask()` | 2,572 |
| `select()` | 2,546 |

**51 tests pass** — 28 unit, 3 integration, 9 proof, 11 doctests (six of which are the refusals).
Note that the refusal tests are the only ones where *passing means the compiler said no*, which
makes them the only tests capable of detecting a rule getting **looser**.

## Performance

Against the hand-written reference loop `planes::ran_and_not_ok`, same data, interleaved arms,
median of per-rep ratios, equality asserted every repetition:

| rows | hand-written | `flux!` | ratio |
|---|---|---|---|
| 41,980 | 0.60 µs | 0.50 µs | 0.83 |
| 1,000,000 | 9.90 µs | 9.30 µs | 0.94 |
| 10,000,000 | 94.0 µs | 88.4 µs | 0.94 |

The semantic boundary costs nothing measurable in this benchmark.

> ⚠ **Scope of that claim.** One expression, one benchmark, three sizes, one machine, one compiler,
> one optimisation level. Not established for every expression, larger expression graphs, other
> targets, or a plane-returning shape (a different emit path, untimed).

### The part worth reading

It was not always so. The macro originally **pasted** its loop into the caller and ran **2.8×
slower**. Four successive explanations were wrong before the real one:

1. *2.5× slower, blamed on the lowering* — right number, wrong cause.
2. *1.00, no gap* — a control that wrapped both arms in `#[inline(never)]`, which deoptimised the
   fast arm by 2× rather than equalising them.
3. *Location — library vs consumer crate* — refuted; the same body is equally fast in both.
4. **The function boundary.** The generated code was always fine. It needed to be somewhere the
   optimiser could work on it as a unit. Emitting a private `fn` and calling it removed the
   entire gap.

And the property is narrower than "use a function": `#[inline(never)]` is as slow as pasting. What
matters is **a boundary the optimiser is allowed to inline as a unit.**

There remains an unexplained, consistent 4–8% by which the FLUX shape *beats* the hand-written loop
whenever either sits behind a boundary. Observed across four arms and three sizes. Cause unknown,
and deliberately not attributed without a controlled experiment.

## Layout

```
moth_flux/
  flux_macro/        the compiler — lexer, parser, semantic validation, lowering, witness
  src/trit.rs        Trit; resolve() is the only door to bool
  src/planes.rs      bit-plane storage and the reference hot loop
  src/event.rs       domain encoding (and why Outcome needs three trits)
  src/flux.rs        Fluxed<T>, re-export, refusal doctests
  tests/flux_proof.rs    the exhaustive 3^n equivalence proofs
  tests/flux_example.rs  worked example: source -> witness -> result
```

## Status

Frozen as the reference implementation — meaning this version is the known-good baseline that
future changes are measured against, not that work has stopped. Adding an operation requires it to
pass the exhaustive both-ways proof first.

---

## Authorship and licence

Directed by **SC_Placeholder**; built, proved and benchmarked by **Elah Moth**, MOTH's overlord AI running on Anthropic's Claude.

The direction is his — including the decisions NOT to build things, which shaped this more than the ones to build. The encoding, the compiler, the proofs, the benchmarks and the recorded failures are hers.

Full statement, including two contributions named rather than omitted, in [NOTICE](NOTICE). Licensed Apache-2.0 ([LICENSE](LICENSE)). Dependencies and their terms in [THIRD_PARTY.md](THIRD_PARTY.md).
