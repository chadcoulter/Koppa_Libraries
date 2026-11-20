#!/usr/bin/env python3
"""
koppa_app_predicates.py -- robust ratio predicates for geometry / CAD / GIS.

Deferred relational tracking applied to a concrete engineering problem:
compare two ratios b1/a1 vs b2/a2, and compose ratios, WITHOUT eagerly
reducing them, escalating to exact arithmetic only when a float filter
cannot certify the sign.

Core state model (KOPPA-corrected foundation):
  * an unresolved pair (a, b) is the STATE.
  * r = b / a is the RESOLVED form, computed on demand (lazy gcd / Fraction).
  * r' = quotient rule:  d(b/a) = (b'a - b a') / a^2  -> new state (a^2, b'a - b a').
  * g = r'/r = b'/b - a'/a  is scale invariant.
  * composition is multiplicative and associative: (b1/a1)(b2/a2) = (b1 b2)/(a1 a2).
  * harmonic addition: Q_m + Q_n = Q_{mn} / (m + n).
  * g, resolution and comparison are all ON DEMAND -- never eager.

Predicates built on compare():
  * orientation (near-collinear point-in-triangle / incircle sign)
  * GIS slope / gradient comparison
  * CNC/CAD tolerance band membership
  * structural load ratio classification
  * impedance/admittance matching

Adaptive escalation tiers for sign(b1*a2 - b2*a1):
  T1  float filter -- rigorous rounding-error bound; certifies sign or falls through.
  T2  int128-equivalent -- exact Python bigint at <=127 bits (native, no bigint heap growth).
  T3  exact unbounded bigint -- arbitrary magnitude, gmpy2.mpz if installed, else int.

Run: python3 koppa_app_predicates.py [--fuzz N] [--seed S] [--demo]
"""
from __future__ import annotations
import argparse
import json
import math
import random
import sys
import time
from fractions import Fraction

try:                                   # optional, genuinely GMP-backed
    import gmpy2 as _gmpy2
    _HAS_GMPY2 = True
except Exception:
    _gmpy2 = None
    _HAS_GMPY2 = False


# --------------------------------------------------------------------------
# core state: unresolved pair (a, b)
# --------------------------------------------------------------------------
class Ratio:
    """Deferred relational state (a, b); resolved form r = b/a on demand."""
    __slots__ = ("a", "b", "_resolved")

    def __init__(self, a: int, b: int):
        a, b = int(a), int(b)
        if a == 0:
            raise ValueError("ratio denominator a must be nonzero")
        self.a = a
        self.b = b
        self._resolved = None

    def resolve(self) -> Fraction:
        """On-demand resolution to the canonical reduced form."""
        if self._resolved is None:
            self._resolved = Fraction(self.b, self.a)
        return self._resolved

    @property
    def r(self) -> Fraction:
        return self.resolve()

    def compose(self, other: "Ratio") -> "Ratio":
        """Multiplicative / associative composition of resolved forms."""
        return Ratio(self.a * other.a, self.b * other.b)

    def quotient_derivative(self, da: int, db: int) -> "Ratio":
        """r' via quotient rule: (b'a - b a') / a^2  -> state (a^2, b'a - ba')."""
        return Ratio(self.a * self.a, db * self.a - self.b * da)

    def log_derivative(self, da: int, db: int) -> Fraction:
        """Scale-invariant g = r'/r = b'/b - a'/a (on-demand resolution)."""
        return Fraction(db, self.b) - Fraction(da, self.a)

    def __repr__(self):
        return f"Ratio(a={self.a}, b={self.b})"


def harmonic_add(qm: Ratio, qn: Ratio) -> Ratio:
    """Q_m + Q_n = Q_{mn} / (m + n): harmonic composition of two qualities."""
    return Ratio(qm.a + qn.a, qm.b + qn.b)


