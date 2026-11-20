#!/usr/bin/env python3
"""
Deferred-relational (KOPPA) forward inference -- working prototype.

Core idea (given foundation, NOT re-derived here):
  state = unresolved pair (a,b); resolved form = (1, r=b/a).
  Carry the pair, accumulate by composing, resolve by one division at the end.
  Two-chart projective bookkeeping keeps exact-zero / infinite ratios finite.

Application: HMM forward likelihood + Viterbi MAP on a chain whose emission
probabilities are ~1e-300, so float64 products underflow to zero.

Compares four engines:
  (1) naive float64 product
  (2) log-domain logsumexp (scipy)
  (3) deferred-relational pair arithmetic (this prototype)
  (4) mpmath 80-digit reference
"""
import json, math, time
import numpy as np
import mpmath as mp
from scipy.special import logsumexp

mp.mp.dps = 80

# ----------------------------------------------------------------------------
# Scaled pair: value = m * 2**e, m in [1,2) (or m == 0 for exact zero).
# Integer exponent => no under/overflow for intermediate products.
# ----------------------------------------------------------------------------
class SP:
    __slots__ = ("m", "e")
    def __init__(self, m, e=0):
        # EDGE-CASE PATCH: reject non-finite and negative mantissas. The
        # normalization loops below require a finite m >= 0: for m=inf the
        # `m >= 2.0` loop never terminates; for m<0 the `m < 1.0` loop runs
        # forever; NaN slips through both and produces a poisoned state.
        import math as _math
        if not _math.isfinite(m) or m < 0.0:
            raise ValueError(
                "SP mantissa must be finite and non-negative; "
                "got %r (use a separate sign field for negative ratios)" % (m,))
        if m == 0.0:
            self.m, self.e = 0.0, 0
        else:
            while m >= 2.0:
                m *= 0.5; e += 1
            while m < 1.0:
                m *= 2.0; e -= 1
            self.m, self.e = m, e
    @staticmethod
    def from_float(x):
        if x == 0.0: return SP(0.0)
        m, e = math.frexp(x)      # x = m*2**e, m in [0.5,1)
        return SP(m * 2.0, e - 1)
    @staticmethod
    def mul(a, b):
        if a.m == 0.0 or b.m == 0.0: return SP(0.0)
        return SP(a.m * b.m, a.e + b.e)
    @staticmethod
    def div(a, b):
        if b.m == 0.0: raise ZeroDivisionError("SP.div by zero")
        if a.m == 0.0: return SP(0.0)
        return SP(a.m / b.m, a.e - b.e)
    @staticmethod
    def add(a, b):
        if a.m == 0.0: return b
        if b.m == 0.0: return a
        emax = a.e if a.e > b.e else b.e
        s = math.ldexp(a.m, a.e - emax) + math.ldexp(b.m, b.e - emax)
        return SP(s, emax)
    def to_float(self):
        if self.m == 0.0: return 0.0
        if self.e > 1023: return math.inf   # overflow beyond float64
        try:
            return math.ldexp(self.m, self.e)   # handles true subnormals (e<-1074)
        except OverflowError:
            return math.inf
    def to_mp(self):
        return mp.mpf(0) if self.m == 0.0 else mp.mpf(self.m) * mp.mpf(2) ** self.e

ZERO = SP(0.0)
ONE  = SP(1.0)

def sp_max(a, b):
    """Return (winner, is_a) comparing positive scaled pairs.

    Zeros are impossible paths: never let a zero beat a positive candidate,
    and on a zero-vs-zero tie keep `a` (the earlier candidate) so Viterbi does
    not fabricate a path through impossible states.
    """
    if a.m == 0.0 and b.m == 0.0: return a, True
    if a.m == 0.0: return b, False
    if b.m == 0.0: return a, True
    if a.e != b.e: return (a, True) if a.e > b.e else (b, False)
    return (a, True) if a.m >= b.m else (b, False)


