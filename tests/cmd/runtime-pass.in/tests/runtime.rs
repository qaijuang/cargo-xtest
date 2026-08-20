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

#[test]
fn network_is_disabled_without_a_directive() {
    let address = "1.1.1.1:80".parse().unwrap();
    let connection =
        std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2));

    assert!(connection.is_err(), "outbound network access unexpectedly succeeded");
    println!("network disabled inside Microsandbox");
}
