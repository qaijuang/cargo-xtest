//@ run-flags: --test-threads 1 --show-output

#[test]
fn reports_a_controlled_failure() -> Result<(), &'static str> {
    println!("failure output from inside Microsandbox");
    Err("controlled Microsandbox failure")
}
