#!/usr/bin/env python3
"""
KOPPA parallel/distributed ratio-factor aggregation prototype.

Concrete application: several workers each hold a multiplicative factor expressed
as an UNRESOLVED ratio pair (numerator, denominator).  A reducer needs cumulative
(prefix) products of these factors for streaming ratio metrics (funnel conversion,
A/B likelihood ratios, error-rate ratios).  Resolving each factor to a float before
multiplying overflows/underflows and loses precision; keeping the factor as an
unresolved pair and composing pairs, resolving only on demand, stays finite and
matches an exact rational oracle.

Representation (solver, float-safe): a pair is (am, ae, bm, be) meaning
    numerator = am * 2**ae,  denominator = bm * 2**be,  am,bm in [0.5,1)
so the factor's ratio = (bm/am) * 2**(be-ae).  Mantissa/exponent separation is
the scale-invariant (g = r'/r) unresolved-pair form; it keeps the pair finite
across hundreds of decades where a single resolved float cannot.

Composition (associative multiplication):
    (a,b) (x,y) = (a*x, b*y) with mantissas renormalized by exact powers of two.
Resolution (on demand, once at the end): r = (bm/am) * 2**(be-ae).

This file is the PRODUCER.  It writes results to a JSON file; a separate checker
(koppa_app_scan_check.py) re-derives load-bearing numbers independently.
"""

import json
import math
import random
import time
from fractions import Fraction

SCRATCH = "/home/sudoshellz/.hermes/cache/scratch"
RESULTS = SCRATCH + "/koppa_app_scan_results.json"

IDENTITY = (0.5, 1, 0.5, 1)  # 0.5*2 = 1 numerator, 0.5*2 = 1 denominator


def pair_from_ratio_pow2(k):
    """Exact pair for ratio 2**k (numerator 1, denominator 2**k)."""
    # numerator = 1 = 0.5*2**1 ; denominator = 2**k = 0.5*2**(k+1)
    return (0.5, 1, 0.5, k + 1)


def pair_from_ints(a, b):
    """Exact pair for ratio b/a from positive Python ints (a,b <= ~1e300)."""
    # EDGE-CASE PATCH: float() overflows for ints beyond ~1.8e308, and a==0
    # would make `bm/am` a division by zero at resolve time. Guard both.
    if a == 0 or b == 0:
        raise ValueError("pair_from_ints requires nonzero a and b; got %r,%r" % (a, b))
    try:
        am, ae = math.frexp(float(a))
        bm, be = math.frexp(float(b))
    except OverflowError as exc:
        raise ValueError("pair_from_ints inputs exceed float64 range (%r,%r)" % (a, b)) from exc
    return (am, ae, bm, be)


def compose(p, q):
    """Associative multiplication of two unresolved ratio pairs."""
    am, ae, bm, be = p
    qm, qe, qn, qf = q
    cm, ce = math.frexp(am * qm)
    dm, de = math.frexp(bm * qn)
    return (cm, ae + qe + ce, dm, be + qf + de)


def resolve(p):
    """On-demand resolution of a pair to its ratio (single float)."""
    am, ae, bm, be = p
    exp = be - ae
    try:
        return math.ldexp(bm / am, exp)
    except OverflowError:
        return math.inf


def scan_double_buffer(seq):
    """Hillis-Steele inclusive prefix scan. Reads `src`, writes `dst` each pass."""
    n = len(seq)
    src = list(seq)
    offset = 1
    while offset < n:
        dst = list(src)                       # DOUBLE BUFFER: fresh copy
        for i in range(offset, n):
            dst[i] = compose(src[i - offset], src[i])  # both reads from src
        src = dst
        offset <<= 1
    return src


def scan_in_place(seq):
    """BUG DEMO: Hillis-Steele with UPWARD in-place updates (no double buffer).

    Within a pass of offset>=2, src[i-offset] has already been overwritten this
    pass, so compose reads a contaminated neighbour and the prefix products are
    wrong.  This is the failure double buffering prevents.
    """
    n = len(seq)
    src = list(seq)
    offset = 1
    while offset < n:
        for i in range(offset, n):            # upward -> reads updated neighbours
            src[i] = compose(src[i - offset], src[i])
        offset <<= 1
    return src


def fold_serial(seq):
    """Serial baseline: left fold with the same associative compose."""
    acc = IDENTITY
    out = []
    for p in seq:
        acc = compose(acc, p)
        out.append(acc)
    return out


# ----------------------------------------------------------------------------
# Independent exact oracle.  Deliberately a DIFFERENT route: exact rational
# arithmetic from the original integer inputs, never using `compose`/`resolve`.
# ----------------------------------------------------------------------------
def oracle_prefixes_ints(pairs_int):
    """pairs_int: list of (a,b) positive ints. Returns list of exact prefix
    Fraction products (Fraction(b,a) composing by exact rational multiply)."""
    out = []
    acc = Fraction(1, 1)
    for a, b in pairs_int:
        acc = acc * Fraction(b, a)
        out.append(acc)
    return out


