use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use anyhow::Error;
use clap::error::ErrorKind;
use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};

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

    #[command(flatten)]
    test: TestArguments,
}

#[derive(Debug, Args)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent Cargo command-line flags are represented directly for Clap"
)]
pub(crate) struct TestArguments {
    /// Test only the specified package.
    #[arg(short = 'p', long = "package", value_name = "SPEC")]
    package: Vec<OsString>,

    /// Test every package in the workspace.
    #[arg(long)]
    workspace: bool,

    /// Deprecated alias for --workspace.
    #[arg(long)]
    all: bool,

    /// Exclude a package from a workspace test.
    #[arg(long, value_name = "SPEC")]
    exclude: Vec<OsString>,

    /// Activate features for the selected packages.
    #[arg(short = 'F', long, value_name = "FEATURES")]
    features: Vec<OsString>,

    /// Activate every available feature.
    #[arg(long)]
    all_features: bool,

    /// Do not activate default features.
    #[arg(long)]
    no_default_features: bool,

    /// Test only the specified integration-test target.
    #[arg(long, value_name = "NAME")]
    test: Vec<OsString>,

    /// Compile test artifacts without starting Microsandbox.
    #[arg(long)]
    no_run: bool,

    /// Continue after an integration-test executable fails.
    #[arg(long)]
    no_fail_fast: bool,

    /// Number of parallel Cargo build jobs.
    #[arg(short = 'j', long = "jobs", value_name = "N")]
    jobs: Option<OsString>,

    /// Build optimized test artifacts.
    #[arg(short = 'r', long)]
    release: bool,

    /// Build with the specified profile.
    #[arg(long, value_name = "NAME")]
    profile: Option<OsString>,

    /// Directory for generated Cargo artifacts.
    #[arg(long, value_name = "DIRECTORY")]
    target_dir: Option<OsString>,

    /// Write Cargo build timing information.
    #[arg(long, value_name = "FMTS", num_args = 0..=1, require_equals = true, default_missing_value = "")]
    timings: Option<OsString>,

    /// Do not print cargo-xtest or Cargo informational output.
    #[arg(short, long)]
    quiet: bool,

    /// Use verbose Cargo output. Repeat for more detail.
    #[arg(short = 'v', long, action = ArgAction::Count)]
    verbose: u8,

    /// Control colored Cargo and test output.
    #[arg(long, value_name = "WHEN")]
    color: Option<OsString>,

    /// Display Cargo's future-incompatibility report.
    #[arg(long)]
    future_incompat_report: bool,

    /// Override a Cargo configuration value.
    #[arg(long, value_name = "KEY=VALUE|PATH")]
    config: Vec<OsString>,

    /// Pass an unstable option to Cargo.
    #[arg(short = 'Z', value_name = "FLAG")]
    unstable: Vec<OsString>,

    /// Path to Cargo.toml.
    #[arg(short = 'm', long, value_name = "PATH")]
    manifest_path: Option<OsString>,

    /// Ignore package rust-version requirements.
    #[arg(long)]
    ignore_rust_version: bool,

    /// Require Cargo.lock to remain unchanged.
    #[arg(long)]
    locked: bool,

    /// Prevent Cargo from accessing the network.
    #[arg(long)]
    offline: bool,

    /// Require Cargo.lock to remain unchanged and stay offline.
    #[arg(long)]
    frozen: bool,

    /// Filter tests by name.
    #[arg(value_name = "TESTNAME")]
    test_filter: Option<String>,

    /// Arguments passed to each integration-test executable.
    #[arg(last = true, value_name = "ARGS")]
    libtest_arguments: Vec<String>,

    #[command(flatten)]
    unsupported: UnsupportedArguments,
}

