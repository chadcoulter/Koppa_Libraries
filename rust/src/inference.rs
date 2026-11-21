//! Deferred-relational (KOPPA) forward inference — Rust port of
//! `koppa/inference.py`.
//!
//! Core idea (given foundation, not re-derived here):
//!   state = unresolved pair (a, b); resolved form = (1, r = b/a).
//!   Carry the pair, accumulate by composing, resolve by one division at the end.
//!   Two-chart projective bookkeeping keeps exact-zero / infinite ratios finite.
//!
//! Edge fixes preserved from the Python prototype:
//!   * `SP` rejects non-finite / negative mantissas (`from_float` returns `Err`).
//!   * `to_float` handles true subnormals and catches overflow (ldexp-equivalent).
//!   * Viterbi excludes zero-probability terminal states (live filter).
//!   * `ProjPair` tracks zero provenance: a shared tag cancels, independent
//!     tags yield `INDETERMINATE_0_0`.

use std::error::Error;
use std::fmt;

/// a scaled pair: value = m * 2^e, with m in [1,2) (or m == 0 for exact zero).
///
/// An integer exponent means no under/overflow for intermediate products.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SP {
    pub m: f64,
    pub e: i32,
}

/// Error returned when a mantissa is not finite and non-negative.
#[derive(Debug, Clone, PartialEq)]
pub struct SpError {
    pub value: f64,
}

impl fmt::Display for SpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SP mantissa must be finite and non-negative; got {:?} \
             (use a separate sign field for negative ratios)",
            self.value
        )
    }
}

impl Error for SpError {}

/// Internal normalization: assumes `m` is finite and `m >= 0`.
/// Mirrors the two normalization loops in Python's `SP.__init__`.
fn normalize(m: f64, e: i32) -> SP {
    if m == 0.0 {
        return SP { m: 0.0, e: 0 };
    }
    let mut m = m;
    let mut e = e;
    while m >= 2.0 {
        m *= 0.5;
        e += 1;
    }
    while m < 1.0 {
        m *= 2.0;
        e -= 1;
    }
    SP { m, e }
}

impl SP {
    /// Construct a scaled pair, validating the mantissa (EDGE-CASE PATCH).
    ///
    /// Non-finite and negative mantissas are rejected with `Err`; without
    /// this the normalization loops would never terminate (inf / negative)
    /// or would poison the state (NaN).
    pub fn new(m: f64, e: i32) -> Result<SP, SpError> {
        if !m.is_finite() || m < 0.0 {
            return Err(SpError { value: m });
        }
        Ok(normalize(m, e))
    }

    /// Build from an f64. `0.0` is allowed; non-finite / negative is rejected.
    ///
    /// Equivalent to Python's `math.frexp(x)` followed by `SP(m*2, e-1)`:
    /// repeated exact doubling/halving yields the same normalized (m, e).
    pub fn from_float(x: f64) -> Result<SP, SpError> {
        if x == 0.0 {
            return Ok(SP { m: 0.0, e: 0 });
        }
        if !x.is_finite() || x < 0.0 {
            return Err(SpError { value: x });
        }
        // Normalize by exact powers of two (identical result to frexp path).
        Ok(normalize(x, 0))
    }

    pub fn zero() -> SP {
        SP { m: 0.0, e: 0 }
    }

    pub fn one() -> SP {
        SP { m: 1.0, e: 0 }
    }

    pub fn is_zero(&self) -> bool {
        self.m == 0.0
    }

    /// Multiply two scaled pairs. Zero is absorbing.
    pub fn mul(a: SP, b: SP) -> SP {
        if a.m == 0.0 || b.m == 0.0 {
            return SP::zero();
        }
        let e = a
            .e
            .checked_add(b.e)
            .expect("SP.mul exponent overflow (i32); use a wider exponent for this input");
        normalize(a.m * b.m, e)
    }

