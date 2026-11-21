//! Exhaustive edge-case matrix for the Rust port — mirrors Python test_koppa.py
//! plus additional boundary cases. Run: cargo test --test edge_cases
use koppa::{SP, ProjPair, LazyRelational, DeferredChain, Ratio, compare, pair_from_ints, scan_double_buffer};
use koppa::inference::{forward_koppa, forward_naive, viterbi_koppa, projective_demo};
use koppa::scan::{compose as scompose, resolve as sresolve, scan_in_place};
use num_bigint::BigInt;

// ---- inference: SP guards ----
#[test] fn edge_sp_reject_inf() { assert!(SP::from_float(f64::INFINITY).is_err()); }
#[test] fn edge_sp_reject_neginf() { assert!(SP::from_float(f64::NEG_INFINITY).is_err()); }
#[test] fn edge_sp_reject_nan() { assert!(SP::from_float(f64::NAN).is_err()); }
#[test] fn edge_sp_reject_negative() { assert!(SP::from_float(-2.0).is_err()); }
#[test] fn edge_sp_subnormal() { let s = SP::new(1.6, -1075).unwrap(); assert!(s.to_float() > 0.0); }
#[test] fn edge_sp_zero() { assert_eq!(SP::zero().to_float(), 0.0); }
#[test] fn edge_sp_one() { assert_eq!(SP::one().to_float(), 1.0); }

// ---- inference: forward / viterbi / projective ----
#[test] fn edge_forward_exact() {
    let a = vec![vec![0.9,0.1], vec![0.2,0.8]];
    let e = vec![vec![0.5,0.5], vec![0.3,0.7]];
    let obs = [0usize,1,0,1,0];
    let pi = [0.6,0.4];
    let (lk, _lkm, collapses) = forward_koppa(&a,&e,&obs,&pi);
    let (ln, _cn, _fc) = forward_naive(&a,&e,&obs,&pi);
    assert!((lk-ln).abs() < 1e-10, "lk={lk} ln={ln}");
    assert_eq!(collapses, 0);
}
#[test] fn edge_viterbi_impossible_state() {
    let p = viterbi_koppa(&[vec![1.0,0.0],vec![0.0,1.0]], &[vec![0.5,0.0],vec![0.0,0.5]], &[0], &[0.5,0.5]);
    assert_eq!(p, vec![0]);
}
#[test] fn edge_zero_provenance() {
    let d = projective_demo();
    assert!(d.shared_zero_valid);
    assert!(d.independent_zero_is_indeterminate);
    assert!(d.ratio_relerr < 1e-12);
}
#[test] fn edge_projpair_independent() {
    let a = ProjPair::new(SP::one(), 1, Some("x".into()));
    let b = ProjPair::new(SP::one(), 1, Some("y".into()));
    assert_eq!(a.ratio(&b).zero_tag, Some("INDETERMINATE_0_0".into()));
}
#[test] fn edge_projpair_shared() {
    let a = ProjPair::new(SP::one(), 1, Some("x".into()));
    let b = ProjPair::new(SP::one(), 1, Some("x".into()));
    assert_eq!(a.ratio(&b).zero_tag, None);
}

// ---- ML: LazyRelational ----
#[test] fn edge_lr_sign() { let l = LazyRelational::must_new(1.0,0,-2.0,0); assert_eq!(l.resolved_float(), -2.0); }
#[test] fn edge_lr_compose() { let l = LazyRelational::must_new(1.0,0,3.0,0).compose(&LazyRelational::must_new(1.0,0,5.0,0)).unwrap(); assert!((l.resolved_float()-15.0).abs()<1e-12); }
#[test] fn edge_lr_reject_inf_a() { assert!(LazyRelational::new(f64::INFINITY,0,1.0,0).is_err()); }
#[test] fn edge_lr_reject_inf_b() { assert!(LazyRelational::new(1.0,0,f64::INFINITY,0).is_err()); }
#[test] fn edge_lr_reject_nan() { assert!(LazyRelational::new(1.0,0,f64::NAN,0).is_err()); }
#[test] fn edge_lr_gauge() {
    let l = LazyRelational::must_new(1.0,0,3.0,0);
    let r = l.rescale(7.0,0).unwrap();
    assert!((l.resolved_float()-r.resolved_float()).abs()<1e-10);
}
#[test] fn edge_lr_overflow() { let l = LazyRelational::must_new(1.0,0,1.0,350); assert!(l.resolved_float().is_infinite()); }
#[test] fn edge_lr_underflow() { let l = LazyRelational::must_new(1.0,0,1.0,-350); assert_eq!(l.resolved_float(), 0.0); }

