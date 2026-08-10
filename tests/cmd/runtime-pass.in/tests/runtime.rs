//@ exec-env: CARGO_XTEST_VM=present
//@ unset-exec-env: PATH
//@ run-flags: --test-threads 1 --show-output

#[test]
fn environment_is_configured_inside_the_vm() {
    assert_eq!(std::env::var("CARGO_XTEST_VM").unwrap(), "present");
    assert!(std::env::var_os("PATH").is_none());
    println!("environment configured inside Microsandbox");
}

#[test]
fn multiple_tests_share_the_libtest_binary() {
    println!("second test in the same libtest binary");
}