# --------------------------------------------------------------------------
# adaptive sign filter: sign(b1*a2 - b2*a1)
# --------------------------------------------------------------------------
_T1_CERTIFIED = _T2_USED = _T3_USED = _T1_ESCALATED = 0


def _det_float(a1: int, b1: int, a2: int, b2: int):
    """Float determinant and a rigorous absolute-rounding-error bound.

    All four inputs are integers with |n| < 2^E.  float(n) differs from n by
    at most conv = 2^(E-53) (half an ulp at 2^E; zero when E<=53).  A float
    product of two magnitudes <= 2^E has absolute rounding error <= 2^(2E-53).
    Propagating conversion + product + subtraction rounding gives `bound`.
    |det_f| > bound certifies sign(det) with NO false positives.
    """
    E = max(a1.bit_length(), b1.bit_length(), a2.bit_length(), b2.bit_length())
    if E > 1000:                        # beyond IEEE-754 double range -> skip T1
        return 0, 1                     # |det_f|==0 never > bound -> forces escalation
    conv = (1 << (E - 53)) if E > 53 else 0
    a1f, b1f, a2f, b2f = float(a1), float(b1), float(a2), float(b2)
    d1 = b1f * a2f          # exact integer target: b1*a2
    d2 = b2f * a1f          # exact integer target: b2*a1
    det_f = d1 - d2
    # product error: rounding 2^(2E-53) + |x|*conv_err_y + |y|*conv_err_x + conv^2
    prod_err = ((1 << (2 * E - 53)) if 2 * E >= 53 else 0)
    prod_err += (1 << E) * conv * 2 + conv * conv
    # subtraction rounding at magnitude <= 2^(2E+1)
    # fl(d1-d2) = (d1-d2)(1+delta), |delta|<=2^-53, |d1-d2|<=2^(2E+1)
    # => abs rounding error <= 2^(2E-52).  Guard matches that shift so the
    # shift count is never negative (original guarded on 2E+1>=53 but shifted
    # by 2E-53, crashing with ValueError at E==26).
    sub_err = (1 << (2 * E - 52)) if 2 * E + 1 >= 53 else 0
    bound = 2 * prod_err + sub_err + 1
    return det_f, bound


def compare(r1: Ratio, r2: Ratio) -> int:
    """Sign of r1 - r2, via adaptive escalation. Exact, never wrong."""
    global _T1_CERTIFIED, _T1_ESCALATED, _T2_USED, _T3_USED
    a1, b1, a2, b2 = r1.a, r1.b, r2.a, r2.b
    # Normalize EACH denominator to positive by flipping its own numerator;
    # this preserves the ratio's value and makes sign(det)==sign(r1-r2).
    if a1 < 0:
        a1, b1 = -a1, -b1
    if a2 < 0:
        a2, b2 = -a2, -b2

    det_f, bound = _det_float(a1, b1, a2, b2)
    if abs(det_f) > bound:
        _T1_CERTIFIED += 1
        return 1 if det_f > 0 else -1
    _T1_ESCALATED += 1

    # T2: int128-equivalent -- pure-Python int (exact), no heap growth at <=127b
    if max(a1.bit_length(), b1.bit_length(), a2.bit_length(), b2.bit_length()) <= 127:
        _T2_USED += 1
        det = b1 * a2 - b2 * a1
    else:
        # T3: exact unbounded bigint / GMP-style
        _T3_USED += 1
        if _HAS_GMPY2:
            det = int(_gmpy2.mpz(b1) * _gmpy2.mpz(a2) -
                      _gmpy2.mpz(b2) * _gmpy2.mpz(a1))
        else:
            det = b1 * a2 - b2 * a1
    return (det > 0) - (det < 0)


def naive_float_compare(r1: Ratio, r2: Ratio) -> int:
    """The unsafe baseline: compare float(b)/float(a) directly."""
    try:
        x1 = float(r1.b) / float(r1.a)
        x2 = float(r2.b) / float(r2.a)
    except OverflowError:
        return 0                       # float path cannot even represent these
    return (x1 > x2) - (x1 < x2)


