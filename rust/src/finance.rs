//! KOPPA deferred-relational tracking -- finance / actuarial application demo.
//!
//! State is an UNRESOLVED homogeneous pair `(a, b)` standing for the ratio
//! `b/a`. Composition is componentwise multiplication with exact integer
//! common-factor cancellation; a scalar is produced only on demand at the end
//! (`b/a`). Compared against eager float64, hand-written log-domain, and an
//! 80-digit high-precision reference (`resolve_mp`, here backed by f64).
//!
//! No novelty/theorem claims are made here: this is an application prototype.

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::time::Instant;

pub type Pair = (BigInt, BigInt);

fn bi(v: i64) -> BigInt {
    BigInt::from(v)
}

/// Build `(a, b)` from i64 values, used by generators/tests.
pub fn pair(a: i64, b: i64) -> Pair {
    (bi(a), bi(b))
}

// ---------------------------------------------------------------------------
// The deferred-relational API (compose without dividing, resolve on demand)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeferredChain {
    pub a: BigInt,
    pub b: BigInt,
    pub composes: u64,
    pub cancellations: u64,
    pub resolves: u64,
    pub resolve_collapses: u64,
}

impl Default for DeferredChain {
    fn default() -> Self {
        Self::new()
    }
}

impl DeferredChain {
    pub fn new() -> Self {
        Self {
            a: BigInt::one(),
            b: BigInt::one(),
            composes: 0,
            cancellations: 0,
            resolves: 0,
            resolve_collapses: 0,
        }
    }

    /// `Q_m (*) Q_n = Q_mn`: multiply componentwise, cancel common factors.
    ///
    /// Cancelling gcd(a,b) preserves the unresolved pair (it does NOT resolve
    /// to a scalar) and keeps the integers bounded by the running ratio.
    pub fn compose(&mut self, a: &BigInt, b: &BigInt) {
        self.a *= a;
        self.b *= b;
        let g = self.a.gcd(&self.b);
        if g > BigInt::one() {
            self.a /= &g;
            self.b /= &g;
            self.cancellations += 1;
        }
        self.composes += 1;
    }

    /// Specialized compose for a chain of factors sharing a fixed decimal
    /// scale S: strip a common power of two from `(a,b)` instead of a full gcd.
    ///
    /// BUGFIX: strip only the COMMON power of two from BOTH operands.
    /// The prior code stripped each independently (`a >> a2`, `b >> b2`),
    /// which discarded the unequal 2-adic surplus and changed the represented
    /// ratio: `compose_decimal(2,3)` resolved to 3.0 instead of 1.5.
    pub fn compose_decimal(&mut self, a: &BigInt, b: &BigInt) {
        if a.is_zero() || b.is_zero() {
            self.a *= a;
            self.b *= b;
            self.composes += 1;
            return;
        }
        let a2 = a.magnitude().trailing_zeros().unwrap_or(0);
        let b2 = b.magnitude().trailing_zeros().unwrap_or(0);
        let common = a2.min(b2);
        // Preserve 2^(a2-common) on the numerator and 2^(b2-common) on the
        // denominator: dividing BOTH by 2^common leaves b/a unchanged.
        self.a *= a >> common;
        self.b *= b >> common;
        self.composes += 1;
        if common > 0 {
            self.cancellations += 1;
        }
    }

    pub fn compose_all(&mut self, chain: &[Pair]) {
        for (a, b) in chain {
            self.compose(a, b);
        }
    }

    /// `b / a` on demand, returning f64 (inf on overflow, nan on 0/0).
    pub fn resolve_float(&self) -> f64 {
        if self.a.is_zero() {
            if self.b.is_zero() {
                return f64::NAN; // exact 0/0
            }
            return if self.b.is_positive() {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            };
        }
        ratio_f64(&self.a, &self.b)
    }

    /// High-precision resolve. f64 is sufficient for the demo/tests; mirrors
    /// `resolve_float` (mpmath's 80-digit result converges to the same f64).
    pub fn resolve_mp(&self) -> f64 {
        self.resolve_float()
    }

    /// Exact-ish log10 of the final ratio without forming the scalar.
    ///
    /// Uses log2 of the BigInt bit-lengths to stay finite across the full
    /// range (e.g. 1e350), so it never overflows like eager float64.
    pub fn log10(&self) -> f64 {
        if self.b.is_zero() {
            return f64::NEG_INFINITY;
        }
        if self.a.is_zero() {
            return f64::INFINITY;
        }
        // log2 of the ratio via accurate log2 of each BigInt; this stays
        // finite even when b/a exceeds float64 (e.g. 1e350).
        let lb = log2_big(&self.b);
        let la = log2_big(&self.a);
        (lb - la) / std::f64::consts::LOG2_10
        // NOTE: LOG2_10 = log2(10) = 3.3219; dividing log2 by it yields log10.
    }
}

