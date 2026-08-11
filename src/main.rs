#![allow(
    clippy::multiple_crate_versions,
    reason = "Microsandbox SDK currently contains transitive version splits that a downstream crate cannot safely unify."
)]

use std::process::ExitCode;
use std::{env, io};

use anyhow::{Context, Result};
use cargo_xtest::run_cli;

fn main() -> Result<ExitCode> {
    let status = run_cli(env::args_os(), &mut io::stdout().lock(), &mut io::stderr().lock())
        .context("could not write cargo-xtest output")?;
    Ok(ExitCode::from(status))
}
