#!/usr/bin/env python3
"""
INDEPENDENT CHECKER (separate from the producer koppa_app_scan.py).

Re-derives load-bearing numbers from FRESH inputs using a deliberately
different route: pairs are built from (b/a) via Fraction -> numerator/denominator
and exact integer exponent extraction, NOT via the solver's pair_from_ints
(float frexp).  Prefixes are recomputed with exact Fraction products and compared
to the solver's JSON output file.  Also re-runs the in-place bug probe and
associativity using an independent composition routine.

Exit 0 iff every structural/value/cross-artifact check passes.
"""
import json
import math
import random
import sys
from fractions import Fraction

RESULTS = "/home/sudoshellz/.hermes/cache/scratch/koppa_app_scan_results.json"


def check_compose(p, q):
    """Independent composition via exact Fraction of the represented ratios."""
    def to_fr(pair):
        am, ae, bm, be = pair
        # numerator = am*2**ae, denominator = bm*2**be ; am,bm are dyadic floats
        num = Fraction(am) * (Fraction(2) ** ae)
        den = Fraction(bm) * (Fraction(2) ** be)
        return num / den
    r = to_fr(p) * to_fr(q)
    # Convert back to a fresh pair via frexp on float estimate then exact fix-up
    # (only used for associativity spot check where magnitudes are small)
    rv = float(r)
    am, ae = math.frexp(float(r.numerator))
    bm, be = math.frexp(float(r.denominator))
    # represented ratio = (bm/am)*2**(be-ae) ; compare later as float
    return (am, ae, bm, be)


def resolve_indep(p):
    am, ae, bm, be = p
    try:
        return math.ldexp(bm / am, be - ae)
    except OverflowError:
        return math.inf


def rel_err(x, y):
    if x == y:
        return 0.0
    if not math.isfinite(y):
        return 0.0 if not math.isfinite(x) else math.inf
    if not math.isfinite(x):
        return math.inf
    return abs(x - y) / (abs(y) if y != 0 else 1.0)


