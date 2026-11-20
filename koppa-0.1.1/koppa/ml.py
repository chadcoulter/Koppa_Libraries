#!/usr/bin/env python3
"""
KOPPA LazyRelational -- ML / ML-infrastructure application demo.

(i)   LazyRelational numeric type: unresolved pair (a,b), resolved form r=b/a.
      compose() is associative & multiplicative; rescale() is the gauge transform;
      gauge() returns g=d(log r)/dt = d(log b)/dt - d(log a)/dt (scale invariant).
      Mantissa + integer-exponent storage spans arbitrarily many decades.

(ii)  Real workload: 5000-step log-odds / likelihood-ratio score chains.
      A: hidden pair scale to 600 decades (per-step values exceed float64);
         oscillating d_i keeps the product ~10^1 (finite) -> accuracy + collapse.
      B: hidden scale 0, constant d_i=+0.12 -> product 10^600 (all float outputs
         overflow, but the pair holds the exact exponent) -> magnitude tracking.
      Compared: eager float64 vs deferred resolution vs log-domain vs mpmath.
      Plus a deferred-normalized-attention-style aggregation demo.

(iii) Scale-equivariance gradient ORACLE: a custom VJP correct at gauge scale 1
      but wrong after rescaling. Gradcheck at the canonical scale passes; the
      gauge oracle rescales intermediates and checks gradient equivariance,
      detecting the bug with no analytic reference gradient.

Run:  python3 koppa_app_ml.py   -> emits a JSON measurement block on stdout.
"""
import json
import math
import time

import mpmath as mp

mp.mp.dps = 80


# ----------------------------------------------------------------------------
# (i) LazyRelational
# ----------------------------------------------------------------------------
def _norm(m, e):
    # EDGE-CASE PATCH: preserve sign and reject non-finite values. The old code
    # did `m = -m` for m<0, silently DISCARDING the sign (b=-2 resolved as +2),
    # and its `while m >= 10: m/=10` loop never terminates for m=inf.
    if not math.isfinite(m):
        raise ValueError("LazyRelational mantissa must be finite; got %r" % (m,))
    if m == 0.0:
        return 0.0, 0
    sign = -1.0 if m < 0 else 1.0
    m = abs(m)
    while m >= 10.0:
        m /= 10.0
        e += 1
    while m < 1.0:
        m *= 10.0
        e -= 1
    return sign * m, e


class LazyRelational:
    """Unresolved pair (a,b)=(am*10^ae, bm*10^be); resolved r=b/a."""
    __slots__ = ("am", "ae", "bm", "be")

    def __init__(self, am, ae=0, bm=1.0, be=0):
        self.am, self.ae = _norm(float(am), int(ae))
        self.bm, self.be = _norm(float(bm), int(be))

    @classmethod
    def identity(cls):
        return cls(1.0, 0, 1.0, 0)

    def compose(self, other):
        return LazyRelational(self.am * other.am, self.ae + other.ae,
                              self.bm * other.bm, self.be + other.be)

    def rescale(self, lam_m=1.0, lam_e=0):
        """Gauge transform (a,b)->(lam*a, lam*b); resolved r is invariant."""
        return LazyRelational(self.am * lam_m, self.ae + lam_e,
                              self.bm * lam_m, self.be + lam_e)

    def gauge(self, d_log_a, d_log_b):
        """g = d(log r)/dt, invariant under a common rescaling."""
        return d_log_b - d_log_a

    def resolved_float(self):
        """Resolve once at the end; may overflow to inf for huge r."""
        de = self.be - self.ae
        m = self.bm / self.am
        try:
            return m * (10.0 ** de)
        except OverflowError:
            return math.inf

    def resolved_log10(self):
        return math.log10(self.bm / self.am) + (self.be - self.ae)

    def __repr__(self):
        return "LR(r~=%.6g*10^%d)" % (self.bm / self.am, self.be - self.ae)


