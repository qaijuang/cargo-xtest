#[test]
fn is_not_compiled() {
    assert_eq!(slow_build::value(), 1);
}
