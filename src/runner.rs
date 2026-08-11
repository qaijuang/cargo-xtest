use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Error, Result};
use cargo_metadata::MetadataCommand;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::process::Command;

use crate::cargo::{
    ArtifactCollector, GuestTarget, TestTarget, cargo_test_command, discover_from_metadata,
    guest_target_for_arch,
};
use crate::execution::{ColorMode, Decision, ExecutionOutcome, decide, execute, sandbox_config};
use crate::helpers::write_live;
use crate::signal::{HostSignal, HostSignals};
use crate::{Diagnostics, load_path};

pub(crate) fn run_current_project(
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
) -> Result<u8> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not initialize the asynchronous runtime")?;

    runtime.block_on(async {
        let project_root =
            env::current_dir().context("could not determine the current directory")?;
        let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let guest = guest_target_for_arch(env::consts::ARCH)?;
        let mut signals = HostSignals::new().context("could not listen for termination signals")?;
        let color = resolve_color_mode(
            env::var_os("CARGO_TERM_COLOR").as_deref(),
            io::stdout().is_terminal(),
        );
        let mut metadata = MetadataCommand::new();
        metadata.cargo_path(cargo.clone()).current_dir(&project_root).no_deps();
        let mut metadata_command = Command::from(metadata.cargo_command());
        let Some(metadata) =
            run_process(&mut metadata_command, "Cargo metadata", None, stderr, &mut signals)
                .await?
        else {
            return Ok(1);
        };
        let metadata = match metadata {
            ProcessOutcome::Exited(metadata) => metadata,
            ProcessOutcome::Interrupted(signal) => return Ok(signal.exit_status()),
        };
        let metadata_status = status_code(metadata.status);
        if !metadata.status.success() {
            writeln!(stderr, "error: Cargo metadata failed with status {metadata_status}")?;
            return Ok(exit_status(metadata_status));
        }

        let metadata = match str::from_utf8(&metadata.stdout) {
            Ok(metadata) => metadata,
            Err(error) => {
                let error = Error::new(error).context("Cargo metadata was not UTF-8");
                writeln!(stderr, "error: {error:#}")?;
                return Ok(1);
            }
        };
        let metadata = match MetadataCommand::parse(metadata) {
            Ok(metadata) => metadata,
            Err(error) => {
                let error = Error::new(error).context("invalid Cargo metadata");
                writeln!(stderr, "error: {error:#}")?;
                return Ok(1);
            }
        };
        let targets = discover_from_metadata(&metadata);

        for target in &targets {
            if let ControlFlow::Break(status) =
                run_target(target, guest, color, &cargo, stdout, stderr, &mut signals).await?
            {
                return Ok(status);
            }
        }
        Ok(0)
    })
}

#[allow(clippy::too_many_lines)]
async fn run_target(
    target: &TestTarget,
    guest: GuestTarget,
    color: ColorMode,
    cargo: &OsString,
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
    signals: &mut HostSignals,
) -> Result<ControlFlow<u8>> {
    let specification = match load_path(&target.source_path) {
        Ok(specification) => specification,
        Err(error) => {
            if let Some(diagnostics) = error.downcast_ref::<Diagnostics>() {
                write_live(stderr, diagnostics.to_string().as_bytes())?;
            } else {
                writeln!(stderr, "error: {error:#}")?;
            }
            return Ok(ControlFlow::Break(1));
        }
    };
    match decide(&specification, guest_architecture(guest), guest.triple) {
        Decision::Run => {}
        Decision::Skip(reason) => {
            writeln!(stdout, "skipped {}: {reason}", target.source_path.display())?;
            stdout.flush()?;
            return Ok(ControlFlow::Continue(()));
        }
    }

    let cargo_specification =
        cargo_test_command(cargo.clone(), target, guest, color == ColorMode::Always);
    let mut cargo_command = Command::new(&cargo_specification.program);
    cargo_command
        .args(&cargo_specification.arguments)
        .envs(cargo_specification.environment.iter().cloned())
        .current_dir(&cargo_specification.current_dir);
    let Some(build) =
        run_process(&mut cargo_command, "Cargo test compilation", Some(target), stderr, signals)
            .await?
    else {
        return Ok(ControlFlow::Break(1));
    };
    let build = match build {
        ProcessOutcome::Exited(build) => build,
        ProcessOutcome::Interrupted(signal) => {
            return Ok(ControlFlow::Break(signal.exit_status()));
        }
    };
    let build_status = status_code(build.status);
    if !build.status.success() {
        writeln!(
            stderr,
            "error: Cargo failed to compile {} with status {build_status}",
            target.source_path.display()
        )?;
        return Ok(ControlFlow::Break(exit_status(build_status)));
    }

    let Some(executable) = build.executable else {
        writeln!(
            stderr,
            "error: Cargo did not report an executable for test target `{}`",
            target.name
        )?;
        return Ok(ControlFlow::Break(1));
    };

    let config = match sandbox_config(&executable, &specification, color) {
        Ok(plan) => plan,
        Err(error) => {
            writeln!(stderr, "{}: error: {error:#}", target.source_path.display())?;
            return Ok(ControlFlow::Break(1));
        }
    };
    writeln!(stdout, "running {}", target.source_path.display())?;
    stdout.flush()?;
    let execution = match execute(&config, stdout, stderr, signals)
        .await
        .context("could not run test in Microsandbox")
    {
        Ok(execution) => execution,
        Err(error) => {
            writeln!(stderr, "error: {error:#}")?;
            return Ok(ControlFlow::Break(1));
        }
    };
    let status = match execution.outcome {
        ExecutionOutcome::Exited(status) => status,
        ExecutionOutcome::Interrupted(signal) => {
            if let Some(error) = execution.cleanup_error {
                writeln!(stderr, "error: {error}")?;
            }
            return Ok(ControlFlow::Break(signal.exit_status()));
        }
    };
    if status == 0 {
        if let Some(error) = execution.cleanup_error {
            writeln!(stderr, "error: {error}")?;
            return Ok(ControlFlow::Break(1));
        }
        return Ok(ControlFlow::Continue(()));
    }

    if status == 101 {
        writeln!(stderr, "test {} failed with status 101", target.source_path.display())?;
    } else {
        writeln!(
            stderr,
            "error: Microsandbox failed while running {} with status {status}",
            target.source_path.display()
        )?;
    }
    if let Some(error) = execution.cleanup_error {
        writeln!(stderr, "error: {error}")?;
    }
    Ok(ControlFlow::Break(exit_status(status)))
}