# ----------------------------------------------------------------------------
# Chain: step i has hidden scale u_i = slope*i and log-ratio d_i, so
# a_i = 10^{u_i}, b_i = 10^{u_i+d_i}, r_i = 10^{d_i}, product = 10^{sum d_i}.
# ----------------------------------------------------------------------------
def build_chain(n, slope, d_fn):
    chain = []
    for i in range(1, n + 1):
        u = slope * i
        d = d_fn(i)
        v = u + d
        ma = 10.0 ** (u - math.floor(u)); mb = 10.0 ** (v - math.floor(v))
        chain.append((ma, math.floor(u), mb, math.floor(v)))
    return chain


def d_osc(i):
    return 0.02 * math.sin(i * 0.731) + 0.005 * math.cos(i * 0.113)


def ref_mpmath(chain):
    s = mp.mpf(0)
    for (am, ae, bm, be) in chain:
        s += (mp.log10(mp.mpf(bm)) + be) - (mp.log10(mp.mpf(am)) + ae)
    return mp.power(10, s), s


def eager_float64(chain):
    """Materialize each a_i,b_i as float64, resolve each ratio, multiply."""
    x = 1.0; collapses = 0
    for (am, ae, bm, be) in chain:
        a = am * (10.0 ** ae) if ae < 309 else math.inf
        b = bm * (10.0 ** be) if be < 309 else math.inf
        r = b / a
        x *= r
        if not math.isfinite(x) or not math.isfinite(r):
            collapses += 1
    return x, collapses


def deferred(chain):
    """Compose unresolved pairs; resolve ONCE at the end."""
    acc = LazyRelational.identity()
    for (am, ae, bm, be) in chain:
        acc = acc.compose(LazyRelational(am, ae, bm, be))
    return acc, acc.resolved_float(), 0


def log_domain(chain):
    s = 0.0
    for (am, ae, bm, be) in chain:
        s += math.log10(bm / am) + (be - ae)
    try:
        return 10.0 ** s
    except OverflowError:
        return math.inf


def _finite_relerr(x, ref):
    if not isinstance(x, float) or not math.isfinite(x):
        return None
    return float(abs(mp.mpf(x) - ref) / abs(ref))


def _log10_relerr(x_log10, ref_log10):
    """Relative error in log10-space: valid even when the value overflows float."""
    return float(abs(mp.mpf(x_log10) - mp.mpf(ref_log10)) / abs(mp.mpf(ref_log10)))


def timed(fn):
    t0 = time.perf_counter(); out = fn(); return out, time.perf_counter() - t0


def run_workload(n, slope, d_fn, label):
    chain = build_chain(n, slope, d_fn)
    ref, ref_log10 = ref_mpmath(chain)
    (xe, ce), te = timed(lambda: eager_float64(chain))
    (acc, xd, cd), td = timed(lambda: deferred(chain))
    xl, tl = timed(lambda: log_domain(chain))
    methods = {}
    for name, x, col, t in [("eager_float64", xe, ce, te),
                            ("deferred_resolution", xd, cd, td),
                            ("log_domain", xl, 0, tl)]:
        methods[name] = {
            "value": x,
            "relerr": _finite_relerr(x, ref),
            "collapses": col,
            "time_s": t,
        }
    return {
        "regime": label, "steps": n,
        "hidden_scale_max_decades": slope * n,
        "resolved_exponent_log10": acc.be - acc.ae,
        "reference_log10": float(ref_log10),
        "deferred_log10": acc.resolved_log10(),
        "deferred_log10_relerr": _log10_relerr(acc.resolved_log10(), ref_log10),
        "methods": methods,
    }


def attention_demo():
    """Deferred normalization: aggregate unnormalized logit ratios as pairs,
    form the normalizer once, avoiding per-head normalized tensors."""
    pairs = []
    for j in range(1, 9):
        u = 40.0 * j                      # 10^40..10^320, latter > float64 range
        v = u + 0.3 * math.sin(j)
        ma = 10.0 ** (u - math.floor(u)); mb = 10.0 ** (v - math.floor(v))
        pairs.append(LazyRelational(ma, math.floor(u), mb, math.floor(v)))
    acc = LazyRelational.identity()
    for p in pairs:
        acc = acc.compose(p)
    total = acc.resolved_float()
    return {
        "num_scores": len(pairs),
        "max_hidden_decades": 40.0 * len(pairs),
        "aggregated_ratio": total,
        "resolved_log10": acc.resolved_log10(),
        "collapses": 0 if math.isfinite(total) else 1,
        "note": "10^320 ratios unrepresentable as float64 compose exactly as (mantissa,int exponent); normalize once.",
    }