def frac_to_float(fr):
    """Correctly-rounded big-int ratio -> float (Python's own Fraction.__float__)."""
    try:
        return float(fr)
    except OverflowError:
        return math.inf


def rel_err(x, y):
    """Relative error |x-y|/|y| with sane handling of 0/inf."""
    if x == y:
        return 0.0
    if not math.isfinite(y):
        return 0.0 if not math.isfinite(x) else math.inf
    if not math.isfinite(x):
        return math.inf
    den = abs(y) if y != 0 else 1.0
    return abs(x - y) / den


def main():
    rng = random.Random(20261118)
    report = {}

    # ---- Experiment 1: 12,000 random ratio pairs, incl. near-degenerate ----
    N = 12000
    pairs_int = []
    # Factors are kept near unity so cumulative prefix products stay in float
    # range: this isolates PRECISION of the unresolved-pair composition from the
    # separate overflow story told by experiment 3.
    kinds = {"random": 0, "near_degenerate": 0, "exact_one": 0}
    for i in range(N):
        r = i % 3
        if r == 0:                       # ~1 +/- 1e-6
            base = 10**12
            a = base + rng.randint(-10**6, 10**6)
            b = base + rng.randint(-10**6, 10**6)
            kinds["random"] += 1
        elif r == 1:                     # ~1 +/- 1e-15 (at double-precision edge)
            a = 10**15 + rng.randint(0, 10**5)
            b = a + rng.choice([-1, 1])
            kinds["near_degenerate"] += 1
        else:                            # exactly 1
            a = b = rng.randint(10**12, 10**13)
            kinds["exact_one"] += 1
        pairs_int.append((a, b))

    seq = [pair_from_ints(a, b) for a, b in pairs_int]

    t0 = time.perf_counter()
    scan_pairs = scan_double_buffer(seq)
    t_scan = time.perf_counter() - t0

    t0 = time.perf_counter()
    fold_pairs = fold_serial(seq)
    t_fold = time.perf_counter() - t0

    t0 = time.perf_counter()
    in_place_pairs = scan_in_place(seq)
    t_inplace = time.perf_counter() - t0

    t0 = time.perf_counter()
    oracle = oracle_prefixes_ints(pairs_int)
    t_oracle = time.perf_counter() - t0

    TOL = 1e-12
    scan_vals = [resolve(p) for p in scan_pairs]
    fold_vals = [resolve(p) for p in fold_pairs]
    inplace_vals = [resolve(p) for p in in_place_pairs]
    oracle_vals = [frac_to_float(f) for f in oracle]

    def stats(vals):
        errs = [rel_err(v, o) for v, o in zip(vals, oracle_vals)]
        finite = [e for e in errs if math.isfinite(e)]
        return {
            "max_rel_err": max(finite) if finite else None,
            "mean_rel_err": (sum(finite) / len(finite)) if finite else None,
            "disagreements_gt_1e-12": sum(1 for e in errs if e > TOL),
            "nonfinite_outputs": sum(1 for v in vals if not math.isfinite(v)),
        }

    report["experiment1"] = {
        "n_pairs": N,
        "kinds": kinds,
        "tolerance": TOL,
        "scan": stats(scan_vals),
        "fold": stats(fold_vals),
        "in_place_buggy": stats(inplace_vals),
        "timing_seconds": {
            "double_buffer_scan": t_scan,
            "serial_fold": t_fold,
            "in_place_scan": t_inplace,
            "fraction_oracle": t_oracle,
        },
        "scan_equals_fold_exact_float": sum(1 for a, b in zip(scan_vals, fold_vals) if a == b),
        "in_place_disagrees_with_scan": sum(1 for a, b in zip(inplace_vals, scan_vals) if a != b),
    }

    # ---- Experiment 2: eager float resolution (single product) vs deferred ----
    # Resolve each factor to a float first, then multiply: this is the approach
    # that overflows/underflows and loses precision on near-degenerate factors.
    eager_vals = []
    eager = 1.0
    for a, b in pairs_int:
        eager *= float(b) / float(a)
        eager_vals.append(eager)
    report["experiment2_eager"] = stats(eager_vals)
    report["experiment2_eager"]["note"] = (
        "resolve-then-multiply baseline over the same 12000 pairs"
    )

    # ---- Experiment 3: end-to-end distributed aggregation scenario ----
    # Simulated MapReduce reduction of 8 A/B-test shards.  Each shard emits a
    # likelihood-ratio factor as an unresolved pair; reducers compute cumulative
    # (prefix) likelihood ratios via the associative scan.  Two factors are
    # astronomically extreme (2**1100 and 2**-1100) -- a single resolved float
    # overflows/underflows, but the product across shards stays moderate.
    shard_log2 = [1100, -1100, 1100, -1100, 50, -50, 3, 1]
    shard_pairs = [pair_from_ratio_pow2(k) for k in shard_log2]
    shard_ints = []
    for k in shard_log2:
        if k >= 0:
            shard_ints.append((1, 1 << k))
        else:
            shard_ints.append((1 << (-k), 1))

    dist_scan = [resolve(p) for p in scan_double_buffer(shard_pairs)]
    dist_oracle = [frac_to_float(f) for f in oracle_prefixes_ints(shard_ints)]
    # eager: resolving shard 0 as a float overflows
    eager_dist = []
    e = 1.0
    try:
        for a, b in shard_ints:
            e *= float(b) / float(a)
    except OverflowError:
        e = math.inf
    eager_dist.append(e)
    report["experiment3_distributed"] = {
        "shard_log2_ratios": shard_log2,
        "scan_prefix_likelihood_ratios": dist_scan,
        "exact_prefix_likelihood_ratios": dist_oracle,
        "max_rel_err": max(rel_err(x, y) for x, y in zip(dist_scan, dist_oracle)),
        "disagreements_gt_1e-12": sum(
            1 for x, y in zip(dist_scan, dist_oracle) if rel_err(x, y) > TOL
        ),
        "eager_float_result_finite": math.isfinite(eager_dist[0]),
        "eager_float_result": "inf/NaN (overflow)" if not math.isfinite(eager_dist[0]) else eager_dist[0],
        "final_deferred_result": dist_scan[-1],
        "final_exact_result": dist_oracle[-1],
    }

    # ---- Associativity spot check (small exhaustive-ish) -------------------
    small = [pair_from_ints(a, b) for a in range(1, 5) for b in range(1, 5)]
    assoc_ok = 0
    assoc_bad = 0
    for p in small[:8]:
        for q in small[8:12]:
            for r in small[12:16]:
                left = compose(compose(p, q), r)
                right = compose(p, compose(q, r))
                if rel_err(resolve(left), resolve(right)) <= 1e-12:
                    assoc_ok += 1
                else:
                    assoc_bad += 1
    report["associativity"] = {"checked": assoc_ok + assoc_bad, "ok": assoc_ok, "bad": assoc_bad}

    # ---- Experiment 4: large-exponent deferred resolution -----------------
    # Alternating 2**1100 and 2**-1100: a single resolved factor overflows float
    # (2**1100 >> 1.8e308).  Deferred pair composition keeps every intermediate
    # PAIR finite (mantissas normalized to [0.5,1)); resolution happens only on
    # demand.  Even-length prefixes equal exactly 1.  Eager float resolution
    # overflows on the first factor and degenerates to NaN.
    chain_log2 = [1100, -1100] * 600
    chain_pairs = [pair_from_ratio_pow2(k) for k in chain_log2]
    chain_fold = fold_serial(chain_pairs)
    chain_scan = scan_double_buffer(chain_pairs)
    # Pair state finiteness: mantissas must stay in [0.5,1), exponents bounded.
    def pairs_finite(ps):
        return all(0.5 <= m < 1.0 and math.isfinite(m) and math.isfinite(e)
                   for p in ps for m, e in ((p[0], p[1]), (p[2], p[3])))
    fold_pairs_finite = pairs_finite(chain_fold)
    scan_pairs_finite = pairs_finite(chain_scan)
    # Even-prefix resolved values should be exactly 1.0; odd prefixes are the
    # true value 2**1100, beyond float (inf) -- that is correct, not a collapse.
    even_prefixes = [resolve(chain_fold[i]) for i in range(1, len(chain_fold), 2)]
    even_all_one = all(v == 1.0 for v in even_prefixes)
    # Eager float: resolving 2**1100 overflows immediately.
    e = 1.0
    eager_states = []
    for k in chain_log2:
        e *= math.ldexp(1.0, k) if k < 1024 else math.inf
        eager_states.append(e)
    eager_nan_count = sum(1 for v in eager_states if math.isnan(v))
    eager_inf_count = sum(1 for v in eager_states if math.isinf(v))
    report["experiment4_large_exponent"] = {
        "n_factors": len(chain_log2),
        "exponent_magnitude_log2": 1100,
        "exponent_span_decades": round(2200 * math.log10(2)),
        "deferred_fold_all_pair_states_finite": fold_pairs_finite,
        "deferred_scan_all_pair_states_finite": scan_pairs_finite,
        "deferred_even_prefixes_exactly_1": even_all_one,
        "deferred_pair_state_collapses": 0 if (fold_pairs_finite and scan_pairs_finite) else 1,
        "eager_nonfinite_states_inf": eager_inf_count,
        "eager_nonfinite_states_nan": eager_nan_count,
        "eager_final": "nan" if math.isnan(eager_states[-1]) else str(eager_states[-1]),
        "deferred_final_resolved": resolve(chain_fold[-1]),
    }

    def sanitize(o):
        """Make the report JSON-strict: non-finite floats become descriptive strings."""
        if isinstance(o, float) and not math.isfinite(o):
            return "inf" if o > 0 else ("-inf" if o < 0 else "nan")
        if isinstance(o, dict):
            return {k: sanitize(v) for k, v in o.items()}
        if isinstance(o, (list, tuple)):
            return [sanitize(v) for v in o]
        return o

    clean = sanitize(report)
    with open(RESULTS, "w") as f:
        json.dump(clean, f, indent=2, allow_nan=False)
    print(json.dumps(clean, indent=2, allow_nan=False))
    return report


if __name__ == "__main__":
    main()