    /// Divide two scaled pairs. Dividing by zero panics (Python raises).
    pub fn div(a: SP, b: SP) -> SP {
        assert!(b.m != 0.0, "SP.div by zero");
        if a.m == 0.0 {
            return SP::zero();
        }
        let e = a
            .e
            .checked_sub(b.e)
            .expect("SP.div exponent overflow (i32)");
        normalize(a.m / b.m, e)
    }

    /// Add two scaled pairs, aligning exponents via an ldexp-equivalent.
    pub fn add(a: SP, b: SP) -> SP {
        if a.m == 0.0 {
            return b;
        }
        if b.m == 0.0 {
            return a;
        }
        let emax = a.e.max(b.e);
        let s = ldexp_norm(a.m, a.e - emax) + ldexp_norm(b.m, b.e - emax);
        normalize(s, emax)
    }

    /// Convert to f64. Handles true subnormals (e < -1074) and catches
    /// overflow (returns +inf), matching Python's guarded `math.ldexp`.
    pub fn to_float(&self) -> f64 {
        if self.m == 0.0 {
            return 0.0;
        }
        if self.e > 1023 {
            return f64::INFINITY;
        }
        ldexp_norm(self.m, self.e)
    }

    /// Reference conversion to an arbitrary-precision value, simulated here
    /// as f64 (the crate deliberately avoids an mp dependency). For a scaled
    /// pair whose value is representable this equals `to_float()`.
    pub fn to_mp(&self) -> f64 {
        self.to_float()
    }

    /// Natural log of the exact value, computed without materializing it.
    /// Used by the f64-based reference helpers (`ratio_mult_vs_llr_add`).
    pub fn log_value(&self) -> f64 {
        if self.m == 0.0 {
            f64::NEG_INFINITY
        } else {
            self.m.ln() + self.e as f64 * std::f64::consts::LN_2
        }
    }
}

/// `ldexp(m, e)` for a normalized mantissa m in [1,2).
///
/// Exact for normal results, correct round-to-nearest-even for subnormals,
/// and returns +inf on overflow (the Python code catches `OverflowError`).
fn ldexp_norm(m: f64, e: i32) -> f64 {
    if m == 0.0 || !m.is_finite() {
        return m;
    }
    // m in [1,2): biased exponent is 1023, implicit significand bit set.
    let bits = m.to_bits();
    let frac = bits & ((1u64 << 52) - 1);
    let sig = (1u64 << 52) | frac; // 53-bit significand, value = sig * 2^-52
    let biased = 1023i32 + e;
    if biased >= 2047 {
        return f64::INFINITY; // overflow -> +inf (ldexp OverflowError equivalent)
    }
    if biased >= 1 {
        return f64::from_bits(((biased as u64) << 52) | frac);
    }
    // Subnormal result: integer subnormal slots = round(sig / 2^sh),
    // where sh = -e - 1022 (>= 1 here), unit = 2^-1074.
    let sh = -e - 1022;
    if sh > 53 {
        return 0.0; // sig < 2^53 => result < 0.5 ulp -> rounds to zero
    }
    let q = sig >> sh;
    let rem = sig & ((1u64 << sh) - 1);
    let half = 1u64 << (sh - 1);
    let sub = if rem > half || (rem == half && (q & 1) == 1) {
        q + 1
    } else {
        q
    };
    f64::from_bits(sub)
}

pub const ZERO: SP = SP { m: 0.0, e: 0 };
pub const ONE: SP = SP { m: 1.0, e: 0 };

/// Return (winner, is_a) comparing positive scaled pairs.
///
/// Zeros are impossible paths: never let a zero beat a positive candidate,
/// and on a zero-vs-zero tie keep `a` (the earlier argument) so Viterbi does
/// not fabricate a path through impossible states.
pub fn sp_max(a: SP, b: SP) -> (SP, bool) {
    if a.m == 0.0 && b.m == 0.0 {
        return (a, true);
    }
    if a.m == 0.0 {
        return (b, false);
    }
    if b.m == 0.0 {
        return (a, true);
    }
    if a.e != b.e {
        return if a.e > b.e { (a, true) } else { (b, false) };
    }
    if a.m >= b.m {
        (a, true)
    } else {
        (b, false)
    }
}

