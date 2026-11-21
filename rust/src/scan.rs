//! KOPPA unresolved-ratio pair aggregation, ported from `koppa/scan.py`.
//!
//! A pair is `(am, ae, bm, be)`: numerator `am * 2^ae` and denominator
//! `bm * 2^be`, with mantissas `am, bm` in `[0.5, 1)` and exponents `i32`.
//! The represented ratio is `(bm / am) * 2^(be - ae)`.
//!
//! Keeping the factor as an unresolved pair (rather than resolving it to a
//! float early) keeps intermediate states finite across hundreds of decades;
//! resolution happens once, on demand, at the end.

use num_bigint::{BigInt, Sign};
use num_traits::ToPrimitive;

/// An unresolved multiplicative ratio factor `(am, ae, bm, be)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    pub am: f64,
    pub ae: i32,
    pub bm: f64,
    pub be: i32,
}

impl Pair {
    #[inline]
    pub fn new(am: f64, ae: i32, bm: f64, be: i32) -> Self {
        Pair { am, ae, bm, be }
    }
}

/// The multiplicative identity: `0.5*2^1 / 0.5*2^1 == 1`.
pub const IDENTITY: Pair = Pair {
    am: 0.5,
    ae: 1,
    bm: 0.5,
    be: 1,
};

/// Error raised when integer inputs cannot form a valid pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanError {
    /// `a == 0` or `b == 0` (would divide by zero at resolve time).
    ZeroInput,
    /// Integer magnitude exceeds the exact-convertible float64 range.
    OutOfRange,
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::ZeroInput => write!(f, "pair_from_ints requires nonzero a and b"),
            ScanError::OutOfRange => write!(
                f,
                "pair_from_ints inputs exceed the float64 range (cannot convert exactly)"
            ),
        }
    }
}

impl std::error::Error for ScanError {}

/// `math.frexp(x)` -> `(m, e)` with `m` in `[0.5, 1)` and `x == m * 2^e`.
///
/// Implemented with exact power-of-two bit manipulation (no rounding), valid
/// for positive finite `x`, including subnormals.
fn frexp(x: f64) -> (f64, i32) {
    debug_assert!(x.is_finite() && x > 0.0, "frexp expects positive finite x");
    let bits = x.to_bits();
    let exp_bits = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & 0x000f_ffff_ffff_ffff;

    if exp_bits == 0 {
        // Subnormal: shift the leading mantissa bit up to bit 52.
        let mut m_bits = frac;
        let mut e = -1021i32;
        while (m_bits & (1u64 << 52)) == 0 {
            m_bits <<= 1;
            e -= 1;
        }
        m_bits &= !(1u64 << 52); // leading 1 becomes the implicit bit
        return (f64::from_bits((1022u64 << 52) | m_bits), e);
    }

    // Normal: replace the biased exponent with 1022 so the value lies in
    // [0.5, 1); x = 1.f * 2^(exp_bits-1023) = m * 2^(exp_bits-1022).
    let m = f64::from_bits((1022u64 << 52) | frac);
    (m, exp_bits - 1022)
}

/// Scale `x` by a power of two, `x * 2^exp`, with graceful overflow/underflow.
///
/// Scaling in chunks keeps `2^exp` itself from overflowing when `exp` is
/// slightly above the largest float exponent but `x < 1` keeps the true
/// result finite (matching `math.ldexp`).
fn ldexp(x: f64, exp: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let mut r = x;
    let mut e = exp;
    while e > 1000 {
        r *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        r *= 2f64.powi(-1000);
        e += 1000;
    }
    r * 2f64.powi(e)
}

/// Exact pair for ratio `2^k`: numerator `1`, denominator `2^k`.
///
/// `1 = 0.5*2^1`, `2^k = 0.5*2^(k+1)`, so the ratio is `2^k`.
pub fn pair_from_ratio_pow2(k: i32) -> Pair {
    Pair::new(0.5, 1, 0.5, k + 1)
}

