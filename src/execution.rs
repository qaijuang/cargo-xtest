use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use microsandbox::Sandbox;
use microsandbox::sandbox::{FsSetAttrs, PullPolicy};

use crate::directive::Capability;
use crate::model::{
    EffectiveRootfs, EffectiveSpecification, EnvironmentChange, FailureRetention, ImageSource,
};

const DEFAULT_IMAGE: &str =
    "alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce";
const GUEST_EXECUTABLE: &str = "/cargo-xtest-test";
static NEXT_SANDBOX_ID: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    for change in &specification.execution.environment {
        match &change.value {
            EnvironmentChange::Set { key, value } => {
                environment.push((key.clone(), value.clone()));
            }
            EnvironmentChange::Unset(key) => {
                if !is_portable_shell_identifier(key) {
                    bail!("`unset-exec-env` key `{key}` is not a portable shell identifier");
                }
                unset_environment.push(key.clone());
            }
        }
    }

    let arguments = specification
        .execution
        .run_flags
        .iter()
        .flat_map(|flags| flags.value.iter().cloned())
        .collect();

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
    })
}

async fn execute_in_microsandbox(config: &SandboxConfig) -> Result<ExecutionOutput> {
    let mut builder = Sandbox::builder(next_sandbox_name())
        .cpus(config.cpus)
        .memory(config.memory_mib)
        .max_duration(config.max_duration_secs)
        .shell(config.shell.clone())
        .disable_network()
        .quiet_logs()
        .ephemeral(true);

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