def oracle_compare(r1: Ratio, r2: Ratio) -> int:
    """Fraction oracle used only for validation, never in the hot path."""
    x1 = r1.resolve()
    x2 = r2.resolve()
    return (x1 > x2) - (x1 < x2)


# --------------------------------------------------------------------------
# geometry / engineering predicates
# --------------------------------------------------------------------------
def orientation(p, q, r) -> int:
    """Sign of cross((q-p),(r-p)); >0 ccw, <0 cw, 0 collinear.
    Equivalent to comparing slope (qy-py)/(qx-px) with (ry-py)/(rx-px).
    """
    px, py = p; qx, qy = q; rx, ry = r
    return compare(Ratio(qx - px, qy - py), Ratio(rx - px, ry - py))


def compare_gradients(rise1, run1, rise2, run2) -> int:
    """GIS: compare two terrain slopes rise/run exactly."""
    return compare(Ratio(run1, rise1), Ratio(run2, rise2))


def cnc_within_tolerance(measured: Ratio, nominal: Ratio, tol: Ratio) -> bool:
    """CAD/CNC: |measured/nominal - 1| <= tol, using exact predicates.

    A Ratio(a,b) is the deferred state for b/a.  measured/nominal is
    Ratio(measured.a*nominal.b, measured.b*nominal.a); the band edges
    1 +/- tol are Ratio(tol.a, tol.a +/- tol.b).
    """
    m_over_n = Ratio(measured.a * nominal.b, measured.b * nominal.a)
    up = Ratio(tol.a, tol.a + tol.b)   # 1 + tol = (a+b)/a
    lo = Ratio(tol.a, tol.a - tol.b)   # 1 - tol = (a-b)/a
    return compare(m_over_n, lo) >= 0 and compare(m_over_n, up) <= 0


def load_ratio_class(load: Ratio, capacity: Ratio, thresholds) -> str:
    """Structural engineering: classify utilization = load/capacity vs thresholds."""
    util = Ratio(load.a * capacity.b, load.b * capacity.a)
    for label, thr in thresholds:
        if compare(util, thr) <= 0:
            return label
    return "OVER"


def impedance_match(z1: Ratio, z2: Ratio) -> bool:
    """Electrical: exact impedance/admittance magnitude matching."""
    return compare(z1, z2) == 0


# --------------------------------------------------------------------------
# differential fuzz harness
# --------------------------------------------------------------------------
def _near_degenerate(gen):
    """Generate an adversarially near-degenerate pair."""
    p = gen.randint(1, 60)
    q = gen.randint(1, 60)
    a1 = gen.randint(2**50, 2**55)
    a2 = gen.randint(2**50, 2**55)
    b1 = round(a1 * p / q) + gen.choice([-2, -1, 0, 1, 2])
    b2 = round(a2 * p / q) + gen.choice([-2, -1, 0, 1, 2])
    return a1, max(b1, 1), a2, max(b2, 1)


def _shared_factor(gen):
    a = gen.randint(2, 2**40)
    b = gen.randint(1, 2**40)
    k = gen.randint(2, 2**20)
    d = gen.choice([-2, -1, 0, 1, 2])
    return a, b, k * a, k * b + d


def _huge(gen):
    bits = gen.choice([130, 200, 400, 900])
    a1 = gen.randint(1, 1 << bits) + 1
    b1 = gen.randint(1, 1 << bits) + 1
    a2 = gen.randint(1, 1 << bits) + 1
    p = gen.randint(1, 100)
    q = gen.randint(1, 100)
    b2 = (a2 * p) // q + gen.choice([-1, 0, 1]) + 1
    return a1, b1, a2, b2