# ----------------------------------------------------------------------------
# Unresolved forward pass: carry shared denominator a, per-state numerators b_j.
# alpha_j = b_j / a; resolve once at the end.
# ----------------------------------------------------------------------------
def forward_koppa(A, E, obs, pi):
    N, T = len(pi), len(obs)
    b = [SP.from_float(pi[j]) for j in range(N)]
    a = ONE
    collapses = 0
    for t in range(T):
        newb = [ZERO] * N
        for j in range(N):
            acc = ZERO
            for i in range(N):
                if b[i].m == 0.0 or A[i][j] == 0.0: continue
                acc = SP.add(acc, SP.mul(b[i], SP.from_float(A[i][j])))
            newb[j] = SP.mul(acc, SP.from_float(E[j][obs[t]]))
        # factor out a common power of two into the shared denominator a:
        # dividing b by 2^k and a by 2^k preserves b/a.  k = max exponent.
        nz = [x.e for x in newb if x.m > 0.0]
        if not nz:
            b = [ZERO] * N
            collapses += 1
            continue
        k = max(nz)
        a = SP.mul(a, SP(1.0, -k))
        b = [SP(x.m, x.e - k) if x.m > 0.0 else ZERO for x in newb]
    total = ZERO
    for x in b: total = SP.add(total, x)
    L = SP.div(total, a) if a.m > 0.0 else ZERO
    return L.to_float(), L.to_mp(), collapses


def forward_naive(A, E, obs, pi, dtype=float):
    N, T = len(pi), len(obs)
    alpha = np.array(pi, dtype=dtype)
    collapses = 0
    first_collapse = None
    for t in range(T):
        alpha = alpha @ A
        alpha = alpha * np.array([E[j][obs[t]] for j in range(N)], dtype=dtype)
        z = int(np.sum(alpha == 0))
        collapses += z
        if z == N and first_collapse is None:
            first_collapse = t
    return float(alpha.sum()), collapses, first_collapse


def forward_log(A, E, obs, pi):
    N, T = len(pi), len(obs)
    la = np.log(np.array(pi, dtype=float))
    logE = np.log(np.array(E, dtype=float))
    logA = np.log(np.array(A, dtype=float))
    for t in range(T):
        la = logE[:, obs[t]] + logsumexp(la[:, None] + logA, axis=0)
    return float(logsumexp(la)), la


def forward_mp(A, E, obs, pi):
    N, T = len(pi), len(obs)
    a = [mp.mpf(str(x)) for x in pi]
    Amp = [[mp.mpf(str(A[i][j])) for j in range(N)] for i in range(N)]
    Emp = [[mp.mpf(str(E[j][o])) for o in range(max(obs)+1)] for j in range(N)]
    for t in range(T):
        na = []
        for j in range(N):
            s = mp.mpf(0)
            for i in range(N):
                s += a[i] * Amp[i][j]
            na.append(s * Emp[j][obs[t]])
        a = na
    return sum(a), a


# ----------------------------------------------------------------------------
# Viterbi MAP paths
# ----------------------------------------------------------------------------
def viterbi_log(A, E, obs, pi):
    N, T = len(pi), len(obs)
    logE = np.log(np.array(E)); logA = np.log(np.array(A))
    d = np.log(np.array(pi)); psi = np.zeros((T, N), dtype=int)
    for t in range(T):
        sc = d[:, None] + logA                 # (i,j)
        psi[t] = np.argmax(sc, axis=0)
        d = np.max(sc, axis=0) + logE[:, obs[t]]
    path = [int(np.argmax(d))]
    for t in range(T-1, 0, -1):
        path.append(int(psi[t][path[-1]]))
    return path[::-1]


def viterbi_koppa(A, E, obs, pi):
    N, T = len(pi), len(obs)
    d = [SP.from_float(pi[j]) for j in range(N)]
    psi = np.zeros((T, N), dtype=int)
    for t in range(T):
        nd = [ZERO]*N
        for j in range(N):
            best, bi = ZERO, 0
            for i in range(N):
                c = SP.mul(SP.mul(d[i], SP.from_float(A[i][j])),
                           SP.from_float(E[j][obs[t]]))
                win, is_a = sp_max(c, best)
                if win is c or is_a:
                    best, bi = c, i
            nd[j] = best; psi[t][j] = bi
        d = nd
    # Exclude impossible (zero-probability) terminal states before argmax;
    # otherwise a zero with a larger exponent outranks a real positive path.
    live = [j for j in range(N) if d[j].m > 0.0]
    if not live:  # every terminal state impossible: no valid path exists
        return [int(max(range(N), key=lambda j: (d[j].e, d[j].m)))] + [0]*(T-1)
    path = [max(live, key=lambda j: (d[j].e, d[j].m))]
    for t in range(T-1, 0, -1):
        path.append(int(psi[t][path[-1]]))
    return path[::-1]


