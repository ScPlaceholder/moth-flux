"""Does FLUX-via-pyo3 beat plain Python at three-valued counting, and where is the crossover?

PREDICTION 5, registered in predict.py BEFORE this ran (predict.py:add, 2026-09-13):
    plain Python wins below ~10,000 rows on FFI marshalling; FLUX wins above; >=5x at 1,000,000.
    Refuted if FLUX also wins at 1,000; if it is under 5x at 1M; if it loses at 1M; or if the
    crossover falls outside 2,000-50,000.
    ⚠ And >50x at 1M counts as REFUTED-BY-BROKEN-HARNESS, not as a triumph.

# The arms, and why there are four

    A  python_loop     the real comparator -- what building_cards._scan_events actually does
    B  flux_bulk       Python hands over a bytes buffer it ALREADY has; Rust classifies and counts
    C  flux_marshal    B, plus the cost of Python building that buffer row by row
    D  build_only      plane construction with no counting, to split marshalling from computation

⚠ B IS THE FLATTERING ARM AND IT IS NOT THE HONEST ONE ON ITS OWN. If a caller must construct the
  code buffer element-by-element in Python, the cost has merely moved and the extension is theatre.
  C is that case. Reporting B alone would be the "every improvement took its gain from somewhere I
  was not measuring" failure, so both are printed side by side and C is the one that decides.

⚠ Equality is asserted EVERY rep, not once at the end. Two arms that disagree are not two timings
  of one computation, and a benchmark that never checks is timing a difference it cannot see.
"""
import os
import random
import statistics
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "target", "release"))
sys.path.insert(0, HERE)

try:
    import flux_py
except ImportError as e:
    print("cannot import flux_py: %s" % e)
    print("build first:  cargo build --release   then copy flux_py.dll -> flux_py.pyd")
    raise SystemExit(2)

# ── the corpus ───────────────────────────────────────────────────────────────────────────────
# Proportions taken from the real log rather than invented: 34,196 absent and 250 cannot_tell out
# of 50,886 rows, per event.rs. A uniform mix would make the branch predictor's life unrealistically
# easy and would flatter whichever arm branches more.
OUTCOMES = ["absent"] * 672 + ["ok"] * 318 + ["cannot_tell"] * 5 + ["timeout", "error", "refused", "crash", "x"]
CODE = {"absent": 0, "ok": 1, "cannot_tell": 2}


def make_rows(n, seed):
    r = random.Random(seed)
    return [r.choice(OUTCOMES) for _ in range(n)]


def python_loop(rows):
    """Arm A — building_cards.py:229-241, transcribed. `failed` excludes absent AND cannot_tell."""
    unk = fail = 0
    for oc in rows:
        if oc == "absent":
            oc = None
        if oc == "cannot_tell":
            unk += 1
        elif oc is not None and oc != "ok":
            fail += 1
    return unk, fail


def encode(rows):
    return bytes(CODE.get(oc, 3) for oc in rows)


def timeit(fn, reps):
    """Median of per-rep times. Median, not mean: one GC pause should not decide the answer."""
    ts = []
    for _ in range(reps):
        t0 = time.perf_counter()
        out = fn()
        ts.append(time.perf_counter() - t0)
    return statistics.median(ts), out


def main():
    print("%-10s %12s %12s %12s %12s   %10s %10s" %
          ("rows", "A py loop", "B flux bulk", "C +marshal", "D build only", "B/A", "C/A"))
    print("-" * 96)

    verdicts = []
    for n in (1_000, 5_000, 10_000, 50_000, 200_000, 1_000_000):
        rows = make_rows(n, seed=1234 + n)
        buf = encode(rows)
        reps = 9 if n <= 200_000 else 5

        want = python_loop(rows)
        got = flux_py.count_unk_fail(buf)
        if want != got:
            print("MISMATCH at n=%d: python=%s flux=%s -- STOP, the arms compute different things"
                  % (n, want, got))
            return 1

        t_a, _ = timeit(lambda: python_loop(rows), reps)
        t_b, _ = timeit(lambda: flux_py.count_unk_fail(buf), reps)
        t_c, _ = timeit(lambda: flux_py.count_unk_fail(encode(rows)), reps)
        t_d, _ = timeit(lambda: flux_py.build_only(buf), reps)

        # equality re-asserted after timing, in case an arm was optimised into nothing
        assert flux_py.count_unk_fail(buf) == want, "arms diverged during timing"

        print("%-10d %12.6f %12.6f %12.6f %12.6f   %10.2f %10.2f"
              % (n, t_a, t_b, t_c, t_d, t_a / t_b, t_a / t_c))
        verdicts.append((n, t_a / t_b, t_a / t_c))

    # ── the wrong rule, on the same data, to show the distinction is not pedantry ─────────────
    rows = make_rows(50_000, seed=7)
    buf = encode(rows)
    right_unk, right_fail = flux_py.count_unk_fail(buf)
    wrong = flux_py.count_not_ok_WRONG(buf)
    print()
    print("The rule city_time_bench.py mislabelled this morning, on 50,000 rows:")
    print("   correct  fail_n (present, and neither ok nor cannot_tell) = %d" % right_fail)
    print("   labelled `outcome != 'ok'`                                = %d" % wrong)
    print("   difference = %d rows -- %d absent + %d cannot_tell swept in as failures."
          % (wrong - right_fail, wrong - right_fail - right_unk, right_unk))

    # ── adjudicate the registered prediction ──────────────────────────────────────────────────
    print()
    b_at_1k = next(r for n, r, _ in verdicts if n == 1_000)
    b_at_1m = next(r for n, r, _ in verdicts if n == 1_000_000)
    cross = next((n for n, r, _ in verdicts if r > 1.0), None)
    print("PREDICTION 5 adjudication")
    print("  B/A at 1,000 rows      : %.2f   (predicted <1 -- python wins)" % b_at_1k)
    print("  B/A at 1,000,000 rows  : %.2f   (predicted >=5)" % b_at_1m)
    print("  crossover              : %s   (predicted 2,000-50,000)" % cross)
    fails = []
    if b_at_1k > 1.0:
        fails.append("(a) FLUX won at 1,000 too -- no crossover")
    if b_at_1m < 5.0:
        fails.append("(b) under 5x at 1M")
    if b_at_1m < 1.0:
        fails.append("(c) FLUX lost at 1M")
    if cross is None or not (2_000 <= cross <= 50_000):
        fails.append("(d) crossover outside 2,000-50,000 (got %s)" % cross)
    if b_at_1m > 50.0:
        fails.append("(e) >50x at 1M -- treat as BROKEN HARNESS, not a win")
    print("  VERDICT: %s" % ("HELD" if not fails else "REFUTED -- " + "; ".join(fails)))
    print("== fluxpybench COMPLETE (rc=0) ==")
    return 0


if __name__ == "__main__":
    sys.exit(main())
