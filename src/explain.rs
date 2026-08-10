use std::fmt::{self, Write as _};

use crate::helpers::AsStr;
use crate::model::{
    EffectiveRootfs, EffectiveSpecification, EnvironmentChange, FailureRetention, GuestProcess,
    ImageSource, Origin, Sandbox, Setting,
};

pub(crate) fn render(specification: &EffectiveSpecification) -> Result<String, fmt::Error> {
    let mut output = String::new();
    writeln!(output, "test: {}", specification.path.display())?;
    writeln!(output)?;
    render_applicability(&mut output, specification)?;
    render_execution(&mut output, specification)?;
    render_sandbox(&mut output, &specification.sandbox)?;
    render_capabilities(&mut output, specification)?;
    Ok(output)
}

fn render_applicability(
    output: &mut String,
    specification: &EffectiveSpecification,
) -> fmt::Result {
    writeln!(output, "applicability:")?;
    if let Some(ignore_test) = &specification.applicability.ignore_test {
        writeln!(output, "  ignore-test: {} {}", ignore_test.value, origin(ignore_test.origin))?;
    } else {
        writeln!(output, "  ignore-test: no")?;
    }
    render_predicates(output, "only", &specification.applicability.only)?;
    render_predicates(output, "ignore", &specification.applicability.ignore)?;
    writeln!(output)
}

fn render_execution(output: &mut String, specification: &EffectiveSpecification) -> fmt::Result {
    writeln!(output, "execution:")?;
    writeln!(
        output,
        "  profile: self-contained-linux-musl {}",
        origin(specification.execution.profile.origin)
    )?;
    if specification.execution.run_flags.is_empty() {
        writeln!(output, "  run-flags: none")?;
    } else {
        for flags in &specification.execution.run_flags {
            writeln!(output, "  run-flags: {} {}", flags.value.join(" | "), origin(flags.origin))?;
        }
    }
    render_environment(output, specification)?;
    writeln!(output)
}

fn render_environment(output: &mut String, specification: &EffectiveSpecification) -> fmt::Result {
    if specification.execution.environment.is_empty() {
        return writeln!(output, "  environment: unchanged");
    }

    writeln!(output, "  environment:")?;
    for change in &specification.execution.environment {
        match &change.value {
            EnvironmentChange::Set { key, value } => {
                writeln!(output, "    - set {key}={value} {}", origin(change.origin))?;
            }
            EnvironmentChange::Unset(key) => {
                writeln!(output, "    - unset {key} {}", origin(change.origin))?;
            }
        }
    }
    Ok(())
}

fn render_sandbox(output: &mut String, sandbox: &Sandbox) -> fmt::Result {
    writeln!(output, "sandbox:")?;
    render_rootfs(output, &sandbox.rootfs)?;
    render_setting(output, "cpus", sandbox.cpus.value, sandbox.cpus.origin)?;
    render_setting(output, "memory-mib", sandbox.memory_mib.value, sandbox.memory_mib.origin)?;
    render_setting(
        output,
        "max-duration-secs",
        sandbox.max_duration_secs.value,
        sandbox.max_duration_secs.origin,
    )?;
    writeln!(output, "  lifecycle: ephemeral {}", origin(sandbox.lifecycle.origin))?;
    writeln!(output, "  network: disabled {}", origin(sandbox.network.origin))?;
    let retention = match sandbox.failure_retention.value {
        FailureRetention::Destroy => "destroy",
        FailureRetention::Preserve => "preserve",
    };
    writeln!(
        output,
        "  failure-retention: {retention} {}",
        origin(sandbox.failure_retention.origin)
    )?;
    render_guest(output, &sandbox.guest)?;
    writeln!(output)
}

fn render_rootfs(output: &mut String, rootfs: &EffectiveRootfs) -> fmt::Result {
    match rootfs {
        EffectiveRootfs::Image { source, pull_policy, root_disk_mib } => {
            match &source.value {
                ImageSource::BuiltInRuntime => {
                    writeln!(output, "  rootfs: built-in-runtime {}", origin(source.origin))?;
                }
                ImageSource::Explicit(image) => {
                    writeln!(output, "  rootfs: image {image} {}", origin(source.origin))?;
                }
            }
            writeln!(
                output,
                "  pull-policy: {} {}",
                pull_policy.value.as_str(),
                origin(pull_policy.origin)
            )?;
            render_setting(output, "root-disk-mib", root_disk_mib.value, root_disk_mib.origin)?;
        }
        EffectiveRootfs::Snapshot { reference } => {
            writeln!(
                output,
                "  rootfs: snapshot {} {}",
                reference.value,
                origin(reference.origin)
            )?;
            writeln!(output, "  pull-policy: not-applicable")?;
            writeln!(output, "  root-disk-mib: inherited-from-snapshot")?;
        }
    }
    Ok(())
}

fn render_guest(output: &mut String, guest: &GuestProcess) -> fmt::Result {
    render_setting(
        output,
        "user",
        guest.user.value.as_deref().unwrap_or("image-default"),
        guest.user.origin,
    )?;
    render_setting(
        output,
        "workdir",
        guest.workdir.value.as_deref().unwrap_or("image-default"),
        guest.workdir.origin,
    )?;
    render_setting(output, "shell", &guest.shell.value, guest.shell.origin)?;
    render_setting(output, "init", guest.init.value.as_deref().unwrap_or("none"), guest.init.origin)
}

fn render_capabilities(output: &mut String, specification: &EffectiveSpecification) -> fmt::Result {
    writeln!(output, "requires:")?;
    if specification.capabilities.is_empty() {
        writeln!(output, "  none")?;
    } else {
        for capability in &specification.capabilities {
            writeln!(output, "  - {} {}", capability.value.as_str(), origin(capability.origin))?;
        }
    }
    Ok(())
}

fn render_predicates(
    output: &mut String,
    name: &str,
    predicates: &[Setting<String>],
) -> fmt::Result {
    if predicates.is_empty() {
        writeln!(output, "  {name}: none")
    } else {
        let rendered = predicates
            .iter()
            .map(|predicate| format!("{} {}", predicate.value, origin(predicate.origin)))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(output, "  {name}: {rendered}")
    }
}

fn render_setting(
    output: &mut String,
    name: &str,
    value: impl fmt::Display,
    source: Origin,
) -> fmt::Result {
    writeln!(output, "  {name}: {value} {}", origin(source))
}

fn origin(origin: Origin) -> String {
    origin.line().map_or_else(|| "[default]".to_owned(), |line| format!("[line {line}]"))
}
