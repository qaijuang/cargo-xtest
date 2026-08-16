#![allow(dead_code)]

#[path = "../src/cargo.rs"]
mod cargo;

use std::ffi::OsString;
use std::path::PathBuf;

use cargo::{
    ArtifactCollector, CompiledTest, SkippedTest, TestArtifact, cargo_test_command,
    guest_target_for_arch,
};

fn collect_messages(messages: &[u8]) -> (Vec<u8>, Vec<TestArtifact>) {
    let mut collector = ArtifactCollector::default();
    let mut output = Vec::new();
    for line in messages.split_inclusive(|byte| *byte == b'\n') {
        if let Some(bytes) = collector.observe(line).unwrap() {
            output.extend_from_slice(&bytes);
        }
    }
    (output, collector.finish())
}

#[test]
fn maps_supported_hosts_to_linux_musl_targets() {
    let x86_64 = guest_target_for_arch("x86_64").unwrap();
    let aarch64 = guest_target_for_arch("aarch64").unwrap();

    assert_eq!(x86_64.triple, "x86_64-unknown-linux-musl");
    assert_eq!(x86_64.linker_environment, "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER");
    assert_eq!(aarch64.triple, "aarch64-unknown-linux-musl");
    assert_eq!(aarch64.linker_environment, "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER");
}

#[test]
fn rejects_an_unsupported_host_architecture() {
    let error = guest_target_for_arch("riscv64").unwrap_err();

    assert_eq!(
        error.to_string(),
        "unsupported host architecture `riscv64` -- cargo-xtest supports x86_64 and aarch64"
    );
}

#[test]
fn constructs_one_cargo_command_for_all_enabled_test_targets() {
    let guest = guest_target_for_arch("aarch64").unwrap();

    let command = cargo_test_command(
        OsString::from("/opt/toolchains/cargo"),
        PathBuf::from("/workspace"),
        guest,
        OsString::from("never"),
        &[],
        false,
    );

    assert_eq!(command.program, OsString::from("/opt/toolchains/cargo"));
    assert_eq!(command.current_dir, PathBuf::from("/workspace"));
    assert_eq!(
        command.arguments,
        [
            "test",
            "--tests",
            "--color",
            "never",
            "--target",
            "aarch64-unknown-linux-musl",
            "--no-run",
            "--message-format=json",
        ]
        .map(OsString::from)
    );
    assert_eq!(
        command.environment,
        [(
            OsString::from("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"),
            OsString::from("rust-lld"),
        )]
    );
}

#[test]
fn forces_cargo_and_rendered_diagnostics_to_use_color() {
    let command = cargo_test_command(
        OsString::from("cargo"),
        PathBuf::from("/workspace"),
        guest_target_for_arch("x86_64").unwrap(),
        OsString::from("always"),
        &[],
        false,
    );

    assert!(
        command
            .arguments
            .iter()
            .any(|argument| argument == "--message-format=json-diagnostic-rendered-ansi")
    );
    assert!(command.arguments.windows(2).any(|arguments| arguments == ["--color", "always"]));
}

#[test]
fn classifies_and_sorts_cargo_test_artifacts() {
    let messages = r#"{"reason":"compiler-artifact","package_id":"path+file:///workspace/selected#0.1.0","manifest_path":"/workspace/selected/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"zeta","src_path":"/workspace/selected/tests/zeta.rs","edition":"2024","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/zeta"],"executable":"/workspace/target/zeta","fresh":false}
{"reason":"compiler-artifact","package_id":"path+file:///workspace/selected#0.1.0","manifest_path":"/workspace/selected/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"selected","src_path":"/workspace/selected/src/lib.rs","edition":"2024","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/selected"],"executable":"/workspace/target/selected","fresh":false}
{"reason":"compiler-artifact","package_id":"path+file:///workspace/dependency#0.1.0","manifest_path":"/workspace/dependency/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"dependency","src_path":"/workspace/dependency/src/lib.rs","edition":"2024","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/workspace/target/libdependency.rlib"],"executable":null,"fresh":false}
{"reason":"build-finished","success":true}
"#;

    let (_, artifacts) = collect_messages(messages.as_bytes());

    assert_eq!(
        artifacts,
        vec![
            TestArtifact::Skip(SkippedTest {
                package_id: "path+file:///workspace/selected#0.1.0".to_owned(),
                name: "selected".to_owned(),
                source_path: PathBuf::from("/workspace/selected/src/lib.rs"),
            }),
            TestArtifact::Run(CompiledTest {
                package_id: "path+file:///workspace/selected#0.1.0".to_owned(),
                name: "zeta".to_owned(),
                source_path: PathBuf::from("/workspace/selected/tests/zeta.rs"),
                executable: PathBuf::from("/workspace/target/zeta"),
            }),
        ]
    );
}

#[test]
fn retains_rendered_compiler_diagnostics() {
    let messages = r#"{"reason":"compiler-message","package_id":"path+file:///workspace/dep#0.1.0","target":{"kind":["lib"],"crate_types":["lib"],"name":"dep","src_path":"/workspace/dep/src/lib.rs","edition":"2024","doc":true,"doctest":true,"test":true},"message":{"message":"dependency warning","code":null,"level":"warning","spans":[],"children":[],"rendered":"warning: dependency warning\n"}}
{"reason":"build-finished","success":true}
"#;

    let (output, _) = collect_messages(messages.as_bytes());

    assert_eq!(output, b"warning: dependency warning\n");
}

#[test]
fn preserves_non_json_and_non_utf8_tool_output() {
    let mut collector = ArtifactCollector::default();

    assert_eq!(
        collector.observe(b"third-party build output\n").unwrap(),
        Some(b"third-party build output\n".to_vec())
    );
    assert_eq!(
        collector.observe(b"third-party output: \xff\n").unwrap(),
        Some(b"third-party output: \xff\n".to_vec())
    );
}
