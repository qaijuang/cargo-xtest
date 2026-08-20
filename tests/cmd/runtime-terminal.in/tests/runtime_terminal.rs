#[test]
fn harness_stages_terminal_support() {
    assert_eq!(std::env::var("TERM").unwrap(), "cargo-xtest");
    assert_eq!(std::env::var("TERMINFO").unwrap(), "/cargo-xtest/terminfo");
    assert!(std::path::Path::new("/cargo-xtest/terminfo/c/cargo-xtest").is_file());
}