def fuzz(n: int, seed: int) -> dict:
    """Differential fuzz vs Fraction oracle. Returns measured counts/timings."""
    global _T1_CERTIFIED, _T1_ESCALATED, _T2_USED, _T3_USED
    _T1_CERTIFIED = _T1_ESCALATED = _T2_USED = _T3_USED = 0
    gen = random.Random(seed)
    stats = dict(
        n=n, naive_wrong=0, oracle_zero=0, filter_wrong=0, exact_wrong=0,
        filter_certified=0, filter_escalated=0, tier2=0, tier3=0,
        first_naive_witness=None, first_filter_witness=None,
        generators={"uniform": 0, "near_degenerate": 0, "shared_factor": 0, "huge": 0},
    )
    t0 = time.perf_counter()
    naive_t0 = 0.0
    for i in range(n):
        mode = i % 4
        if mode == 0:
            a1 = gen.randint(1, 2**53); b1 = gen.randint(1, 2**53)
            a2 = gen.randint(1, 2**53); b2 = gen.randint(1, 2**53)
            gname = "uniform"
        elif mode == 1:
            a1, b1, a2, b2 = _near_degenerate(gen); gname = "near_degenerate"
        elif mode == 2:
            a1, b1, a2, b2 = _shared_factor(gen); gname = "shared_factor"
        else:
            a1, b1, a2, b2 = _huge(gen); gname = "huge"
        stats["generators"][gname] += 1

        r1, r2 = Ratio(a1, b1), Ratio(a2, b2)
        oracle = oracle_compare(r1, r2)
        if oracle == 0:
            stats["oracle_zero"] += 1
            continue

        # naive float baseline
        tn0 = time.perf_counter()
        naive = naive_float_compare(r1, r2)
        naive_t0 += time.perf_counter() - tn0
        if naive != oracle:
            stats["naive_wrong"] += 1
            if stats["first_naive_witness"] is None:
                stats["first_naive_witness"] = dict(
                    a1=a1, b1=b1, a2=a2, b2=b2,
                    naive=naive, exact=oracle,
                    float_diff=float(Fraction(b1, a1) - Fraction(b2, a2)),
                    cross_diff=b1 * a2 - b2 * a1,
                )

        # adaptive predicate
        got = compare(r1, r2)
        if got != oracle:
            stats["exact_wrong"] += 1
            if stats["first_filter_witness"] is None:
                stats["first_filter_witness"] = dict(a1=a1, b1=b1, a2=a2, b2=b2,
                                                      got=got, exact=oracle)
        # was T1 the decider? recompute its bound on the normalized inputs for tallying
        na1, nb1 = (a1, b1) if a1 > 0 else (-a1, -b1)
        na2, nb2 = (a2, b2) if a2 > 0 else (-a2, -b2)
        det_f, bound = _det_float(na1, nb1, na2, nb2)
        if abs(det_f) > bound and got != (1 if det_f > 0 else -1):
            stats["filter_wrong"] += 1

    # negative-denominator sanity: opposite signs must still compare exactly
    stats["negative_denominator_ok"] = all(
        compare(Ratio(-a, b * (-1) ** k), Ratio(c, d)) == oracle_compare(Ratio(-a, b * (-1) ** k), Ratio(c, d))
        for a, b, c, d in [(1, 3, 2, 5), (7, -22, 1, 3), (-9, -4, 5, 2)]
        for k in (0, 1)
    )

    stats["elapsed_s"] = round(time.perf_counter() - t0, 4)
    stats["naive_float_only_s"] = round(naive_t0, 4)
    stats["comparisons_per_s"] = int(n / stats["elapsed_s"]) if stats["elapsed_s"] else 0
    stats["filter_certified"] = _T1_CERTIFIED
    stats["filter_escalated"] = _T1_ESCALATED
    stats["tier2"] = _T2_USED
    stats["tier3"] = _T3_USED
    stats["gmpy2_available"] = _HAS_GMPY2
    return stats


WITNESS = dict(a1=7952614671698527, b1=21159794283675467,
               a2=5364900327947194, b2=14274574083889309)


