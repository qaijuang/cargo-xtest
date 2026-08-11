use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use microsandbox::sandbox::PullPolicy;
use microsandbox_network::config::PublishedPort;
use microsandbox_network::dns::Nameserver;
use microsandbox_network::policy::{Action, Rule};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, SourceSpan};
use crate::directive::{
    Capability, Directive, LocatedDirective, Location, NetworkMode, ScopedCertificate,
    ScopedVerification,
};
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

#[derive(Debug, Clone)]
pub(crate) enum NetworkAccess {
    Disabled,
    Enabled(Box<NetworkConfiguration>),
}

#[derive(Debug, Clone)]
pub(crate) struct NetworkConfiguration {
    pub(crate) mode: NetworkMode,
    pub(crate) default_egress: Setting<Action>,
    pub(crate) default_ingress: Setting<Action>,
    pub(crate) rules: Vec<Setting<Rule>>,
    pub(crate) ports: Vec<Setting<PublishedPort>>,
    pub(crate) dns: DnsConfiguration,
    pub(crate) tls: TlsConfiguration,
    pub(crate) max_connections: Setting<Option<usize>>,
    pub(crate) trust_host_cas: Setting<bool>,
    pub(crate) interface: NetworkInterface,
}

#[derive(Debug, Clone)]
pub(crate) struct DnsConfiguration {
    pub(crate) servers: Vec<Setting<Nameserver>>,
    pub(crate) query_timeout_ms: Setting<u64>,
    pub(crate) rebind_protection: Setting<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct TlsConfiguration {
    pub(crate) enabled: Setting<bool>,
    pub(crate) intercepted_ports: Vec<Setting<u16>>,
    pub(crate) bypass: Vec<Setting<String>>,
    pub(crate) verify_upstream: Setting<bool>,
    pub(crate) scoped_verification: Vec<Setting<ScopedVerification>>,
    pub(crate) block_quic: Setting<bool>,
    pub(crate) upstream_ca_certificates: Vec<Setting<String>>,
    pub(crate) scoped_upstream_ca_certificates: Vec<Setting<ScopedCertificate>>,
    pub(crate) intercept_ca_certificate: Setting<Option<String>>,
    pub(crate) intercept_ca_key: Setting<Option<String>>,
    pub(crate) cache_capacity: Setting<usize>,
    pub(crate) validity_hours: Setting<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct NetworkInterface {
    pub(crate) mac: Setting<Option<[u8; 6]>>,
    pub(crate) mtu: Setting<Option<u16>>,
    pub(crate) ipv4: Setting<Option<Ipv4Addr>>,
    pub(crate) ipv4_pool: Setting<Option<String>>,
    pub(crate) ipv6: Setting<Option<Ipv6Addr>>,
    pub(crate) ipv6_pool: Setting<Option<String>>,
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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
    let mut network_configuration_location = None;
    let mut custom_network_location = None;
    let mut network_default_egress = None;
    let mut network_default_ingress = None;
    let mut network_rules = Vec::new();
    let mut published_ports = Vec::new();
    let mut dns_servers = Vec::new();
    let mut dns_query_timeout = None;
    let mut dns_rebind_protection = None;
    let mut tls_intercept = None;
    let mut tls_configuration_location = None;
    let mut tls_intercept_ports = Vec::new();
    let mut tls_bypass = Vec::new();
    let mut tls_verify_upstream = None;
    let mut tls_scoped_verification = Vec::new();
    let mut tls_block_quic = None;
    let mut tls_upstream_ca_certificates = Vec::new();
    let mut tls_scoped_upstream_ca_certificates = Vec::new();
    let mut tls_intercept_ca_certificate = None;
    let mut tls_intercept_ca_key = None;
    let mut tls_cache_capacity = None;
    let mut tls_validity_hours = None;
    let mut max_network_connections = None;
    let mut trust_host_cas = None;
    let mut network_mac = None;
    let mut network_mtu = None;
    let mut network_ipv4 = None;
    let mut network_ipv4_pool = None;
    let mut network_ipv6 = None;
    let mut network_ipv6_pool = None;
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
            Directive::Network(mode) => {
                set_once(path, diagnostics, &mut network, "network", mode, location);
            }
            Directive::NetworkDefaultEgress(action) => {
                remember_location(&mut network_configuration_location, &location);
                remember_location(&mut custom_network_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut network_default_egress,
                    "network-default-egress",
                    action,
                    location,
                );
            }
            Directive::NetworkDefaultIngress(action) => {
                remember_location(&mut network_configuration_location, &location);
                remember_location(&mut custom_network_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut network_default_ingress,
                    "network-default-ingress",
                    action,
                    location,
                );
            }
            Directive::NetworkRule(rule) => {
                remember_location(&mut network_configuration_location, &location);
                remember_location(&mut custom_network_location, &location);
                network_rules.push(Setting::explicit(rule, &location));
            }
            Directive::PublishPort(port) => {
                remember_location(&mut network_configuration_location, &location);
                published_ports.push(Setting::explicit(port, &location));
            }
            Directive::DnsServer(server) => {
                remember_location(&mut network_configuration_location, &location);
                dns_servers.push(Setting::explicit(server, &location));
            }
            Directive::DnsQueryTimeout(timeout) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut dns_query_timeout,
                    "dns-query-timeout",
                    timeout,
                    location,
                );
            }
            Directive::NoDnsRebindProtection => {
                remember_location(&mut network_configuration_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut dns_rebind_protection,
                    "no-dns-rebind-protection",
                    false,
                    location,
                );
            }
            Directive::TlsIntercept => {
                remember_location(&mut network_configuration_location, &location);
                set_once(path, diagnostics, &mut tls_intercept, "tls-intercept", true, location);
            }
            Directive::TlsInterceptPort(port) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                tls_intercept_ports.push(Setting::explicit(port, &location));
            }
            Directive::TlsBypass(pattern) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                tls_bypass.push(Setting::explicit(pattern, &location));
            }
            Directive::NoTlsVerifyUpstream => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                set_once(
                    path,
                    diagnostics,
                    &mut tls_verify_upstream,
                    "no-tls-verify-upstream",
                    false,
                    location,
                );
            }
            Directive::TlsVerifyUpstreamFor(verification) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                tls_scoped_verification.push(Setting::explicit(verification, &location));
            }
            Directive::NoTlsBlockQuic => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                set_once(
                    path,
                    diagnostics,
                    &mut tls_block_quic,
                    "no-tls-block-quic",
                    false,
                    location,
                );
            }
            Directive::TlsUpstreamCaCert(certificate) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                tls_upstream_ca_certificates.push(Setting::explicit(certificate, &location));
            }
            Directive::TlsUpstreamCaCertFor(certificate) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                tls_scoped_upstream_ca_certificates.push(Setting::explicit(certificate, &location));
            }
            Directive::TlsInterceptCaCert(certificate) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                set_once(
                    path,
                    diagnostics,
                    &mut tls_intercept_ca_certificate,
                    "tls-intercept-ca-cert",
                    certificate,
                    location,
                );
            }
            Directive::TlsInterceptCaKey(key) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                set_once(
                    path,
                    diagnostics,
                    &mut tls_intercept_ca_key,
                    "tls-intercept-ca-key",
                    key,
                    location,
                );
            }
            Directive::TlsCertCacheCapacity(capacity) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                set_once(
                    path,
                    diagnostics,
                    &mut tls_cache_capacity,
                    "tls-cert-cache-capacity",
                    capacity,
                    location,
                );
            }
            Directive::TlsCertValidityHours(hours) => {
                remember_tls_locations(
                    &mut network_configuration_location,
                    &mut tls_configuration_location,
                    &location,
                );
                set_once(
                    path,
                    diagnostics,
                    &mut tls_validity_hours,
                    "tls-cert-validity-hours",
                    hours,
                    location,
                );
            }
            Directive::MaxNetworkConnections(max) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut max_network_connections,
                    "max-network-connections",
                    max,
                    location,
                );
            }
            Directive::TrustHostCas => {
                remember_location(&mut network_configuration_location, &location);
                set_once(path, diagnostics, &mut trust_host_cas, "trust-host-cas", true, location);
            }
            Directive::NetworkMac(mac) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(path, diagnostics, &mut network_mac, "network-mac", mac, location);
            }
            Directive::NetworkMtu(mtu) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(path, diagnostics, &mut network_mtu, "network-mtu", mtu, location);
            }
            Directive::NetworkIpv4(address) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(path, diagnostics, &mut network_ipv4, "network-ipv4", address, location);
            }
            Directive::NetworkIpv4Pool(pool) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut network_ipv4_pool,
                    "network-ipv4-pool",
                    pool,
                    location,
                );
            }
            Directive::NetworkIpv6(address) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(path, diagnostics, &mut network_ipv6, "network-ipv6", address, location);
            }
            Directive::NetworkIpv6Pool(pool) => {
                remember_location(&mut network_configuration_location, &location);
                set_once(
                    path,
                    diagnostics,
                    &mut network_ipv6_pool,
                    "network-ipv6-pool",
                    pool,
                    location,
                );
            }
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

    validate_network_contract(
        path,
        diagnostics,
        network.as_ref(),
        network_configuration_location.as_ref(),
        custom_network_location.as_ref(),
        tls_intercept.as_ref(),
        tls_configuration_location.as_ref(),
        tls_intercept_ca_certificate.as_ref(),
        tls_intercept_ca_key.as_ref(),
    );

    let effective_network = network.map_or_else(
        || Setting::default(NetworkAccess::Disabled),
        |LocatedValue { value: mode, location }| {
            let intercepted_ports = if tls_intercept_ports.is_empty() {
                vec![Setting::default(443)]
            } else {
                tls_intercept_ports
            };
            Setting::explicit(
                NetworkAccess::Enabled(Box::new(NetworkConfiguration {
                    mode,
                    default_egress: into_setting(network_default_egress, Action::Deny),
                    default_ingress: into_setting(network_default_ingress, Action::Deny),
                    rules: network_rules,
                    ports: published_ports,
                    dns: DnsConfiguration {
                        servers: dns_servers,
                        query_timeout_ms: into_setting(dns_query_timeout, 5000),
                        rebind_protection: into_setting(dns_rebind_protection, true),
                    },
                    tls: TlsConfiguration {
                        enabled: into_setting(tls_intercept, false),
                        intercepted_ports,
                        bypass: tls_bypass,
                        verify_upstream: into_setting(tls_verify_upstream, true),
                        scoped_verification: tls_scoped_verification,
                        block_quic: into_setting(tls_block_quic, true),
                        upstream_ca_certificates: tls_upstream_ca_certificates,
                        scoped_upstream_ca_certificates: tls_scoped_upstream_ca_certificates,
                        intercept_ca_certificate: into_optional_setting(
                            tls_intercept_ca_certificate,
                        ),
                        intercept_ca_key: into_optional_setting(tls_intercept_ca_key),
                        cache_capacity: into_setting(tls_cache_capacity, 1000),
                        validity_hours: into_setting(tls_validity_hours, 24),
                    },
                    max_connections: into_optional_setting(max_network_connections),
                    trust_host_cas: into_setting(trust_host_cas, false),
                    interface: NetworkInterface {
                        mac: into_optional_setting(network_mac),
                        mtu: into_optional_setting(network_mtu),
                        ipv4: into_optional_setting(network_ipv4),
                        ipv4_pool: into_optional_setting(network_ipv4_pool),
                        ipv6: into_optional_setting(network_ipv6),
                        ipv6_pool: into_optional_setting(network_ipv6_pool),
                    },
                })),
                &location,
            )
        },
    );

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
            network: effective_network,
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

