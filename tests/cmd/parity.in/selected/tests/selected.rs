//@ memory: deliberately-invalid-for-no-run

#[cfg(not(feature = "enabled"))]
compile_error!("the enabled feature was not forwarded to Cargo");

#[test]
fn selected_integration_test_compiles() {
    assert!(selected::library_is_available());
}
