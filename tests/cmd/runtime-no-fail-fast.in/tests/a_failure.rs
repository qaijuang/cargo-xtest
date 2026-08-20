//@ run-flags: --nocapture

use std::io::Write as _;

#[test]
fn controlled_exit_status() {
    println!("first integration-test binary ran inside Microsandbox");
    std::io::stdout().flush().unwrap();
    std::process::exit(42);
}
