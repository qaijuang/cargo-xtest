#![cfg(not(miri))]

mod support;

use std::time::Duration;

use support::is_vm_tests_enabled;

const CASE_TIMEOUT: Duration = Duration::from_secs(300);

#[test]
fn cli() {
    let cases = trycmd::TestCases::new();
    cases.env("CARGO_TERM_COLOR", "never");
    cases.case("tests/cmd/*.toml").timeout(CASE_TIMEOUT);
    cases.skip("tests/cmd/runtime-*.toml").skip("tests/cmd/compile-color.toml").run();

    // Color-sensitive cases must not inherit the deterministic colorless setting above.
    trycmd::TestCases::new().case("tests/cmd/compile-color.toml").timeout(CASE_TIMEOUT).run();

    if is_vm_tests_enabled() {
        // Trycmd runs cases in one runner concurrently, so isolate each VM case.
        for runtime_case in [
            "tests/cmd/runtime-pass.toml",
            "tests/cmd/runtime-failure.toml",
            "tests/cmd/runtime-network.toml",
        ] {
            let cases = trycmd::TestCases::new();
            cases.env("CARGO_TERM_COLOR", "never");
            cases.case(runtime_case).timeout(CASE_TIMEOUT).run();
        }
        // This case also needs the host's color policy.
        trycmd::TestCases::new()
            .case("tests/cmd/runtime-terminal.toml")
            .timeout(CASE_TIMEOUT)
            .run();
    }
}