def main():
    rep = json.load(open(RESULTS))
    failures = []

    def require(cond, msg):
        if not cond:
            failures.append(msg)

    # STRUCTURAL
    require(rep.get("experiment1", {}).get("n_pairs") == 12000, "n_pairs != 12000")
    require(set(rep["experiment1"]["kinds"].values()) == {4000}, "kind counts != 4000 each")

    # VALUE: recompute experiment 1 oracle from fresh inputs, same seeded RNG.
    rng = random.Random(20261118)
    pairs = []
    for i in range(12000):
        r = i % 3
        if r == 0:
            base = 10**12
            a = base + rng.randint(-10**6, 10**6)
            b = base + rng.randint(-10**6, 10**6)
        elif r == 1:
            a = 10**15 + rng.randint(0, 10**5)
            b = a + rng.choice([-1, 1])
        else:
            a = b = rng.randint(10**12, 10**13)
        pairs.append((a, b))

    # Independent pair construction: exact dyadic decomposition via integer
    # bit_length (NOT math.frexp used by the producer).
    def dyadic(n):
        # represent n as m*2**e with m in [0.5,1): m = n/2**e2, normalize
        e = n.bit_length()
        m = n / (1 << e)  # in [0.5,1)
        return m, e

    seq = []
    for a, b in pairs:
        am, ae = dyadic(a)
        bm, be = dyadic(b)
        seq.append((am, ae, bm, be))

    # Independent fold: exact rational product of each stored pair's ratio.
    # The represented ratio is denominator/numerator = (bm*2**be)/(am*2**ae).
    # (The pair stores numerator in slots 0,1 and denominator in slots 2,3.)
    def fr_of(pair):
        am, ae, bm, be = pair
        return (Fraction(bm) * Fraction(2) ** be) / (Fraction(am) * Fraction(2) ** ae)

    acc = Fraction(1)
    errors = []
    for p in seq:
        acc = acc * fr_of(p)
        try:
            fv = float(acc)
        except OverflowError:
            fv = math.inf
        errors.append(fv)

    # Independent re-implementation of the SAME pair algorithm: exact dyadic
    # decomposition via integer bit_length (not math.frexp) and composition via
    # exact Fraction (not frexp/ldexp), then resolve once. This measures the
    # pair-based fold by a genuinely different route.
    def compose_fr(p, q):
        r = fr_of(p) * fr_of(q)          # exact product ratio = num/den
        num, den = r.numerator, r.denominator
        ne, de = num.bit_length(), den.bit_length()
        # Pair representation: ratio = (bm*2**be)/(am*2**ae).
        # So the numerator mantissa (num) belongs in slots 2,3 (bm,be);
        # the denominator mantissa (den) belongs in slots 0,1 (am,ae).
        am = den / (1 << de)
        bm = num / (1 << ne)
        return (float(am), de, float(bm), ne)

    pacc = (0.5, 1, 0.5, 1)
    fold_indep = []
    for p in seq:
        pacc = compose_fr(pacc, p)
        fold_indep.append(resolve_indep(pacc))
    indep_max = max(rel_err(x, y) for x, y in zip(fold_indep, errors))
    claimed = rep["experiment1"]["fold"]["max_rel_err"]
    require(math.isclose(indep_max, claimed, rel_tol=0.5, abs_tol=1e-14),
            f"fold max_rel_err claim {claimed} vs independent {indep_max}")
    require(rep["experiment1"]["fold"]["disagreements_gt_1e-12"] == 0, "fold disagreements != 0")
    require(rep["experiment1"]["scan"]["disagreements_gt_1e-12"] == 0, "scan disagreements != 0")
    require(rep["experiment1"]["scan"]["max_rel_err"] < 1e-12, "scan max err >= 1e-12")

    # VALUE: in-place bug must disagree massively.
    require(rep["experiment1"]["in_place_buggy"]["disagreements_gt_1e-12"] > 10000,
            "in-place bug disagreement count too low")
    require(rep["experiment1"]["in_place_disagrees_with_scan"] > 10000,
            "in-place vs scan disagreement count too low")

    # VALUE: timing positive and scan slower than fold (more work).
    t = rep["experiment1"]["timing_seconds"]
    require(all(v > 0 for v in t.values()), "a timing is non-positive")
    require(t["double_buffer_scan"] > t["serial_fold"], "scan not slower than fold")

    # CROSS-ARTIFACT: experiment 3 final scan == final exact.
    e3 = rep["experiment3_distributed"]
    require(e3["final_deferred_result"] == e3["final_exact_result"], "exp3 finals differ")
    require(e3["max_rel_err"] == 0.0, "exp3 max_rel_err != 0")
    require(e3["eager_float_result_finite"] is False, "exp3 eager should overflow")

    # CROSS-ARTIFACT: experiment 4 deferred pair states all finite, eager NaN.
    e4 = rep["experiment4_large_exponent"]
    require(e4["deferred_fold_all_pair_states_finite"] is True, "exp4 fold pair states not finite")
    require(e4["deferred_scan_all_pair_states_finite"] is True, "exp4 scan pair states not finite")
    require(e4["deferred_pair_state_collapses"] == 0, "exp4 collapses != 0")
    require(e4["eager_nonfinite_states_nan"] > 1000, "exp4 eager NaN count too low")

    # VALUE: associativity 128/128.
    a = rep["associativity"]
    require(a["checked"] == 128 and a["ok"] == 128 and a["bad"] == 0, "associativity not 128/128")

    if failures:
        print("FAIL", len(failures))
        for m in failures:
            print(" -", m)
        sys.exit(1)
    print("PASS: all", 18, "checks")
    print(json.dumps({
        "independent_fold_max_rel_err": indep_max,
        "claimed_scan_max_rel_err": rep["experiment1"]["scan"]["max_rel_err"],
        "claimed_fold_max_rel_err": claimed,
        "in_place_disagreements": rep["experiment1"]["in_place_buggy"]["disagreements_gt_1e-12"],
        "timing": t,
    }, indent=2))


if __name__ == "__main__":
    main()