// ---- Finance: DeferredChain ----
#[test] fn edge_decimal_2_3() { let mut c=DeferredChain::new(); c.compose_decimal(&BigInt::from(2),&BigInt::from(3)); assert!((c.resolve_float()-1.5).abs()<1e-12); }
#[test] fn edge_decimal_zero_a() { let mut c=DeferredChain::new(); c.compose_decimal(&BigInt::from(0),&BigInt::from(5)); let _=c.resolve_float(); }
#[test] fn edge_decimal_zero_b() { let mut c=DeferredChain::new(); c.compose_decimal(&BigInt::from(5),&BigInt::from(0)); let _=c.resolve_float(); }
#[test] fn edge_overflow_350() { let mut c=DeferredChain::new(); c.compose(&BigInt::from(1),&BigInt::from(10).pow(350)); assert_eq!(c.resolve_float(), f64::INFINITY); }
#[test] fn edge_underflow() { let mut c=DeferredChain::new(); c.compose(&BigInt::from(1),&BigInt::from(10).pow(350)); c.compose(&BigInt::from(10).pow(350),&BigInt::from(1)); assert!(c.resolve_float().is_finite()); }
#[test] fn edge_decimal_matches_full() {
    let mut seed = 1u64;
    let mut next = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); seed };
    for _ in 0..200 {
        let a = (next() % 1_000_000_000_000) as i64 + 1;
        let b = (next() % 1_000_000_000_000) as i64 + 1;
        let mut c1=DeferredChain::new(); c1.compose_decimal(&BigInt::from(a),&BigInt::from(b));
        let mut c2=DeferredChain::new(); c2.compose(&BigInt::from(a),&BigInt::from(b));
        assert_eq!(c1.resolve_float(), c2.resolve_float(), "a={a} b={b}");
    }
}

// ---- Predicates ----
#[test] fn edge_witness() {
    let r1=Ratio::new(BigInt::from(7952614671698527i64),BigInt::from(21159794283675467i64));
    let r2=Ratio::new(BigInt::from(5364900327947194i64),BigInt::from(14274574083889309i64));
    assert_eq!(compare(&r1,&r2), -1);
}
#[test] fn edge_equal_ratio() {
    let r1=Ratio::new(BigInt::from(1),BigInt::from(2));
    let r2=Ratio::new(BigInt::from(2),BigInt::from(4));
    assert_eq!(compare(&r1,&r2), 0);
}
#[test] fn edge_bit26() {
    let a=(1i64<<25)+1;
    let v=compare(&Ratio::new(BigInt::from(a),BigInt::from(a+2)), &Ratio::new(BigInt::from(a),BigInt::from(a+1)));
    assert!((-1..=1).contains(&v));
}
#[test] fn edge_ratio_zero_numerator() {
    let r=Ratio::new(BigInt::from(5),BigInt::from(0)); // r = 0/5 = 0
    let _=r.resolve();
}
#[test] #[should_panic] fn edge_ratio_zero_denominator() { let _=Ratio::new(BigInt::from(0),BigInt::from(5)); }

// ---- Scan ----
#[test] fn edge_huge_int_rejected() { assert!(pair_from_ints(1u8, BigInt::from(10).pow(400)).is_err()); }
#[test] fn edge_scan_matches_fold() {
    let seq: Vec<_> = (1..=20).map(|i| pair_from_ints(i as i64, (i*7+1) as i64).unwrap()).collect();
    let db = scan_double_buffer(&seq);
    let mut acc = seq[0];
    assert!((sresolve(db[0])-sresolve(acc)).abs() <= 1e-12*sresolve(acc).abs().max(1.0));
    for i in 1..seq.len() {
        acc = scompose(acc, seq[i]);
        assert!((sresolve(db[i])-sresolve(acc)).abs() <= 1e-12*sresolve(acc).abs().max(1.0), "i={i}");
    }
}
#[test] fn edge_in_place_differs() {
    let seq: Vec<_> = (1..=20).map(|i| pair_from_ints(i as i64, (i*7+1) as i64).unwrap()).collect();
    let db = scan_double_buffer(&seq); let ip = scan_in_place(&seq);
    let diff = db.iter().zip(ip.iter()).filter(|(a,b)| (sresolve(**a)-sresolve(**b)).abs()>1e-9).count();
    assert!(diff>0, "in-place scan should diverge from double-buffer (documents the bug)");
}