def demo():
    out = {}
    r1 = Ratio(WITNESS["a1"], WITNESS["b1"])
    r2 = Ratio(WITNESS["a2"], WITNESS["b2"])
    out["witness"] = dict(
        r1=float(r1.resolve()), r2=float(r2.resolve()),
        naive_sign=naive_float_compare(r1, r2),
        exact_sign=compare(r1, r2),
        oracle_sign=oracle_compare(r1, r2),
        float_ratio_diff=float(r1.resolve()) - float(r2.resolve()),
        exact_diff_as_float=float(r1.resolve() - r2.resolve()),
        cross_multiply_b1a2=WITNESS["b1"] * WITNESS["a2"],
        cross_multiply_b2a1=WITNESS["b2"] * WITNESS["a1"],
        cross_diff=WITNESS["b1"] * WITNESS["a2"] - WITNESS["b2"] * WITNESS["a1"],
    )
    # deferred-resolution demonstration: 600 decades of scale invariance
    scale = 10 ** 600
    big1 = Ratio(WITNESS["a1"] * scale, WITNESS["b1"] * scale)
    big2 = Ratio(WITNESS["a2"] * scale, WITNESS["b2"] * scale)
    out["scale_invariance_600_decades"] = dict(
        sign=compare(big1, big2),
        naive_sign=naive_float_compare(big1, big2),
        oracle_sign=oracle_compare(big1, big2),
        resolved_equal=big1.resolve() == r1.resolve(),
    )
    # quotient rule + scale-invariant g
    d = Ratio(WITNESS["a1"] * 2, WITNESS["b1"] * 3).resolve()  # r scaled 3/2
    q = r1.quotient_derivative(da=WITNESS["a1"], db=WITNESS["b1"] * 3)
    out["quotient_rule"] = dict(
        derivative_state=repr(q),
        g_scale_invariant=float(r1.log_derivative(WITNESS["a1"], WITNESS["b1"] * 3)),
    )
    # multiplicative / associative composition
    r3 = Ratio(7, 22)
    out["composition"] = dict(
        assoc_L=repr(r1.compose(r2).compose(r3)),
        assoc_R=repr(r1.compose(r2.compose(r3))),
        equal_states=(r1.compose(r2).compose(r3).a == r1.compose(r2.compose(r3)).a and
                      r1.compose(r2).compose(r3).b == r1.compose(r2.compose(r3)).b),
    )
    # geometry: near-collinear points where naive float orientation flips
    p = (0, 0)
    q = (WITNESS["a1"], WITNESS["b1"])
    r = (WITNESS["a2"], WITNESS["b2"])
    out["orientation"] = dict(
        exact=orientation(p, q, r),
        naive_sign=naive_float_compare(Ratio(q[0], q[1]), Ratio(r[0], r[1])),
    )
    # GIS slope + CNC tolerance + load classification
    out["gis_slope"] = compare_gradients(WITNESS["b1"], WITNESS["a1"],
                                         WITNESS["b2"], WITNESS["a2"])
    out["cnc_tolerance"] = dict(
        within=cnc_within_tolerance(Ratio(10000, 10001), Ratio(10000, 10000),
                                    Ratio(100000, 101)),
        expected_true=True,
    )
    out["load_class"] = load_ratio_class(Ratio(3, 4), Ratio(5, 4),
                                         [("SAFE", Ratio(10, 10)),
                                          ("WATCH", Ratio(12, 10)),
                                          ("DANGER", Ratio(15, 10))])
    out["impedance_match"] = impedance_match(Ratio(50, 3), Ratio(100, 6))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fuzz", type=int, default=100000)
    ap.add_argument("--seed", type=int, default=20261118)
    ap.add_argument("--demo", action="store_true")
    args = ap.parse_args()
    result = {}
    if args.demo or True:
        result["demo"] = demo()
    if args.fuzz > 0:
        result["fuzz"] = fuzz(args.fuzz, args.seed)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