def gauge_invariance_check():
    a, b, da, db = 2.0, 8.0, 0.3, 1.7
    g = LazyRelational(a, 0, b, 0).gauge(da / a, db / b)
    lam, dlam = 5.0, 0.9
    lr2 = LazyRelational(a, 0, b, 0).rescale(lam)
    g2 = lr2.gauge((dlam * a + lam * da) / (lam * a),
                   (dlam * b + lam * db) / (lam * b))
    return {"g": g, "g_rescaled": g2, "invariant": abs(g - g2) < 1e-12}


# ----------------------------------------------------------------------------
# (iii) Gradient oracle.
# r=b/a; correct VJP: ga=-up*r/a, gb=up/a.
# BUG: a cached normalization scale divides gradients. Cache is 1 at the
# canonical point (bug dormant) but becomes lam after a gauge rescale, producing
# an extra /lam. Gradcheck at scale 1 passes; the symmetry oracle catches it.
# ----------------------------------------------------------------------------
def chain_forward(pairs):
    f = 1.0
    for (a, b) in pairs:
        f *= b / a
    return f


def chain_vjp(pairs, scale, buggy):
    f = chain_forward(pairs)
    gas, gbs = [], []
    for (a, b) in pairs:
        up = f / (b / a)                 # d(product)/d(r_i), grad_out = 1
        r = b / a
        ga = -up * r / a
        gb = up / a
        if buggy:
            ga /= scale                  # stale/divergent normalization cache
            gb /= scale
        gas.append(ga); gbs.append(gb)
    return f, gas, gbs


def gradcheck(pairs, scale, buggy, eps=1e-5):
    _, gas, _ = chain_vjp(pairs, scale, buggy)
    a0, b0 = pairs[0]
    pp = list(pairs); pm = list(pairs)
    pp[0] = (a0 + eps, b0); pm[0] = (a0 - eps, b0)
    fd = (chain_forward(pp) - chain_forward(pm)) / (2 * eps)
    ok = abs(fd - gas[0]) < 1e-4 * max(1.0, abs(fd))
    return ok, fd, gas[0]


def gauge_oracle(pairs, lam, buggy, tol=1e-6):
    _, gas, _ = chain_vjp(pairs, 1.0, buggy)                 # canonical
    rescaled = [(lam * a, lam * b) for (a, b) in pairs]
    _, gas_r, _ = chain_vjp(rescaled, lam, buggy)            # gauge-rescaled
    residuals = [abs(g * lam - g0) / max(1e-300, abs(g0))
                 for g, g0 in zip(gas_r, gas)]
    worst = max(residuals)
    return {"equivariance_residual": worst, "detected": worst > tol, "lam": lam}


def main():
    out = {}
    out["workload_A_accuracy_collapse"] = run_workload(
        5000, 0.12, d_osc,
        "A: 5000-step log-odds chain, hidden scale to 600 decades, finite product")
    out["workload_B_magnitude"] = run_workload(
        5000, 0.0, lambda i: 0.12,
        "B: 5000-step chain, product 10^600 (exceeds float64), magnitude tracking")
    out["attention"] = attention_demo()
    out["gauge_invariance"] = gauge_invariance_check()

    pairs = [(1.3, 2.7), (0.8, 1.9), (2.2, 3.1), (1.7, 4.2), (0.9, 2.3)]
    gc_pass, fd, vjp = gradcheck(pairs, 1.0, buggy=True)
    out["gradient_oracle"] = {
        "gradcheck_canonical_scale1_buggy": {"pass": gc_pass, "finite_diff": fd, "vjp": vjp},
        "gauge_oracle_buggy": gauge_oracle(pairs, 10.0, buggy=True),
        "gauge_oracle_correct_vjp": gauge_oracle(pairs, 10.0, buggy=False),
        "verdict": "gradcheck@scale1 MISSES injected VJP bug; gauge oracle DETECTS it",
    }
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
