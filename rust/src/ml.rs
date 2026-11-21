//! LazyRelational -- ML / ML-infrastructure application demo (Rust port).
//!
//! (i)   LazyRelational numeric type: unresolved pair (a,b), resolved form r=b/a.
//!       compose() is associative & multiplicative; rescale() is the gauge transform;
//!       gauge() returns g=d(log r)/dt = d(log b)/dt - d(log a)/dt (scale invariant).
//!       Mantissa + integer-exponent storage spans arbitrarily many decades.
//!
//! (ii)  Real workload: log-odds / likelihood-ratio score chains whose hidden
//!       scales exceed float64 while the resolved product stays finite (or
//!       overflows). Compared: eager float64 vs deferred resolution vs log-domain.
//!
//! (iii) Scale-equivariance gradient ORACLE: a custom VJP correct at gauge scale 1
//!       but wrong after rescaling. Gradcheck at the canonical scale passes; the
//!       gauge oracle rescales intermediates and checks gradient equivariance,
//!       detecting the bug with no analytic reference gradient.

use std::error::Error;
use std::fmt;

// ---------------------------------------------------------------------------
// Error type (ValueError-equivalent)
// ---------------------------------------------------------------------------

/// Error raised when a mantissa is not finite -- the Rust equivalent of the
/// Python `ValueError("LazyRelational mantissa must be finite")`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MlError {
    pub message: String,
}

impl MlError {
    fn non_finite(m: f64) -> Self {
        MlError {
            message: format!("LazyRelational mantissa must be finite; got {}", m),
        }
    }
}

impl fmt::Display for MlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for MlError {}

// ---------------------------------------------------------------------------
// (i) LazyRelational
// ---------------------------------------------------------------------------

/// Normalize a mantissa to [1,10) while preserving its sign, rejecting non-finite
/// values. Mirrors the Python `_norm` edge-case patch: the old code did
/// `m = -m` for m<0 (discarding sign) and looped forever for m=inf.
fn norm(m: f64, e: i32) -> Result<(f64, i32), MlError> {
    if !m.is_finite() {
        return Err(MlError::non_finite(m));
    }
    if m == 0.0 {
        return Ok((0.0, 0));
    }
    let sign = if m < 0.0 { -1.0 } else { 1.0 };
    let mut m = m.abs();
    let mut e = e;
    while m >= 10.0 {
        m /= 10.0;
        e = e.saturating_add(1);
    }
    while m < 1.0 {
        m *= 10.0;
        e = e.saturating_sub(1);
    }
    Ok((sign * m, e))
}

/// 10^de with float64 saturation, mirroring Python's `10.0 ** de` (which raises
/// `OverflowError` for `de > 308`; here that becomes `inf`).
fn pow10(de: i64) -> f64 {
    if de > 308 {
        f64::INFINITY
    } else if de < -323 {
        0.0
    } else {
        10f64.powi(de as i32)
    }
}

/// Unresolved pair (a,b) = (am*10^ae, bm*10^be); resolved r = b/a.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LazyRelational {
    pub am: f64,
    pub ae: i32,
    pub bm: f64,
    pub be: i32,
}

impl LazyRelational {
    /// Construct from mantissas and integer exponents. Non-finite mantissas are
    /// rejected with an Err (the ValueError-equivalent); mantissas are normalized
    /// to [1,10) with sign preserved.
    pub fn new(am: f64, ae: i32, bm: f64, be: i32) -> Result<Self, MlError> {
        let (am, ae) = norm(am, ae)?;
        let (bm, be) = norm(bm, be)?;
        Ok(LazyRelational { am, ae, bm, be })
    }

    /// Panicking guard variant of `new` for call sites that know inputs are finite.
    pub fn must_new(am: f64, ae: i32, bm: f64, be: i32) -> Self {
        Self::new(am, ae, bm, be).expect("LazyRelational mantissa must be finite")
    }

    /// The multiplicative identity: resolved r = 1.
    pub fn identity() -> Self {
        LazyRelational {
            am: 1.0,
            ae: 0,
            bm: 1.0,
            be: 0,
        }
    }

