#!/usr/bin/env python3
"""
KOPPA deferred-relational tracking -- finance / actuarial application demo.

State is an UNRESOLVED homogeneous pair (a, b) standing for the ratio b/a.
Composition is componentwise multiplication (Q_m (*) Q_n = Q_mn) with exact
integer common-factor cancellation; a scalar is produced only on demand at the
end (b/a).  Compared against eager float64, hand-written log-domain, and an
80-digit mpmath reference.

No novelty/theorem claims are made here: this is an application prototype.
"""
from __future__ import annotations

import json
import math
import random
import time
from dataclasses import dataclass
from typing import List, Tuple

import mpmath as mp

Pair = Tuple[int, int]  # (a, b) with b/a the represented ratio

mp.mp.dps = 80


# --------------------------------------------------------------------------
# The deferred-relational API (compose without dividing, resolve on demand)
# --------------------------------------------------------------------------
class DeferredChain:
    __slots__ = ("a", "b", "composes", "cancellations", "resolves", "resolve_collapses")

    def __init__(self) -> None:
        self.a = 1
        self.b = 1
        self.composes = 0
        self.cancellations = 0
        self.resolves = 0
        self.resolve_collapses = 0

    def compose(self, a: int, b: int) -> None:
        """Q_m (*) Q_n = Q_mn : multiply componentwise, cancel common factors.

        Cancelling gcd(a,b) preserves the unresolved pair (it does NOT resolve
        to a scalar) and keeps the integers bounded by the running ratio.
        """
        self.a *= a
        self.b *= b
        g = math.gcd(self.a, self.b)
        if g > 1:
            self.a //= g
            self.b //= g
            self.cancellations += 1
        self.composes += 1

    def compose_decimal(self, a: int, b: int) -> None:
        """Specialized compose for a chain of factors sharing a fixed decimal
        scale S: strip a common power of two from (a,b) instead of a full gcd.
        Both numerator and denominator carry S^k; stripping 2-adic common
        factors keeps the pair small without ever resolving to a scalar.
        """
        # BUGFIX: strip only the COMMON power of two from BOTH operands.
        # The prior code stripped each independently (a>>a2, b>>b2), which
        # discarded the unequal 2-adic surplus and changed the represented
        # ratio: compose_decimal(2,3) resolved to 3.0 instead of 1.5.
        if a == 0 or b == 0:
            self.a *= a
            self.b *= b
            self.composes += 1
            return
        a2 = (a & -a).bit_length() - 1
        b2 = (b & -b).bit_length() - 1
        common = min(a2, b2)
        # Preserve 2^(a2-common) on the numerator and 2^(b2-common) on the
        # denominator: dividing BOTH by 2^common leaves b/a unchanged.
        self.a *= (a >> common)
        self.b *= (b >> common)
        self.composes += 1
        self.cancellations += (1 if common else 0)

    def compose_all(self, chain: List[Pair]) -> None:
        for a, b in chain:
            self.compose(a, b)

    def resolve_float(self) -> float:
        if self.a == 0:
            if self.b == 0:
                return float("nan")   # exact 0/0
            return float("inf") if self.b > 0 else -float("inf")  # sign of inf
        try:
            return self.b / self.a  # single division, on demand
        except OverflowError:
            # EDGE-CASE PATCH: b/a can exceed float64 max even though both are
            # exact ints (e.g. 1e350). Return the signed infinity, as the
            # scan/ml demos already do, instead of crashing.
            return math.inf if (self.a > 0) == (self.b > 0) else -math.inf

    def resolve_mp(self) -> mp.mpf:
        if self.a == 0:
            if self.b == 0:
                return mp.mpf('nan')
            return mp.inf if self.b > 0 else -mp.inf
        return mp.mpf(self.b) / mp.mpf(self.a)

    def log10(self) -> float:
        """Exact-ish log10 of the final ratio without forming the scalar."""
        if self.b == 0:
            return float("-inf")
        if self.a == 0:
            return float("inf")
        return float(mp.log10(mp.mpf(self.b) / mp.mpf(self.a)))


