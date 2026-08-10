use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Error;
use clap::{Args, Parser, Subcommand};

use crate::helpers::CliOrRunOutput;
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

/// Execute command-line behavior without writing to process streams.
///
/// `arguments` includes the executable name and Cargo's leading `xtest`
/// argument.
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> CliOrRunOutput {
    let arguments = match CargoCli::try_parse_from(arguments) {
        Ok(CargoCli::Xtest(arguments)) => arguments,
        Err(error) => return claperr(&error),
    };

    let result = match arguments.command {
        None => run_project(),
        Some(Command::Explain { test_file }) => explain_path(&test_file)
            .map(|stdout| CliOrRunOutput { stdout, stderr: String::new(), status: 0 }),
    };
    result.unwrap_or_else(|error| apperr(&error))
}

fn claperr(error: &clap::Error) -> CliOrRunOutput {
    let status = u8::try_from(error.exit_code()).unwrap_or(2);
    if error.use_stderr() {
        CliOrRunOutput { stdout: String::new(), stderr: error.to_string(), status }
    } else {
        CliOrRunOutput { stdout: error.to_string(), stderr: String::new(), status }
    }
}

fn apperr(error: &Error) -> CliOrRunOutput {
    let stderr = error
        .downcast_ref::<Diagnostics>()
        .map_or_else(|| format!("error: {error:#}\n"), ToString::to_string);
    CliOrRunOutput { stdout: String::new(), stderr, status: 1 }
}
