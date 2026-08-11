use std::ffi::OsString;
use std::io::Cursor;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use cargo_metadata::{Message, Metadata};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestTarget {
    pub(crate) package_id: String,
    pub(crate) workspace_root: PathBuf,
    pub(crate) name: String,
    pub(crate) source_path: PathBuf,
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

#[derive(Debug)]
pub(crate) struct ArtifactCollector<'a> {
    target: &'a TestTarget,
    executables: Vec<PathBuf>,
}

impl<'a> ArtifactCollector<'a> {
    pub(crate) fn new(target: &'a TestTarget) -> Self {
        Self { target, executables: Vec::new() }
    }

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
            Message::CompilerArtifact(artifact)
                if artifact.package_id.repr == self.target.package_id
                    && artifact.target.name == self.target.name
                    && artifact.target.src_path.as_std_path() == self.target.source_path
                    && artifact.target.is_test() =>
            {
                let executable = artifact.executable.ok_or_else(|| {
                    anyhow!(
                        "Cargo artifact for test target `{}` did not include an executable",
                        self.target.name
                    )
                })?;
                self.executables.push(executable.into_std_path_buf());
                Ok(None)
            }
            Message::TextLine(_) => Ok(Some(line.to_vec())),
            _ => Ok(None),
        }
    }

    pub(crate) fn finish(mut self) -> Result<PathBuf> {
        if self.executables.len() != 1 {
            bail!(
                "Cargo reported {} matching executables for test target `{}` -- expected exactly one",
                self.executables.len(),
                self.target.name
            );
        }
        Ok(self.executables.remove(0))
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
        unsupported => bail!(
            "unsupported host architecture `{unsupported}` -- cargo-xtest supports x86_64 and aarch64"
        ),
    }
}

pub(crate) fn cargo_test_command(
    program: OsString,
    target: &TestTarget,
    guest: GuestTarget,
    color: bool,
) -> CargoCommand {
    let message_format = if color {
        "--message-format=json-diagnostic-rendered-ansi"
    } else {
        "--message-format=json"
    };
    CargoCommand {
        program,
        arguments: [
            "test",
            "--package",
            &target.package_id,
            "--test",
            &target.name,
            "--target",
            guest.triple,
            "--no-run",
            message_format,
            "--quiet",
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
        environment: vec![(OsString::from(guest.linker_environment), OsString::from("rust-lld"))],
        current_dir: target.workspace_root.clone(),
    }
}

pub(crate) fn discover_from_metadata(metadata: &Metadata) -> Vec<TestTarget> {
    let workspace_root = metadata.workspace_root.clone().into_std_path_buf();
    let mut targets = Vec::new();

    for package in metadata.workspace_default_packages() {
        for target in &package.targets {
            if !target.test || !target.is_test() {
                continue;
            }

            targets.push(TestTarget {
                package_id: package.id.repr.clone(),
                workspace_root: workspace_root.clone(),
                name: target.name.clone(),
                source_path: target.src_path.clone().into_std_path_buf(),
            });
        }
    }

    targets.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then_with(|| left.package_id.cmp(&right.package_id))
            .then_with(|| left.name.cmp(&right.name))
    });
    targets
}
