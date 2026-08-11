//@ run-flags: --nocapture --test-threads 1

use std::io::Write as _;
use std::time::Duration;

#[test]
fn emits_output_before_completion() {
    println!("guest output is live");
    std::io::stdout().flush().unwrap();
    eprintln!("guest stderr is live");
    std::io::stderr().flush().unwrap();
    std::thread::sleep(Duration::from_secs(4));
}