// ---------------------------------------------------------------------------
// Unresolved forward pass: carry shared denominator a, per-state numerators b.
// alpha_j = b_j / a; resolve once at the end.
// ---------------------------------------------------------------------------

/// Forward likelihood via deferred relational pair arithmetic.
/// Returns (L.to_float(), L.to_mp(), collapses).
pub fn forward_koppa(A: &[Vec<f64>], E: &[Vec<f64>], obs: &[usize], pi: &[f64]) -> (f64, f64, usize) {
    let n = pi.len();
    let mut b: Vec<SP> = pi
        .iter()
        .map(|&x| SP::from_float(x).expect("pi must be finite and non-negative"))
        .collect();
    let mut a = ONE;
    let mut collapses = 0usize;

    for &o in obs {
        let mut newb = vec![ZERO; n];
        for j in 0..n {
            let mut acc = ZERO;
            for i in 0..n {
                if b[i].m == 0.0 || A[i][j] == 0.0 {
                    continue;
                }
                let aj = SP::from_float(A[i][j]).expect("a must be finite and non-negative");
                acc = SP::add(acc, SP::mul(b[i], aj));
            }
            let ej = SP::from_float(E[j][o]).expect("e must be finite and non-negative");
            newb[j] = SP::mul(acc, ej);
        }
        // Factor out a common power of two into the shared denominator a:
        // dividing b by 2^k and a by 2^k preserves b/a. k = max exponent.
        let nz: Vec<i32> = newb.iter().filter(|x| x.m > 0.0).map(|x| x.e).collect();
        if nz.is_empty() {
            b = vec![ZERO; n];
            collapses += 1;
            continue;
        }
        let k = *nz.iter().max().unwrap();
        a = SP::mul(a, SP { m: 1.0, e: -k });
        b = newb
            .iter()
            .map(|&x| {
                if x.m > 0.0 {
                    SP { m: x.m, e: x.e - k }
                } else {
                    ZERO
                }
            })
            .collect();
    }
    let mut total = ZERO;
    for &x in &b {
        total = SP::add(total, x);
    }
    let l = if a.m > 0.0 { SP::div(total, a) } else { ZERO };
    (l.to_float(), l.to_mp(), collapses)
}

/// Naive float64 forward product. Returns (sum, collapses, first_collapse).
pub fn forward_naive(
    a: &[Vec<f64>],
    e: &[Vec<f64>],
    obs: &[usize],
    pi: &[f64],
) -> (f64, usize, Option<usize>) {
    let n = pi.len();
    let mut alpha: Vec<f64> = pi.to_vec();
    let mut collapses = 0usize;
    let mut first_collapse: Option<usize> = None;
    for (t, &o) in obs.iter().enumerate() {
        let mut next = vec![0.0f64; n];
        for j in 0..n {
            let mut s = 0.0;
            for i in 0..n {
                s += alpha[i] * a[i][j];
            }
            next[j] = s * e[j][o];
        }
        alpha = next;
        let z = alpha.iter().filter(|&&v| v == 0.0).count();
        collapses += z;
        if z == n && first_collapse.is_none() {
            first_collapse = Some(t);
        }
    }
    (alpha.iter().sum(), collapses, first_collapse)
}

fn logsumexp(xs: &[f64]) -> f64 {
    let max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max == f64::NEG_INFINITY {
        return f64::NEG_INFINITY;
    }
    let s: f64 = xs.iter().map(|&x| (x - max).exp()).sum();
    max + s.ln()
}

