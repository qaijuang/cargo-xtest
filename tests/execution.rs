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

use execution::{ColorMode, Decision, SandboxRootfs, decide, network_config, sandbox_config};
use microsandbox::sandbox::PullPolicy;
use microsandbox_network::config::PortProtocol;
use microsandbox_network::policy::{Action, Destination, DestinationGroup, Direction};
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

    assert_eq!(config.executable, PathBuf::from("/host/target/alpha"));
    assert_eq!(
        config.rootfs,
        SandboxRootfs::Image {
            reference: DEFAULT_IMAGE.to_owned(),
            pull_policy: PullPolicy::IfMissing,
            root_disk_mib: 4096,
        }
    );
    assert_eq!(config.cpus, 1);
    assert_eq!(config.memory_mib, 512);
    assert_eq!(config.max_duration_secs, 600);
    assert_eq!(config.user, None);
    assert_eq!(config.workdir, None);
    assert_eq!(config.shell, "/bin/sh");
    assert_eq!(config.init, None);
    assert_eq!(config.environment, Vec::<(String, String)>::new());
    assert_eq!(config.unset_environment, Vec::<String>::new());
    assert_eq!(config.arguments, ["--color=never"]);
    assert!(!config.stage_terminfo);
    assert!(config.network.is_none());
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
";

    let config = sandbox_config(
        Path::new("/host/target/configured"),
        &specification(source),
        ColorMode::Never,
    )
    .unwrap();

    assert_eq!(config.executable, PathBuf::from("/host/target/configured"));
    assert_eq!(
        config.rootfs,
        SandboxRootfs::Image {
            reference: "example.test/runtime@sha256:1234".to_owned(),
            pull_policy: PullPolicy::Always,
            root_disk_mib: 8192,
        }
    );
    assert_eq!(config.cpus, 3);
    assert_eq!(config.memory_mib, 1024);
    assert_eq!(config.max_duration_secs, 45);
    assert_eq!(config.user.as_deref(), Some("1000:1000"));
    assert_eq!(config.workdir.as_deref(), Some("/workspace"));
    assert_eq!(config.shell, "/bin/bash");
    assert_eq!(config.init.as_deref(), Some("auto"));
    assert_eq!(
        config.environment,
        [("FIRST".to_owned(), "one".to_owned()), ("SECOND".to_owned(), "two words".to_owned())]
    );
    assert_eq!(config.unset_environment, Vec::<String>::new());
    assert_eq!(
        config.arguments,
        ["--test-threads", "1", "name with space", "--show-output", "--color=never"]
    );
    assert!(!config.stage_terminfo);
    assert!(config.network.is_none());
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
fn translates_the_complete_network_contract_to_the_sdk() {
    let specification = specification(include_str!("cmd/fixtures/network.rs"));

    let network = network_config(&specification).unwrap().unwrap();

    assert!(network.enabled);
    assert_eq!(network.policy.default_egress, Action::Deny);
    assert_eq!(network.policy.default_ingress, Action::Deny);
    assert_eq!(network.policy.rules.len(), 2);
    assert_eq!(network.policy.rules[0].direction, Direction::Egress);
    assert_eq!(network.policy.rules[0].action, Action::Allow);
    assert!(matches!(network.policy.rules[0].destination, Destination::DomainSuffix(_)));
    assert_eq!(network.policy.rules[1].direction, Direction::Ingress);
    assert!(matches!(
        network.policy.rules[1].destination,
        Destination::Group(DestinationGroup::Public)
    ));
    assert_eq!(network.ports.len(), 3);
    assert_eq!(network.ports[0].protocol, PortProtocol::Tcp);
    assert_eq!(network.ports[0].host_bind.to_string(), "127.0.0.1");
    assert_eq!(network.ports[0].host_port, 18080);
    assert_eq!(network.ports[0].guest_port, 8080);
    assert_eq!(network.ports[1].protocol, PortProtocol::Udp);
    assert_eq!(network.ports[2].protocol, PortProtocol::Tcp);
    assert_eq!(network.ports[2].host_bind.to_string(), "::1");
    assert_eq!(network.ports[2].host_port, 18081);
    assert_eq!(network.ports[2].guest_port, 8081);
    assert_eq!(
        network.dns.nameservers.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["1.1.1.1:53", "dns.google:53"]
    );
    assert_eq!(network.dns.query_timeout_ms, 2500);
    assert!(!network.dns.rebind_protection);
    assert!(network.tls.enabled);
    assert_eq!(network.tls.intercepted_ports, [443, 8443]);
    assert_eq!(network.tls.bypass, ["*.internal.example"]);
    assert!(!network.tls.verify_upstream);
    assert_eq!(network.tls.scoped_verify_upstream.len(), 1);
    assert_eq!(network.tls.scoped_verify_upstream[0].pattern, "api.example.com");
    assert!(network.tls.scoped_verify_upstream[0].verify);
    assert!(!network.tls.block_quic_on_intercept);
    assert_eq!(network.tls.upstream_ca_cert, [PathBuf::from("tests/certificates/upstream.pem")]);
    assert_eq!(network.tls.scoped_upstream_ca_cert.len(), 1);
    assert_eq!(
        network.tls.scoped_upstream_ca_cert[0].path,
        PathBuf::from("tests/certificates/internal.pem")
    );
    assert_eq!(
        network.tls.intercept_ca.cert_path.as_deref(),
        Some(Path::new("tests/certificates/intercept.pem"))
    );
    assert_eq!(
        network.tls.intercept_ca.key_path.as_deref(),
        Some(Path::new("tests/certificates/intercept-key.pem"))
    );
    assert_eq!(network.tls.cache.capacity, 128);
    assert_eq!(network.tls.cache.validity_hours, 12);
    assert_eq!(network.max_connections, Some(64));
    assert!(network.trust_host_cas);
    assert_eq!(network.interface.mac, Some([0x02, 0, 0, 0, 0, 0x2a]));
    assert_eq!(network.interface.mtu, Some(1400));
    assert_eq!(network.interface.ipv4_address.unwrap().to_string(), "172.20.0.2");
    assert_eq!(network.interface.ipv4_pool.unwrap().to_string(), "172.20.0.0/16");
    assert_eq!(network.interface.ipv6_address.unwrap().to_string(), "fd42:6d73:62::2");
    assert_eq!(network.interface.ipv6_pool.unwrap().to_string(), "fd42:6d73:62::/48");
}

