/* MOTH FLUX versus — the C arms. Mirrors src/bin/versus.rs exactly.
 *
 * Compile: bench\build_versus.bat   (MSVC cl /O2 — MSVC's highest standard level; no /O3 exists)
 * Run:     target\versus_c.exe target\versus_<n>.bin target\versus_expected.txt
 *
 * HOW THIS STAYS COMPARABLE TO THE RUST SIDE
 *  - Data: read from the file the Rust program wrote. Not regenerated — shared by construction.
 *  - ⛔ Counts are ASSERTED against the expectation file BEFORE any timing is printed. A faster
 *    wrong answer is not a result; it is an invalid benchmark, and this program exits 1 on it.
 *  - No intrinsics. The Rust target has no POPCNT (default x86_64 features), so u64::count_ones
 *    is LLVM's SWAR expansion; popcount64() below is the same SWAR by hand. Giving C __popcnt64
 *    while Rust does SWAR (or vice versa) would shield one arm at the ISA level.
 *  - Same rep counts, arms interleaved inside one loop, median-of-per-rep values — the noise
 *    lesson from adversarial.rs applies across languages too.
 *  - The volatile sink defeats dead-code elimination, C's equivalent of black_box on every arm.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <limits.h>
#include <windows.h>

/* Idiomatic C record: 8 bytes, the same size and meaning as Rust's Event{Option<Verdict>,u8,u32}.
 * outcome: 0 = absent, 1 = ok, 2 = fail, 3 = cannot_tell, 4 = no_match. */
typedef struct {
    uint8_t outcome;
    uint8_t kind;
    uint32_t target;
} Ev;

/* The bit-plane pair, same layout as moth_flux::planes::Planes. */
typedef struct {
    uint64_t *known;
    uint64_t *value;
} Planes;

static volatile uint64_t sink; /* black_box */

static uint64_t popcount64(uint64_t x) { /* Hacker's Delight SWAR — see header for why no intrinsic */
    x = x - ((x >> 1) & 0x5555555555555555ULL);
    x = (x & 0x3333333333333333ULL) + ((x >> 2) & 0x3333333333333333ULL);
    x = (x + (x >> 4)) & 0x0F0F0F0F0F0F0F0FULL;
    return (x * 0x0101010101010101ULL) >> 56;
}

/* Arm 1: idiomatic struct-array scan. Written the way a C programmer writes it, on purpose. */
static size_t scan_aos(const Ev *ev, size_t n) {
    size_t cnt = 0;
    for (size_t i = 0; i < n; i++) {
        if (ev[i].outcome != 0 && ev[i].outcome != 1) cnt++;
    }
    return cnt;
}

/* Arm 2: byte-column scan, branchless single compare — the fair conventional baseline,
 * identical predicate shape to Rust's `(o >= 2) as usize` sum. */
static size_t scan_bytes(const uint8_t *col, size_t n) {
    size_t cnt = 0;
    for (size_t i = 0; i < n; i++) cnt += (col[i] >= 2);
    return cnt;
}

/* Arm 3: MOTH bit-planes, the same algorithm as planes::ran_and_not_ok — two ANDs, an ANDNOT,
 * a tail mask on the last word, popcount. */
static size_t scan_planes(const Planes *ran, const Planes *ver, size_t n) {
    size_t words = (n + 63) / 64;
    uint64_t cnt = 0;
    for (size_t w = 0; w < words; w++) {
        uint64_t ran_yes = ran->known[w] & ran->value[w];
        uint64_t ver_yes = ver->known[w] & ver->value[w];
        uint64_t hit = ran_yes & ~ver_yes;
        if (w == words - 1 && (n % 64) != 0) hit &= (1ULL << (n % 64)) - 1;
        cnt += popcount64(hit);
    }
    return (size_t)cnt;
}

static double now_us(void) {
    static LARGE_INTEGER freq = {0};
    LARGE_INTEGER t;
    if (!freq.QuadPart) QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&t);
    return (double)t.QuadPart * 1e6 / (double)freq.QuadPart;
}

