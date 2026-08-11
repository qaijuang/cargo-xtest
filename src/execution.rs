#[path = "execution/terminfo.rs"]
mod terminfo;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use microsandbox::Sandbox;
use microsandbox::sandbox::{FsSetAttrs, PullPolicy};
use microsandbox_network::builder::NetworkBuilder;
use microsandbox_network::config::NetworkConfig;
use microsandbox_network::policy::NetworkPolicy;

use crate::directive::{Capability, NetworkMode};
use crate::model::{
    EffectiveRootfs, EffectiveSpecification, EnvironmentChange, FailureRetention, ImageSource,
    NetworkAccess, NetworkConfiguration,
};

const DEFAULT_IMAGE: &str =
    "alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce";
const GUEST_EXECUTABLE: &str = "/cargo-xtest-test";
static NEXT_SANDBOX_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorMode {
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Run,
    Skip(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SandboxRootfs {
    Image { reference: String, pull_policy: PullPolicy, root_disk_mib: u32 },
    Snapshot { reference: String },
}

#[derive(Debug, Clone)]
pub(crate) struct SandboxConfig {
    pub(crate) executable: PathBuf,
    pub(crate) rootfs: SandboxRootfs,
    pub(crate) cpus: u8,
    pub(crate) memory_mib: u32,
    pub(crate) max_duration_secs: u64,
    pub(crate) user: Option<String>,
    pub(crate) workdir: Option<String>,
    pub(crate) shell: String,
    pub(crate) init: Option<String>,
    pub(crate) environment: Vec<(String, String)>,
    pub(crate) unset_environment: Vec<String>,
    pub(crate) arguments: Vec<String>,
    pub(crate) stage_terminfo: bool,
    pub(crate) network: Option<NetworkConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionOutput {
    pub(crate) status: i32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) cleanup_error: Option<String>,
}

pub(crate) fn execute(config: &SandboxConfig) -> Result<ExecutionOutput> {
    let config = config.clone();
    run_sdk_worker(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("could not initialize the Microsandbox SDK runtime")?;
        runtime.block_on(execute_in_microsandbox(&config))
    })
}

fn run_sdk_worker<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let worker = std::thread::Builder::new()
        .name("cargo-xtest-microsandbox".to_owned())
        .spawn(operation)
        .context("could not start the Microsandbox SDK worker")?;
    worker.join().map_err(|_| anyhow!("Microsandbox SDK worker panicked"))?
}

pub(crate) fn decide(
    specification: &EffectiveSpecification,
    guest_architecture: &str,
    guest_triple: &str,
) -> Result<Decision> {
    if specification.sandbox.failure_retention.value == FailureRetention::Preserve {
        bail!("`preserve-on-failure` is not supported by one-shot Microsandbox execution");
    }
    if let Some(ignore_test) = &specification.applicability.ignore_test {
        return Ok(Decision::Skip(format!("ignored by `ignore-test`: {}", ignore_test.value)));
    }

    for predicate in &specification.applicability.only {
        match predicate_matches(&predicate.value, guest_architecture, guest_triple) {
            Some(true) => {}
            Some(false) => {
                return Ok(Decision::Skip(format!(
                    "`only-{}` does not match the Linux-musl guest",
                    predicate.value
                )));
            }
            None => {
                return Ok(Decision::Skip(format!(
                    "unknown target predicate `{}`",
                    predicate.value
                )));
            }
        }
    }

    for predicate in &specification.applicability.ignore {
        match predicate_matches(&predicate.value, guest_architecture, guest_triple) {
            Some(true) => {
                return Ok(Decision::Skip(format!(
                    "`ignore-{}` matches the Linux-musl guest",
                    predicate.value
                )));
            }
            Some(false) => {}
            None => {
                return Ok(Decision::Skip(format!(
                    "unknown target predicate `{}`",
                    predicate.value
                )));
            }
        }
    }

    if specification
        .capabilities
        .iter()
        .any(|capability| capability.value == Capability::DynamicLinking)
    {
        return Ok(Decision::Skip(
            "`needs-dynamic-linking` is unavailable in the self-contained Linux-musl profile"
                .to_owned(),
        ));
    }

    Ok(Decision::Run)
}