    /// Compose two pairs: multiply mantissas, add exponents, renormalize.
    /// Associative & multiplicative under resolution.
    pub fn compose(&self, other: &Self) -> Result<Self, MlError> {
        Self::new(
            self.am * other.am,
            self.ae.saturating_add(other.ae),
            self.bm * other.bm,
            self.be.saturating_add(other.be),
        )
    }

    /// Gauge transform (a,b) -> (lam*a, lam*b); resolved r is invariant.
    pub fn rescale(&self, lam_m: f64, lam_e: i32) -> Result<Self, MlError> {
        if !lam_m.is_finite() {
            return Err(MlError::non_finite(lam_m));
        }
        Self::new(
            self.am * lam_m,
            self.ae.saturating_add(lam_e),
            self.bm * lam_m,
            self.be.saturating_add(lam_e),
        )
    }

    /// g = d(log r)/dt, invariant under a common rescaling.
    pub fn gauge(&self, d_log_a: f64, d_log_b: f64) -> f64 {
        d_log_b - d_log_a
    }

    /// Resolve once at the end; may overflow to inf for huge r.
    pub fn resolved_float(&self) -> f64 {
        let de = self.be as i64 - self.ae as i64;
        (self.bm / self.am) * pow10(de)
    }

    /// log10 of the resolved value; valid even when the value overflows float64.
    pub fn resolved_log10(&self) -> f64 {
        (self.bm / self.am).log10() + (self.be as i64 - self.ae as i64) as f64
    }
}

impl fmt::Display for LazyRelational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LR(r~={:.6}*10^{})",
            self.bm / self.am,
            self.be as i64 - self.ae as i64
        )
    }
}

// ---------------------------------------------------------------------------
// Chain helpers
// ---------------------------------------------------------------------------

/// Step i has hidden scale u_i = slope*i and log-ratio d_i, so
/// a_i = 10^{u_i}, b_i = 10^{u_i+d_i}, r_i = 10^{d_i}, product = 10^{sum d_i}.
/// Returns (am, ae, bm, be) tuples with mantissas in [1,10).
pub fn build_chain<F>(n: usize, slope: f64, d_fn: F) -> Vec<(f64, i32, f64, i32)>
where
    F: Fn(usize) -> f64,
{
    let mut chain = Vec::with_capacity(n);
    for i in 1..=n {
        let u = slope * i as f64;
        let d = d_fn(i);
        let v = u + d;
        let ma = 10f64.powf(u - u.floor());
        let mb = 10f64.powf(v - v.floor());
        chain.push((ma, u.floor() as i32, mb, v.floor() as i32));
    }
    chain
}

/// Oscillating log-ratio d_i, keeps the composed product near 10^1.
pub fn d_osc(i: usize) -> f64 {
    0.02 * ((i as f64) * 0.731).sin() + 0.005 * ((i as f64) * 0.113).cos()
}

/// Materialize each a_i,b_i as float64, resolve each ratio, multiply.
/// Returns (product, number_of_collapses).
pub fn eager_float64(chain: &[(f64, i32, f64, i32)]) -> (f64, usize) {
    let mut x = 1.0;
    let mut collapses = 0usize;
    for &(am, ae, bm, be) in chain {
        let a = if ae < 309 { am * pow10(ae as i64) } else { f64::INFINITY };
        let b = if be < 309 { bm * pow10(be as i64) } else { f64::INFINITY };
        let r = b / a;
        x *= r;
        if !x.is_finite() || !r.is_finite() {
            collapses += 1;
        }
    }
    (x, collapses)
}

/// Compose unresolved pairs; resolve ONCE at the end.
pub fn deferred(chain: &[(f64, i32, f64, i32)]) -> (LazyRelational, f64, usize) {
    let mut acc = LazyRelational::identity();
    for &(am, ae, bm, be) in chain {
        acc = acc
            .compose(&LazyRelational::must_new(am, ae, bm, be))
            .expect("normalized chain values must remain finite");
    }
    let resolved = acc.resolved_float();
    (acc, resolved, 0)
}