fn remember_location(slot: &mut Option<Location>, location: &Location) {
    if slot.is_none() {
        *slot = Some(location.clone());
    }
}

fn remember_tls_locations(
    network: &mut Option<Location>,
    tls: &mut Option<Location>,
    location: &Location,
) {
    remember_location(network, location);
    remember_location(tls, location);
}

#[allow(clippy::too_many_arguments)]
fn validate_network_contract(
    path: &Path,
    diagnostics: &mut Diagnostics,
    network: Option<&LocatedValue<NetworkMode>>,
    network_configuration: Option<&Location>,
    custom_configuration: Option<&Location>,
    tls_intercept: Option<&LocatedValue<bool>>,
    tls_configuration: Option<&Location>,
    intercept_ca_certificate: Option<&LocatedValue<String>>,
    intercept_ca_key: Option<&LocatedValue<String>>,
) {
    let Some(network) = network else {
        if let Some(location) = network_configuration {
            diagnostics.push(
                Diagnostic::new(
                    DiagnosticCode::Conflict,
                    "network configuration requires a `network` directive",
                    path,
                    location.label("network setting declared here"),
                )
                .help("add `//@ network` or `//@ network: value`"),
            );
        }
        return;
    };

    if !matches!(network.value, NetworkMode::Custom)
        && let Some(location) = custom_configuration
    {
        diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::Conflict,
                "custom policy settings require `network: custom`",
                path,
                location.label("custom policy setting declared here"),
            )
            .related(network.location.span, "network mode declared here")
            .help("write `//@ network: custom`"),
        );
    }

    if tls_intercept.is_none()
        && let Some(location) = tls_configuration
    {
        diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::Conflict,
                "TLS settings require `tls-intercept`",
                path,
                location.label("TLS setting declared here"),
            )
            .help("add `//@ tls-intercept`"),
        );
        return;
    }

    match (intercept_ca_certificate, intercept_ca_key) {
        (Some(certificate), None) => diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::Conflict,
                "`tls-intercept-ca-cert` requires `tls-intercept-ca-key`",
                path,
                certificate.location.label("certificate declared here"),
            )
            .help("provide both interception CA files or neither"),
        ),
        (None, Some(key)) => diagnostics.push(
            Diagnostic::new(
                DiagnosticCode::Conflict,
                "`tls-intercept-ca-key` requires `tls-intercept-ca-cert`",
                path,
                key.location.label("private key declared here"),
            )
            .help("provide both interception CA files or neither"),
        ),
        (Some(_), Some(_)) | (None, None) => {}
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
