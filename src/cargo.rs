use std::ffi::OsString;
use std::io::Cursor;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use cargo_metadata::Message;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledTest {
    pub(crate) package_id: String,
    pub(crate) name: String,
    pub(crate) source_path: PathBuf,
    pub(crate) executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkippedTest {
    pub(crate) package_id: String,
    pub(crate) name: String,
    pub(crate) source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TestArtifact {
    Run(CompiledTest),
    Skip(SkippedTest),
}

impl TestArtifact {
    fn sort_key(&self) -> (&PathBuf, &str, &str) {
        match self {
            Self::Run(test) => (&test.source_path, &test.package_id, &test.name),
            Self::Skip(test) => (&test.source_path, &test.package_id, &test.name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuestTarget {
    pub(crate) triple: &'static str,
    pub(crate) linker_environment: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoCommand {
    pub(crate) program: OsString,
    pub(crate) arguments: Vec<OsString>,
    pub(crate) environment: Vec<(OsString, OsString)>,
    pub(crate) current_dir: PathBuf,
}

#[derive(Debug, Default)]
pub(crate) struct ArtifactCollector {
    artifacts: Vec<TestArtifact>,
}

impl ArtifactCollector {
    pub(crate) fn observe(&mut self, line: &[u8]) -> Result<Option<Vec<u8>>> {
        if str::from_utf8(line).is_err() {
            return Ok(Some(line.to_vec()));
        }

        let Some(message) = Message::parse_stream(Cursor::new(line)).next() else {
            return Ok(None);
        };
        match message.context("could not read Cargo JSON message")? {
            Message::CompilerMessage(message) => {
                Ok(message.message.rendered.map(String::into_bytes))
            }
            Message::CompilerArtifact(artifact) if artifact.target.is_test() => {
                let executable = artifact.executable.ok_or_else(|| {
                    anyhow!(
                        "Cargo artifact for integration-test target `{}` did not include an executable",
                        artifact.target.name
                    )
                })?;
                self.artifacts.push(TestArtifact::Run(CompiledTest {
                    package_id: artifact.package_id.repr,
                    name: artifact.target.name,
                    source_path: artifact.target.src_path.into_std_path_buf(),
                    executable: executable.into_std_path_buf(),
                }));
                Ok(None)
            }
            Message::CompilerArtifact(artifact)
                if artifact.profile.test && artifact.executable.is_some() =>
            {
                self.artifacts.push(TestArtifact::Skip(SkippedTest {
                    package_id: artifact.package_id.repr,
                    name: artifact.target.name,
                    source_path: artifact.target.src_path.into_std_path_buf(),
                }));
                Ok(None)
            }
            Message::TextLine(_) => Ok(Some(line.to_vec())),
            _ => Ok(None),
        }
    }

    pub(crate) fn finish(mut self) -> Vec<TestArtifact> {
        self.artifacts.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        self.artifacts
    }
}

pub(crate) fn guest_target_for_arch(architecture: &str) -> Result<GuestTarget> {
    match architecture {
        "x86_64" => Ok(GuestTarget {
            triple: "x86_64-unknown-linux-musl",
            linker_environment: "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER",
        }),
        "aarch64" => Ok(GuestTarget {
            triple: "aarch64-unknown-linux-musl",
            linker_environment: "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER",
        }),
        unsupported => anyhow::bail!(
            "unsupported host architecture `{unsupported}` -- cargo-xtest supports x86_64 and aarch64"
        ),
    }
}

pub(crate) fn cargo_test_command(
    program: OsString,
    current_dir: PathBuf,
    guest: GuestTarget,
    color: OsString,
    user_arguments: &[OsString],
    selects_tests: bool,
) -> CargoCommand {
    let message_format = if color == "always" {
        "--message-format=json-diagnostic-rendered-ansi"
    } else {
        "--message-format=json"
    };
    let mut arguments = vec![OsString::from("test")];
    arguments.extend_from_slice(user_arguments);
    if !selects_tests {
        arguments.push(OsString::from("--tests"));
    }
    arguments.extend([OsString::from("--color"), color]);
    arguments.extend(
        ["--target", guest.triple, "--no-run", message_format].into_iter().map(OsString::from),
    );
    CargoCommand {
        program,
        arguments,
        environment: vec![(OsString::from(guest.linker_environment), OsString::from("rust-lld"))],
        current_dir,
    }
}