/// Sum log10 ratios directly, then exponentiate once.
pub fn log_domain(chain: &[(f64, i32, f64, i32)]) -> f64 {
    let mut s = 0.0;
    for &(am, ae, bm, be) in chain {
        s += (bm / am).log10() + (be as i64 - ae as i64) as f64;
    }
    // 10^s with the fractional part preserved; powf saturates to inf on overflow.
    10f64.powf(s)
}

/// Deferred normalization attention-style aggregation result.
pub struct AttentionResult {
    pub num_scores: usize,
    pub max_hidden_decades: f64,
    pub aggregated_ratio: f64,
    pub resolved_log10: f64,
    pub collapses: usize,
    pub note: &'static str,
}

/// Aggregate unnormalized logit ratios as pairs, form the normalizer once,
/// avoiding per-head normalized tensors (hidden 10^40..10^320, > float64).
pub fn attention_demo() -> AttentionResult {
    let mut pairs = Vec::with_capacity(8);
    for j in 1..=8 {
        let u = 40.0 * j as f64;
        let v = u + 0.3 * (j as f64).sin();
        let ma = 10f64.powf(u - u.floor());
        let mb = 10f64.powf(v - v.floor());
        pairs.push(LazyRelational::must_new(
            ma,
            u.floor() as i32,
            mb,
            v.floor() as i32,
        ));
    }
    let mut acc = LazyRelational::identity();
    for p in &pairs {
        acc = acc.compose(p).expect("normalized pairs must remain finite");
    }
    let total = acc.resolved_float();
    AttentionResult {
        num_scores: pairs.len(),
        max_hidden_decades: 40.0 * pairs.len() as f64,
        aggregated_ratio: total,
        resolved_log10: acc.resolved_log10(),
        collapses: if total.is_finite() { 0 } else { 1 },
        note: "10^320 ratios unrepresentable as float64 compose exactly as (mantissa,int exponent); normalize once.",
    }
}

/// Gauge invariance check result.
pub struct GaugeInvarianceResult {
    pub g: f64,
    pub g_rescaled: f64,
    pub invariant: bool,
}

/// Check that g = d(log r)/dt is invariant under a common rescaling.
pub fn gauge_invariance_check() -> GaugeInvarianceResult {
    let a = 2.0;
    let b = 8.0;
    let da = 0.3;
    let db = 1.7;
    let g = LazyRelational::must_new(a, 0, b, 0).gauge(da / a, db / b);
    let lam = 5.0;
    let dlam = 0.9;
    let lr2 = LazyRelational::must_new(a, 0, b, 0)
        .rescale(lam, 0)
        .expect("finite scale");
    let g2 = lr2.gauge(
        (dlam * a + lam * da) / (lam * a),
        (dlam * b + lam * db) / (lam * b),
    );
    GaugeInvarianceResult {
        g,
        g_rescaled: g2,
        invariant: (g - g2).abs() < 1e-12,
    }
}

// ---------------------------------------------------------------------------
// (iii) Gradient oracle
// ---------------------------------------------------------------------------

/// r=b/a; correct VJP: ga=-up*r/a, gb=up/a.
pub fn chain_forward(pairs: &[(f64, f64)]) -> f64 {
    let mut f = 1.0;
    for &(a, b) in pairs {
        f *= b / a;
    }
    f
}

/// Returns (forward_value, grads_wrt_a, grads_wrt_b).
/// If `buggy`, a stale normalization cache divides gradients by `scale`.
pub fn chain_vjp(pairs: &[(f64, f64)], scale: f64, buggy: bool) -> (f64, Vec<f64>, Vec<f64>) {
    let f = chain_forward(pairs);
    let mut gas = Vec::with_capacity(pairs.len());
    let mut gbs = Vec::with_capacity(pairs.len());
    for &(a, b) in pairs {
        let up = f / (b / a); // d(product)/d(r_i), grad_out = 1
        let r = b / a;
        let mut ga = -up * r / a;
        let mut gb = up / a;
        if buggy {
            ga /= scale; // stale/divergent normalization cache
            gb /= scale;
        }
        gas.push(ga);
        gbs.push(gb);
    }
    (f, gas, gbs)
}