fn predicate_matches(
    predicate: &str,
    guest_architecture: &str,
    guest_triple: &str,
) -> Option<bool> {
    match predicate {
        "test" | "linux" | "musl" | "linux-musl" | "unix" | "elf" | "64bit" => Some(true),
        "windows" | "macos" | "gnu" | "msvc" | "32bit" => Some(false),
        "x86_64" | "aarch64" => Some(predicate == guest_architecture),
        "x86_64-unknown-linux-musl" | "aarch64-unknown-linux-musl" => {
            Some(predicate == guest_triple)
        }
        _ => None,
    }
}

pub(crate) fn sandbox_config(
    executable: &Path,
    specification: &EffectiveSpecification,
    color: ColorMode,
) -> Result<SandboxConfig> {
    let rootfs = match &specification.sandbox.rootfs {
        EffectiveRootfs::Image { source, pull_policy, root_disk_mib } => SandboxRootfs::Image {
            reference: match &source.value {
                ImageSource::BuiltInRuntime => DEFAULT_IMAGE.to_owned(),
                ImageSource::Explicit(image) => image.clone(),
            },
            pull_policy: pull_policy.value,
            root_disk_mib: root_disk_mib.value,
        },
        EffectiveRootfs::Snapshot { reference } => {
            SandboxRootfs::Snapshot { reference: reference.value.clone() }
        }
    };

    let mut environment = Vec::new();
    let mut unset_environment = Vec::new();
    let mut terminal_environment_is_explicit = false;
    for change in &specification.execution.environment {
        match &change.value {
            EnvironmentChange::Set { key, value } => {
                terminal_environment_is_explicit |= key == "TERM" || key == "TERMINFO";
                environment.push((key.clone(), value.clone()));
            }
            EnvironmentChange::Unset(key) => {
                if !is_portable_shell_identifier(key) {
                    bail!("`unset-exec-env` key `{key}` is not a portable shell identifier");
                }
                terminal_environment_is_explicit |= key == "TERM" || key == "TERMINFO";
                unset_environment.push(key.clone());
            }
        }
    }

    let mut arguments: Vec<_> = specification
        .execution
        .run_flags
        .iter()
        .flat_map(|flags| flags.value.iter().cloned())
        .collect();
    let force_color = apply_test_color(&mut arguments, color);
    let stage_terminfo = force_color && !terminal_environment_is_explicit;
    if stage_terminfo {
        environment.extend([
            ("TERM".to_owned(), terminfo::NAME.to_owned()),
            ("TERMINFO".to_owned(), terminfo::DIRECTORY.to_owned()),
        ]);
    }
    let network = network_config(specification)?;

    Ok(SandboxConfig {
        executable: executable.to_owned(),
        rootfs,
        cpus: specification.sandbox.cpus.value,
        memory_mib: specification.sandbox.memory_mib.value,
        max_duration_secs: specification.sandbox.max_duration_secs.value,
        user: specification.sandbox.guest.user.value.clone(),
        workdir: specification.sandbox.guest.workdir.value.clone(),
        shell: specification.sandbox.guest.shell.value.clone(),
        init: specification.sandbox.guest.init.value.clone(),
        environment,
        unset_environment,
        arguments,
        stage_terminfo,
        network,
    })
}

