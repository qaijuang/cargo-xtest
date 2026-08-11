#![cfg(not(miri))]

use std::env;
use std::time::Duration;

#[test]
fn cli() {
    let cases = trycmd::TestCases::new();
    cases.env("CARGO_TERM_COLOR", "never");
    cases.case("tests/cmd/*.toml").timeout(Duration::from_secs(300));
    cases.skip("tests/cmd/runtime-*.toml").run();

    if is_vm_tests_enabled() {
        for runtime_case in [
            "tests/cmd/runtime-pass.toml",
            "tests/cmd/runtime-failure.toml",
            "tests/cmd/runtime-network.toml",
        ] {
            let cases = trycmd::TestCases::new();
            cases.env("CARGO_TERM_COLOR", "never");
            cases.case(runtime_case).timeout(Duration::from_secs(300)).run();
        }
        trycmd::TestCases::new()
            .case("tests/cmd/runtime-terminal.toml")
            .timeout(Duration::from_secs(300))
            .run();
    }
}

fn is_vm_tests_enabled() -> bool {
    env::var("CARGO_XTEST_VM_TESTS").is_ok_and(|value| value == "1")
        && matches!(
            (env::consts::OS, env::consts::ARCH),
            ("linux", "x86_64") | ("macos", "aarch64")
        )
}
