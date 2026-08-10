use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::process::{Command, ExitStatus, Output};

use anyhow::{Context, Error, Result};

use crate::cargo::{
    GuestTarget, TestTarget, cargo_test_command, discover_from_metadata, guest_target_for_arch,
    parse_artifact_messages, rendered_diagnostics,
};
use crate::execution::{Decision, decide, execute, sandbox_config};
use crate::{CliOrRunOutput, Diagnostics, load_path};

pub(crate) fn run_current_project() -> Result<CliOrRunOutput> {
    let project_root = env::current_dir().context("could not determine the current directory")?;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let guest = guest_target_for_arch(env::consts::ARCH)?;
    let mut result = CliOrRunOutput::default();
    let mut metadata_command = Command::new(&cargo);
    metadata_command
        .args(["metadata", "--format-version=1", "--no-deps", "--color=never"])
        .current_dir(&project_root);
    let Some(metadata) = start_process(&mut metadata_command, "Cargo metadata", &mut result)?
    else {
        return Ok(result);
    };
    append_bytes(&mut result.stderr, &metadata.stderr);
    let metadata_status = status_code(metadata.status);
    if !metadata.status.success() {
        result.status = exit_status(metadata_status);
        writeln!(result.stderr, "error: Cargo metadata failed with status {metadata_status}")?;
        return Ok(result);
    }

    let metadata = match str::from_utf8(&metadata.stdout) {
        Ok(metadata) => metadata,
        Err(error) => {
            let error = Error::new(error).context("Cargo metadata was not UTF-8");
            writeln!(result.stderr, "error: {error:#}")?;
            result.status = 1;
            return Ok(result);
        }
    };
    let targets = match discover_from_metadata(metadata) {
        Ok(targets) => targets,
        Err(error) => {
            writeln!(result.stderr, "error: {error:#}")?;
            result.status = 1;
            return Ok(result);
        }
    };

    for target in &targets {
        if !run_target(target, guest, &cargo, &mut result)? {
            break;
        }
    }
    Ok(result)
}

#[allow(clippy::too_many_lines)]
fn run_target(
    target: &TestTarget,
    guest: GuestTarget,
    cargo: &OsString,
    result: &mut CliOrRunOutput,
) -> Result<bool> {
    let specification = match load_path(&target.source_path) {
        Ok(specification) => specification,
        Err(error) => {
            if let Some(diagnostics) = error.downcast_ref::<Diagnostics>() {
                result.stderr.push_str(&diagnostics.to_string());
            } else {
                writeln!(result.stderr, "error: {error:#}")?;
            }
            result.status = 1;
            return Ok(false);
        }
    };
    match decide(&specification, guest_architecture(guest), guest.triple) {
        Ok(Decision::Run) => {}
        Ok(Decision::Skip(reason)) => {
            writeln!(result.stdout, "skipped {}: {reason}", target.source_path.display())?;
            return Ok(true);
        }
        Err(error) => {
            writeln!(result.stderr, "{}: error: {error:#}", target.source_path.display())?;
            result.status = 1;
            return Ok(false);
        }
    }

    let cargo_specification = cargo_test_command(cargo.clone(), target, guest);
    let mut cargo_command = Command::new(&cargo_specification.program);
    cargo_command
        .args(&cargo_specification.arguments)
        .envs(cargo_specification.environment.iter().cloned())
        .current_dir(&cargo_specification.current_dir);
    let Some(build) = start_process(&mut cargo_command, "Cargo test compilation", result)? else {
        return Ok(false);
    };
    let messages = match str::from_utf8(&build.stdout) {
        Ok(messages) => messages,
        Err(error) => {
            append_bytes(&mut result.stderr, &build.stderr);
            let error = Error::new(error).context("Cargo build output was not UTF-8");
            writeln!(result.stderr, "error: {error:#}")?;
            result.status = 1;
            return Ok(false);
        }
    };
    let build_status = status_code(build.status);
    if !build.status.success() {
        match rendered_diagnostics(messages) {
            Ok(diagnostics) => result.stderr.push_str(&diagnostics),
            Err(error) => writeln!(result.stderr, "error: {error:#}")?,
        }
        append_bytes(&mut result.stderr, &build.stderr);
        writeln!(
            result.stderr,
            "error: Cargo failed to compile {} with status {build_status}",
            target.source_path.display()
        )?;
        result.status = exit_status(build_status);
        return Ok(false);
    }

    let artifact = match parse_artifact_messages(messages, target) {
        Ok(artifact) => artifact,
        Err(error) => {
            append_bytes(&mut result.stderr, &build.stderr);
            writeln!(result.stderr, "error: {error:#}")?;
            result.status = 1;
            return Ok(false);
        }
    };
    result.stderr.push_str(&artifact.diagnostics);
    append_bytes(&mut result.stderr, &build.stderr);

    let config = match sandbox_config(&artifact.executable, &specification) {
        Ok(plan) => plan,
        Err(error) => {
            writeln!(result.stderr, "{}: error: {error:#}", target.source_path.display())?;
            result.status = 1;
            return Ok(false);
        }
    };
    writeln!(result.stdout, "running {}", target.source_path.display())?;
    let execution = match execute(&config).context("could not run test in Microsandbox") {
        Ok(execution) => execution,
        Err(error) => {
            writeln!(result.stderr, "error: {error:#}")?;
            result.status = 1;
            return Ok(false);
        }
    };
    append_bytes(&mut result.stdout, &execution.stdout);
    append_bytes(&mut result.stderr, &execution.stderr);
    if execution.status == 0 {
        if let Some(error) = execution.cleanup_error {
            writeln!(result.stderr, "error: {error}")?;
            result.status = 1;
            return Ok(false);
        }
        return Ok(true);
    }

    result.status = exit_status(execution.status);
    if execution.status == 101 {
        writeln!(result.stderr, "test {} failed with status 101", target.source_path.display())?;
    } else {
        writeln!(
            result.stderr,
            "error: Microsandbox failed while running {} with status {}",
            target.source_path.display(),
            execution.status
        )?;
    }
    if let Some(error) = execution.cleanup_error {
        writeln!(result.stderr, "error: {error}")?;
    }
    Ok(false)
}

fn start_process(
    command: &mut Command,
    operation: &str,
    result: &mut CliOrRunOutput,
) -> Result<Option<Output>> {
    match command.output().with_context(|| format!("could not start {operation}")) {
        Ok(output) => Ok(Some(output)),
        Err(error) => {
            writeln!(result.stderr, "error: {error:#}")?;
            result.status = 1;
            Ok(None)
        }
    }
}

fn guest_architecture(guest: GuestTarget) -> &'static str {
    guest.triple.split_once('-').map_or(guest.triple, |(architecture, _)| architecture)
}

fn append_bytes(output: &mut String, bytes: &[u8]) {
    output.push_str(&String::from_utf8_lossy(bytes));
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn exit_status(status: i32) -> u8 {
    u8::try_from(status).unwrap_or(1)
}
