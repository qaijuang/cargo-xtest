use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use microsandbox::sandbox::PullPolicy;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, SourceSpan};
use crate::directive::{Capability, Directive, LocatedDirective, Location};
use crate::helpers::AsStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Default,
    Directive(SourceSpan),
}

impl Origin {
    pub(crate) const fn line(self) -> Option<usize> {
        match self {
            Self::Default => None,
            Self::Directive(span) => Some(span.line),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setting<T> {
    pub(crate) value: T,
    pub(crate) origin: Origin,
}

impl<T> Setting<T> {
    fn default(value: T) -> Self {
        Self { value, origin: Origin::Default }
    }

    fn explicit(value: T, location: &Location) -> Self {
        Self { value, origin: Origin::Directive(location.span) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionProfile {
    SelfContainedLinuxMusl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageSource {
    BuiltInRuntime,
    Explicit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectiveRootfs {
    Image {
        source: Setting<ImageSource>,
        pull_policy: Setting<PullPolicy>,
        root_disk_mib: Setting<u32>,
    },
    Snapshot {
        reference: Setting<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Ephemeral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkAccess {
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureRetention {
    Destroy,
    Preserve,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnvironmentChange {
    Set { key: String, value: String },
    Unset(String),
}

impl EnvironmentChange {
    fn key(&self) -> &str {
        match self {
            Self::Set { key, .. } | Self::Unset(key) => key,
        }
    }

    fn same_operation(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Set { .. }, Self::Set { .. }) | (Self::Unset(_), Self::Unset(_))
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Applicability {
    pub(crate) ignore_test: Option<Setting<String>>,
    pub(crate) only: Vec<Setting<String>>,
    pub(crate) ignore: Vec<Setting<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Execution {
    pub(crate) profile: Setting<ExecutionProfile>,
    pub(crate) run_flags: Vec<Setting<Vec<String>>>,
    pub(crate) environment: Vec<Setting<EnvironmentChange>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestProcess {
    pub(crate) user: Setting<Option<String>>,
    pub(crate) workdir: Setting<Option<String>>,
    pub(crate) shell: Setting<String>,
    pub(crate) init: Setting<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sandbox {
    pub(crate) rootfs: EffectiveRootfs,
    pub(crate) cpus: Setting<u8>,
    pub(crate) memory_mib: Setting<u32>,
    pub(crate) max_duration_secs: Setting<u64>,
    pub(crate) lifecycle: Setting<Lifecycle>,
    pub(crate) network: Setting<NetworkAccess>,
    pub(crate) failure_retention: Setting<FailureRetention>,
    pub(crate) guest: GuestProcess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectiveSpecification {
    pub(crate) path: PathBuf,
    pub(crate) applicability: Applicability,
    pub(crate) execution: Execution,
    pub(crate) sandbox: Sandbox,
    pub(crate) capabilities: Vec<Setting<Capability>>,
}

#[derive(Debug, Clone)]
struct LocatedValue<T> {
    value: T,
    location: Location,
}

#[derive(Debug, Clone)]
enum RootfsChoice {
    Image(String),
    Snapshot(String),
}

impl RootfsChoice {
    const fn directive_name(&self) -> &'static str {
        match self {
            Self::Image(_) => "image",
            Self::Snapshot(_) => "from-snapshot",
        }
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn reduce(
    path: &Path,
    directives: Vec<LocatedDirective>,
    diagnostics: &mut Diagnostics,
) -> EffectiveSpecification {
    let mut ignore_test = None;
    let mut only = Vec::new();
    let mut only_seen = BTreeMap::new();
    let mut ignore = Vec::new();
    let mut ignore_seen = BTreeMap::new();
    let mut capabilities = Vec::new();
    let mut capability_seen = BTreeMap::new();
    let mut run_flags = Vec::new();
    let mut environment = Vec::new();
    let mut environment_seen: BTreeMap<String, LocatedValue<EnvironmentChange>> = BTreeMap::new();
    let mut rootfs: Option<LocatedValue<RootfsChoice>> = None;
    let mut pull_policy = None;
    let mut cpus = None;
    let mut memory = None;
    let mut root_disk = None;
    let mut max_duration = None;
    let mut user = None;
    let mut workdir = None;
    let mut shell = None;
    let mut init = None;
    let mut network = None;
    let mut failure_retention = None;

    for LocatedDirective { value, location } in directives {
        match value {
            Directive::IgnoreTest(reason) => {
                set_once(path, diagnostics, &mut ignore_test, "ignore-test", reason, location);
            }
            Directive::Only(predicate) => push_unique(
                path,
                diagnostics,
                &mut only_seen,
                &mut only,
                "only",
                predicate,
                &location,
            ),
            Directive::Ignore(predicate) => push_unique(
                path,
                diagnostics,
                &mut ignore_seen,
                &mut ignore,
                "ignore",
                predicate,
                &location,
            ),
            Directive::Capability(capability) => {
                if let Some(first) = capability_seen.get(&capability) {
                    diagnostics.push(duplicate_diagnostic(
                        path,
                        &format!("needs-{}", capability.as_str()),
                        &location,
                        first,
                    ));
                } else {
                    capability_seen.insert(capability, location.clone());
                    capabilities.push(Setting::explicit(capability, &location));
                }
            }
            Directive::RunFlags(flags) => {
                run_flags.push(Setting::explicit(flags, &location));
            }
            Directive::ExecEnv { key, value } => push_environment(
                path,
                diagnostics,
                &mut environment_seen,
                &mut environment,
                EnvironmentChange::Set { key, value },
                &location,
            ),
            Directive::UnsetExecEnv(key) => push_environment(
                path,
                diagnostics,
                &mut environment_seen,
                &mut environment,
                EnvironmentChange::Unset(key),
                &location,
            ),
            Directive::Image(image) => {
                set_rootfs(path, diagnostics, &mut rootfs, RootfsChoice::Image(image), location);
            }
            Directive::FromSnapshot(snapshot) => set_rootfs(
                path,
                diagnostics,
                &mut rootfs,
                RootfsChoice::Snapshot(snapshot),
                location,
            ),
            Directive::PullPolicy(policy) => {
                set_once(path, diagnostics, &mut pull_policy, "pull-policy", policy, location);
            }
            Directive::Cpus(value) => {
                set_once(path, diagnostics, &mut cpus, "cpus", value, location);
            }
            Directive::Memory(value) => {
                set_once(path, diagnostics, &mut memory, "memory", value, location);
            }
            Directive::RootDisk(value) => {
                set_once(path, diagnostics, &mut root_disk, "root-disk", value, location);
            }
            Directive::MaxDuration(value) => {
                set_once(path, diagnostics, &mut max_duration, "max-duration", value, location);
            }
            Directive::User(value) => {
                set_once(path, diagnostics, &mut user, "user", value, location);
            }
            Directive::Workdir(value) => {
                set_once(path, diagnostics, &mut workdir, "workdir", value, location);
            }
            Directive::Shell(value) => {
                set_once(path, diagnostics, &mut shell, "shell", value, location);
            }
            Directive::Init(value) => {
                set_once(path, diagnostics, &mut init, "init", value, location);
            }
            Directive::DisableNetwork => set_once(
                path,
                diagnostics,
                &mut network,
                "disable-network",
                NetworkAccess::Disabled,
                location,
            ),
            Directive::PreserveOnFailure => set_once(
                path,
                diagnostics,
                &mut failure_retention,
                "preserve-on-failure",
                FailureRetention::Preserve,
                location,
            ),
        }
    }

    if let Some(LocatedValue { value: RootfsChoice::Snapshot(_), location: snapshot_location }) =
        &rootfs
    {
        reject_snapshot_setting(
            path,
            diagnostics,
            "pull-policy",
            pull_policy.as_ref(),
            snapshot_location,
        );
        reject_snapshot_setting(
            path,
            diagnostics,
            "root-disk",
            root_disk.as_ref(),
            snapshot_location,
        );
    }

    let effective_rootfs = match rootfs {
        Some(LocatedValue { value: RootfsChoice::Snapshot(reference), location }) => {
            EffectiveRootfs::Snapshot { reference: Setting::explicit(reference, &location) }
        }
        Some(LocatedValue { value: RootfsChoice::Image(image), location }) => {
            EffectiveRootfs::Image {
                source: Setting::explicit(ImageSource::Explicit(image), &location),
                pull_policy: into_setting(pull_policy, PullPolicy::IfMissing),
                root_disk_mib: into_setting(root_disk, 4096),
            }
        }
        None => EffectiveRootfs::Image {
            source: Setting::default(ImageSource::BuiltInRuntime),
            pull_policy: into_setting(pull_policy, PullPolicy::IfMissing),
            root_disk_mib: into_setting(root_disk, 4096),
        },
    };

    EffectiveSpecification {
        path: path.to_owned(),
        applicability: Applicability {
            ignore_test: ignore_test.map(explicit_setting),
            only,
            ignore,
        },
        execution: Execution {
            profile: Setting::default(ExecutionProfile::SelfContainedLinuxMusl),
            run_flags,
            environment,
        },
        sandbox: Sandbox {
            rootfs: effective_rootfs,
            cpus: into_setting(cpus, 1),
            memory_mib: into_setting(memory, 512),
            max_duration_secs: into_setting(max_duration, 600),
            lifecycle: Setting::default(Lifecycle::Ephemeral),
            network: into_setting(network, NetworkAccess::Disabled),
            failure_retention: into_setting(failure_retention, FailureRetention::Destroy),
            guest: GuestProcess {
                user: into_optional_setting(user),
                workdir: into_optional_setting(workdir),
                shell: into_setting(shell, "/bin/sh".to_owned()),
                init: into_optional_setting(init),
            },
        },
        capabilities,
    }
}

fn set_once<T>(
    path: &Path,
    diagnostics: &mut Diagnostics,
    slot: &mut Option<LocatedValue<T>>,
    name: &str,
    value: T,
    location: Location,
) {
    if let Some(first) = slot {
        diagnostics.push(duplicate_diagnostic(path, name, &location, &first.location));
    } else {
        *slot = Some(LocatedValue { value, location });
    }
}

fn set_rootfs(
    path: &Path,
    diagnostics: &mut Diagnostics,
    slot: &mut Option<LocatedValue<RootfsChoice>>,
    value: RootfsChoice,
    location: Location,
) {
    if let Some(first) = slot {
        let current_name = value.directive_name();
        let first_name = first.value.directive_name();
        if current_name == first_name {
            diagnostics.push(duplicate_diagnostic(path, current_name, &location, &first.location));
        } else {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticCode::Conflict,
                    format!("`{current_name}` conflicts with the earlier `{first_name}` directive"),
                    path,
                    location.label("conflicting rootfs source"),
                )
                .related(first.location.span, "first rootfs source declared"),
            );
        }
    } else {
        *slot = Some(LocatedValue { value, location });
    }
}

fn push_unique(
    path: &Path,
    diagnostics: &mut Diagnostics,
    seen: &mut BTreeMap<String, Location>,
    output: &mut Vec<Setting<String>>,
    name: &str,
    value: String,
    location: &Location,
) {
    if let Some(first) = seen.get(&value) {
        diagnostics.push(duplicate_diagnostic(path, &format!("{name}-{value}"), location, first));
    } else {
        seen.insert(value.clone(), location.clone());
        output.push(Setting::explicit(value, location));
    }
}

fn push_environment(
    path: &Path,
    diagnostics: &mut Diagnostics,
    seen: &mut BTreeMap<String, LocatedValue<EnvironmentChange>>,
    output: &mut Vec<Setting<EnvironmentChange>>,
    change: EnvironmentChange,
    location: &Location,
) {
    let key = change.key().to_owned();
    if let Some(first) = seen.get(&key) {
        let (code, message) = if first.value.same_operation(&change) {
            (DiagnosticCode::Duplicate, format!("duplicate environment key `{key}`"))
        } else {
            (DiagnosticCode::Conflict, format!("environment key `{key}` is both set and unset"))
        };
        diagnostics.push(
            Diagnostic::new(code, message, path, location.label("conflicting environment change"))
                .related(first.location.span, "first environment change declared"),
        );
    } else {
        seen.insert(key, LocatedValue { value: change.clone(), location: location.clone() });
        output.push(Setting::explicit(change, location));
    }
}

fn reject_snapshot_setting<T>(
    path: &Path,
    diagnostics: &mut Diagnostics,
    name: &str,
    setting: Option<&LocatedValue<T>>,
    snapshot_location: &Location,
) {
    if let Some(setting) = setting {
        diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::Conflict,
                format!("`{name}` cannot be used with `from-snapshot`"),
                path,
                setting.location.label("not valid for snapshot rootfs"),
            )
            .related(snapshot_location.span, "snapshot rootfs declared"),
        );
    }
}

fn duplicate_diagnostic(
    path: &Path,
    name: &str,
    duplicate: &Location,
    first: &Location,
) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::Duplicate,
        format!("duplicate `{name}` directive"),
        path,
        duplicate.label("duplicate declaration"),
    )
    .related(first.span, "first declared")
    .help(format!("keep one `{name}` directive"))
}

fn into_setting<T>(value: Option<LocatedValue<T>>, default: T) -> Setting<T> {
    value.map_or_else(|| Setting::default(default), explicit_setting)
}

fn explicit_setting<T>(value: LocatedValue<T>) -> Setting<T> {
    Setting::explicit(value.value, &value.location)
}

fn into_optional_setting<T>(value: Option<LocatedValue<T>>) -> Setting<Option<T>> {
    value.map_or_else(
        || Setting::default(None),
        |value| Setting::explicit(Some(value.value), &value.location),
    )
}
