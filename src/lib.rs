#![allow(
    clippy::multiple_crate_versions,
    reason = "Microsandbox SDK currently contains transitive version splits that a downstream crate cannot safely unify."
)]

mod cargo;
mod cli;
mod diagnostic;
mod directive;
mod execution;
mod explain;
mod helpers;
mod model;
mod runner;
mod signal;

use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
pub use cli::run_cli;
pub use diagnostic::Diagnostics;

fn run_project(
    arguments: &cli::TestArguments,
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
) -> Result<u8> {
    runner::run_current_project(arguments, stdout, stderr)
}

/// Parse and explain Rust test source supplied by a buffered reader.
///
/// # Errors
///
/// Returns an error when the reader fails, the source is not UTF-8, or the
/// source contains invalid directives. Directive diagnostics retain source
/// spans -- reader errors retain their I/O source.
pub fn explain_reader(path: &Path, reader: impl BufRead) -> Result<String> {
    let specification = load_reader(path, reader)?;
    Ok(explain::render(&specification)?)
}

/// Parse and explain a Rust test source file.
///
/// # Errors
///
/// Returns diagnostics when the file cannot be opened or read, or when its
/// directives are invalid.
pub fn explain_path(path: &Path) -> Result<String> {
    let specification = load_path(path)?;
    Ok(explain::render(&specification)?)
}

fn load_path(path: &Path) -> Result<model::EffectiveSpecification> {
    let file = File::open(path)
        .with_context(|| format!("could not open test source `{}`", path.display()))?;
    load_reader(path, BufReader::new(file))
}

fn load_reader(path: &Path, reader: impl BufRead) -> Result<model::EffectiveSpecification> {
    let directive::ParseOutput { directives, mut diagnostics } =
        directive::parse_reader(path, reader)
            .with_context(|| format!("could not read test source `{}`", path.display()))?;
    let result = model::reduce(path, directives, &mut diagnostics);
    diagnostics.sort();
    if diagnostics.is_empty() { Ok(result) } else { Err(diagnostics.into()) }
}
