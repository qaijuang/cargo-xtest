#![allow(
    clippy::multiple_crate_versions,
    reason = "Microsandbox SDK currently contains transitive version splits that a downstream crate cannot safely unify."
)]

use std::process::ExitCode;
use std::{env, io};

use cargo_xtest::{CliOrRunOutput, run_cli};

fn main() -> ExitCode {
    let output = run_cli(env::args_os());
    let write_result = write_output(&output, &mut io::stdout(), &mut io::stderr());

    if write_result.is_err() { ExitCode::FAILURE } else { ExitCode::from(output.status) }
}

fn write_output(
    output: &CliOrRunOutput,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> io::Result<()> {
    stdout.write_all(output.stdout.as_bytes())?;
    stderr.write_all(output.stderr.as_bytes())
}
