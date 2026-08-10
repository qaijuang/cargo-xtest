use std::ffi::OsString;
use std::io::Cursor;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use cargo_metadata::{Message, MetadataCommand, TargetKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestTarget {
    pub(crate) package_id: String,
    pub(crate) package_root: PathBuf,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactOutput {
    pub(crate) executable: PathBuf,
    pub(crate) diagnostics: String,
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
) -> CargoCommand {
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
            "--message-format=json-render-diagnostics",
            "--color=never",
            "--quiet",
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
        environment: vec![(OsString::from(guest.linker_environment), OsString::from("rust-lld"))],
        current_dir: target.package_root.clone(),
    }
}

pub(crate) fn parse_artifact_messages(
    messages: &str,
    target: &TestTarget,
) -> Result<ArtifactOutput> {
    let (diagnostics, mut executables) = scan_messages(messages, Some(target))?;

    if executables.len() != 1 {
        bail!(
            "Cargo reported {} matching executables for test target `{}` -- expected exactly one",
            executables.len(),
            target.name
        );
    }

    Ok(ArtifactOutput { executable: executables.remove(0), diagnostics })
}

pub(crate) fn rendered_diagnostics(messages: &str) -> Result<String> {
    scan_messages(messages, None).map(|(diagnostics, _)| diagnostics)
}

fn scan_messages(messages: &str, target: Option<&TestTarget>) -> Result<(String, Vec<PathBuf>)> {
    let mut diagnostics = String::new();
    let mut executables = Vec::new();

    for message in Message::parse_stream(Cursor::new(messages)) {
        match message.context("could not read Cargo JSON messages")? {
            Message::CompilerMessage(message) => {
                if let Some(rendered) = message.message.rendered {
                    diagnostics.push_str(&rendered);
                }
            }
            Message::CompilerArtifact(artifact) => {
                if let Some(target) = target
                    && artifact.package_id.repr == target.package_id
                    && artifact.target.name == target.name
                    && artifact.target.src_path.as_std_path() == target.source_path
                    && artifact.target.is_test()
                    && artifact.target.test
                    && artifact.profile.test
                {
                    let executable = artifact.executable.ok_or_else(|| {
                        anyhow!(
                            "Cargo artifact for test target `{}` did not include an executable",
                            target.name
                        )
                    })?;
                    executables.push(executable.into_std_path_buf());
                }
            }
            Message::TextLine(line) if !line.is_empty() => {
                bail!("invalid Cargo JSON message: {line}");
            }
            _ => {}
        }
    }
    Ok((diagnostics, executables))
}

pub(crate) fn discover_from_metadata(metadata: &str) -> Result<Vec<TestTarget>> {
    let metadata = MetadataCommand::parse(metadata).context("invalid Cargo metadata")?;
    if metadata.workspace_default_members.is_missing() {
        bail!("Cargo metadata did not report default workspace members");
    }
    let mut targets = Vec::new();

    for package in metadata.packages {
        if !metadata.workspace_default_members.contains(&package.id) {
            continue;
        }

        let package_root = package.manifest_path.parent().ok_or_else(|| {
            anyhow!(
                "invalid Cargo metadata: manifest path `{}` has no parent",
                package.manifest_path
            )
        })?;

        for target in package.targets {
            if !target.test || !target.kind.contains(&TargetKind::Test) {
                continue;
            }

            targets.push(TestTarget {
                package_id: package.id.repr.clone(),
                package_root: package_root.to_owned().into_std_path_buf(),
                name: target.name,
                source_path: target.src_path.into_std_path_buf(),
            });
        }
    }

    targets.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then_with(|| left.package_id.cmp(&right.package_id))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(targets)
}