/// Log-domain forward pass (f64 logsumexp, no scipy).
/// Returns (log_likelihood, final log alphas).
pub fn forward_log(A: &[Vec<f64>], E: &[Vec<f64>], obs: &[usize], pi: &[f64]) -> (f64, Vec<f64>) {
    let n = pi.len();
    let log_a: Vec<Vec<f64>> = A
        .iter()
        .map(|row| row.iter().map(|&x| x.ln()).collect())
        .collect();
    let log_e: Vec<Vec<f64>> = E
        .iter()
        .map(|row| row.iter().map(|&x| x.ln()).collect())
        .collect();
    let mut la: Vec<f64> = pi.iter().map(|&x| x.ln()).collect();
    for &o in obs {
        la = (0..n)
            .map(|j| {
                let terms: Vec<f64> = (0..n).map(|i| la[i] + log_a[i][j]).collect();
                log_e[j][o] + logsumexp(&terms)
            })
            .collect();
    }
    (logsumexp(&la), la)
}

/// Arbitrary-precision forward pass, simulated in f64 (see `to_mp`).
/// Returns (sum, alphas).
pub fn forward_mp(A: &[Vec<f64>], E: &[Vec<f64>], obs: &[usize], pi: &[f64]) -> (f64, Vec<f64>) {
    let n = pi.len();
    let mut a: Vec<f64> = pi.to_vec();
    for &o in obs {
        let mut na = vec![0.0f64; n];
        for j in 0..n {
            let mut s = 0.0;
            for i in 0..n {
                s += a[i] * A[i][j];
            }
            na[j] = s * E[j][o];
        }
        a = na;
    }
    (a.iter().sum(), a)
}

// ---------------------------------------------------------------------------
// Viterbi MAP paths
// ---------------------------------------------------------------------------

/// Viterbi MAP path in the log domain (f64).
pub fn viterbi_log(A: &[Vec<f64>], E: &[Vec<f64>], obs: &[usize], pi: &[f64]) -> Vec<usize> {
    let n = pi.len();
    let t_steps = obs.len();
    let log_a: Vec<Vec<f64>> = A
        .iter()
        .map(|row| row.iter().map(|&x| x.ln()).collect())
        .collect();
    let log_e: Vec<Vec<f64>> = E
        .iter()
        .map(|row| row.iter().map(|&x| x.ln()).collect())
        .collect();
    let mut d: Vec<f64> = pi.iter().map(|&x| x.ln()).collect();
    let mut psi = vec![vec![0usize; n]; t_steps];
    for t in 0..t_steps {
        let mut nd = vec![f64::NEG_INFINITY; n];
        for j in 0..n {
            let mut best = f64::NEG_INFINITY;
            let mut bi = 0usize;
            for i in 0..n {
                let sc = d[i] + log_a[i][j];
                if sc > best {
                    best = sc;
                    bi = i;
                }
            }
            psi[t][j] = bi;
            nd[j] = best + log_e[j][obs[t]];
        }
        d = nd;
    }
    let mut last = 0usize;
    for j in 1..n {
        if d[j] > d[last] {
            last = j;
        }
    }
    let mut path = vec![last];
    for t in (0..t_steps).rev() {
        if t == 0 {
            break;
        }
        path.push(psi[t][*path.last().unwrap()]);
    }
    path.reverse();
    path
}