/// Finite-difference check of the first gradient against the analytic VJP.
/// Returns (ok, finite_difference, vjp_gradient).
pub fn gradcheck(pairs: &[(f64, f64)], scale: f64, buggy: bool, eps: f64) -> (bool, f64, f64) {
    let (_, gas, _) = chain_vjp(pairs, scale, buggy);
    let (a0, b0) = pairs[0];
    let mut pp = pairs.to_vec();
    let mut pm = pairs.to_vec();
    pp[0] = (a0 + eps, b0);
    pm[0] = (a0 - eps, b0);
    let fd = (chain_forward(&pp) - chain_forward(&pm)) / (2.0 * eps);
    let ok = (fd - gas[0]).abs() < 1e-4 * fd.abs().max(1.0);
    (ok, fd, gas[0])
}

pub struct GaugeOracleResult {
    pub equivariance_residual: f64,
    pub detected: bool,
    pub lam: f64,
}

/// Rescale intermediates and check gradient equivariance: a correct VJP has
/// g_scaled * lam == g_canonical; the buggy VJP has an extra /lam.
pub fn gauge_oracle(pairs: &[(f64, f64)], lam: f64, buggy: bool, tol: f64) -> GaugeOracleResult {
    let (_, gas, _) = chain_vjp(pairs, 1.0, buggy);
    let rescaled: Vec<(f64, f64)> = pairs.iter().map(|&(a, b)| (lam * a, lam * b)).collect();
    let (_, gas_r, _) = chain_vjp(&rescaled, lam, buggy);
    let worst = gas_r
        .iter()
        .zip(gas.iter())
        .map(|(g, g0)| (g * lam - g0).abs() / g0.abs().max(1e-300))
        .fold(f64::NEG_INFINITY, f64::max);
    GaugeOracleResult {
        equivariance_residual: worst,
        detected: worst > tol,
        lam,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_3_times_5_is_15() {
        let a = LazyRelational::new(1.0, 0, 3.0, 0).unwrap(); // r = 3
        let b = LazyRelational::new(1.0, 0, 5.0, 0).unwrap(); // r = 5
        let c = a.compose(&b).unwrap();
        assert!((c.resolved_float() - 15.0).abs() < 1e-12, "got {}", c);
        // composition is associative
        let d = LazyRelational::new(1.0, 0, 2.0, 0).unwrap();
        let left = a.compose(&b).unwrap().compose(&d).unwrap();
        let right = a.compose(&b.compose(&d).unwrap()).unwrap();
        assert!((left.resolved_float() - right.resolved_float()).abs() < 1e-12);
    }

    #[test]
    fn negative_two_stays_negative() {
        // The old bug did `m = -m` and resolved -2 as +2.
        let lr = LazyRelational::new(1.0, 0, -2.0, 0).unwrap();
        assert!(lr.resolved_float() < 0.0, "sign was discarded: {}", lr);
        assert!((lr.resolved_float() + 2.0).abs() < 1e-12, "got {}", lr);
        // negative mantissa on the 'a' side flips the ratio sign too
        let lr2 = LazyRelational::new(-2.0, 0, 1.0, 0).unwrap();
        assert!((lr2.resolved_float() + 0.5).abs() < 1e-12, "got {}", lr2);
    }

    #[test]
    fn non_finite_rejected() {
        assert!(LazyRelational::new(f64::NAN, 0, 1.0, 0).is_err());
        assert!(LazyRelational::new(f64::INFINITY, 0, 1.0, 0).is_err());
        assert!(LazyRelational::new(f64::NEG_INFINITY, 0, 1.0, 0).is_err());
        assert!(LazyRelational::new(1.0, 0, f64::NAN, 0).is_err());
        assert!(LazyRelational::new(1.0, 0, f64::INFINITY, 0).is_err());
        // rescale with a non-finite gauge scale is rejected too
        let lr = LazyRelational::identity();
        assert!(lr.rescale(f64::INFINITY, 0).is_err());
    }

    #[test]
    fn normalization_and_exponents() {
        let lr = LazyRelational::new(30.0, 2, 0.05, 4).unwrap();
        assert!((lr.am - 3.0).abs() < 1e-12);
        assert_eq!(lr.ae, 3);
        assert!((lr.bm - 5.0).abs() < 1e-12);
        assert_eq!(lr.be, 2);
        // resolved = (5*10^2)/(3*10^3) = 0.1666...
        assert!((lr.resolved_float() - 1.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn identity_and_rescale_invariant() {
        let lr = LazyRelational::new(2.0, 0, 8.0, 0).unwrap();
        let r0 = lr.resolved_float();
        let scaled = lr.rescale(7.0, 3).unwrap();
        assert!((scaled.resolved_float() - r0).abs() < 1e-12);
        assert!((identity_comp()).abs() < 1e-12);
        let _ = lr.gauge(0.1, 0.4);
    }

    fn identity_comp() -> f64 {
        let c = LazyRelational::identity().compose(&LazyRelational::identity()).unwrap();
        c.resolved_float() - 1.0
    }

    #[test]
    fn gauge_invariance_scale_covariance() {
        // Direct gauge() invariance.
        let a = 2.0;
        let b = 8.0;
        let da = 0.3;
        let db = 1.7;
        let g = LazyRelational::new(a, 0, b, 0).unwrap().gauge(da / a, db / b);
        let lam = 5.0;
        let dlam = 0.9;
        let g2 = LazyRelational::new(a, 0, b, 0)
            .unwrap()
            .rescale(lam, 0)
            .unwrap()
            .gauge(
                (dlam * a + lam * da) / (lam * a),
                (dlam * b + lam * db) / (lam * b),
            );
        assert!((g - g2).abs() < 1e-12, "g={} g2={}", g, g2);

        // And via the helper.
        let r = gauge_invariance_check();
        assert!(r.invariant);
        assert!((r.g - r.g_rescaled).abs() < 1e-12);
    }

    #[test]
    fn deferred_handles_huge_hidden_scale() {
        // 5000 steps with hidden scale to 600 decades; oscillating d_i keeps
        // the resolved product near 10^0, so deferred resolution stays finite.
        let chain = build_chain(5000, 0.12, d_osc);
        let (acc, resolved, collapses) = deferred(&chain);
        assert_eq!(collapses, 0);
        assert!(resolved.is_finite(), "deferred result overflowed: {}", acc);
        assert!((acc.resolved_log10() - 0.0).abs() < 1.0, "log10={}", acc.resolved_log10());
        // Eager float64 collapses at least once under this hidden scale.
        let (xe, ce) = eager_float64(&chain);
        assert!(ce > 0 || !xe.is_finite());
    }

    #[test]
    fn huge_exponent_resolves_to_inf() {
        let lr = LazyRelational::new(1.0, 0, 1.0, 400).unwrap();
        assert!(lr.resolved_float().is_infinite());
        assert!((lr.resolved_log10() - 400.0).abs() < 1e-12);
    }

    #[test]
    fn attention_and_oracle() {
        let att = attention_demo();
        assert_eq!(att.num_scores, 8);
        assert!((att.max_hidden_decades - 320.0).abs() < 1e-9);
        assert!(att.resolved_log10.is_finite());

        let pairs = [(1.3, 2.7), (0.8, 1.9), (2.2, 3.1), (1.7, 4.2), (0.9, 2.3)];
        // Buggy VJP passes gradcheck at the canonical scale...
        let (ok, _, _) = gradcheck(&pairs, 1.0, true, 1e-5);
        assert!(ok);
        // ...but the gauge oracle detects it after rescaling.
        let bug = gauge_oracle(&pairs, 10.0, true, 1e-6);
        assert!(bug.detected, "residual={}", bug.equivariance_residual);
        // The correct VJP is equivariant.
        let good = gauge_oracle(&pairs, 10.0, false, 1e-6);
        assert!(!good.detected);
        assert!(good.equivariance_residual < 1e-6);
    }
}
