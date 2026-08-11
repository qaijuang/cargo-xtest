use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use anyhow::Error;
use clap::{Args, Parser, Subcommand};

use crate::{Diagnostics, explain_path, run_project};

#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo", disable_help_subcommand = true)]
enum CargoCli {
    Xtest(Arguments),
}

#[derive(Debug, Args)]
#[command(version, about, disable_help_subcommand = true)]
struct Arguments {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show the effective test specification.
    Explain {
        /// Rust integration-test file to explain.
        test_file: PathBuf,
    },
}

/// Execute command-line behavior and forward output as it becomes available.
///
/// `arguments` includes the executable name and Cargo's leading `xtest`
/// argument.
///
/// # Errors
///
/// Returns an error when writing to either output stream fails.
pub fn run_cli(
    arguments: impl IntoIterator<Item = OsString>,
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
) -> io::Result<u8> {
    let arguments = match CargoCli::try_parse_from(arguments) {
        Ok(CargoCli::Xtest(arguments)) => arguments,
        Err(error) => return claperr(&error, stdout, stderr),
    };

    let result = match arguments.command {
        None => run_project(stdout, stderr),
        Some(Command::Explain { test_file }) => explain_path(&test_file).and_then(|output| {
            stdout.write_all(output.as_bytes())?;
            Ok(0)
        }),
    };
    match result {
        Ok(status) => Ok(status),
        Err(error) => apperr(&error, stderr),
    }
}

fn claperr(
    error: &clap::Error,
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
) -> io::Result<u8> {
    let status = u8::try_from(error.exit_code()).unwrap_or(2);
    if error.use_stderr() {
        stderr.write_all(error.to_string().as_bytes())?;
    } else {
        stdout.write_all(error.to_string().as_bytes())?;
    }
    Ok(status)
}

fn apperr(error: &Error, stderr: &mut dyn io::Write) -> io::Result<u8> {
    let output = error
        .downcast_ref::<Diagnostics>()
        .map_or_else(|| format!("error: {error:#}\n"), ToString::to_string);
    stderr.write_all(output.as_bytes())?;
    Ok(1)
}