pub(crate) fn network_config(
    specification: &EffectiveSpecification,
) -> Result<Option<NetworkConfig>> {
    let NetworkAccess::Enabled(configuration) = &specification.sandbox.network.value else {
        return Ok(None);
    };

    let policy = match &configuration.mode {
        NetworkMode::Profiles(profiles) => NetworkPolicy::from_profiles(profiles.iter().copied()),
        NetworkMode::None => NetworkPolicy::none(),
        NetworkMode::AllowAll => NetworkPolicy::allow_all(),
        NetworkMode::Custom => NetworkPolicy {
            default_egress: configuration.default_egress.value,
            default_ingress: configuration.default_ingress.value,
            rules: configuration.rules.iter().map(|rule| rule.value.clone()).collect(),
        },
    };
    let ports = configuration.ports.iter().map(|port| port.value.clone()).collect();
    let mut config = NetworkConfig { policy, ports, ..NetworkConfig::default() };
    config.dns.nameservers =
        configuration.dns.servers.iter().map(|server| server.value.clone()).collect();
    config.dns.query_timeout_ms = configuration.dns.query_timeout_ms.value;
    config.dns.rebind_protection = configuration.dns.rebind_protection.value;
    config.max_connections = configuration.max_connections.value;
    config.trust_host_cas = configuration.trust_host_cas.value;
    config.interface.mac = configuration.interface.mac.value;
    config.interface.mtu = configuration.interface.mtu.value;
    config.interface.ipv4_address = configuration.interface.ipv4.value;
    config.interface.ipv4_pool = configuration
        .interface
        .ipv4_pool
        .value
        .as_deref()
        .map(str::parse)
        .transpose()
        .context("invalid validated IPv4 network pool")?;
    config.interface.ipv6_address = configuration.interface.ipv6.value;
    config.interface.ipv6_pool = configuration
        .interface
        .ipv6_pool
        .value
        .as_deref()
        .map(str::parse)
        .transpose()
        .context("invalid validated IPv6 network pool")?;
    config.tls = tls_network_config(&specification.path, configuration)?.tls;
    Ok(Some(config))
}

fn tls_network_config(
    test_source: &Path,
    configuration: &NetworkConfiguration,
) -> Result<NetworkConfig> {
    let mut builder = NetworkBuilder::new();
    if configuration.tls.enabled.value {
        builder = builder.tls(|mut tls| {
            tls = tls.intercepted_ports(
                configuration.tls.intercepted_ports.iter().map(|port| port.value).collect(),
            );
            for pattern in &configuration.tls.bypass {
                tls = tls.bypass(pattern.value.clone());
            }
            tls = tls
                .verify_upstream(configuration.tls.verify_upstream.value)
                .block_quic(configuration.tls.block_quic.value);
            for verification in &configuration.tls.scoped_verification {
                tls = tls.verify_upstream_for(
                    verification.value.pattern.clone(),
                    verification.value.verify,
                );
            }
            for certificate in &configuration.tls.upstream_ca_certificates {
                tls = tls.upstream_ca_cert(resolve_host_path(test_source, &certificate.value));
            }
            for certificate in &configuration.tls.scoped_upstream_ca_certificates {
                tls = tls.upstream_ca_cert_for(
                    certificate.value.pattern.clone(),
                    resolve_host_path(test_source, &certificate.value.path),
                );
            }
            if let Some(certificate) = &configuration.tls.intercept_ca_certificate.value {
                tls = tls.intercept_ca_cert(resolve_host_path(test_source, certificate));
            }
            if let Some(key) = &configuration.tls.intercept_ca_key.value {
                tls = tls.intercept_ca_key(resolve_host_path(test_source, key));
            }
            tls
        });
    }
    let mut config = builder.build().context("invalid network configuration")?;
    config.tls.cache.capacity = configuration.tls.cache_capacity.value;
    config.tls.cache.validity_hours = configuration.tls.validity_hours.value;
    Ok(config)
}

fn resolve_host_path(test_source: &Path, value: &str) -> PathBuf {
    let Some(suffix) = value.strip_prefix("{{src-base}}") else {
        return PathBuf::from(value);
    };
    let base = test_source.parent().unwrap_or_else(|| Path::new("."));
    base.join(suffix.strip_prefix('/').unwrap_or(suffix))
}

fn apply_test_color(arguments: &mut Vec<String>, color: ColorMode) -> bool {
    let options_end =
        arguments.iter().position(|argument| argument == "--").unwrap_or(arguments.len());
    let explicit_force_color =
        arguments[..options_end].iter().enumerate().find_map(|(index, argument)| {
            argument.strip_prefix("--color=").map(|value| value == "always").or_else(|| {
                (argument == "--color")
                    .then(|| arguments.get(index + 1).is_some_and(|value| value == "always"))
            })
        });
    if let Some(force_color) = explicit_force_color {
        force_color
    } else {
        arguments.insert(
            options_end,
            match color {
                ColorMode::Always => "--color=always".to_owned(),
                ColorMode::Never => "--color=never".to_owned(),
            },
        );
        color == ColorMode::Always
    }
}