# --------------------------------------------------------------------------
# Chain generators
# --------------------------------------------------------------------------
def stress_chain(n: int, decades: int = 600, seed: int = 12345) -> Tuple[List[Pair], dict]:
    """n ratios; cumulative log10 performs a REFLECTING random walk within a
    +/- (decades/2 + 50) band, so the running product traverses 600+ decades of
    dynamic range (and crosses float64's +/-308 window) while the exact integers
    stay bounded (a few hundred digits).  Each step is an integer power of 10.
    """
    rng = random.Random(seed)
    band = decades // 2 + 50          # +/-350  => 700-decade span (> 600)
    chain: List[Pair] = []
    cum = 0
    lo = hi = 0
    step_exps_max = 0
    for _ in range(n):
        e = rng.randint(-60, 60)
        nxt = cum + e
        if nxt > band:                  # reflect at band edge
            nxt = band - (nxt - band)
        if nxt < -band:
            nxt = -band - (nxt + band)
        nxt = max(-band, min(band, nxt))
        de = nxt - cum
        cum = nxt
        lo = min(lo, cum)
        hi = max(hi, cum)
        step_exps_max = max(step_exps_max, abs(de))
        if de >= 0:
            chain_append = (1, 10 ** de)
        else:
            chain_append = (10 ** (-de), 1)
        chain.append(chain_append)
    meta = {
        "band_plus_minus_log10": band,
        "per_step_max_abs_log10": step_exps_max,
        "cumulative_min_log10": lo,
        "cumulative_max_log10": hi,
        "cumulative_span_decades": hi - lo,
        "final_log10": cum,
    }
    return chain, meta