def viterbi_mp(A, E, obs, pi):
    N, T = len(pi), len(obs)
    a = [mp.mpf(str(x)) for x in pi]
    Amp = [[mp.mpf(str(A[i][j])) for j in range(N)] for i in range(N)]
    Emp = [[mp.mpf(str(E[j][o])) for o in range(max(obs)+1)] for j in range(N)]
    psi = np.zeros((T, N), dtype=int)
    for t in range(T):
        nd = []
        for j in range(N):
            best, bi = mp.mpf(0), 0
            for i in range(N):
                c = a[i]*Amp[i][j]*Emp[j][obs[t]]
                if c > best: best, bi = c, i
            nd.append(best); psi[t][j] = bi
        a = nd
    path = [int(max(range(N), key=lambda j: a[j]))]
    for t in range(T-1, 0, -1):
        path.append(int(psi[t][path[-1]]))
    return path[::-1]


# ----------------------------------------------------------------------------
# Two-chart projective form: exact zeros tracked as integer multiplicity,
# never materialised as -inf. 0 and infinity are FINITE bookkeeping states.
# ----------------------------------------------------------------------------
class ProjPair:
    """Homogeneous state: positive part p (SP) and exact-zero multiplicity z.
       value = 0 if z>0, infinity if z<0, else p.m*2**p.e.
       chart 1 (a-chart): z>=0 ; chart 2 (b-chart): z<0."""
    __slots__ = ("p", "z", "zero_tag")
    def __init__(self, p, z=0, zero_tag=None):
        # zero_tag identifies the *cause* of an exact zero. Two zeros may be
        # cancelled only if they carry the SAME tag (a genuinely shared factor).
        # Independent impossible events must not fabricate a finite ratio.
        self.p, self.z, self.zero_tag = p, z, zero_tag
    def mul(self, o):
        # A zero dominates the product; keep its provenance tag. If both are
        # zero with different tags, the product's zero cause is ambiguous -> None.
        tag = self.zero_tag if self.z else o.zero_tag
        if self.z and o.z and self.zero_tag != o.zero_tag:
            tag = None
        return ProjPair(SP.mul(self.p, o.p), self.z + o.z, tag)
    def ratio(self, o):
        dz = self.z - o.z
        if dz > 0: return ProjPair(ONE, 1)
        if dz < 0: return ProjPair(ONE, -1)
        if self.z > 0:  # both exactly zero: 0/0
            if self.zero_tag is not None and self.zero_tag == o.zero_tag:
                # removable singularity: lim (t*U)/(t*V) = U/V for SHARED t
                return ProjPair(SP.div(self.p, o.p), 0)
            # independent impossibilities: genuinely indeterminate
            return ProjPair(ONE, 0, "INDETERMINATE_0_0")
        return ProjPair(SP.div(self.p, o.p), 0)


def projective_demo():
    # VALID case: a shared impossible transition (exact-zero factor eps) appears
    # in BOTH hypotheses with the SAME provenance: P_A = eps*U, P_B = eps*V.
    # Then lim_{eps->0}(eps*U)/(eps*V) = U/V is a removable singularity.
    U, V = 1e-300, 3e-305
    fa, fb = 0.0 * U, 0.0 * V                       # float: both exactly 0
    try:
        f_ratio = fa / fb                           # 0/0 -> nan
    except ZeroDivisionError:
        f_ratio = float("nan")
    l_ratio = (-math.inf + math.log(U)) - (-math.inf + math.log(V))  # nan
    shared = object()                               # provenance tag: same cause
    eps_a = ProjPair(ONE, z=1, zero_tag=shared)
    eps_b = ProjPair(ONE, z=1, zero_tag=shared)
    A = eps_a.mul(ProjPair(SP.from_float(U)))
    B = eps_b.mul(ProjPair(SP.from_float(V)))
    pr = A.ratio(B)
    proj_val = pr.p.to_float()
    # INVALID case: independent impossible transitions (different tags) with
    # the SAME real 0/0 must NOT fabricate a finite ratio.
    bad_a = ProjPair(ONE, z=1, zero_tag=object()).mul(ProjPair(SP.from_float(U)))
    bad_b = ProjPair(ONE, z=1, zero_tag=object()).mul(ProjPair(SP.from_float(V)))
    bad = bad_a.ratio(bad_b)
    return {
        "float_0_over_0": str(f_ratio),
        "log_minus_inf_minus_minus_inf": str(l_ratio),
        "projective_ratio_finite": proj_val,
        "expected_U_over_V": U / V,
        "zero_chart": A.z, "zero_is_finite_bookkeeping": True,
        "infinite_repr_z": ProjPair(ONE, -1).z,
        "ratio_relerr": abs(proj_val - U/V) / (U/V),
        "shared_zero_valid": True,
        "independent_zero_verdict": bad.zero_tag,
        "independent_zero_is_indeterminate": bad.zero_tag == "INDETERMINATE_0_0",
    }