/// Exact pair for the ratio `b / a` from positive integers.
///
/// Values too large to convert to float64 (e.g. `10^400`) are rejected with
/// `ScanError::OutOfRange`, the ValueError equivalent of the Python source.
/// Zero (or negative) inputs are rejected with `ScanError::ZeroInput`.
pub fn pair_from_ints<A: Into<BigInt>, B: Into<BigInt>>(a: A, b: B) -> Result<Pair, ScanError> {
    let a: BigInt = a.into();
    let b: BigInt = b.into();
    if a.sign() == Sign::NoSign || b.sign() == Sign::NoSign || a.sign() == Sign::Minus
        || b.sign() == Sign::Minus
    {
        return Err(ScanError::ZeroInput);
    }
    if a.bits() > 1024 || b.bits() > 1024 {
        return Err(ScanError::OutOfRange);
    }
    let af = a.to_f64().ok_or(ScanError::OutOfRange)?;
    let bf = b.to_f64().ok_or(ScanError::OutOfRange)?;
    let (am, ae) = frexp(af);
    let (bm, be) = frexp(bf);
    Ok(Pair::new(am, ae, bm, be))
}

/// Associative multiplication of two unresolved ratio pairs.
pub fn compose(p: Pair, q: Pair) -> Pair {
    let (cm, ce) = frexp(p.am * q.am);
    let (dm, de) = frexp(p.bm * q.bm);
    Pair::new(cm, p.ae + q.ae + ce, dm, p.be + q.be + de)
}