def finance_chain(n: int, seed: int = 777, S: int = 10 ** 5) -> Tuple[List[Pair], dict]:
    """Realistic actuarial/reserve chain: daily compounding of (1+r).

    Each step is a reserve/return factor (1+r_i) with r_i ~ N(0, 1.5%),
    plus a deterministic leverage cycle (x10 / /10) and occasional FX
    cross-rate repricing.  Stored as exact decimal integer ratios at scale S.
    NOTE: the exact pair grows ~n*log10(S) digits; this is the known
    non-compression property, so the demo keeps n and S modest.
    """
    rng = random.Random(seed)
    chain: List[Pair] = []
    cum = 0.0
    clamped = 0
    for i in range(n):
        r = rng.gauss(0.0, 0.015)
        factor = 1.0 + r
        # periodic leverage regime: reserve ratio swings by a decade
        if i % 1000 == 0 and i > 0:
            factor *= 10.0 if (i // 1000) % 2 else 0.1
        # represent factor as exact decimal ratio
        a = S
        b = int(round(S * factor))
        if b == 0:
            # A factor of exactly zero would make the chain's ratio zero and
            # log10 undefined. Keep it finite but FLAG the clamp instead of
            # silently rewriting a negative sign as a positive ratio.
            b = 1
            clamped += 1
        if b < 0:
            # Negative (1+r) is outside the generator's r~N(0,0.015) model;
            # reject rather than silently producing a positive factor.
            raise ValueError("finance_chain: negative compounding factor at step %d (r=%.6g)" % (i, r))
        chain.append((a, b))
        cum += math.log10(b / a)
    meta = {
        "description": "daily (1+r) compounding, r~N(0,1.5%), decade leverage cycles, 1e5 decimal scale",
        "cumulative_log10_end": cum,
        "zero_factor_clamps": clamped,
    }
    return chain, meta


# --------------------------------------------------------------------------
# Methods
# --------------------------------------------------------------------------
def eager_float(chain: List[Pair]) -> dict:
    t0 = time.perf_counter()
    p = 1.0
    collapses = 0
    for a, b in chain:
        r = b / a            # eager resolution every step
        p *= r
        if not math.isfinite(p) or p == 0.0:
            collapses += 1
    dt = time.perf_counter() - t0
    return {"value": p, "collapses": collapses, "sec": dt, "per_step_us": dt / len(chain) * 1e6}


def log_domain(chain: List[Pair]) -> dict:
    t0 = time.perf_counter()
    s = 0.0
    for a, b in chain:
        s += math.log(b) - math.log(a)   # hand-written, no library logsumexp
    collapses = 0
    try:
        value = math.exp(s)
        if not math.isfinite(value):
            collapses = 1
    except OverflowError:
        value = math.inf
        collapses = 1
    dt = time.perf_counter() - t0
    return {"value": value, "log10": s / math.log(10.0), "collapses": collapses,
            "sec": dt, "per_step_us": dt / len(chain) * 1e6}


def relational(chain: List[Pair], resolve_every: bool = False) -> dict:
    c = DeferredChain()
    t0 = time.perf_counter()
    vals = []
    for a, b in chain:
        c.compose(a, b)
        if resolve_every:
            try:
                v = c.b / c.a
            except OverflowError:
                c.resolve_collapses += 1
                v = math.inf
            vals.append(v)
            c.resolves += 1
    dt = time.perf_counter() - t0
    return {"value": c.resolve_float(), "log10": c.log10(), "collapses": 0,
            "sec": dt, "per_step_us": dt / len(chain) * 1e6,
            "composes": c.composes, "cancellations": c.cancellations,
            "resolves": c.resolves, "resolve_collapses": c.resolve_collapses,
            "resolve_every_sec": (dt if resolve_every else None),
            "num_vals": len(vals)}


def reference_mp(chain: List[Pair]) -> dict:
    t0 = time.perf_counter()
    p = mp.mpf(1)
    for a, b in chain:
        p *= mp.mpf(b) / mp.mpf(a)
    dt = time.perf_counter() - t0
    return {"value": p, "log10": float(mp.log10(p)),
            "sec": dt, "per_step_us": dt / len(chain) * 1e6}


# --------------------------------------------------------------------------
# Benchmark / reporting
# --------------------------------------------------------------------------
def benchmark(name: str, chain: List[Pair], meta: dict) -> dict:
    n = len(chain)
    ref = reference_mp(chain)
    ref_log10 = ref["log10"]

    res = {
        "chain": name,
        "steps": n,
        "meta": meta,
        "reference": {"lib": "mpmath", "dps": 80, "log10": ref_log10,
                      "sec": ref["sec"], "per_step_us": ref["per_step_us"]},
    }

    e = eager_float(chain)
    lg = log_domain(chain)
    rel = relational(chain)
    rel_eager = relational(chain, resolve_every=True)

    def log10_of(v: float):
        if v == math.inf:
            return math.inf
        if v <= 0.0:
            return -math.inf
        return math.log10(v)

    def err_log10(lv):
        if not math.isfinite(lv) or not math.isfinite(ref_log10):
            return None
        return abs(lv - ref_log10) / max(1.0, abs(ref_log10))

    e_log = log10_of(e["value"])
    res["eager_float64"] = {
        "value": e["value"], "log10": e_log,
        "relerr_log10": err_log10(e_log), "collapses": e["collapses"],
        "sec": e["sec"], "per_step_us": e["per_step_us"],
    }
    lg_log = lg["log10"]
    res["log_domain"] = {
        "value": lg["value"], "log10": lg_log,
        "relerr_log10": err_log10(lg_log), "collapses": lg["collapses"],
        "sec": lg["sec"], "per_step_us": lg["per_step_us"],
    }
    r_log = rel["log10"]
    res["deferred_relational"] = {
        "value": rel["value"], "log10": r_log,
        "relerr_log10": err_log10(r_log), "collapses": rel["collapses"],
        "composes": rel["composes"], "cancellations": rel["cancellations"],
        "sec": rel["sec"], "per_step_us": rel["per_step_us"],
        "resolve_every_step_sec": rel_eager["sec"],
        "resolve_every_step_collapses": rel_eager.get("resolve_collapses", 0),
        "on_demand_speedup": rel_eager["sec"] / rel["sec"] if rel["sec"] > 0 else None,
    }
    return res


def main() -> None:
    stress, smeta = stress_chain(10000, 600, seed=12345)
    finance, fmeta = finance_chain(1000, seed=777)

    out = {
        "application": "Deferred-relational ratio chains for finance/actuarial long-horizon compounding",
        "note": "Unresolved pair (a,b) is the state; scalar b/a produced only on demand. No compression claimed.",
        "benchmarks": [
            benchmark("stress_600decades_10000steps", stress, smeta),
            benchmark("finance_compounding_1000steps", finance, fmeta),
        ],
    }
    print(json.dumps(out, indent=2, default=str))


if __name__ == "__main__":
    main()