impl TestArguments {
    pub(crate) fn cargo_arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::new();
        push_values(&mut arguments, "--package", &self.package);
        push_flag(&mut arguments, "--workspace", self.workspace);
        push_flag(&mut arguments, "--all", self.all);
        push_values(&mut arguments, "--exclude", &self.exclude);
        push_values(&mut arguments, "--features", &self.features);
        push_flag(&mut arguments, "--all-features", self.all_features);
        push_flag(&mut arguments, "--no-default-features", self.no_default_features);
        push_values(&mut arguments, "--test", &self.test);
        push_option(&mut arguments, "--jobs", self.jobs.as_ref());
        push_flag(&mut arguments, "--release", self.release);
        push_option(&mut arguments, "--profile", self.profile.as_ref());
        push_option(&mut arguments, "--target-dir", self.target_dir.as_ref());
        if let Some(formats) = &self.timings {
            if formats.is_empty() {
                arguments.push(OsString::from("--timings"));
            } else {
                let mut argument = OsString::from("--timings=");
                argument.push(formats);
                arguments.push(argument);
            }
        }
        if self.quiet {
            arguments.push(OsString::from("--quiet"));
        }
        arguments.extend((0..self.verbose).map(|_| OsString::from("--verbose")));
        push_option(&mut arguments, "--color", self.color.as_ref());
        push_flag(&mut arguments, "--future-incompat-report", self.future_incompat_report);
        push_values(&mut arguments, "--config", &self.config);
        push_values(&mut arguments, "-Z", &self.unstable);
        push_option(&mut arguments, "--manifest-path", self.manifest_path.as_ref());
        push_flag(&mut arguments, "--ignore-rust-version", self.ignore_rust_version);
        push_flag(&mut arguments, "--locked", self.locked);
        push_flag(&mut arguments, "--offline", self.offline);
        push_flag(&mut arguments, "--frozen", self.frozen);
        arguments
    }

    pub(crate) fn selects_tests(&self) -> bool {
        !self.test.is_empty()
    }

    pub(crate) fn compile_only(&self) -> bool {
        self.no_run
    }

    pub(crate) fn keep_going(&self) -> bool {
        self.no_fail_fast
    }

    pub(crate) fn quiet(&self) -> bool {
        self.quiet
    }

    pub(crate) fn color(&self) -> Option<&std::ffi::OsStr> {
        self.color.as_deref()
    }

    pub(crate) fn guest_arguments(&self) -> Vec<String> {
        self.test_filter.iter().cloned().chain(self.libtest_arguments.iter().cloned()).collect()
    }

    pub(crate) fn unsupported_argument(&self) -> Option<&'static str> {
        self.unsupported.argument()
    }
}

#[derive(Debug, Args)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "unsupported Cargo selectors must parse as independent flags before policy errors"
)]
struct UnsupportedArguments {
    #[arg(long, hide = true)]
    lib: bool,
    #[arg(long, hide = true)]
    bins: bool,
    #[arg(long, hide = true)]
    bin: Vec<OsString>,
    #[arg(long, hide = true)]
    examples: bool,
    #[arg(long, hide = true)]
    example: Vec<OsString>,
    #[arg(long, hide = true)]
    tests: bool,
    #[arg(long, hide = true)]
    benches: bool,
    #[arg(long, hide = true)]
    bench: Vec<OsString>,
    #[arg(long, hide = true)]
    all_targets: bool,
    #[arg(long, hide = true)]
    doc: bool,
    #[arg(long, hide = true)]
    target: Vec<OsString>,
    #[arg(long, hide = true)]
    message_format: Vec<OsString>,
    #[arg(long, hide = true)]
    unit_graph: bool,
}

impl UnsupportedArguments {
    fn argument(&self) -> Option<&'static str> {
        [
            (self.lib, "--lib"),
            (self.bins, "--bins"),
            (!self.bin.is_empty(), "--bin"),
            (self.examples, "--examples"),
            (!self.example.is_empty(), "--example"),
            (self.tests, "--tests"),
            (self.benches, "--benches"),
            (!self.bench.is_empty(), "--bench"),
            (self.all_targets, "--all-targets"),
            (self.doc, "--doc"),
            (!self.target.is_empty(), "--target"),
            (!self.message_format.is_empty(), "--message-format"),
            (self.unit_graph, "--unit-graph"),
        ]
        .into_iter()
        .find_map(|(present, argument)| present.then_some(argument))
    }
}

fn push_flag(arguments: &mut Vec<OsString>, name: &'static str, present: bool) {
    if present {
        arguments.push(OsString::from(name));
    }
}

fn push_option(arguments: &mut Vec<OsString>, name: &'static str, value: Option<&OsString>) {
    if let Some(value) = value {
        arguments.push(OsString::from(name));
        arguments.push(value.clone());
    }
}

fn push_values(arguments: &mut Vec<OsString>, name: &'static str, values: &[OsString]) {
    for value in values {
        arguments.push(OsString::from(name));
        arguments.push(value.clone());
    }
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
    if let Some(argument) = arguments.test.unsupported_argument() {
        let message = match argument {
            "--target" => {
                "`--target` is not supported: cargo-xtest selects the Linux-musl guest target"
                    .to_owned()
            }
            "--message-format" => {
                "`--message-format` is not supported: cargo-xtest reserves Cargo's JSON output"
                    .to_owned()
            }
            "--unit-graph" => {
                "`--unit-graph` is not supported: cargo-xtest requires compiled test artifacts"
                    .to_owned()
            }
            _ => format!(
                "`{argument}` is not supported: cargo-xtest runs integration-test targets only"
            ),
        };
        return claperr(&validation_error(message), stdout, stderr);
    }

    let result = match arguments.command {
        None => run_project(&arguments.test, stdout, stderr),
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

fn validation_error(message: impl Into<String>) -> clap::Error {
    let message = message.into();
    let mut command = CargoCli::command();
    command.find_subcommand_mut("xtest").map_or_else(
        || clap::Error::raw(ErrorKind::InvalidValue, &message),
        |command| {
            command.set_bin_name("cargo xtest");
            command.error(ErrorKind::InvalidValue, &message)
        },
    )
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