#[test]
fn translates_each_network_mode_without_weakening_the_disabled_default() {
    assert!(network_config(&specification("")).unwrap().is_none());

    let public = network_config(&specification("//@ network\n")).unwrap().unwrap();
    assert_eq!(public.policy.default_egress, Action::Deny);
    assert_eq!(public.policy.default_ingress, Action::Allow);
    assert_eq!(public.policy.rules.len(), 2);
    assert!(matches!(
        public.policy.rules[1].destination,
        Destination::Group(DestinationGroup::Public)
    ));

    let duplicate_public =
        network_config(&specification("//@ network: public,public\n")).unwrap().unwrap();
    assert_eq!(duplicate_public.policy.default_egress, Action::Deny);
    assert_eq!(duplicate_public.policy.default_ingress, Action::Allow);
    assert_eq!(duplicate_public.policy.rules.len(), 2);
    assert!(matches!(
        duplicate_public.policy.rules[1].destination,
        Destination::Group(DestinationGroup::Public)
    ));

    let none = network_config(&specification("//@ network: none\n")).unwrap().unwrap();
    assert_eq!(none.policy.default_egress, Action::Deny);
    assert_eq!(none.policy.default_ingress, Action::Deny);
    assert_eq!(none.policy.rules.len(), 0);

    let allow_all = network_config(&specification("//@ network: allow-all\n")).unwrap().unwrap();
    assert_eq!(allow_all.policy.default_egress, Action::Allow);
    assert_eq!(allow_all.policy.default_ingress, Action::Allow);
    assert_eq!(allow_all.policy.rules.len(), 0);

    let private_host =
        network_config(&specification("//@ network: private,host\n")).unwrap().unwrap();
    assert_eq!(private_host.policy.default_egress, Action::Deny);
    assert_eq!(private_host.policy.default_ingress, Action::Allow);
    assert_eq!(private_host.policy.rules.len(), 3);
    assert!(matches!(
        private_host.policy.rules[1].destination,
        Destination::Group(DestinationGroup::Private)
    ));
    assert!(matches!(
        private_host.policy.rules[2].destination,
        Destination::Group(DestinationGroup::Host)
    ));
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
