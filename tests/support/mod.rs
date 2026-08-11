pub(crate) fn is_vm_tests_enabled() -> bool {
    std::env::var("CARGO_XTEST_VM_TESTS").is_ok_and(|value| value == "1")
        && matches!(
            (std::env::consts::OS, std::env::consts::ARCH),
            ("linux", "x86_64") | ("macos", "aarch64")
        )
}
