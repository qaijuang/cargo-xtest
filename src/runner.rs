use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal};
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use tokio::process::Command;

use crate::cargo::{
    ArtifactCollector, CompiledTest, GuestTarget, TestArtifact, cargo_test_command,
    guest_target_for_arch,
};
use crate::cli::TestArguments;
use crate::execution::{ColorMode, Decision, ExecutionOutcome, decide, execute, sandbox_config};
use crate::helpers::write_live;
use crate::signal::{HostSignal, HostSignals};
use crate::{Diagnostics, load_path};

pub(crate) fn run_current_project(
    arguments: &TestArguments,
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
        let cargo_term_color = env::var_os("CARGO_TERM_COLOR");
        let requested_color = arguments.color().or(cargo_term_color.as_deref());
        let test_color = resolve_color_mode(requested_color, io::stdout().is_terminal());
        let cargo_color = match requested_color {
            Some(value) if value != "auto" => value.to_os_string(),
            _ => match resolve_color_mode(requested_color, io::stderr().is_terminal()) {
                ColorMode::Always => OsString::from("always"),
                ColorMode::Never => OsString::from("never"),
            },
        };
        let guest_arguments = arguments.guest_arguments();
        let cargo_specification = cargo_test_command(
            cargo,
            project_root.clone(),
            guest,
            cargo_color,
            &arguments.cargo_arguments(),
            arguments.selects_tests(),
        );
        let mut cargo_command = Command::new(&cargo_specification.program);
        cargo_command
            .args(&cargo_specification.arguments)
            .envs(cargo_specification.environment.iter().cloned())
            .current_dir(&cargo_specification.current_dir);
        let Some(build) =
            run_process(&mut cargo_command, "Cargo test compilation", stderr, &mut signals).await?
        else {
            return Ok(1);
        };
        let build = match build {
            ProcessOutcome::Exited(build) => build,
            ProcessOutcome::Interrupted(signal) => return Ok(signal.exit_status()),
        };
        if !build.status.success() {
            return Ok(exit_status(status_code(build.status)));
        }
        if arguments.compile_only() {
            return Ok(0);
        }

        let target_options = TargetOptions {
            guest,
            color: test_color,
            test_arguments: &guest_arguments,
            quiet: arguments.quiet(),
        };
        let mut test_failed = false;
        for artifact in build.artifacts {
            match artifact {
                TestArtifact::Skip(target) => {
                    if !arguments.quiet() {
                        let source_path = target
                            .source_path
                            .strip_prefix(&project_root)
                            .unwrap_or(&target.source_path);
                        writeln!(
                            stdout,
                            "skipped {}: cargo-xtest runs integration-test targets only",
                            source_path.display()
                        )?;
                        stdout.flush()?;
                    }
                }
                TestArtifact::Run(target) => {
                    match run_target(&target, &target_options, stdout, stderr, &mut signals).await?
                    {
                        TargetOutcome::Passed => {}
                        TargetOutcome::TestFailed if arguments.keep_going() => {
                            test_failed = true;
                        }
                        TargetOutcome::TestFailed => return Ok(101),
                        TargetOutcome::Stop(status) => return Ok(status),
                    }
                }
            }
        }
        Ok(if test_failed { 101 } else { 0 })
    })
}

enum TargetOutcome {
    Passed,
    TestFailed,
    Stop(u8),
}

struct TargetOptions<'a> {
    guest: GuestTarget,
    color: ColorMode,
    test_arguments: &'a [String],
    quiet: bool,
}

#[allow(clippy::too_many_lines)]
async fn run_target(
    target: &CompiledTest,
    options: &TargetOptions<'_>,
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
    signals: &mut HostSignals,
) -> Result<TargetOutcome> {
    let specification = match load_path(&target.source_path) {
        Ok(specification) => specification,
        Err(error) => {
            if let Some(diagnostics) = error.downcast_ref::<Diagnostics>() {
                write_live(stderr, diagnostics.to_string().as_bytes())?;
            } else {
                writeln!(stderr, "error: {error:#}")?;
            }
            return Ok(TargetOutcome::Stop(1));
        }
    };
    match decide(&specification, guest_architecture(options.guest), options.guest.triple) {
        Decision::Run => {}
        Decision::Skip(reason) => {
            if !options.quiet {
                writeln!(stdout, "skipped {}: {reason}", target.source_path.display())?;
                stdout.flush()?;
            }
            return Ok(TargetOutcome::Passed);
        }
    }

    let config = match sandbox_config(
        &target.executable,
        &specification,
        options.color,
        options.test_arguments,
    ) {
        Ok(config) => config,
        Err(error) => {
            writeln!(stderr, "{}: error: {error:#}", target.source_path.display())?;
            return Ok(TargetOutcome::Stop(1));
        }
    };
    if !options.quiet {
        writeln!(stdout, "running {}", target.source_path.display())?;
        stdout.flush()?;
    }
    let execution = match execute(&config, stdout, stderr, signals)
        .await
        .context("could not run test in Microsandbox")
    {
        Ok(execution) => execution,
        Err(error) => {
            writeln!(stderr, "error: {error:#}")?;
            return Ok(TargetOutcome::Stop(1));
        }
    };
    let status = match execution.outcome {
        ExecutionOutcome::Exited(status) => status,
        ExecutionOutcome::Interrupted(signal) => {
            if let Some(error) = execution.cleanup_error {
                writeln!(stderr, "error: {error}")?;
            }
            return Ok(TargetOutcome::Stop(signal.exit_status()));
        }
    };
    if status == 0 {
        if let Some(error) = execution.cleanup_error {
            writeln!(stderr, "error: {error}")?;
            return Ok(TargetOutcome::Stop(1));
        }
        return Ok(TargetOutcome::Passed);
    }

    writeln!(stderr, "test {} failed with status {status}", target.source_path.display())?;
    if let Some(error) = execution.cleanup_error {
        writeln!(stderr, "error: {error}")?;
        return Ok(TargetOutcome::Stop(1));
    }
    Ok(TargetOutcome::TestFailed)
}

#[derive(Debug)]
struct ProcessOutput {
    status: ExitStatus,
    artifacts: Vec<TestArtifact>,
}

#[derive(Debug)]
enum ProcessOutcome {
    Exited(ProcessOutput),
    Interrupted(HostSignal),
}

async fn run_process(
    command: &mut Command,
    operation: &str,
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
    let mut stdout_line = Vec::new();
    let mut stderr_buffer = [0; 8192];
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut status: Option<ExitStatus> = None;
    let mut artifacts = ArtifactCollector::default();

    loop {
        if let Some(status) = status
            && !stdout_open
            && !stderr_open
        {
            let artifacts = if status.success() { artifacts.finish() } else { Vec::new() };
            return Ok(Some(ProcessOutcome::Exited(ProcessOutput { status, artifacts })));
        }

        tokio::select! {
            read = child_stdout.read_until(b'\n', &mut stdout_line), if stdout_open => {
                let read = read.context("could not read Cargo stdout")?;
                if read == 0 {
                    stdout_open = false;
                } else if let Some(output) = artifacts.observe(&stdout_line)? {
                    write_live(stderr, &output)?;
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
