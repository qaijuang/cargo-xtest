use std::time::Duration;

fn main() {
    fast_warning::emit_warning();
    std::thread::sleep(Duration::from_secs(4));
    panic!("controlled build-script failure");
}