static int cmp_dbl(const void *a, const void *b) {
    double d = *(const double *)a - *(const double *)b;
    return (d > 0) - (d < 0);
}
static double median(double *v, size_t n) {
    qsort(v, n, sizeof(double), cmp_dbl);
    return v[n / 2];
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: versus_c <data.bin> <expected.txt>\n");
        return 2;
    }

    /* ---- load the shared dataset ---------------------------------------------------------- */
    FILE *f = fopen(argv[1], "rb");
    if (!f) { fprintf(stderr, "cannot open %s\n", argv[1]); return 2; }
    fseek(f, 0, SEEK_END);
    long long fn = _ftelli64(f);
    fseek(f, 0, SEEK_SET);
    size_t n = (size_t)fn;
    uint8_t *col = malloc(n);
    if (!col || fread(col, 1, n, f) != n) { fprintf(stderr, "read failed\n"); return 2; }
    fclose(f);

    /* ---- find the expected count for this n ------------------------------------------------ */
    FILE *e = fopen(argv[2], "r");
    if (!e) { fprintf(stderr, "cannot open %s\n", argv[2]); return 2; }
    unsigned long long en, ecount, expected = ULLONG_MAX;
    while (fscanf(e, "%llu %llu", &en, &ecount) == 2)
        if ((size_t)en == n) expected = ecount;
    fclose(e);
    if (expected == ULLONG_MAX) {
        fprintf(stderr, "no expectation recorded for n=%zu — refusing to run unanchored\n", n);
        return 2;
    }

    /* ---- build the other two representations from the same bytes -------------------------- */
    Ev *ev = malloc(n * sizeof(Ev));
    if (!ev) { fprintf(stderr, "OOM building struct array (n=%zu)\n", n); return 2; }
    for (size_t i = 0; i < n; i++) {
        ev[i].outcome = col[i];
        ev[i].kind = (uint8_t)(i % 5);
        ev[i].target = (uint32_t)(i % 160);
    }

    size_t words = (n + 63) / 64;
    Planes ran = { calloc(words, 8), calloc(words, 8) };
    Planes ver = { calloc(words, 8), calloc(words, 8) };
    if (!ran.known || !ran.value || !ver.known || !ver.value) { fprintf(stderr, "OOM planes\n"); return 2; }
    for (size_t i = 0; i < n; i++) {
        uint8_t o = col[i];
        if (o == 0) { /* nothing ran: known=1 in ran plane, value=0 */
            ran.known[i >> 6] |= 1ULL << (i & 63);
        } else {
            ran.known[i >> 6] |= 1ULL << (i & 63);
            ran.value[i >> 6] |= 1ULL << (i & 63);
            if (o == 1) { ver.known[i >> 6] |= 1ULL << (i & 63); ver.value[i >> 6] |= 1ULL << (i & 63); }
            else if (o == 2) { ver.known[i >> 6] |= 1ULL << (i & 63); }
        }
    }

    /* ---- the gate: every arm must agree with the Rust-side expectation BEFORE timing ------ */
    size_t a = scan_aos(ev, n), b = scan_bytes(col, n), p = scan_planes(&ran, &ver, n);
    if (a != expected || b != expected || p != expected) {
        fprintf(stderr, "COUNT MISMATCH n=%zu: aos=%zu bytes=%zu planes=%zu expected=%llu — "
                        "benchmark INVALID, no timings.\n", n, a, b, p, expected);
        return 1;
    }

    /* ---- timed, interleaved, same rep policy as the Rust side ----------------------------- */
    uint32_t reps = n <= 500000 ? 500 : n <= 5000000 ? 50 : 5;
    double *aos_t = malloc(reps * sizeof(double));
    double *byt_t = malloc(reps * sizeof(double));
    double *pln_t = malloc(reps * sizeof(double));
    for (uint32_t r = 0; r < reps; r++) {
        double t0 = now_us(); sink += scan_aos(ev, n);            aos_t[r] = now_us() - t0;
        double t1 = now_us(); sink += scan_bytes(col, n);         byt_t[r] = now_us() - t1;
        double t2 = now_us(); sink += scan_planes(&ran, &ver, n); pln_t[r] = now_us() - t2;
    }

    printf("C(MSVC /O2) %-12zu aos %10.1f us   bytes %10.1f us   planes %10.1f us   count %zu OK\n",
           n, median(aos_t, reps), median(byt_t, reps), median(pln_t, reps), p);
    printf("== versus_c COMPLETE (rc=0) ==\n");
    return 0;
}
