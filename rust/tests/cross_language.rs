use koppa::{compare, Ratio, LazyRelational, DeferredChain};
use num_bigint::BigInt;

#[test]
fn rs_python_witness() {
    let a1 = BigInt::from(7952614671698527i64); let b1 = BigInt::from(21159794283675467i64);
    let a2 = BigInt::from(5364900327947194i64); let b2 = BigInt::from(14274574083889309i64);
    let r1 = Ratio::new(a1,b1); let r2 = Ratio::new(a2,b2);
    assert_eq!(compare(&r1,&r2), -1);
}
#[test]
fn rs_python_compose_decimal() {
    let mut c = DeferredChain::new();
    c.compose_decimal(&BigInt::from(2), &BigInt::from(3));
    assert!((c.resolve_float()-1.5).abs() < 1e-12);
}
#[test]
fn rs_python_negative_sign() {
    let lr = LazyRelational::new(1.0,0,-2.0,0).unwrap();
    assert!((lr.resolved_float()+2.0).abs() < 1e-12);
}