/// Viterbi MAP path with deferred relational pairs.
pub fn viterbi_koppa(A: &[Vec<f64>], E: &[Vec<f64>], obs: &[usize], pi: &[f64]) -> Vec<usize> {
    let n = pi.len();
    let t_steps = obs.len();
    let mut d: Vec<SP> = pi
        .iter()
        .map(|&x| SP::from_float(x).expect("pi must be finite and non-negative"))
        .collect();
    let mut psi = vec![vec![0usize; n]; t_steps];
    for t in 0..t_steps {
        let mut nd = vec![ZERO; n];
        for j in 0..n {
            let mut best = ZERO;
            let mut bi = 0usize;
            let ej = SP::from_float(E[j][obs[t]]).expect("e must be finite and non-negative");
            for i in 0..n {
                if d[i].m == 0.0 || A[i][j] == 0.0 {
                    // Python still computes these (as zeros); skipping them
                    // is equivalent because sp_max never lets zero win here.
                    continue;
                }
                let aj = SP::from_float(A[i][j]).expect("a must be finite and non-negative");
                let c = SP::mul(SP::mul(d[i], aj), ej);
                let (_win, is_a) = sp_max(c, best);
                if is_a {
                    best = c;
                    bi = i;
                }
            }
            nd[j] = best;
            psi[t][j] = bi;
        }
        d = nd;
    }
    // Exclude impossible (zero-probability) terminal states before argmax;
    // otherwise a zero with a larger exponent outranks a real positive path.
    let live: Vec<usize> = (0..n).filter(|&j| d[j].m > 0.0).collect();
    // Compare (exponent, mantissa) lexicographically; f64 uses total_cmp.
    let cmp_em = |x: &&usize, y: &&usize| {
        d[**x]
            .e
            .cmp(&d[**y].e)
            .then_with(|| d[**x].m.total_cmp(&d[**y].m))
    };
    let last = if live.is_empty() {
        // every terminal state impossible: no valid path exists
        // Python's max() keeps the FIRST maximum on a tie; Rust's max_by
        // keeps the last, so fold manually to preserve first-wins.
        (1..n).fold(0usize, |best, j| {
            if cmp_em(&&j, &&best) == std::cmp::Ordering::Greater {
                j
            } else {
                best
            }
        })
    } else {
        // Python's max(live, key=...) keeps the FIRST maximum on a tie.
        let mut it = live.iter();
        let mut best = *it.next().unwrap();
        for &j in it {
            if cmp_em(&&j, &&best) == std::cmp::Ordering::Greater {
                best = j;
            }
        }
        best
    };
    let mut path = vec![last];
    for t in (0..t_steps).rev() {
        if t == 0 {
            break;
        }
        path.push(psi[t][*path.last().unwrap()]);
    }
    path.reverse();
    path
}

/// Viterbi MAP path, f64 simulation of the mpmath reference.
pub fn viterbi_mp(A: &[Vec<f64>], E: &[Vec<f64>], obs: &[usize], pi: &[f64]) -> Vec<usize> {
    let n = pi.len();
    let t_steps = obs.len();
    let mut a: Vec<f64> = pi.to_vec();
    let mut psi = vec![vec![0usize; n]; t_steps];
    for t in 0..t_steps {
        let mut nd = vec![0.0f64; n];
        for j in 0..n {
            let mut best = 0.0f64;
            let mut bi = 0usize;
            for i in 0..n {
                let c = a[i] * A[i][j] * E[j][obs[t]];
                if c > best {
                    best = c;
                    bi = i;
                }
            }
            nd[j] = best;
            psi[t][j] = bi;
        }
        a = nd;
    }
    let mut last = 0usize;
    for j in 1..n {
        if a[j] > a[last] {
            last = j;
        }
    }
    let mut path = vec![last];
    for t in (0..t_steps).rev() {
        if t == 0 {
            break;
        }
        path.push(psi[t][*path.last().unwrap()]);
    }
    path.reverse();
    path
}

// ---------------------------------------------------------------------------
// Two-chart projective form: exact zeros tracked as integer multiplicity,
// never materialised as -inf. 0 and infinity are FINITE bookkeeping states.
// ---------------------------------------------------------------------------

/// Homogeneous state: positive part `p` (SP) and exact-zero multiplicity `z`.
/// value = 0 if z > 0, infinity if z < 0, else p.m * 2^p.e.
/// chart 1 (a-chart): z >= 0 ; chart 2 (b-chart): z < 0.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjPair {
    pub p: SP,
    pub z: i32,
    /// Cause of an exact zero. Two zeros may be cancelled only if they carry
    /// the SAME tag (a genuinely shared factor).
    pub zero_tag: Option<String>,
}

impl ProjPair {
    pub fn new(p: SP, z: i32, zero_tag: Option<String>) -> ProjPair {
        ProjPair { p, z, zero_tag }
    }

    /// a zero dominates the product; keep its provenance tag. If both are
    /// zero with different tags, the product's zero cause is ambiguous -> None.
    pub fn mul(&self, o: &ProjPair) -> ProjPair {
        let mut tag = if self.z != 0 {
            self.zero_tag.clone()
        } else {
            o.zero_tag.clone()
        };
        if self.z != 0 && o.z != 0 && self.zero_tag != o.zero_tag {
            tag = None;
        }
        ProjPair {
            p: SP::mul(self.p, o.p),
            z: self.z + o.z,
            zero_tag: tag,
        }
    }