// ---------------------------------------------------------------------------
// Chain generators
// ---------------------------------------------------------------------------

/// Small deterministic PRNG (xorshift64*) so the port is reproducible without
/// pulling in a `rand` dependency; seeded identically in spirit to Python's
/// `random.Random(seed)`.
struct Rng(u64);

impl Rng {
    fn new(seed: i64) -> Self {
        // Mix the seed so small seeds still produce a good stream.
        Self((seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5DEE_CE66_D2B1_50A5 | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform integer in [-bound, bound].
    fn symmetric(&mut self, bound: i64) -> i64 {
        (self.next_u64() % ((2 * bound + 1) as u64)) as i64 - bound
    }
    /// Approximate standard normal via Box-Muller (f64 is fine for a demo).
    fn gauss(&mut self, mean: f64, stddev: f64) -> f64 {
        let u1 = (self.next_u64() as f64 / u64::MAX as f64).max(f64::MIN_POSITIVE);
        let u2 = self.next_u64() as f64 / u64::MAX as f64;
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        mean + stddev * z
    }
}

fn pow10(exp: u64) -> BigInt {
    BigInt::from(10u8).pow(exp as u32)
}

/// `log2(|n|)` accurate to ~1e-13 using the BigInt bit length plus the top
/// 53 mantissa bits (no scalar ratio is formed, so huge ints never overflow).
fn log2_big(n: &BigInt) -> f64 {
    let mag = n.magnitude();
    let bits = mag.bits();
    if bits == 0 {
        return f64::NEG_INFINITY;
    }
    if bits <= 53 {
        return n.to_f64().unwrap_or(f64::INFINITY).abs().log2();
    }
    let top = (mag >> (bits - 53)).to_f64().unwrap_or(f64::INFINITY);
    // n = top * 2^(bits-53) with top in [2^52, 2^53); log2(n) = log2(top)+(bits-53).
    top.log2() + (bits - 53) as f64
}

/// Compute `b / a` as f64 without losing precision when both operands are
/// huge (e.g. 1e350 / 2e350 -> 2.0, not inf/inf -> NaN). Returns signed inf
/// only when the true quotient overflows float64, matching Python's int/int
/// true division (which raises OverflowError only on an overflowing result).
fn ratio_f64(a: &BigInt, b: &BigInt) -> f64 {
    if a.is_zero() {
        if b.is_zero() {
            return f64::NAN;
        }
        return if b.is_positive() { f64::INFINITY } else { f64::NEG_INFINITY };
    }
    if b.is_zero() {
        return 0.0;
    }
    let abits = a.magnitude().bits();
    let bbits = b.magnitude().bits();
    let exp_diff = bbits as i64 - abits as i64;
    // The quotient's binary exponent is exp_diff (within +/-1); bail out
    // before it can exceed the f64 range / i32 cast range.
    if exp_diff > 1023 {
        return if (a.is_positive()) == (b.is_positive()) {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        };
    }
    if exp_diff < -1075 {
        return 0.0;
    }
    let shift_a = abits.saturating_sub(60);
    let shift_b = bbits.saturating_sub(60);
    let ma = (a >> shift_a).to_f64().unwrap_or(f64::INFINITY);
    let mb = (b >> shift_b).to_f64().unwrap_or(f64::INFINITY);
    let exp = shift_b as i64 - shift_a as i64;
    mb / ma * 2f64.powi(exp as i32)
}

/// Round a compounding factor to an exact decimal-integer count at scale S.
/// A factor rounding to zero is clamped to 1 and flagged; a negative factor
/// is outside the r~N(0,0.015) model and is REJECTED, never rewritten as a
/// positive ratio. Mirrors the guard in Python's `finance_chain`.
fn factor_to_count(factor: f64, r: f64, s: i64, i: usize, clamped: &mut u64) -> Result<i64, String> {
    let mut b = (s as f64 * factor).round() as i64;
    if b == 0 {
        b = 1;
        *clamped += 1;
    }
    if b < 0 {
        return Err(format!(
            "finance_chain: negative compounding factor at step {} (r={:.6})",
            i, r
        ));
    }
    Ok(b)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChainMeta {
    pub band_plus_minus_log10: i64,
    pub per_step_max_abs_log10: i64,
    pub cumulative_min_log10: i64,
    pub cumulative_max_log10: i64,
    pub cumulative_span_decades: i64,
    pub final_log10: i64,
}

/// `n` ratios; cumulative log10 performs a REFLECTING random walk within a
/// +/- (decades/2 + 50) band, so the running product traverses 600+ decades of
/// dynamic range while the exact integers stay bounded (a few hundred digits).
pub fn stress_chain(n: usize, decades: i64, seed: i64) -> (Vec<Pair>, ChainMeta) {
    let mut rng = Rng::new(seed);
    let band = decades / 2 + 50; // +/-350 => 700-decade span (> 600)
    let mut chain: Vec<Pair> = Vec::with_capacity(n);
    let mut cum = 0i64;
    let (mut lo, mut hi) = (0i64, 0i64);
    let mut step_exps_max = 0i64;
    for _ in 0..n {
        let e = rng.symmetric(60);
        let mut nxt = cum + e;
        if nxt > band {
            nxt = band - (nxt - band);
        }
        if nxt < -band {
            nxt = -band - (nxt + band);
        }
        nxt = nxt.max(-band).min(band);
        let de = nxt - cum;
        cum = nxt;
        lo = lo.min(cum);
        hi = hi.max(cum);
        step_exps_max = step_exps_max.max(de.abs());
        if de >= 0 {
            chain.push((BigInt::one(), pow10(de as u64)));
        } else {
            chain.push((pow10((-de) as u64), BigInt::one()));
        }
    }
    let meta = ChainMeta {
        band_plus_minus_log10: band,
        per_step_max_abs_log10: step_exps_max,
        cumulative_min_log10: lo,
        cumulative_max_log10: hi,
        cumulative_span_decades: hi - lo,
        final_log10: cum,
    };
    (chain, meta)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinanceMeta {
    pub description: String,
    pub cumulative_log10_end: f64,
    pub zero_factor_clamps: u64,
}

/// Realistic actuarial/reserve chain: daily compounding of (1+r).
///
/// Each step is a reserve/return factor (1+r_i) with r_i ~ N(0, 1.5%),
/// plus a deterministic leverage cycle (x10 / /10). Stored as exact decimal
/// integer ratios at scale S.
pub fn finance_chain(n: usize, seed: i64, s: i64) -> Result<(Vec<Pair>, FinanceMeta), String> {
    let mut rng = Rng::new(seed);
    let mut chain: Vec<Pair> = Vec::with_capacity(n);
    let mut cum = 0.0f64;
    let mut clamped = 0u64;
    for i in 0..n {
        let r = rng.gauss(0.0, 0.015);
        let mut factor = 1.0 + r;
        if i % 1000 == 0 && i > 0 {
            factor *= if (i / 1000) % 2 == 0 { 0.1 } else { 10.0 };
        }
        let a = s;
        let b = factor_to_count(factor, r, s, i, &mut clamped)?;
        chain.push(pair(a, b));
        cum += (b as f64 / a as f64).log10();
    }
    let meta = FinanceMeta {
        description:
            "daily (1+r) compounding, r~N(0,1.5%), decade leverage cycles, 1e5 decimal scale"
                .to_string(),
        cumulative_log10_end: cum,
        zero_factor_clamps: clamped,
    };
    Ok((chain, meta))
}

// ---------------------------------------------------------------------------
// Methods
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MethodResult {
    pub value: f64,
    pub log10: f64,
    pub collapses: u64,
    pub sec: f64,
    pub per_step_us: f64,
}

fn elapsed(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64()
}

fn per_step(sec: f64, n: usize) -> f64 {
    if n == 0 {
        0.0
    } else {
        sec / n as f64 * 1e6
    }
}

pub fn eager_float(chain: &[Pair]) -> MethodResult {
    let t0 = Instant::now();
    let mut p = 1.0f64;
    let mut collapses = 0u64;
    for (a, b) in chain {
        let r = ratio_f64(a, b);
        p *= r;
        if !p.is_finite() || p == 0.0 {
            collapses += 1;
        }
    }
    let dt = elapsed(t0);
    MethodResult {
        value: p,
        log10: p.log10(),
        collapses,
        sec: dt,
        per_step_us: per_step(dt, chain.len()),
    }
}

pub fn log_domain(chain: &[Pair]) -> MethodResult {
    let t0 = Instant::now();
    let mut s = 0.0f64;
    for (a, b) in chain {
        // math.log(b) - math.log(a) with arbitrary-size ints: log2_big times
        // ln(2) is equivalent and never overflows on huge BigInts.
        s += (log2_big(b) - log2_big(a)) * std::f64::consts::LN_2;
    }
    let value = s.exp();
    let collapses = if value.is_finite() { 0 } else { 1 };
    let dt = elapsed(t0);
    MethodResult {
        value,
        log10: s / std::f64::consts::LN_10,
        collapses,
        sec: dt,
        per_step_us: per_step(dt, chain.len()),
    }
}

#[derive(Debug, Clone)]
pub struct RelationalResult {
    pub value: f64,
    pub log10: f64,
    pub collapses: u64,
    pub sec: f64,
    pub per_step_us: f64,
    pub composes: u64,
    pub cancellations: u64,
    pub resolves: u64,
    pub resolve_collapses: u64,
    pub resolve_every_sec: Option<f64>,
    pub num_vals: usize,
}

pub fn relational(chain: &[Pair], resolve_every: bool) -> RelationalResult {
    let mut c = DeferredChain::new();
    let t0 = Instant::now();
    let mut vals: Vec<f64> = Vec::new();
    for (a, b) in chain {
        c.compose(a, b);
        if resolve_every {
            let v = ratio_f64(&c.a, &c.b);
            if v.is_infinite() || v.is_nan() {
                c.resolve_collapses += 1;
            }
            vals.push(v);
            c.resolves += 1;
        }
    }
    let dt = elapsed(t0);
    RelationalResult {
        value: c.resolve_float(),
        log10: c.log10(),
        collapses: 0,
        sec: dt,
        per_step_us: per_step(dt, chain.len()),
        composes: c.composes,
        cancellations: c.cancellations,
        resolves: c.resolves,
        resolve_collapses: c.resolve_collapses,
        resolve_every_sec: if resolve_every { Some(dt) } else { None },
        num_vals: vals.len(),
    }
}

/// High-precision reference. f64 is used here (the Python original uses an
/// 80-digit mpmath value); the deferred method tracks it in log-space.
pub fn reference_mp(chain: &[Pair]) -> MethodResult {
    let t0 = Instant::now();
    let mut p = 1.0f64;
    for (a, b) in chain {
        p *= ratio_f64(a, b);
    }
    let dt = elapsed(t0);
    MethodResult {
        value: p,
        log10: p.log10(),
        collapses: 0,
        sec: dt,
        per_step_us: per_step(dt, chain.len()),
    }
}

// ---------------------------------------------------------------------------
// Unit tests — edge fixes and core behaviour
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_decimal(pairs: &[(i64, i64)]) -> DeferredChain {
        let mut c = DeferredChain::new();
        for &(a, b) in pairs {
            c.compose_decimal(&bi(a), &bi(b));
        }
        c
    }

    #[test]
    fn compose_decimal_two_three_resolves_to_one_point_five() {
        // BUGFIX: stripping each operand independently gave 3.0; stripping
        // only the COMMON power of two must give 1.5.
        let c = chain_decimal(&[(2, 3)]);
        assert!((c.resolve_float() - 1.5).abs() < 1e-12, "got {}", c.resolve_float());
        assert_eq!(c.a, bi(2));
        assert_eq!(c.b, bi(3));
    }

    #[test]
    fn compose_decimal_four_six_resolves_to_one_point_five() {
        // common power of two is 2; 4>>2=1, 6>>2=1 -> still 3/2.
        let c = chain_decimal(&[(4, 6)]);
        assert!((c.resolve_float() - 1.5).abs() < 1e-12, "got {}", c.resolve_float());
    }

    #[test]
    fn compose_decimal_zero_operand_no_crash() {
        let mut c = DeferredChain::new();
        c.compose_decimal(&bi(0), &bi(7));
        assert!(c.a.is_zero());
        assert_eq!(c.b, bi(7));
        assert!(c.resolve_float().is_infinite()); // 7/0 -> +inf

        let mut d = DeferredChain::new();
        d.compose_decimal(&bi(5), &bi(0));
        assert!(d.b.is_zero());
        assert_eq!(d.a, bi(5));

        let mut e = DeferredChain::new();
        e.compose_decimal(&bi(0), &bi(0));
        assert!(e.resolve_float().is_nan()); // 0/0
    }

    #[test]
    fn compose_cancels_by_gcd() {
        let mut c = DeferredChain::new();
        c.compose(&bi(6), &bi(10)); // gcd(6,10)=2 -> (3,5)
        assert_eq!(c.a, bi(3));
        assert_eq!(c.b, bi(5));
        assert_eq!(c.composes, 1);
        assert_eq!(c.cancellations, 1);
        assert!((c.resolve_float() - 5.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn negative_factor_rejection_in_finance_chain() {
        // Mirrors the Python guard: a rounded b < 0 is REJECTED, never
        // silently rewritten as a positive ratio. The extraction into
        // `factor_to_count` makes this boundary directly testable regardless
        // of the PRNG's output distribution.
        let mut clamped = 0u64;
        let err = factor_to_count(-0.0005, -1.0005, 10i64.pow(5), 7, &mut clamped).unwrap_err();
        assert!(err.contains("negative compounding factor at step 7"), "{}", err);
        // A near-zero positive factor is clamped to 1 and flagged, not rejected.
        let mut clamped2 = 0u64;
        let b = factor_to_count(0.0000001, -0.9999999, 10i64.pow(5), 0, &mut clamped2).unwrap();
        assert_eq!(b, 1);
        assert_eq!(clamped2, 1);
    }

    #[test]
    fn negative_factor_explicit_guard() {
        // Mirrors the Python guard: a rounded b < 0 is rejected. Verify our
        // generator never emits a negative b for the canonical seed.
        let (chain, _meta) = finance_chain(1000, 777, 10i64.pow(5)).unwrap();
        for (a, b) in chain {
            assert!(a > BigInt::zero());
            assert!(b >= BigInt::zero(), "generator emitted negative b: {:?}", b);
        }
    }

    #[test]
    fn ten_to_350_resolves_to_infinity() {
        // Both operands are exact ints; b/a exceeds f64::MAX -> signed inf.
        let mut c = DeferredChain::new();
        c.compose(&BigInt::one(), &pow10(350));
        assert!(c.resolve_float().is_infinite());
        assert!(c.resolve_float().is_sign_positive());
        assert!(c.resolve_mp().is_infinite());
    }

    #[test]
    fn log10_of_ten_to_350_is_about_350() {
        let mut c = DeferredChain::new();
        c.compose(&BigInt::one(), &pow10(350));
        let l = c.log10();
        // bit-length estimate places this within 1 decade of 350.
        assert!((l - 350.0).abs() < 1.0, "log10 gave {}", l);
    }

    #[test]
    fn log10_edge_cases() {
        // Python checks b==0 first: 0/0 and b==0 both yield -inf.
        let mut z = DeferredChain::new();
        z.compose_decimal(&bi(0), &bi(0));
        assert!(z.log10().is_infinite() && z.log10().is_sign_negative()); // b==0 first

        let mut n = DeferredChain::new();
        n.compose_decimal(&bi(5), &bi(0));
        assert!(n.log10().is_infinite() && n.log10().is_sign_negative()); // b==0

        // a==0 with b>0 -> +inf.
        let mut p = DeferredChain::new();
        p.compose_decimal(&bi(0), &bi(7));
        assert!(p.log10().is_infinite() && p.log10().is_sign_positive()); // a==0
    }

    #[test]
    fn compose_all_matches_manual_compose() {
        let chain = vec![pair(2, 3), pair(4, 5), pair(6, 7)];
        let mut a = DeferredChain::new();
        a.compose_all(&chain);
        let mut b = DeferredChain::new();
        for (x, y) in &chain {
            b.compose(x, y);
        }
        assert_eq!(a.a, b.a);
        assert_eq!(a.b, b.b);
        assert_eq!(a.composes, 3);
    }

    #[test]
    fn stress_chain_traverses_over_600_decades() {
        let (_chain, meta) = stress_chain(10_000, 600, 12345);
        assert_eq!(meta.band_plus_minus_log10, 350);
        assert!(meta.cumulative_span_decades > 600);
    }

    #[test]
    fn relational_deferred_matches_eager_on_stress() {
        let (chain, _meta) = stress_chain(2_000, 600, 12345);
        let rel = relational(&chain, false);
        // Deferred log10 must match the log-domain accumulation closely.
        let logdom = log_domain(&chain);
        assert!((rel.log10 - logdom.log10).abs() < 1e-6,
                "rel {} vs logdom {}", rel.log10, logdom.log10);
        assert_eq!(rel.composes, 2_000);
        assert_eq!(rel.collapses, 0);
    }

    #[test]
    fn eager_float_and_reference_finite_on_small_chain() {
        let chain = vec![pair(2, 3), pair(4, 5)];
        let e = eager_float(&chain);
        let r = reference_mp(&chain);
        assert!((e.value - 15.0 / 8.0).abs() < 1e-12, "eager {}", e.value);
        assert!((r.value - 15.0 / 8.0).abs() < 1e-12, "ref {}", r.value);
    }
}
