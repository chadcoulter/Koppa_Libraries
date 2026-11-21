//! Robust ratio predicates — Rust port of `koppa/predicates.py`.
//!
//! Core state is the unresolved pair `(a, b)`; the resolved form `r = b / a`
//! is computed on demand (lazy gcd reduction).  Comparison uses adaptive
//! escalation:
//!
//!   T1  float filter with a rigorous absolute-rounding-error bound,
//!   T2  i128 fast path (checked),
//!   T3  exact `BigInt` fallback.
//!
//! `compare` returns the sign of `b1*a2 - b2*a1` after normalising each
//! denominator to positive, matching the Python implementation exactly.

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

// ---------------------------------------------------------------------------
// statistics (mirror the Python module-level counters)
// ---------------------------------------------------------------------------

pub static T1_CERTIFIED: AtomicU64 = AtomicU64::new(0);
pub static T1_ESCALATED: AtomicU64 = AtomicU64::new(0);
pub static T2_USED: AtomicU64 = AtomicU64::new(0);
pub static T3_USED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompareStats {
    pub t1_certified: u64,
    pub t1_escalated: u64,
    pub t2_used: u64,
    pub t3_used: u64,
}

pub fn stats() -> CompareStats {
    CompareStats {
        t1_certified: T1_CERTIFIED.load(AtomicOrdering::Relaxed),
        t1_escalated: T1_ESCALATED.load(AtomicOrdering::Relaxed),
        t2_used: T2_USED.load(AtomicOrdering::Relaxed),
        t3_used: T3_USED.load(AtomicOrdering::Relaxed),
    }
}

pub fn reset_stats() {
    T1_CERTIFIED.store(0, AtomicOrdering::Relaxed);
    T1_ESCALATED.store(0, AtomicOrdering::Relaxed);
    T2_USED.store(0, AtomicOrdering::Relaxed);
    T3_USED.store(0, AtomicOrdering::Relaxed);
}

// ---------------------------------------------------------------------------
// core state: unresolved pair (a, b)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ratio {
    /// Denominator of `r = b / a`.  Must be nonzero.
    pub a: BigInt,
    /// Numerator of `r = b / a`.
    pub b: BigInt,
}

impl Ratio {
    pub fn new(a: BigInt, b: BigInt) -> Self {
        if a.is_zero() {
            panic!("ratio denominator a must be nonzero");
        }
        Ratio { a, b }
    }

    /// On-demand resolution to the canonical reduced form (lazy).
    pub fn resolve(&self) -> BigRational {
        BigRational::new(self.b.clone(), self.a.clone())
    }

    pub fn r(&self) -> BigRational {
        self.resolve()
    }

    /// Multiplicative / associative composition of resolved forms.
    pub fn compose(&self, other: &Ratio) -> Ratio {
        Ratio::new(&self.a * &other.a, &self.b * &other.b)
    }

    /// `r' = (b'a - b a') / a^2` -> state `(a^2, b'a - b a')`.
    pub fn quotient_derivative(&self, da: &BigInt, db: &BigInt) -> Ratio {
        Ratio::new(&self.a * &self.a, db * &self.a - &self.b * da)
    }

    /// Scale invariant `g = r'/r = b'/b - a'/a` (on-demand resolution).
    pub fn log_derivative(&self, da: &BigInt, db: &BigInt) -> BigRational {
        BigRational::new(db.clone(), self.b.clone()) - BigRational::new(da.clone(), self.a.clone())
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ratio(a={}, b={})", self.a, self.b)
    }
}

/// Minimal `Fraction`-like reduced rational: `num / den`, `den > 0`.
#[derive(Clone, Debug, Eq)]
pub struct BigRational {
    pub num: BigInt,
    pub den: BigInt,
}