    pub fn ratio(&self, o: &ProjPair) -> ProjPair {
        let dz = self.z - o.z;
        if dz > 0 {
            return ProjPair::new(ONE, 1, None);
        }
        if dz < 0 {
            return ProjPair::new(ONE, -1, None);
        }
        if self.z > 0 {
            // both exactly zero: 0/0
            if self.zero_tag.is_some() && self.zero_tag == o.zero_tag {
                // removable singularity: lim (t*U)/(t*V) = U/V for SHARED t
                return ProjPair::new(SP::div(self.p, o.p), 0, None);
            }
            // independent impossibilities: genuinely indeterminate
            return ProjPair::new(ONE, 0, Some("INDETERMINATE_0_0".to_string()));
        }
        ProjPair::new(SP::div(self.p, o.p), 0, None)
    }
}

/// Result of `projective_demo`, mirroring the Python dict fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectiveDemo {
    pub float_0_over_0: String,
    pub log_minus_inf_minus_minus_inf: String,
    pub projective_ratio_finite: f64,
    pub expected_u_over_v: f64,
    pub zero_chart: i32,
    pub zero_is_finite_bookkeeping: bool,
    pub infinite_repr_z: i32,
    pub ratio_relerr: f64,
    pub shared_zero_valid: bool,
    pub independent_zero_verdict: Option<String>,
    pub independent_zero_is_indeterminate: bool,
}

/// VALID case: a shared impossible transition (exact-zero factor eps) appears
/// in BOTH hypotheses with the SAME provenance: P_A = eps*U, P_B = eps*V.
/// Then lim_{eps->0}(eps*U)/(eps*V) = U/V is a removable singularity.
/// INVALID case: independent impossible transitions (different tags) with the
/// SAME real 0/0 must NOT fabricate a finite ratio.
pub fn projective_demo() -> ProjectiveDemo {
    let (u, v) = (1e-300f64, 3e-305f64);
    let fa = 0.0 * u; // float: exactly 0
    let fb = 0.0 * v; // float: exactly 0
    let f_ratio = fa / fb; // 0/0 -> NaN (Python catches ZeroDivisionError -> nan)
    let l_ratio = (f64::NEG_INFINITY + u.ln()) - (f64::NEG_INFINITY + v.ln()); // nan

    let shared = Some("shared".to_string());
    let eps_a = ProjPair::new(ONE, 1, shared.clone());
    let eps_b = ProjPair::new(ONE, 1, shared);
    let a = eps_a.mul(&ProjPair::new(SP::from_float(u).unwrap(), 0, None));
    let b = eps_b.mul(&ProjPair::new(SP::from_float(v).unwrap(), 0, None));
    let pr = a.ratio(&b);
    let proj_val = pr.p.to_float();

    let bad_a = ProjPair::new(ONE, 1, Some("bad_a".to_string()))
        .mul(&ProjPair::new(SP::from_float(u).unwrap(), 0, None));
    let bad_b = ProjPair::new(ONE, 1, Some("bad_b".to_string()))
        .mul(&ProjPair::new(SP::from_float(v).unwrap(), 0, None));
    let bad = bad_a.ratio(&bad_b);

    let nan_str = |x: f64| {
        if x.is_nan() {
            "nan".to_string()
        } else {
            x.to_string()
        }
    };
    let expected = u / v;
    ProjectiveDemo {
        float_0_over_0: nan_str(f_ratio),
        log_minus_inf_minus_minus_inf: nan_str(l_ratio),
        projective_ratio_finite: proj_val,
        expected_u_over_v: expected,
        zero_chart: a.z,
        zero_is_finite_bookkeeping: true,
        infinite_repr_z: ProjPair::new(ONE, -1, None).z,
        ratio_relerr: (proj_val - expected).abs() / expected,
        shared_zero_valid: pr.z == 0 && pr.zero_tag.is_none(),
        independent_zero_verdict: bad.zero_tag.clone(),
        independent_zero_is_indeterminate: bad.zero_tag.as_deref() == Some("INDETERMINATE_0_0"),
    }
}