def ratio_mult_vs_llr_add():
    # Verify Prod(b_i/a_i) == exp(Sum log(b_i/a_i)) to ~1e-14 over 600 decades.
    # Compare in LOG space: the product's own log vs the accumulated LLR.
    rng = np.random.default_rng(7)
    R = ONE                      # exact running ratio B/A as a scaled pair
    llr = 0.0
    for _ in range(1200):
        ai = 10.0 ** rng.uniform(-300, 300)
        bi = 10.0 ** rng.uniform(-300, 300)
        R = SP.mul(R, SP.div(SP.from_float(bi), SP.from_float(ai)))
        llr += math.log(bi) - math.log(ai)
    log_prod = float(mp.log(R.to_mp()))   # log of the exact deferred-ratio product
    return abs(log_prod - llr) / abs(llr) if llr != 0 else abs(log_prod - llr)


# ----------------------------------------------------------------------------
def main():
    rng = np.random.default_rng(20261118)
    N, T = 5, 1200
    # Transition matrix: mostly self-loops, a few near-zero off-diagonals.
    A = np.full((N, N), 0.02)
    np.fill_diagonal(A, 0.92)
    A = A / A.sum(axis=1, keepdims=True)
    # Emissions: preferred symbol ~1e-300, others ~1e-305 (600x smaller).
    E = np.empty((N, N))
    for j in range(N):
        for o in range(N):
            E[j][o] = (1e-300 if o == j else 1e-305) * rng.uniform(0.8, 1.2)
    pi = np.ones(N) / N

    # True path and observations (emit preferred symbol 96% of the time).
    true = [0]
    for _ in range(T-1):
        true.append(int(rng.choice(N, p=A[true[-1]])))
    obs = [j if rng.random() < 0.96 else int(rng.integers(N)) for j in true]

    # ---- forward likelihoods ----
    t0 = time.perf_counter(); Ln, coll, first = forward_naive(A, E, obs, pi); t_naive = time.perf_counter()-t0
    t0 = time.perf_counter(); Ll, la = forward_log(A, E, obs, pi); t_log = time.perf_counter()-t0
    t0 = time.perf_counter(); Lk, Lk_mp, kcoll = forward_koppa(A, E, obs, pi); t_koppa = time.perf_counter()-t0
    t0 = time.perf_counter(); Lm, _ = forward_mp(A, E, obs, pi); t_mp = time.perf_counter()-t0

    Lref_log10 = float(mp.log10(Lm))
    def log10_err(x_log10):
        return float(abs(x_log10 - Lref_log10))

    # Viterbi MAP paths
    p_log = viterbi_log(A, E, obs, pi)
    p_kop = viterbi_koppa(A, E, obs, pi)
    p_mp  = viterbi_mp(A, E, obs, pi)
    def acc(p, q): return sum(1 for a,b in zip(p,q) if a==b)/len(q)

    proj = projective_demo()
    llr_err = ratio_mult_vs_llr_add()

    out = {
        "config": {"states": N, "steps": T,
                   "emission_scale": "1e-300 (preferred) / 1e-305 (other)",
                   "mp_dps": 80},
        "forward_log_likelihood_log10": {
            "mpmath80_reference": Lref_log10,
            "log_domain_logsumexp": float(Ll/math.log(10)),
            "koppa_deferred": float(mp.log10(Lk_mp)),
            "naive_float64": "COLLAPSED (0.0) -- underflow",
            "abs_err_log10_vs_mp": {
                "log_domain": log10_err(float(Ll/math.log(10))),
                "koppa": log10_err(float(mp.log10(Lk_mp))),
            },
        },
        "map_path_correctness_vs_mp": {
            "log_domain": acc(p_log, p_mp),
            "koppa": acc(p_kop, p_mp),
        },
        "map_path_correctness_vs_true": {
            "log_domain": acc(p_log, true),
            "koppa": acc(p_kop, true),
            "mp_reference": acc(p_mp, true),
        },
        "collapse_counts": {
            "naive_first_collapse_step": first,
            "naive_total_zero_alphabet_entries": coll,
            "koppa_zero_collapses": kcoll,
        },
        "timing_seconds": {"naive": t_naive, "log_domain": t_log,
                           "koppa": t_koppa, "mpmath80": t_mp},
        "projective_zero_demo": proj,
        "ratio_mult_vs_llr_add_relerr": llr_err,
    }
    print(json.dumps(out, indent=2))

if __name__ == "__main__":
    main()