#[derive(Debug)]
struct ProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    executable: Option<PathBuf>,
}

#[derive(Debug)]
enum ProcessOutcome {
    Exited(ProcessOutput),
    Interrupted(HostSignal),
}

async fn run_process(
    command: &mut Command,
    operation: &str,
    target: Option<&TestTarget>,
    stderr: &mut dyn io::Write,
    signals: &mut HostSignals,
) -> Result<Option<ProcessOutcome>> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = match command.spawn().with_context(|| format!("could not start {operation}")) {
        Ok(child) => child,
        Err(error) => {
            writeln!(stderr, "error: {error:#}")?;
            return Ok(None);
        }
    };
    let child_stdout = child.stdout.take().context("could not capture Cargo stdout")?;
    let mut child_stdout = BufReader::new(child_stdout);
    let mut child_stderr = child.stderr.take().context("could not capture Cargo stderr")?;
    let mut stdout = Vec::new();
    let mut stdout_line = Vec::new();
    let mut stderr_buffer = [0; 8192];
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut status: Option<ExitStatus> = None;
    let mut artifacts = target.map(ArtifactCollector::new);

    loop {
        if let Some(status) = status
            && !stdout_open
            && !stderr_open
        {
            let executable = match artifacts {
                Some(artifacts) if status.success() => Some(artifacts.finish()?),
                _ => None,
            };
            return Ok(Some(ProcessOutcome::Exited(ProcessOutput { status, stdout, executable })));
        }

        tokio::select! {
            read = child_stdout.read_until(b'\n', &mut stdout_line), if stdout_open => {
                let read = read.context("could not read Cargo stdout")?;
                if read == 0 {
                    stdout_open = false;
                } else if let Some(artifacts) = &mut artifacts {
                    if let Some(output) = artifacts.observe(&stdout_line)? {
                        write_live(stderr, &output)?;
                    }
                } else {
                    stdout.extend_from_slice(&stdout_line);
                }
                stdout_line.clear();
            }
            read = child_stderr.read(&mut stderr_buffer), if stderr_open => {
                let read = read.context("could not read Cargo stderr")?;
                if read == 0 {
                    stderr_open = false;
                } else {
                    write_live(stderr, &stderr_buffer[..read])?;
                }
            }
            waited = child.wait(), if status.is_none() => {
                status = Some(waited.context("could not wait for Cargo")?);
            }
            received = signals.receive() => {
                let received = received?;
                match child.start_kill() {
                    Ok(()) => {
                        if let Err(error) = child.wait().await {
                            writeln!(stderr, "error: could not wait for Cargo after interruption: {error}")?;
                        }
                    }
                    Err(error) if child.try_wait()?.is_none() => {
                        writeln!(stderr, "error: could not stop Cargo after interruption: {error}")?;
                    }
                    Err(_) => {}
                }
                return Ok(Some(ProcessOutcome::Interrupted(received)));
            }
        }
    }
}

fn guest_architecture(guest: GuestTarget) -> &'static str {
    guest.triple.split_once('-').map_or(guest.triple, |(architecture, _)| architecture)
}

fn resolve_color_mode(value: Option<&OsStr>, is_terminal: bool) -> ColorMode {
    match value.and_then(OsStr::to_str) {
        Some("always") => ColorMode::Always,
        Some("never") => ColorMode::Never,
        _ if is_terminal => ColorMode::Always,
        _ => ColorMode::Never,
    }
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn exit_status(status: i32) -> u8 {
    u8::try_from(status).unwrap_or(1)
}