/// Verify Prod(b_i/a_i) == exp(Sum log(b_i/a_i)) to ~1e-14 over 600 decades.
/// Uses an f64 simulation of the deferred product (no mpmath).
pub fn ratio_mult_vs_llr_add() -> f64 {
    let mut state: u64 = 7; // deterministic PRNG (xorshift64), replaces rng(7)
    let mut rand = || -> f64 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as f64) / (u64::MAX as f64)
    };
    let mut r = ONE;
    let mut llr = 0.0f64;
    for _ in 0..1200 {
        let ai = 10.0f64.powf(-300.0 + 600.0 * rand());
        let bi = 10.0f64.powf(-300.0 + 600.0 * rand());
        r = SP::mul(r, SP::div(SP::from_float(bi).unwrap(), SP::from_float(ai).unwrap()));
        llr += bi.ln() - ai.ln();
    }
    let log_prod = r.log_value();
    if llr != 0.0 {
        (log_prod - llr).abs() / llr.abs()
    } else {
        (log_prod - llr).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sp_zero() {
        let z = SP::from_float(0.0).unwrap();
        assert_eq!(z.m, 0.0);
        assert_eq!(z.e, 0);
        assert_eq!(z.to_float(), 0.0);
        // Constructor normalizes zero's exponent away.
        assert_eq!(SP::new(0.0, 17).unwrap(), SP { m: 0.0, e: 0 });
        assert_eq!(SP::mul(SP::one(), z), z);
        assert_eq!(SP::add(z, SP::one()), SP::one());
    }

    #[test]
    fn inf_nan_and_negative_rejected() {
        assert!(SP::from_float(f64::INFINITY).is_err());
        assert!(SP::from_float(f64::NEG_INFINITY).is_err());
        assert!(SP::from_float(f64::NAN).is_err());
        assert!(SP::from_float(-1.0).is_err());
        assert!(SP::new(f64::INFINITY, 0).is_err());
        assert!(SP::new(f64::NAN, 0).is_err());
        assert!(SP::new(-0.5, 0).is_err());
        assert!(SP::from_float(1.0).is_ok());
    }

    #[test]
    fn subnormal_and_overflow_to_float() {
        // 5e-324 is the smallest subnormal: value = 2^-1074.
        let tiny = SP::from_float(f64::from_bits(1)).unwrap();
        assert_eq!(tiny.e, -1074);
        assert_eq!(tiny.m, 1.0);
        assert_eq!(tiny.to_float(), f64::from_bits(1));
        // ldexp(1.5, -1074) rounds to 1e-323.
        assert_eq!(SP::new(1.5, -1074).unwrap().to_float(), 1e-323);
        // True subnormal beyond -1074 still rounds correctly.
        assert_eq!(SP::new(1.5, -1075).unwrap().to_float(), f64::from_bits(1));
        assert_eq!(SP::new(1.0, -1075).unwrap().to_float(), 0.0);
        // Overflow beyond float64 -> +inf (guarded and ldexp-catch equivalent).
        assert_eq!(SP::new(1.0, 1024).unwrap().to_float(), f64::INFINITY);
        assert_eq!(SP::new(1.0, 1023).unwrap().to_float(), 2f64.powi(1023));
    }

    #[test]
    fn shared_zero_provenance_cancels() {
        let demo = projective_demo();
        assert!(demo.shared_zero_valid);
        assert_eq!(demo.zero_chart, 1);
        assert_eq!(demo.infinite_repr_z, -1);
        assert!((demo.projective_ratio_finite - demo.expected_u_over_v).abs() / demo.expected_u_over_v < 1e-12);
        assert_eq!(demo.float_0_over_0, "nan");
        assert_eq!(demo.log_minus_inf_minus_minus_inf, "nan");
        // Direct construction: same tag on both zeros -> removable singularity.
        let u = SP::from_float(2.0).unwrap();
        let v = SP::from_float(8.0).unwrap();
        let eps = Some("shared-factor".to_string());
        let a = ProjPair::new(ONE, 1, eps.clone()).mul(&ProjPair::new(u, 0, None));
        let b = ProjPair::new(ONE, 1, eps).mul(&ProjPair::new(v, 0, None));
        let r = a.ratio(&b);
        assert_eq!(r.z, 0);
        assert_eq!(r.zero_tag, None);
        assert_eq!(r.p.to_float(), 0.25);
    }

    #[test]
    fn independent_zero_is_indeterminate() {
        let demo = projective_demo();
        assert_eq!(demo.independent_zero_verdict, Some("INDETERMINATE_0_0".to_string()));
        assert!(demo.independent_zero_is_indeterminate);
        let u = SP::from_float(2.0).unwrap();
        let v = SP::from_float(8.0).unwrap();
        let a = ProjPair::new(ONE, 1, Some("a".into())).mul(&ProjPair::new(u, 0, None));
        let b = ProjPair::new(ONE, 1, Some("B".into())).mul(&ProjPair::new(v, 0, None));
        let r = a.ratio(&b);
        assert_eq!(r.z, 0);
        assert_eq!(r.zero_tag, Some("INDETERMINATE_0_0".to_string()));
    }

    #[test]
    fn sp_max_never_lets_zero_win() {
        let pos = SP::from_float(1e-300).unwrap(); // e ~ -997
        let zero = ZERO; // m=0, e=0 (larger exponent!)
        assert_eq!(sp_max(pos, zero), (pos, true));
        assert_eq!(sp_max(zero, pos), (pos, false));
        assert_eq!(sp_max(zero, zero), (zero, true));
    }

    #[test]
    fn viterbi_excludes_zero_state() {
        // State 1 has zero emission for the only observation. Its terminal SP
        // is ZERO (e=0), which would beat the real state 0 path (e ~ -997) if
        // zero states were not excluded before argmax.
        let a = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let e = vec![vec![1e-300, 1.0], vec![0.0, 1.0]];
        let obs = vec![0usize];
        let pi = vec![1.0, 1.0];
        let path = viterbi_koppa(&a, &e, &obs, &pi);
        assert_eq!(path, vec![0], "zero terminal state must not win the argmax");

        // All terminal states impossible: fallback picks state 0, then zeros.
        let pi_zero = vec![0.0, 0.0];
        let fallback = viterbi_koppa(&a, &e, &obs, &pi_zero);
        assert_eq!(fallback, vec![0]);
    }

    #[test]
    fn forward_koppa_matches_log_domain() {
        // Small HMM where all four engines agree; exercises the deferred pass.
        let a = vec![vec![0.7, 0.3], vec![0.4, 0.6]];
        let e = vec![vec![0.9, 0.1], vec![0.2, 0.8]];
        let obs = vec![0usize, 1, 0];
        let pi = vec![0.6, 0.4];
        let (lk, lk_mp, coll) = forward_koppa(&a, &e, &obs, &pi);
        let (ll, _) = forward_log(&a, &e, &obs, &pi);
        let (ln, _, _) = forward_naive(&a, &e, &obs, &pi);
        let (lm, _) = forward_mp(&a, &e, &obs, &pi);
        assert_eq!(coll, 0);
        assert!((lk - ln).abs() < 1e-12, "koppa {} vs naive {}", lk, ln);
        assert!((lk - lk_mp).abs() < 1e-12);
        assert!((lk - lm).abs() < 1e-12);
        assert!((ll - lk.ln()).abs() < 1e-10, "log {} vs koppa {}", ll, lk.ln());
    }

    #[test]
    fn ratio_mult_vs_llr_add_is_tiny() {
        let err = ratio_mult_vs_llr_add();
        assert!(err < 1e-11, "relative log error too large: {}", err);
    }
}
