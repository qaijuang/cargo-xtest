#![allow(dead_code)]

#[path = "../src/diagnostic.rs"]
mod diagnostic;
#[path = "../src/directive.rs"]
mod directive;
#[path = "../src/execution.rs"]
mod execution;
#[path = "../src/model.rs"]
mod model;

#[path = "../src/helpers.rs"]
mod helpers;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use execution::{ColorMode, Decision, SandboxConfig, SandboxRootfs, decide, sandbox_config};
use microsandbox::sandbox::PullPolicy;
use model::EffectiveSpecification;

const DEFAULT_IMAGE: &str =
    "alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce";

fn specification(source: &str) -> EffectiveSpecification {
    let path = Path::new("tests/example.rs");
    let directive::ParseOutput { directives, mut diagnostics } =
        directive::parse_reader(path, Cursor::new(source)).unwrap();
    let specification = model::reduce(path, directives, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics}");
    specification
}

#[test]
fn decides_directive_applicability_before_execution() {
    let cases = [
        ("", Decision::Run),
        (
            "//@ ignore-test (temporarily disabled)\n",
            Decision::Skip("ignored by `ignore-test`: (temporarily disabled)".to_owned()),
        ),
        ("//@ only-linux\n", Decision::Run),
        (
            "//@ only-windows\n",
            Decision::Skip("`only-windows` does not match the Linux-musl guest".to_owned()),
        ),
        (
            "//@ ignore-linux\n",
            Decision::Skip("`ignore-linux` matches the Linux-musl guest".to_owned()),
        ),
        ("//@ ignore-windows\n", Decision::Run),
        ("//@ only-stage1\n", Decision::Skip("unknown target predicate `stage1`".to_owned())),
    ];

    for (source, expected) in cases {
        assert_eq!(
            decide(&specification(source), "aarch64", "aarch64-unknown-linux-musl").unwrap(),
            expected,
            "source: {source:?}"
        );
    }
}

#[test]
fn skips_unavailable_dynamic_linking_but_runs_supported_capabilities() {
    let unavailable = specification("//@ needs-dynamic-linking\n");
    let supported = specification(
        "//@ needs-threads\n//@ needs-subprocess\n//@ needs-symlink\n//@ needs-target-std\n//@ needs-unwind\n",
    );

    assert_eq!(
        decide(&unavailable, "x86_64", "x86_64-unknown-linux-musl").unwrap(),
        Decision::Skip(
            "`needs-dynamic-linking` is unavailable in the self-contained Linux-musl profile"
                .to_owned()
        )
    );
    assert_eq!(decide(&supported, "x86_64", "x86_64-unknown-linux-musl").unwrap(), Decision::Run);
}

#[test]
fn rejects_preserve_on_failure_before_execution() {
    let error = decide(
        &specification("//@ preserve-on-failure\n"),
        "aarch64",
        "aarch64-unknown-linux-musl",
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "`preserve-on-failure` is not supported by one-shot Microsandbox execution"
    );
}

#[test]
fn constructs_the_safe_default_sandbox_config() {
    let config =
        sandbox_config(Path::new("/host/target/alpha"), &specification(""), ColorMode::Never)
            .unwrap();

    assert_eq!(
        config,
        SandboxConfig {
            executable: PathBuf::from("/host/target/alpha"),
            rootfs: SandboxRootfs::Image {
                reference: DEFAULT_IMAGE.to_owned(),
                pull_policy: PullPolicy::IfMissing,
                root_disk_mib: 4096,
            },
            cpus: 1,
            memory_mib: 512,
            max_duration_secs: 600,
            user: None,
            workdir: None,
            shell: "/bin/sh".to_owned(),
            init: None,
            environment: Vec::new(),
            unset_environment: Vec::new(),
            arguments: vec!["--color=never".to_owned()],
            stage_terminfo: false,
        }
    );
}

#[test]
fn stages_private_terminal_support_when_color_is_enabled() {
    let config = sandbox_config(
        Path::new("/host/target/colored"),
        &specification("//@ image: example.test/runtime@sha256:1234\n"),
        ColorMode::Always,
    )
    .unwrap();

    assert_eq!(
        config.environment,
        [
            ("TERM".to_owned(), "cargo-xtest".to_owned()),
            ("TERMINFO".to_owned(), "/cargo-xtest/terminfo".to_owned()),
        ]
    );
    assert_eq!(config.arguments, ["--color=always"]);
    assert!(config.stage_terminfo);
}

