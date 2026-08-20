//@ run-flags: --nocapture --test-threads 1

use std::io::Write as _;
use std::time::Duration;

#[test]
fn waits_for_an_interrupt() {
    println!("guest is waiting for an interrupt");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(300));
}
