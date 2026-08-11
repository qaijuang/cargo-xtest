#![allow(dead_code)]

#[path = "../src/cargo.rs"]
mod cargo;

use std::ffi::OsString;
use std::path::PathBuf;

use cargo::{
    ArtifactCollector, TestTarget, cargo_test_command, discover_from_metadata,
    guest_target_for_arch,
};
use cargo_metadata::MetadataCommand;

fn selected_target() -> TestTarget {
    TestTarget {
        package_id: "path+file:///workspace/selected#0.1.0".to_owned(),
        workspace_root: PathBuf::from("/workspace"),
        name: "alpha".to_owned(),
        source_path: PathBuf::from("/workspace/selected/tests/alpha.rs"),
    }
}

fn collect_messages(messages: &[u8], target: &TestTarget) -> (Vec<u8>, anyhow::Result<PathBuf>) {
    let mut collector = ArtifactCollector::new(target);
    let mut output = Vec::new();
    for line in messages.split_inclusive(|byte| *byte == b'\n') {
        if let Some(bytes) = collector.observe(line).unwrap() {
            output.extend_from_slice(&bytes);
        }
    }
    (output, collector.finish())
}

#[test]
#[allow(clippy::too_many_lines)]
fn discovers_default_member_integration_tests() {
    let metadata = r#"
{
  "packages": [
    {
      "name": "selected",
      "version": "0.1.0",
      "id": "path+file:///workspace/selected#0.1.0",
      "dependencies": [],
      "features": {},
      "manifest_path": "/workspace/selected/Cargo.toml",
      "targets": [
        {
          "kind": ["test"],
          "crate_types": ["bin"],
          "name": "zeta",
          "src_path": "/workspace/selected/tests/zeta.rs",
          "edition": "2024",
          "doc": false,
          "doctest": false,
          "test": true
        },
        {
          "kind": ["test"],
          "crate_types": ["bin"],
          "name": "alpha",
          "src_path": "/workspace/selected/tests/alpha.rs",
          "edition": "2024",
          "doc": false,
          "doctest": false,
          "test": true
        },
        {
          "kind": ["test"],
          "crate_types": ["bin"],
          "name": "disabled",
          "src_path": "/workspace/selected/tests/disabled.rs",
          "edition": "2024",
          "doc": false,
          "doctest": false,
          "test": false
        },
        {
          "kind": ["bin"],
          "crate_types": ["bin"],
          "name": "selected",
          "src_path": "/workspace/selected/src/main.rs",
          "edition": "2024",
          "doc": true,
          "doctest": false,
          "test": true
        }
      ]
    },
    {
      "name": "excluded",
      "version": "0.1.0",
      "id": "path+file:///workspace/excluded#0.1.0",
      "dependencies": [],
      "features": {},
      "manifest_path": "/workspace/excluded/Cargo.toml",
      "targets": [
        {
          "kind": ["test"],
          "crate_types": ["bin"],
          "name": "excluded",
          "src_path": "/workspace/excluded/tests/excluded.rs",
          "edition": "2024",
          "doc": false,
          "doctest": false,
          "test": true
        }
      ]
    }
  ],
  "workspace_default_members": [
    "path+file:///workspace/selected#0.1.0"
  ],
  "workspace_members": [
    "path+file:///workspace/selected#0.1.0",
    "path+file:///workspace/excluded#0.1.0"
  ],
  "workspace_root": "/workspace",
  "target_directory": "/workspace/target",
  "version": 1
}
"#;

    let metadata = MetadataCommand::parse(metadata).unwrap();
    let targets = discover_from_metadata(&metadata);

    assert_eq!(
        targets,
        vec![
            TestTarget {
                package_id: "path+file:///workspace/selected#0.1.0".to_owned(),
                workspace_root: PathBuf::from("/workspace"),
                name: "alpha".to_owned(),
                source_path: PathBuf::from("/workspace/selected/tests/alpha.rs"),
            },
            TestTarget {
                package_id: "path+file:///workspace/selected#0.1.0".to_owned(),
                workspace_root: PathBuf::from("/workspace"),
                name: "zeta".to_owned(),
                source_path: PathBuf::from("/workspace/selected/tests/zeta.rs"),
            },
        ]
    );
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
fn constructs_the_per_test_file_cargo_command() {
    let target = selected_target();
    let guest = guest_target_for_arch("aarch64").unwrap();

    let command =
        cargo_test_command(OsString::from("/opt/toolchains/cargo"), &target, guest, false);

    assert_eq!(command.program, OsString::from("/opt/toolchains/cargo"));
    assert_eq!(command.current_dir, PathBuf::from("/workspace"));
    assert_eq!(
        command.arguments,
        [
            "test",
            "--package",
            "path+file:///workspace/selected#0.1.0",
            "--test",
            "alpha",
            "--target",
            "aarch64-unknown-linux-musl",
            "--no-run",
            "--message-format=json",
            "--quiet",
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
fn requests_ansi_rendered_diagnostics_when_color_is_forced() {
    let command = cargo_test_command(
        OsString::from("cargo"),
        &selected_target(),
        guest_target_for_arch("x86_64").unwrap(),
        true,
    );

    assert!(
        command
            .arguments
            .iter()
            .any(|argument| argument == "--message-format=json-diagnostic-rendered-ansi")
    );
    assert!(!command.arguments.iter().any(|argument| argument == "--color=never"));
}

#[test]
fn retains_rendered_compiler_diagnostics() {
    let messages = r#"{"reason":"compiler-message","package_id":"path+file:///workspace/dep#0.1.0","target":{"kind":["lib"],"crate_types":["lib"],"name":"dep","src_path":"/workspace/dep/src/lib.rs","edition":"2024","doc":true,"doctest":true,"test":true},"message":{"message":"dependency warning","code":null,"level":"warning","spans":[],"children":[],"rendered":"warning: dependency warning\n"}}
{"reason":"compiler-artifact","package_id":"path+file:///workspace/selected#0.1.0","manifest_path":"/workspace/selected/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"alpha","src_path":"/workspace/selected/tests/alpha.rs","edition":"2024","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/alpha"],"executable":"/workspace/target/alpha","fresh":false}
{"reason":"build-finished","success":true}
"#;

    let target = selected_target();
    let (output, executable) = collect_messages(messages.as_bytes(), &target);

    assert_eq!(output, b"warning: dependency warning\n");
    executable.unwrap();
}

#[test]
fn preserves_non_json_tool_output() {
    let messages = r#"third-party build output
{"reason":"compiler-artifact","package_id":"path+file:///workspace/selected#0.1.0","manifest_path":"/workspace/selected/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"alpha","src_path":"/workspace/selected/tests/alpha.rs","edition":"2024","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/alpha"],"executable":"/workspace/target/alpha","fresh":false}
{"reason":"build-finished","success":true}
"#;

    let target = selected_target();
    let (output, executable) = collect_messages(messages.as_bytes(), &target);

    assert_eq!(output, b"third-party build output\n");
    executable.unwrap();
}

#[test]
fn preserves_non_utf8_tool_output_from_cargo() {
    let target = selected_target();
    let mut collector = ArtifactCollector::new(&target);

    let output = collector.observe(b"third-party output: \xff\n").unwrap();

    assert_eq!(output, Some(b"third-party output: \xff\n".to_vec()));
}

#[test]
fn selects_the_single_matching_test_executable() {
    let messages = r#"{"reason":"compiler-artifact","package_id":"path+file:///workspace/other#0.1.0","manifest_path":"/workspace/other/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"alpha","src_path":"/workspace/other/tests/alpha.rs","edition":"2024","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/other-alpha"],"executable":"/workspace/target/other-alpha","fresh":false}
{"reason":"compiler-artifact","package_id":"path+file:///workspace/selected#0.1.0","manifest_path":"/workspace/selected/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"alpha","src_path":"/workspace/selected/tests/alpha.rs","edition":"2024","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/alpha"],"executable":"/workspace/target/alpha","fresh":false}
{"reason":"build-finished","success":true}
"#;

    let target = selected_target();
    let (_, executable) = collect_messages(messages.as_bytes(), &target);

    assert_eq!(executable.unwrap(), PathBuf::from("/workspace/target/alpha"));
}

#[test]
fn rejects_multiple_matching_test_executables() {
    let artifact = r#"{"reason":"compiler-artifact","package_id":"path+file:///workspace/selected#0.1.0","manifest_path":"/workspace/selected/Cargo.toml","target":{"kind":["test"],"crate_types":["bin"],"name":"alpha","src_path":"/workspace/selected/tests/alpha.rs","edition":"2024","doc":false,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":true},"features":[],"filenames":["/workspace/target/alpha"],"executable":"/workspace/target/alpha","fresh":false}"#;
    let messages = format!("{artifact}\n{artifact}\n");

    let target = selected_target();
    let (_, executable) = collect_messages(messages.as_bytes(), &target);
    let error = executable.unwrap_err();

    assert_eq!(
        error.to_string(),
        "Cargo reported 2 matching executables for test target `alpha` -- expected exactly one"
    );
}