#[test]
fn explicit_terminal_environment_takes_precedence() {
    let cases = [
        ("//@ exec-env: TERM=custom\n", vec![("TERM", "custom")], Vec::new()),
        ("//@ exec-env: TERMINFO=/custom\n", vec![("TERMINFO", "/custom")], Vec::new()),
        ("//@ unset-exec-env: TERM\n", Vec::new(), vec!["TERM"]),
        ("//@ unset-exec-env: TERMINFO\n", Vec::new(), vec!["TERMINFO"]),
    ];

    for (source, environment, unset_environment) in cases {
        let config = sandbox_config(
            Path::new("/host/target/colored"),
            &specification(source),
            ColorMode::Always,
        )
        .unwrap();

        assert_eq!(
            config
                .environment
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect::<Vec<_>>(),
            environment
        );
        assert_eq!(config.unset_environment, unset_environment);
        assert_eq!(config.arguments, ["--color=always"]);
        assert!(!config.stage_terminfo);
    }
}

#[test]
fn explicit_libtest_color_takes_precedence() {
    let disabled = sandbox_config(
        Path::new("/host/target/plain"),
        &specification("//@ run-flags: --color never\n"),
        ColorMode::Always,
    )
    .unwrap();
    assert_eq!(disabled.arguments, ["--color", "never"]);
    assert!(!disabled.stage_terminfo);

    let enabled = sandbox_config(
        Path::new("/host/target/colored"),
        &specification("//@ run-flags: --color=always\n"),
        ColorMode::Never,
    )
    .unwrap();
    assert_eq!(enabled.arguments, ["--color=always"]);
    assert!(enabled.stage_terminfo);
}

#[test]
fn places_the_default_color_option_before_the_option_terminator() {
    let enabled = sandbox_config(
        Path::new("/host/target/colored"),
        &specification("//@ run-flags: -- named-test\n"),
        ColorMode::Always,
    )
    .unwrap();
    assert_eq!(enabled.arguments, ["--color=always", "--", "named-test"]);
    assert!(enabled.stage_terminfo);

    let disabled = sandbox_config(
        Path::new("/host/target/plain"),
        &specification("//@ run-flags: -- --color=always\n"),
        ColorMode::Never,
    )
    .unwrap();
    assert_eq!(disabled.arguments, ["--color=never", "--", "--color=always"]);
    assert!(!disabled.stage_terminfo);
}

#[test]
fn translates_explicit_image_resources_guest_and_libtest_configuration() {
    let source = r"
//@ image: example.test/runtime@sha256:1234
//@ pull-policy: always
//@ cpus: 3
//@ memory: 1024
//@ root-disk: 8192
//@ max-duration: 45
//@ exec-env: FIRST=one
//@ exec-env: SECOND=two words
//@ run-flags: --test-threads 1 'name with space' --show-output
//@ user: 1000:1000
//@ workdir: /workspace
//@ shell: /bin/bash
//@ init: auto
//@ disable-network
";

    let config = sandbox_config(
        Path::new("/host/target/configured"),
        &specification(source),
        ColorMode::Never,
    )
    .unwrap();

    assert_eq!(
        config,
        SandboxConfig {
            executable: PathBuf::from("/host/target/configured"),
            rootfs: SandboxRootfs::Image {
                reference: "example.test/runtime@sha256:1234".to_owned(),
                pull_policy: PullPolicy::Always,
                root_disk_mib: 8192,
            },
            cpus: 3,
            memory_mib: 1024,
            max_duration_secs: 45,
            user: Some("1000:1000".to_owned()),
            workdir: Some("/workspace".to_owned()),
            shell: "/bin/bash".to_owned(),
            init: Some("auto".to_owned()),
            environment: vec![
                ("FIRST".to_owned(), "one".to_owned()),
                ("SECOND".to_owned(), "two words".to_owned()),
            ],
            unset_environment: Vec::new(),
            arguments: vec![
                "--test-threads".to_owned(),
                "1".to_owned(),
                "name with space".to_owned(),
                "--show-output".to_owned(),
                "--color=never".to_owned(),
            ],
            stage_terminfo: false,
        }
    );
}

#[test]
fn translates_snapshot_rootfs_without_image_only_options() {
    let config = sandbox_config(
        Path::new("/host/target/snapshot"),
        &specification("//@ from-snapshot: nightly-base\n"),
        ColorMode::Never,
    )
    .unwrap();

    assert_eq!(config.rootfs, SandboxRootfs::Snapshot { reference: "nightly-base".to_owned() });
}

#[test]
fn uses_a_guest_wrapper_only_for_portable_environment_unsets() {
    let source = "//@ exec-env: KEEP=yes\n//@ unset-exec-env: REMOVE_ME\n";
    let config =
        sandbox_config(Path::new("/host/target/unset"), &specification(source), ColorMode::Never)
            .unwrap();

    assert_eq!(config.environment, [("KEEP".to_owned(), "yes".to_owned())]);
    assert_eq!(config.unset_environment, ["REMOVE_ME"]);

    let error = sandbox_config(
        Path::new("/host/target/unset"),
        &specification("//@ unset-exec-env: NOT PORTABLE\n"),
        ColorMode::Never,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "`unset-exec-env` key `NOT PORTABLE` is not a portable shell identifier"
    );
}