impl BigRational {
    pub fn new(num: BigInt, den: BigInt) -> Self {
        assert!(!den.is_zero(), "fraction denominator must be nonzero");
        let mut n = num;
        let mut d = den;
        if d.is_negative() {
            n = -n;
            d = -d;
        }
        let g = n.gcd(&d);
        if !g.is_zero() && !g.is_one() {
            n /= &g;
            d /= &g;
        }
        BigRational { num: n, den: d }
    }
}

impl PartialEq for BigRational {
    fn eq(&self, other: &Self) -> bool {
        &self.num * &other.den == &other.num * &self.den
    }
}

impl Ord for BigRational {
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.num * &other.den).cmp(&(&other.num * &self.den))
    }
}

impl PartialOrd for BigRational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::ops::Sub for BigRational {
    type Output = BigRational;
    fn sub(self, rhs: BigRational) -> BigRational {
        BigRational::new(
            &self.num * &rhs.den - &rhs.num * &self.den,
            &self.den * &rhs.den,
        )
    }
}

impl fmt::Display for BigRational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den.is_one() {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Correctly-rounded rational -> f64, matching Python `float(Fraction)`.
/// Assumes `d > 0` and the value is a normal, finite number.
#[cfg(test)]
fn frac_to_f64(n: &BigInt, d: &BigInt) -> f64 {
    let sign = if n.is_negative() { -1.0 } else { 1.0 };
    let na = n.abs();
    // e = floor(log2(na/d)); start from the bit-length estimate and adjust.
    let mut e = na.bits() as i64 - d.bits() as i64;
    let cmp_shift = |ee: i64| -> Ordering {
        let lhs = if ee >= 0 { na.clone() } else { &na << ((-ee) as u64) };
        let rhs = if ee >= 0 { d << (ee as u64) } else { d.clone() };
        lhs.cmp(&rhs)
    };
    if cmp_shift(e) == Ordering::Less {
        e -= 1;
    }
    while cmp_shift(e + 1) != Ordering::Less {
        e += 1;
    }
    // q = round(na/d * 2^(52-e)) lies in [2^52, 2^53).
    let shift = 52 - e;
    let (ns, ds) = if shift >= 0 {
        (&na << (shift as u64), d.clone())
    } else {
        (na.clone(), d << ((-shift) as u64))
    };
    let mut q = &ns / &ds;
    let rem = &ns % &ds;
    match (&rem * BigInt::from(2u8)).cmp(&ds) {
        Ordering::Greater => q += 1,
        Ordering::Equal => {
            if !(&q & BigInt::one()).is_zero() {
                q += 1;
            }
        }
        Ordering::Less => {}
    }
    let two53 = BigInt::one() << 53u64;
    let (q, e) = if q >= two53 { (q >> 1u64, e + 1) } else { (q, e) };
    let m = q.to_u64().unwrap();
    let biased = (e - 52 + 1023) as u64;
    debug_assert!((1..=2046).contains(&biased), "value must be normal");
    sign * f64::from_bits((biased << 52) | (m & 0x000f_ffff_ffff_ffff))
}

/// `Q_m + Q_n = Q_{m+n}` in state terms (Python `harmonic_add` sums a and b).
pub fn harmonic_add(qm: &Ratio, qn: &Ratio) -> Ratio {
    Ratio::new(&qm.a + &qn.a, &qm.b + &qn.b)
}

// ---------------------------------------------------------------------------
// adaptive sign filter: sign(b1*a2 - b2*a1)
// ---------------------------------------------------------------------------

fn bit_len(n: &BigInt) -> u64 {
    n.bits()
}

fn max_bits(vals: &[&BigInt]) -> u64 {
    vals.iter().map(|v| bit_len(v)).max().unwrap_or(0)
}

fn sign_big(n: &BigInt) -> i32 {
    if n.is_zero() {
        0
    } else if n.is_positive() {
        1
    } else {
        -1
    }
}

/// Float determinant and a rigorous absolute-rounding-error bound.
///
/// Mirrors Python `_det_float`.  `bound` uses `BigInt` because at E near 1000
/// the shift counts exceed machine integer width.
fn det_float(a1: &BigInt, b1: &BigInt, a2: &BigInt, b2: &BigInt) -> (f64, BigInt) {
    let e = max_bits(&[a1, b1, a2, b2]) as usize;
    if e > 1000 {
        // beyond IEEE-754 double range -> skip T1 (forces escalation)
        return (0.0, BigInt::one());
    }
    let conv = if e > 53 {
        BigInt::one() << (e - 53)
    } else {
        BigInt::zero()
    };
    let a1f = a1.to_f64().unwrap_or(0.0);
    let b1f = b1.to_f64().unwrap_or(0.0);
    let a2f = a2.to_f64().unwrap_or(0.0);
    let b2f = b2.to_f64().unwrap_or(0.0);
    let d1 = b1f * a2f;
    let d2 = b2f * a1f;
    let det_f = d1 - d2;

    let mut prod_err = if 2 * e >= 53 {
        BigInt::one() << (2 * e - 53)
    } else {
        BigInt::zero()
    };
    let shift_e = BigInt::one() << e;
    prod_err += &shift_e * &conv * BigInt::from(2u8);
    prod_err += &conv * &conv;
    // Guard uses 2E+1 >= 53 but shifts by 2E-52: at E==26 the shift is 0,
    // never negative (the original Python bug that crashed at E==26).
    let sub_err = if 2 * e + 1 >= 53 {
        BigInt::one() << (2 * e - 52)
    } else {
        BigInt::zero()
    };
    let bound = prod_err * BigInt::from(2u8) + sub_err + BigInt::one();
    (det_f, bound)
}

/// Exact `|det_f| > bound` comparison.  Python's int-vs-float comparison is
/// exact, so we decompose the f64 into (significand, exponent) and compare to
/// the integer bound without lossy rounding.
fn certify(det_f: f64, bound: &BigInt) -> bool {
    if det_f.is_nan() {
        return false;
    }
    if det_f.is_infinite() {
        // abs(inf) > any finite bound
        return true;
    }
    let x = det_f.abs();
    if x == 0.0 {
        return false; // bound >= 1
    }
    let bits = x.to_bits();
    let exp_bits = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & 0x000f_ffff_ffff_ffff;
    let (m, e) = if exp_bits == 0 {
        (frac, -1074i32)
    } else {
        (frac | (1u64 << 52), exp_bits - 1023 - 52)
    };
    let lhs = BigInt::from(m);
    if e >= 0 {
        (lhs << (e as usize)) > *bound
    } else {
        lhs > (bound << ((-e) as usize))
    }
}

/// Sign of `r1 - r2`: sign of `b1*a2 - b2*a1`, exact and never wrong.
pub fn compare(r1: &Ratio, r2: &Ratio) -> i32 {
    let (mut a1, mut b1) = (r1.a.clone(), r1.b.clone());
    let (mut a2, mut b2) = (r2.a.clone(), r2.b.clone());

    // Normalize each denominator to positive by flipping its own numerator.
    if a1.is_negative() {
        a1 = -a1;
        b1 = -b1;
    }
    if a2.is_negative() {
        a2 = -a2;
        b2 = -b2;
    }

    let (det_f, bound) = det_float(&a1, &b1, &a2, &b2);
    if certify(det_f, &bound) {
        T1_CERTIFIED.fetch_add(1, AtomicOrdering::Relaxed);
        return if det_f > 0.0 { 1 } else { -1 };
    }
    T1_ESCALATED.fetch_add(1, AtomicOrdering::Relaxed);

    let mb = max_bits(&[&a1, &b1, &a2, &b2]);
    if mb <= 127 {
        T2_USED.fetch_add(1, AtomicOrdering::Relaxed);
        // i128 fast path with checked arithmetic; if an intermediate would
        // overflow, fall back to exact BigInt within the same tier.
        if let (Some(ai1), Some(bi1), Some(ai2), Some(bi2)) =
            (a1.to_i128(), b1.to_i128(), a2.to_i128(), b2.to_i128())
        {
            if let (Some(p1), Some(p2)) =
                (bi1.checked_mul(ai2), bi2.checked_mul(ai1))
            {
                if let Some(det) = p1.checked_sub(p2) {
                    return if det > 0 { 1 } else if det < 0 { -1 } else { 0 };
                }
            }
        }
        let det = &b1 * &a2 - &b2 * &a1;
        sign_big(&det)
    } else {
        T3_USED.fetch_add(1, AtomicOrdering::Relaxed);
        let det = &b1 * &a2 - &b2 * &a1;
        sign_big(&det)
    }
}

/// Unsafe baseline: compare `float(b)/float(a)` directly.
pub fn naive_float_compare(r1: &Ratio, r2: &Ratio) -> i32 {
    // Python's float(int) raises OverflowError for bit_length > 1024.
    if bit_len(&r1.a) > 1024
        || bit_len(&r1.b) > 1024
        || bit_len(&r2.a) > 1024
        || bit_len(&r2.b) > 1024
    {
        return 0;
    }
    let x1 = r1.b.to_f64().unwrap_or(0.0) / r1.a.to_f64().unwrap_or(0.0);
    let x2 = r2.b.to_f64().unwrap_or(0.0) / r2.a.to_f64().unwrap_or(0.0);
    if x1 > x2 {
        1
    } else if x1 < x2 {
        -1
    } else {
        0
    }
}

/// `Fraction` oracle used only for validation, never in the hot path.
pub fn oracle_compare(r1: &Ratio, r2: &Ratio) -> i32 {
    let x1 = r1.resolve();
    let x2 = r2.resolve();
    match x1.cmp(&x2) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

// ---------------------------------------------------------------------------
// geometry / engineering predicates
// ---------------------------------------------------------------------------

/// Sign of `cross((q-p),(r-p))`; `>0` ccw, `<0` cw, `0` collinear.
pub fn orientation<N: Into<BigInt>>(p: (N, N), q: (N, N), r: (N, N)) -> i32 {
    let (px, py) = (p.0.into(), p.1.into());
    let (qx, qy) = (q.0.into(), q.1.into());
    let (rx, ry) = (r.0.into(), r.1.into());
    compare(
        &Ratio::new(&qx - &px, &qy - &py),
        &Ratio::new(&rx - &px, &ry - &py),
    )
}

/// GIS: compare two terrain slopes `rise/run` exactly.
pub fn compare_gradients<N: Into<BigInt>>(rise1: N, run1: N, rise2: N, run2: N) -> i32 {
    compare(
        &Ratio::new(run1.into(), rise1.into()),
        &Ratio::new(run2.into(), rise2.into()),
    )
}

/// CAD/CNC: `|measured/nominal - 1| <= tol`, using exact predicates.
pub fn cnc_within_tolerance(measured: &Ratio, nominal: &Ratio, tol: &Ratio) -> bool {
    let m_over_n = Ratio::new(&measured.a * &nominal.b, &measured.b * &nominal.a);
    let up = Ratio::new(tol.a.clone(), &tol.a + &tol.b); // 1 + tol = (a+b)/a
    let lo = Ratio::new(tol.a.clone(), &tol.a - &tol.b); // 1 - tol = (a-b)/a
    compare(&m_over_n, &lo) >= 0 && compare(&m_over_n, &up) <= 0
}

/// Structural engineering: classify `utilization = load/capacity`.
pub fn load_ratio_class(load: &Ratio, capacity: &Ratio, thresholds: &[(&str, Ratio)]) -> String {
    let util = Ratio::new(&load.a * &capacity.b, &load.b * &capacity.a);
    for (label, thr) in thresholds {
        if compare(&util, &thr) <= 0 {
            return (*label).to_string();
        }
    }
    "OVER".to_string()
}

/// Electrical: exact impedance/admittance magnitude matching.
pub fn impedance_match(z1: &Ratio, z2: &Ratio) -> bool {
    compare(z1, z2) == 0
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn bi(s: &str) -> BigInt {
        BigInt::from_str(s).unwrap()
    }

    fn witness() -> (Ratio, Ratio) {
        (
            Ratio::new(bi("7952614671698527"), bi("21159794283675467")),
            Ratio::new(bi("5364900327947194"), bi("14274574083889309")),
        )
    }

    #[test]
    fn witness_equal_float_but_exact_minus_one() {
        let (r1, r2) = witness();
        // The properly-reduced float forms are equal (Python demo's
        // float(r1.resolve()) == float(r2.resolve())), yet the exact sign is -1.
        let x1 = r1.resolve();
        let x2 = r2.resolve();
        let fr1 = frac_to_f64(&x1.num, &x1.den);
        let fr2 = frac_to_f64(&x2.num, &x2.den);
        assert_eq!(fr1, fr2, "resolved float forms must be equal");
        assert_eq!(oracle_compare(&r1, &r2), -1, "oracle sign must be -1");
        assert_eq!(compare(&r1, &r2), -1, "exact predicate sign must be -1");
        let cross = &r1.b * &r2.a - &r2.b * &r1.a;
        assert!(cross.is_negative(), "b1*a2 - b2*a1 must be negative");
    }

    #[test]
    fn bit_length_26_does_not_crash() {
        // 2^25 has bit_length 26, 2^26 - 3 has bit_length 26.
        let a1 = BigInt::from(1u8) << 25;
        let b1 = (BigInt::from(1u8) << 26) - BigInt::from(3u8);
        let a2 = (BigInt::from(1u8) << 25) + BigInt::from(7u8);
        let b2 = (BigInt::from(1u8) << 26) - BigInt::from(11u8);
        let r1 = Ratio::new(a1, b1);
        let r2 = Ratio::new(a2, b2);
        let (det_f, bound) = det_float(&r1.a, &r1.b, &r2.a, &r2.b);
        assert!(bound > BigInt::zero(), "bound must be positive at E=26");
        let _ = det_f;
        let got = compare(&r1, &r2);
        assert_eq!(got, oracle_compare(&r1, &r2), "E=26 comparison must be exact");
    }

    #[test]
    fn resolve_reduces_to_canonical_form() {
        let r = Ratio::new(bi("12"), bi("18")); // 18/12 = 3/2
        let x = r.resolve();
        assert_eq!(x.num, bi("3"));
        assert_eq!(x.den, bi("2"));
        let neg = Ratio::new(bi("-12"), bi("18")); // 18/-12 = -3/2
        let y = neg.resolve();
        assert_eq!(y.num, bi("-3"));
        assert_eq!(y.den, bi("2"));
    }

    #[test]
    fn compose_is_associative() {
        let r1 = Ratio::new(bi("2"), bi("3"));
        let r2 = Ratio::new(bi("5"), bi("7"));
        let r3 = Ratio::new(bi("11"), bi("13"));
        let left = r1.compose(&r2).compose(&r3);
        let right = r1.compose(&r2.compose(&r3));
        assert_eq!(left.a, bi("110"));
        assert_eq!(left.b, bi("273"));
        assert_eq!(left, right);
    }

    #[test]
    fn quotient_and_log_derivative() {
        let r = Ratio::new(bi("4"), bi("9")); // 9/4
        let da = bi("2");
        let db = bi("3");
        let q = r.quotient_derivative(&da, &db);
        // (3*4 - 9*2)/16 = (12-18)/16 = -6/16
        assert_eq!(q.a, bi("16"));
        assert_eq!(q.b, bi("-6"));
        let g = r.log_derivative(&da, &db);
        // 3/9 - 2/4 = 1/3 - 1/2 = -1/6
        assert_eq!(g, BigRational::new(bi("-1"), bi("6")));
    }

    #[test]
    fn harmonic_addition() {
        let a = Ratio::new(bi("2"), bi("3"));
        let b = Ratio::new(bi("4"), bi("5"));
        let h = harmonic_add(&a, &b);
        assert_eq!(h.a, bi("6"));
        assert_eq!(h.b, bi("8"));
    }

    #[test]
    fn orientation_and_gradients() {
        let p = (0i64, 0i64);
        let q = (7952614671698527i64, 21159794283675467i64);
        let r = (5364900327947194i64, 14274574083889309i64);
        assert_eq!(orientation(p, q, r), -1);
        assert_eq!(
            compare_gradients(
                21159794283675467i64,
                7952614671698527i64,
                14274574083889309i64,
                5364900327947194i64,
            ),
            -1
        );
    }

    #[test]
    fn engineering_predicates() {
        let measured = Ratio::new(bi("10000"), bi("10001"));
        let nominal = Ratio::new(bi("10000"), bi("10000"));
        let tol = Ratio::new(bi("100000"), bi("101"));
        assert!(cnc_within_tolerance(&measured, &nominal, &tol));

        let load = Ratio::new(bi("3"), bi("4"));
        let cap = Ratio::new(bi("5"), bi("4"));
        let thresholds = [
            ("SAFE", Ratio::new(bi("10"), bi("10"))),
            ("WATCH", Ratio::new(bi("12"), bi("10"))),
            ("DANGER", Ratio::new(bi("15"), bi("10"))),
        ];
        assert_eq!(load_ratio_class(&load, &cap, &thresholds), "OVER");
        assert!(impedance_match(
            &Ratio::new(bi("50"), bi("3")),
            &Ratio::new(bi("100"), bi("6"))
        ));
    }

    #[test]
    fn negative_denominators() {
        let cases = [((-1i64, 3i64), (2i64, 5i64)), ((7, -22), (1, 3)), ((-9, -4), (5, 2))];
        for ((a, b), (c, d)) in cases {
            for k in 0..2 {
                let bb = if k == 0 { b } else { -b };
                let r1 = Ratio::new(bi(&a.to_string()), bi(&bb.to_string()));
                let r2 = Ratio::new(bi(&c.to_string()), bi(&d.to_string()));
                assert_eq!(compare(&r1, &r2), oracle_compare(&r1, &r2));
            }
        }
    }

    #[test]
    fn huge_values_exact_600_decades() {
        let mut s = String::from("1");
        for _ in 0..600 {
            s.push('0');
        }
        let scale = bi(&s);
        let (r1, r2) = witness();
        let big1 = Ratio::new(&r1.a * &scale, &r1.b * &scale);
        let big2 = Ratio::new(&r2.a * &scale, &r2.b * &scale);
        assert_eq!(compare(&big1, &big2), -1);
        assert_eq!(oracle_compare(&big1, &big2), -1);
        assert!(big1.resolve() == r1.resolve());
    }

    #[test]
    fn differential_vs_oracle() {
        let mut state: u64 = 123456789;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state
        };
        for _ in 0..2000 {
            let a1 = (((next() % 2000) as i64) - 1000) | 1;
            let b1 = (((next() % 2000) as i64) - 1000) | 1;
            let a2 = (((next() % 2000) as i64) - 1000) | 1;
            let b2 = (((next() % 2000) as i64) - 1000) | 1;
            let r1 = Ratio::new(bi(&a1.to_string()), bi(&b1.to_string()));
            let r2 = Ratio::new(bi(&a2.to_string()), bi(&b2.to_string()));
            assert_eq!(
                compare(&r1, &r2),
                oracle_compare(&r1, &r2),
                "a1={} b1={} a2={} b2={}",
                a1,
                b1,
                a2,
                b2
            );
        }
    }
}