async fn execute_in_microsandbox(config: &SandboxConfig) -> Result<ExecutionOutput> {
    let mut builder = Sandbox::builder(next_sandbox_name())
        .cpus(config.cpus)
        .memory(config.memory_mib)
        .max_duration(config.max_duration_secs)
        .shell(config.shell.clone())
        .quiet_logs()
        .ephemeral(true);

    builder = if let Some(network) = &config.network {
        let network = network.clone();
        builder.network(|_| NetworkBuilder::from_config(network))
    } else {
        builder.disable_network()
    };

    builder = match &config.rootfs {
        SandboxRootfs::Image { reference, pull_policy, root_disk_mib } => {
            builder.image(reference.clone()).root_disk(*root_disk_mib).pull_policy(*pull_policy)
        }
        SandboxRootfs::Snapshot { reference } => builder.from_snapshot(reference.clone()),
    };
    if let Some(user) = &config.user {
        builder = builder.user(user.clone());
    }
    if let Some(workdir) = &config.workdir {
        builder = builder.workdir(workdir.clone());
    }
    if let Some(init) = &config.init {
        builder = builder.init(init.clone());
    }

    let sandbox = builder.create().await.context("could not create Microsandbox VM")?;
    let execution = execute_test(&sandbox, config).await;
    let cleanup = sandbox.stop().await.context("could not stop Microsandbox VM");

    match (execution, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Err(error), Ok(())) => Err(error),
        (Ok(mut output), Err(error)) => {
            output.cleanup_error = Some(format!("{error:#}"));
            Ok(output)
        }
        (Err(execution), Err(cleanup)) => {
            Err(anyhow!("{execution:#} -- additionally, Microsandbox cleanup failed: {cleanup:#}"))
        }
    }
}

async fn execute_test(sandbox: &Sandbox, config: &SandboxConfig) -> Result<ExecutionOutput> {
    sandbox.fs().copy_from_host(&config.executable, GUEST_EXECUTABLE).await.with_context(|| {
        format!(
            "could not copy test executable `{}` into Microsandbox",
            config.executable.display()
        )
    })?;
    sandbox
        .fs()
        .set_stat(GUEST_EXECUTABLE, true, FsSetAttrs { mode: Some(0o755), ..FsSetAttrs::default() })
        .await
        .context("could not make the test executable runnable in Microsandbox")?;
    if config.stage_terminfo {
        sandbox
            .fs()
            .mkdir(terminfo::ENTRY_DIRECTORY)
            .await
            .context("could not create the private terminfo directory in Microsandbox")?;
        sandbox
            .fs()
            .write(terminfo::ENTRY_PATH, terminfo::ENTRY)
            .await
            .context("could not write the private terminfo entry in Microsandbox")?;
    }

    let output = if config.unset_environment.is_empty() {
        sandbox
            .exec_with(GUEST_EXECUTABLE, |exec| {
                exec.args(config.arguments.iter().cloned())
                    .envs(config.environment.iter().cloned())
                    .stdin_null()
            })
            .await
    } else {
        let script = format!("unset {}\nexec \"$@\"", config.unset_environment.join(" "));
        let arguments =
            ["-c".to_owned(), script, "cargo-xtest".to_owned(), GUEST_EXECUTABLE.to_owned()]
                .into_iter()
                .chain(config.arguments.iter().cloned());
        sandbox
            .exec_with(config.shell.clone(), |exec| {
                exec.args(arguments).envs(config.environment.iter().cloned()).stdin_null()
            })
            .await
    }
    .context("could not execute test in Microsandbox")?;

    Ok(ExecutionOutput {
        status: output.status().code,
        stdout: output.stdout_bytes().to_vec(),
        stderr: output.stderr_bytes().to_vec(),
        cleanup_error: None,
    })
}

fn next_sandbox_name() -> String {
    let id = NEXT_SANDBOX_ID.fetch_add(1, Ordering::Relaxed);
    format!("cargo-xtest-{}-{id}", std::process::id())
}

fn is_portable_shell_identifier(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes.next().is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}
