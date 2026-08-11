//@ run-flags: --nocapture --test-threads 1

use std::io::Write as _;

#[test]
fn writes_a_non_utf8_byte() {
    std::io::stdout().write_all(b"guest byte: \xff\n").unwrap();
}