/// On-demand resolution of a pair to its ratio (a single float).
pub fn resolve(p: Pair) -> f64 {
    let exp = (p.be as i64 - p.ae as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    ldexp(p.bm / p.am, exp)
}

/// Hillis-Steele inclusive prefix scan. Reads `src`, writes a fresh `dst`
/// each pass (double buffering), so no pass observes a neighbour it just wrote.
pub fn scan_double_buffer(seq: &[Pair]) -> Vec<Pair> {
    let n = seq.len();
    let mut src = seq.to_vec();
    let mut offset = 1usize;
    while offset < n {
        let mut dst = src.clone(); // fresh copy: both reads come from src
        for i in offset..n {
            dst[i] = compose(src[i - offset], src[i]);
        }
        src = dst;
        offset <<= 1;
    }
    src
}

/// BUG DEMO: Hillis-Steele with upward in-place updates (no double buffer).
///
/// Within a pass of `offset >= 2`, `src[i - offset]` has already been
/// overwritten this pass, so `compose` reads a contaminated neighbour and the
/// prefix products are wrong. This is the failure double buffering prevents.
pub fn scan_in_place(seq: &[Pair]) -> Vec<Pair> {
    let n = seq.len();
    let mut src = seq.to_vec();
    let mut offset = 1usize;
    while offset < n {
        for i in offset..n {
            // upward: reads neighbours updated earlier in this same pass
            src[i] = compose(src[i - offset], src[i]);
        }
        offset <<= 1;
    }
    src
}

/// Serial baseline: a left fold with the same associative `compose`.
pub fn fold_serial(seq: &[Pair]) -> Vec<Pair> {
    let mut acc = IDENTITY;
    let mut out = Vec::with_capacity(seq.len());
    for &p in seq {
        acc = compose(acc, p);
        out.push(acc);
    }
    out
}

/// Relative error `|x-y|/|y|` with sane handling of zero / non-finite values.
#[cfg(test)]
fn rel_err(x: f64, y: f64) -> f64 {
    if x == y {
        return 0.0;
    }
    if !y.is_finite() {
        return if !x.is_finite() { 0.0 } else { f64::INFINITY };
    }
    if !x.is_finite() {
        return f64::INFINITY;
    }
    let den = if y != 0.0 { y.abs() } else { 1.0 };
    (x - y).abs() / den
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity() {
        let p = pair_from_ratio_pow2(17);
        assert_eq!(compose(IDENTITY, p), p);
        assert_eq!(compose(p, IDENTITY), p);

        let q = pair_from_ints(1_000_000_000_003u64, 999_999_999_991u64).unwrap();
        assert_eq!(compose(IDENTITY, q), q);
        assert_eq!(compose(q, IDENTITY), q);
    }

    #[test]
    fn compose_associativity() {
        let mut seq: Vec<Pair> = Vec::new();
        for a in 1u64..5 {
            for b in 1u64..5 {
                seq.push(pair_from_ints(a, b).unwrap());
            }
        }
        seq.push(pair_from_ratio_pow2(1100));
        seq.push(pair_from_ratio_pow2(-1100));
        seq.push(pair_from_ratio_pow2(3));

        let mut checked = 0;
        for &p in &seq {
            for &q in &seq {
                for &r in &seq {
                    let left = compose(compose(p, q), r);
                    let right = compose(p, compose(q, r));
                    assert!(
                        rel_err(resolve(left), resolve(right)) <= 1e-12,
                        "associativity failed for {:?}, {:?}, {:?}",
                        p,
                        q,
                        r
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn double_buffer_matches_fold() {
        // Exact powers of two: composition is exact, so scan and serial fold
        // must agree bit-for-bit, including astronomically large exponents.
        let ks: [i32; 10] = [1100, -1100, 1100, -1100, 50, -50, 3, 1, -7, 9];
        let seq: Vec<Pair> = ks.iter().map(|&k| pair_from_ratio_pow2(k)).collect();
        let scan = scan_double_buffer(&seq);
        let fold = fold_serial(&seq);
        assert_eq!(scan.len(), fold.len());
        for i in 0..scan.len() {
            assert_eq!(scan[i], fold[i], "exact pair mismatch at prefix {}", i);
            assert_eq!(resolve(scan[i]), resolve(fold[i]));
        }

        // General near-unity integer ratios: agreement within double precision.
        let ints: Vec<(u64, u64)> = (0..64)
            .map(|i| {
                let a = 1_000_000_000_000u64 + (i % 7) * 1_000_003;
                let b = a + 1 + (i % 3);
                (a, b)
            })
            .collect();
        let seq2: Vec<Pair> = ints
            .iter()
            .map(|&(a, b)| pair_from_ints(a, b).unwrap())
            .collect();
        let scan2 = scan_double_buffer(&seq2);
        let fold2 = fold_serial(&seq2);
        for i in 0..scan2.len() {
            assert!(
                rel_err(resolve(scan2[i]), resolve(fold2[i])) <= 1e-12,
                "scan/fold disagree at prefix {}",
                i
            );
        }
    }

    #[test]
    fn huge_int_rejected() {
        let huge = BigInt::from(10u32).pow(400); // 10^400, beyond float64
        assert_eq!(
            pair_from_ints(huge.clone(), 1u32),
            Err(ScanError::OutOfRange)
        );
        assert_eq!(pair_from_ints(1u32, huge), Err(ScanError::OutOfRange));
        // A comparable-magnitude input that does fit must still work.
        assert!(pair_from_ints(1_000_000_000_000u64, 999_999_999_999u64).is_ok());
        // Zero is also rejected (ValueError equivalent).
        assert_eq!(pair_from_ints(0u32, 1u32), Err(ScanError::ZeroInput));
    }

    #[test]
    fn in_place_differs_from_double_buffer() {
        // Deliberate bug documentation: the upward in-place pass contaminates
        // already-updated neighbours once offset >= 2.
        let ks: [i32; 4] = [1, 2, 3, 4];
        let seq: Vec<Pair> = ks.iter().map(|&k| pair_from_ratio_pow2(k)).collect();
        let correct = scan_double_buffer(&seq);
        let buggy = scan_in_place(&seq);

        let correct_vals: Vec<f64> = correct.iter().map(|&p| resolve(p)).collect();
        let buggy_vals: Vec<f64> = buggy.iter().map(|&p| resolve(p)).collect();
        // Correct inclusive prefixes of [2, 4, 8, 16] are [2, 8, 64, 1024].
        assert_eq!(correct_vals, vec![2.0, 8.0, 64.0, 1024.0]);
        // In-place upward computes [2, 8, 128, 8192] -- wrong from index 2 on.
        assert_eq!(buggy_vals[0], 2.0);
        assert_eq!(buggy_vals[1], 8.0);
        assert_ne!(buggy_vals[2], correct_vals[2], "in-place bug must show at i=2");
        assert_ne!(buggy_vals[3], correct_vals[3], "in-place bug must show at i=3");
    }
}
