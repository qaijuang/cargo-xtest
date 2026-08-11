use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use microsandbox::sandbox::PullPolicy;
use microsandbox_network::builder::NetworkBuilder;
use microsandbox_network::config::{PortProtocol, PublishedPort};
use microsandbox_network::dns::Nameserver;
use microsandbox_network::policy::{
    Action, DestinationGroup, Direction, NetworkPolicy, NetworkProfile, PortRange, Protocol, Rule,
};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, SourceLabel, SourceSpan};
use crate::helpers::AsStr;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirectiveForm {
    Presence,
    Value(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectiveLine {
    name: String,
    form: DirectiveForm,
    remark: Option<String>,
    span: SourceSpan,
    line_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Capability {
    Threads,
    Subprocess,
    Symlink,
    DynamicLinking,
    TargetStd,
    Unwind,
}

impl AsStr for Capability {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Threads => "threads",
            Self::Subprocess => "subprocess",
            Self::Symlink => "symlink",
            Self::DynamicLinking => "dynamic-linking",
            Self::TargetStd => "target-std",
            Self::Unwind => "unwind",
        }
    }
}

impl AsStr for PullPolicy {
    fn as_str(&self) -> &'static str {
        match self {
            Self::IfMissing => "if-missing",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Directive {
    IgnoreTest(String),
    Only(String),
    Ignore(String),
    Capability(Capability),
    RunFlags(Vec<String>),
    ExecEnv { key: String, value: String },
    UnsetExecEnv(String),
    Image(String),
    FromSnapshot(String),
    PullPolicy(PullPolicy),
    Cpus(u8),
    Memory(u32),
    RootDisk(u32),
    MaxDuration(u64),
    User(String),
    Workdir(String),
    Shell(String),
    Init(String),
    Network(NetworkMode),
    NetworkDefaultEgress(Action),
    NetworkDefaultIngress(Action),
    NetworkRule(Rule),
    PublishPort(PublishedPort),
    DnsServer(Nameserver),
    DnsQueryTimeout(u64),
    NoDnsRebindProtection,
    TlsIntercept,
    TlsInterceptPort(u16),
    TlsBypass(String),
    NoTlsVerifyUpstream,
    TlsVerifyUpstreamFor(ScopedVerification),
    NoTlsBlockQuic,
    TlsUpstreamCaCert(String),
    TlsUpstreamCaCertFor(ScopedCertificate),
    TlsInterceptCaCert(String),
    TlsInterceptCaKey(String),
    TlsCertCacheCapacity(usize),
    TlsCertValidityHours(u64),
    MaxNetworkConnections(usize),
    TrustHostCas,
    NetworkMac([u8; 6]),
    NetworkMtu(u16),
    NetworkIpv4(Ipv4Addr),
    NetworkIpv4Pool(String),
    NetworkIpv6(Ipv6Addr),
    NetworkIpv6Pool(String),
    PreserveOnFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkMode {
    Profiles(Vec<NetworkProfile>),
    None,
    AllowAll,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedVerification {
    pub(crate) pattern: String,
    pub(crate) verify: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedCertificate {
    pub(crate) pattern: String,
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Location {
    pub(crate) span: SourceSpan,
    pub(crate) line_text: String,
}

impl Location {
    pub(crate) fn label(&self, message: impl Into<String>) -> SourceLabel {
        SourceLabel {
            span: self.span,
            line_text: self.line_text.clone(),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LocatedDirective {
    pub(crate) value: Directive,
    pub(crate) location: Location,
}

pub(crate) struct ParseOutput {
    pub(crate) directives: Vec<LocatedDirective>,
    pub(crate) diagnostics: Diagnostics,
}

pub(crate) fn parse_reader(path: &Path, mut reader: impl BufRead) -> io::Result<ParseOutput> {
    let mut directives = Vec::new();
    let mut diagnostics = Diagnostics::default();
    let mut line = String::new();
    let mut line_number = 0;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_number += 1;
        let source_line = line.strip_suffix('\n').unwrap_or(&line);
        if let Some(directive_line) = scan_line(line_number, source_line) {
            match parse_directive(path, &directive_line) {
                Ok(directive) => directives.push(directive),
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
    }

    Ok(ParseOutput { directives, diagnostics })
}

fn scan_line(line_number: usize, line: &str) -> Option<DirectiveLine> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let content = line.trim_start();
    let marker_offset = line.len() - content.len();
    let after_marker = content.strip_prefix("//@")?;
    let directive = after_marker.trim_start();
    let directive_offset = after_marker.len() - directive.len();
    let name_end = directive
        .find(|character: char| character == ':' || character.is_whitespace())
        .unwrap_or(directive.len());
    let name = &directive[..name_end];
    let remainder = &directive[name_end..];

    let (form, remark) = if let Some(value) = remainder.strip_prefix(':') {
        (DirectiveForm::Value(value.trim().to_owned()), None)
    } else {
        let remark = remainder.trim();
        let remark = (!remark.is_empty()).then(|| remark.to_owned());
        (DirectiveForm::Presence, remark)
    };

    Some(DirectiveLine {
        name: name.to_owned(),
        form,
        remark,
        span: SourceSpan {
            line: line_number,
            column: marker_offset + "//@".len() + directive_offset + 1,
            length: name.len().max(1),
        },
        line_text: line.to_owned(),
    })
}

fn parse_directive(path: &Path, line: &DirectiveLine) -> Result<LocatedDirective, Diagnostic> {
    let location = Location { span: line.span, line_text: line.line_text.clone() };
    if line.name.starts_with('[') {
        return Err(diagnostic(
            DiagnosticCode::Unsupported,
            "revision-qualified directives are not supported",
            path,
            &location,
        )
        .help("use separate Cargo test targets or library-level cases"));
    }
    if line.name.is_empty() {
        return Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            "directive name is missing",
            path,
            &location,
        ));
    }

    if matches!(line.form, DirectiveForm::Presence)
        && line
            .remark
            .as_deref()
            .and_then(|remark| remark.split_whitespace().next())
            .is_some_and(is_known_name)
    {
        return Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            "a directive must be the only directive on its line",
            path,
            &location,
        )
        .help("put each `//@` directive on its own line"));
    }

    let value = if let Some(value) = parse_test_directive(path, line, &location)? {
        value
    } else {
        parse_sandbox_directive(path, line, &location)?
    };

    Ok(LocatedDirective { value, location })
}

fn parse_test_directive(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Option<Directive>, Diagnostic> {
    let name = line.name.as_str();
    if name == "ignore-test" {
        return parse_ignore_test(path, line, location).map(Some);
    }
    if let Some(predicate) = name.strip_prefix("only-").filter(|value| !value.is_empty()) {
        expect_presence(path, line, location)?;
        return Ok(Some(Directive::Only(predicate.to_owned())));
    }
    if let Some(predicate) = name.strip_prefix("ignore-").filter(|value| !value.is_empty()) {
        expect_presence(path, line, location)?;
        return Ok(Some(Directive::Ignore(predicate.to_owned())));
    }

    let directive = match name {
        "needs-threads" => presence_capability(path, line, location, Capability::Threads)?,
        "needs-subprocess" => presence_capability(path, line, location, Capability::Subprocess)?,
        "needs-symlink" => presence_capability(path, line, location, Capability::Symlink)?,
        "needs-dynamic-linking" => {
            presence_capability(path, line, location, Capability::DynamicLinking)?
        }
        "needs-target-std" => presence_capability(path, line, location, Capability::TargetStd)?,
        "needs-unwind" => presence_capability(path, line, location, Capability::Unwind)?,
        "run-flags" => parse_run_flags(path, line, location)?,
        "exec-env" => parse_exec_env(path, line, location)?,
        "unset-exec-env" => {
            let key = expect_value(path, line, location)?;
            validate_environment_key(path, location, key)?;
            Directive::UnsetExecEnv(key.trim().to_owned())
        }
        _ => return Ok(None),
    };
    Ok(Some(directive))
}

#[allow(clippy::too_many_lines)]
fn parse_sandbox_directive(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let value = || expect_value(path, line, location);
    match line.name.as_str() {
        "image" => Ok(Directive::Image(value()?.to_owned())),
        "from-snapshot" => Ok(Directive::FromSnapshot(value()?.to_owned())),
        "pull-policy" => parse_pull_policy(path, line, location),
        "cpus" => Ok(Directive::Cpus(parse_positive(path, location, value()?, "cpus")?)),
        "memory" => Ok(Directive::Memory(parse_positive(path, location, value()?, "memory")?)),
        "root-disk" => {
            Ok(Directive::RootDisk(parse_positive(path, location, value()?, "root-disk")?))
        }
        "max-duration" => {
            Ok(Directive::MaxDuration(parse_positive(path, location, value()?, "max-duration")?))
        }
        "user" => Ok(Directive::User(value()?.to_owned())),
        "workdir" => {
            Ok(Directive::Workdir(expect_absolute_guest_path(path, line, location, "workdir")?))
        }
        "shell" => Ok(Directive::Shell(expect_absolute_guest_path(path, line, location, "shell")?)),
        "init" => parse_init(path, line, location),
        "network" => parse_network(path, line, location),
        "network-default-egress" => {
            parse_network_action(path, line, location).map(Directive::NetworkDefaultEgress)
        }
        "network-default-ingress" => {
            parse_network_action(path, line, location).map(Directive::NetworkDefaultIngress)
        }
        "network-rule" => parse_network_rule(path, line, location),
        "publish-port" => parse_published_port(path, line, location),
        "dns-server" => parse_dns_server(path, line, location),
        "dns-query-timeout" => Ok(Directive::DnsQueryTimeout(parse_integer(
            path,
            location,
            value()?,
            "dns-query-timeout",
        )?)),
        "no-dns-rebind-protection" => {
            expect_presence(path, line, location)?;
            Ok(Directive::NoDnsRebindProtection)
        }
        "tls-intercept" => {
            expect_presence(path, line, location)?;
            Ok(Directive::TlsIntercept)
        }
        "tls-intercept-port" => Ok(Directive::TlsInterceptPort(parse_integer(
            path,
            location,
            value()?,
            "tls-intercept-port",
        )?)),
        "tls-bypass" => Ok(Directive::TlsBypass(value()?.to_owned())),
        "no-tls-verify-upstream" => {
            expect_presence(path, line, location)?;
            Ok(Directive::NoTlsVerifyUpstream)
        }
        "tls-verify-upstream-for" => parse_scoped_verification(path, line, location),
        "no-tls-block-quic" => {
            expect_presence(path, line, location)?;
            Ok(Directive::NoTlsBlockQuic)
        }
        "tls-upstream-ca-cert" => {
            parse_host_path(path, location, value()?).map(Directive::TlsUpstreamCaCert)
        }
        "tls-upstream-ca-cert-for" => parse_scoped_certificate(path, line, location),
        "tls-intercept-ca-cert" => {
            parse_host_path(path, location, value()?).map(Directive::TlsInterceptCaCert)
        }
        "tls-intercept-ca-key" => {
            parse_host_path(path, location, value()?).map(Directive::TlsInterceptCaKey)
        }
        "tls-cert-cache-capacity" => Ok(Directive::TlsCertCacheCapacity(parse_positive(
            path,
            location,
            value()?,
            "tls-cert-cache-capacity",
        )?)),
        "tls-cert-validity-hours" => Ok(Directive::TlsCertValidityHours(parse_integer(
            path,
            location,
            value()?,
            "tls-cert-validity-hours",
        )?)),
        "max-network-connections" => Ok(Directive::MaxNetworkConnections(parse_integer(
            path,
            location,
            value()?,
            "max-network-connections",
        )?)),
        "trust-host-cas" => {
            expect_presence(path, line, location)?;
            Ok(Directive::TrustHostCas)
        }
        "network-mac" => parse_mac(path, line, location),
        "network-mtu" => {
            Ok(Directive::NetworkMtu(parse_integer(path, location, value()?, "network-mtu")?))
        }
        "network-ipv4" => parse_address::<Ipv4Addr>(path, line, location, "network-ipv4"),
        "network-ipv4-pool" => parse_ipv4_pool(path, line, location),
        "network-ipv6" => parse_address::<Ipv6Addr>(path, line, location, "network-ipv6"),
        "network-ipv6-pool" => parse_ipv6_pool(path, line, location),
        "preserve-on-failure" => {
            expect_presence(path, line, location)?;
            Ok(Directive::PreserveOnFailure)
        }
        name => Err(diagnostic(
            DiagnosticCode::UnknownDirective,
            format!("unknown directive `{name}`"),
            path,
            location,
        )),
    }
}

fn parse_network(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let mode = match &line.form {
        DirectiveForm::Presence => NetworkMode::Profiles(vec![NetworkProfile::Public]),
        DirectiveForm::Value(raw) if raw == "none" => NetworkMode::None,
        DirectiveForm::Value(raw) if raw == "allow-all" => NetworkMode::AllowAll,
        DirectiveForm::Value(raw) if raw == "custom" => NetworkMode::Custom,
        DirectiveForm::Value(raw) if raw.is_empty() => {
            return Err(invalid_value(path, location, "`network` value must not be empty"));
        }
        DirectiveForm::Value(raw) => {
            let mut profiles = Vec::new();
            for name in raw.split(',').map(str::trim) {
                let profile = match name {
                    "public" => NetworkProfile::Public,
                    "private" => NetworkProfile::Private,
                    "host" => NetworkProfile::Host,
                    _ => {
                        return Err(invalid_value(path, location, "invalid `network` value")
                            .help("expected `public`, `private`, `host`, `none`, `allow-all`, or `custom`"));
                    }
                };
                profiles.push(profile);
            }
            NetworkMode::Profiles(profiles)
        }
    };
    Ok(Directive::Network(mode))
}

fn parse_network_action(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Action, Diagnostic> {
    match expect_value(path, line, location)? {
        "allow" => Ok(Action::Allow),
        "deny" => Ok(Action::Deny),
        _ => Err(invalid_value(path, location, format!("invalid `{}` value", line.name))
            .help("expected `allow` or `deny`")),
    }
}

#[derive(Debug, Clone)]
enum RuleDestination {
    Any,
    Ip(String),
    Cidr(String),
    Domain(String),
    DomainSuffix(String),
    Group(DestinationGroup),
}

fn parse_network_rule(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let mut fields = raw.split_whitespace();
    let direction = match fields.next() {
        Some("egress") => Direction::Egress,
        Some("ingress") => Direction::Ingress,
        Some("any") => Direction::Any,
        _ => return Err(invalid_network_rule(path, location)),
    };
    let action = match fields.next() {
        Some("allow") => Action::Allow,
        Some("deny") => Action::Deny,
        _ => return Err(invalid_network_rule(path, location)),
    };
    let rule_destination = fields
        .next()
        .and_then(parse_rule_destination)
        .ok_or_else(|| invalid_network_rule(path, location))?;
    let mut protocols = Vec::new();
    let mut ports = Vec::new();
    let mut has_protocols = false;
    let mut has_ports = false;
    for option in fields {
        if let Some(raw_protocols) = option.strip_prefix("protocols=") {
            if has_protocols {
                return Err(invalid_value(path, location, "duplicate `protocols` rule option"));
            }
            has_protocols = true;
            protocols = parse_protocols(path, location, raw_protocols)?;
        } else if let Some(raw_ports) = option.strip_prefix("ports=") {
            if has_ports {
                return Err(invalid_value(path, location, "duplicate `ports` rule option"));
            }
            has_ports = true;
            ports = parse_port_ranges(path, location, raw_ports)?;
        } else {
            return Err(invalid_network_rule(path, location));
        }
    }

    let policy = NetworkPolicy::builder()
        .rule(|rule| {
            match direction {
                Direction::Egress => rule.egress(),
                Direction::Ingress => rule.ingress(),
                Direction::Any => rule.any(),
            };
            for protocol in &protocols {
                match protocol {
                    Protocol::Tcp => rule.tcp(),
                    Protocol::Udp => rule.udp(),
                    Protocol::Icmpv4 => rule.icmpv4(),
                    Protocol::Icmpv6 => rule.icmpv6(),
                };
            }
            for range in &ports {
                rule.port_range(range.start, range.end);
            }
            let destination_builder = match action {
                Action::Allow => rule.allow(),
                Action::Deny => rule.deny(),
            };
            match &rule_destination {
                RuleDestination::Any => destination_builder.any(),
                RuleDestination::Ip(value) => destination_builder.ip(value),
                RuleDestination::Cidr(value) => destination_builder.cidr(value),
                RuleDestination::Domain(value) => destination_builder.domain(value),
                RuleDestination::DomainSuffix(value) => destination_builder.domain_suffix(value),
                RuleDestination::Group(value) => destination_builder.group(*value),
            };
            rule
        })
        .build()
        .map_err(|error| {
            invalid_value(path, location, format!("invalid `network-rule`: {error}"))
        })?;
    let rule =
        policy.rules.into_iter().next().ok_or_else(|| invalid_network_rule(path, location))?;
    Ok(Directive::NetworkRule(rule))
}

fn parse_rule_destination(raw: &str) -> Option<RuleDestination> {
    if raw == "any" {
        return Some(RuleDestination::Any);
    }
    let (kind, value) = raw.split_once('=')?;
    if value.is_empty() {
        return None;
    }
    match kind {
        "ip" => Some(RuleDestination::Ip(value.to_owned())),
        "cidr" => Some(RuleDestination::Cidr(value.to_owned())),
        "domain" => Some(RuleDestination::Domain(value.to_owned())),
        "domain-suffix" => Some(RuleDestination::DomainSuffix(value.to_owned())),
        "group" => parse_destination_group(value).map(RuleDestination::Group),
        _ => None,
    }
}

fn parse_destination_group(raw: &str) -> Option<DestinationGroup> {
    match raw {
        "public" => Some(DestinationGroup::Public),
        "loopback" => Some(DestinationGroup::Loopback),
        "private" => Some(DestinationGroup::Private),
        "link-local" => Some(DestinationGroup::LinkLocal),
        "metadata" => Some(DestinationGroup::Metadata),
        "multicast" => Some(DestinationGroup::Multicast),
        "host" => Some(DestinationGroup::Host),
        _ => None,
    }
}

fn parse_protocols(
    path: &Path,
    location: &Location,
    raw: &str,
) -> Result<Vec<Protocol>, Diagnostic> {
    let mut protocols = Vec::new();
    for name in raw.split(',') {
        let protocol = match name {
            "tcp" => Protocol::Tcp,
            "udp" => Protocol::Udp,
            "icmpv4" => Protocol::Icmpv4,
            "icmpv6" => Protocol::Icmpv6,
            _ => {
                return Err(invalid_value(path, location, "invalid `network-rule` protocol")
                    .help("expected `tcp`, `udp`, `icmpv4`, or `icmpv6`"));
            }
        };
        protocols.push(protocol);
    }
    if protocols.is_empty() {
        Err(invalid_value(path, location, "`network-rule` protocols must not be empty"))
    } else {
        Ok(protocols)
    }
}

fn parse_port_ranges(
    path: &Path,
    location: &Location,
    raw: &str,
) -> Result<Vec<PortRange>, Diagnostic> {
    let mut ranges = Vec::new();
    for item in raw.split(',') {
        let (start, end) = item.split_once('-').map_or((item, item), |(start, end)| (start, end));
        let start = parse_integer(path, location, start, "network-rule port")?;
        let end = parse_integer(path, location, end, "network-rule port")?;
        ranges.push(PortRange { start, end });
    }
    if ranges.is_empty() {
        Err(invalid_value(path, location, "`network-rule` ports must not be empty"))
    } else {
        Ok(ranges)
    }
}

fn invalid_network_rule(path: &Path, location: &Location) -> Diagnostic {
    invalid_value(path, location, "invalid `network-rule` value")
        .help("expected `DIRECTION ACTION DESTINATION [protocols=LIST] [ports=LIST]`")
}

fn parse_published_port(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let mut fields = raw.split_whitespace();
    let protocol = match fields.next() {
        Some("tcp") => PortProtocol::Tcp,
        Some("udp") => PortProtocol::Udp,
        _ => return Err(invalid_published_port(path, location)),
    };
    let mapping = fields.next().ok_or_else(|| invalid_published_port(path, location))?;
    if fields.next().is_some() {
        return Err(invalid_published_port(path, location));
    }
    let (host, guest_port) =
        mapping.rsplit_once(':').ok_or_else(|| invalid_published_port(path, location))?;
    let guest_port = parse_integer(path, location, guest_port, "publish-port guest port")?;
    let (host_bind, host_port) = if let Some((bind, port)) = host.rsplit_once(':') {
        let bind = bind.strip_prefix('[').and_then(|value| value.strip_suffix(']')).unwrap_or(bind);
        let bind = bind.parse::<IpAddr>().map_err(|_| invalid_published_port(path, location))?;
        (bind, parse_integer(path, location, port, "publish-port host port")?)
    } else {
        (
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            parse_integer(path, location, host, "publish-port host port")?,
        )
    };
    Ok(Directive::PublishPort(PublishedPort { protocol, host_bind, host_port, guest_port }))
}

fn invalid_published_port(path: &Path, location: &Location) -> Diagnostic {
    invalid_value(path, location, "invalid `publish-port` value")
        .help("expected `tcp|udp [HOST-ADDRESS:]HOST-PORT:GUEST-PORT`")
}

fn parse_dns_server(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    expect_value(path, line, location)?
        .parse::<Nameserver>()
        .map(Directive::DnsServer)
        .map_err(|error| invalid_value(path, location, error.to_string()))
}

fn parse_scoped_verification(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let mut fields = raw.split_whitespace();
    let pattern = fields.next().filter(|value| !value.is_empty());
    let verify = match fields.next() {
        Some("yes") => Some(true),
        Some("no") => Some(false),
        _ => None,
    };
    let (Some(pattern), Some(verify), None) = (pattern, verify, fields.next()) else {
        return Err(invalid_value(path, location, "invalid `tls-verify-upstream-for` value")
            .help("expected `HOST-PATTERN yes|no`"));
    };
    Ok(Directive::TlsVerifyUpstreamFor(ScopedVerification { pattern: pattern.to_owned(), verify }))
}

fn parse_scoped_certificate(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let (pattern, certificate) = raw.split_once('=').ok_or_else(|| {
        invalid_value(path, location, "invalid `tls-upstream-ca-cert-for` value")
            .help("expected `HOST-PATTERN=HOST-PATH`")
    })?;
    if pattern.is_empty() {
        return Err(invalid_value(
            path,
            location,
            "`tls-upstream-ca-cert-for` host pattern must not be empty",
        ));
    }
    Ok(Directive::TlsUpstreamCaCertFor(ScopedCertificate {
        pattern: pattern.to_owned(),
        path: parse_host_path(path, location, certificate)?,
    }))
}

fn parse_host_path(path: &Path, location: &Location, raw: &str) -> Result<String, Diagnostic> {
    if Path::new(raw).is_absolute() || raw == "{{src-base}}" || raw.starts_with("{{src-base}}/") {
        Ok(raw.to_owned())
    } else {
        Err(invalid_value(
            path,
            location,
            "host path must be absolute or start with `{{src-base}}`",
        ))
    }
}

fn parse_mac(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let octets = raw
        .split(':')
        .map(|octet| u8::from_str_radix(octet, 16))
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .and_then(|octets| <[u8; 6]>::try_from(octets).ok())
        .ok_or_else(|| {
            invalid_value(path, location, "`network-mac` must contain six hexadecimal octets")
        })?;
    Ok(Directive::NetworkMac(octets))
}

fn parse_address<T>(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
    name: &str,
) -> Result<Directive, Diagnostic>
where
    T: std::str::FromStr,
    Directive: From<T>,
{
    expect_value(path, line, location)?
        .parse::<T>()
        .map(Directive::from)
        .map_err(|_| invalid_value(path, location, format!("`{name}` must be an IP address")))
}

impl From<Ipv4Addr> for Directive {
    fn from(value: Ipv4Addr) -> Self {
        Self::NetworkIpv4(value)
    }
}

impl From<Ipv6Addr> for Directive {
    fn from(value: Ipv6Addr) -> Self {
        Self::NetworkIpv6(value)
    }
}

fn parse_ipv4_pool(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let pool = raw
        .parse()
        .map_err(|_| invalid_value(path, location, "`network-ipv4-pool` must be an IPv4 CIDR"))?;
    NetworkBuilder::new()
        .ipv4_pool(pool)
        .build()
        .map_err(|error| invalid_value(path, location, error.to_string()))?;
    Ok(Directive::NetworkIpv4Pool(raw.to_owned()))
}

fn parse_ipv6_pool(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let pool = raw
        .parse()
        .map_err(|_| invalid_value(path, location, "`network-ipv6-pool` must be an IPv6 CIDR"))?;
    NetworkBuilder::new()
        .ipv6_pool(pool)
        .build()
        .map_err(|error| invalid_value(path, location, error.to_string()))?;
    Ok(Directive::NetworkIpv6Pool(raw.to_owned()))
}

fn parse_ignore_test(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    expect_presence(path, line, location)?;
    line.remark
        .as_ref()
        .filter(|remark| !remark.trim().is_empty())
        .map(|reason| Directive::IgnoreTest(reason.clone()))
        .ok_or_else(|| {
            diagnostic(
                DiagnosticCode::InvalidValue,
                "`ignore-test` requires an explanatory remark",
                path,
                location,
            )
            .help("write `//@ ignore-test (reason)`")
        })
}

fn parse_run_flags(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let flags = split_flags(expect_value(path, line, location)?);
    if flags.is_empty() {
        Err(invalid_value(path, location, "`run-flags` must not be empty"))
    } else {
        Ok(Directive::RunFlags(flags))
    }
}

fn parse_exec_env(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let raw = expect_value(path, line, location)?;
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| invalid_value(path, location, "`exec-env` requires `KEY=VALUE`"))?;
    validate_environment_key(path, location, key)?;
    Ok(Directive::ExecEnv { key: key.trim().to_owned(), value: value.to_owned() })
}

fn parse_pull_policy(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let policy = match expect_value(path, line, location)? {
        "if-missing" => PullPolicy::IfMissing,
        "always" => PullPolicy::Always,
        "never" => PullPolicy::Never,
        _ => {
            return Err(invalid_value(path, location, "invalid `pull-policy` value")
                .help("expected `if-missing`, `always`, or `never`"));
        }
    };
    Ok(Directive::PullPolicy(policy))
}

fn parse_init(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<Directive, Diagnostic> {
    let value = expect_value(path, line, location)?;
    if value == "auto" || value.starts_with('/') {
        Ok(Directive::Init(value.to_owned()))
    } else {
        Err(invalid_value(path, location, "`init` must be `auto` or an absolute guest path"))
    }
}

fn presence_capability(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
    capability: Capability,
) -> Result<Directive, Diagnostic> {
    expect_presence(path, line, location)?;
    Ok(Directive::Capability(capability))
}

fn expect_presence(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
) -> Result<(), Diagnostic> {
    if matches!(line.form, DirectiveForm::Presence) {
        Ok(())
    } else {
        Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            format!("`{}` is a presence directive", line.name),
            path,
            location,
        )
        .help(format!("write `//@ {}` without a colon", line.name)))
    }
}

fn expect_value<'a>(
    path: &Path,
    line: &'a DirectiveLine,
    location: &Location,
) -> Result<&'a str, Diagnostic> {
    match &line.form {
        DirectiveForm::Value(value) if !value.is_empty() => Ok(value),
        DirectiveForm::Value(_) => {
            Err(invalid_value(path, location, format!("`{}` value must not be empty", line.name)))
        }
        DirectiveForm::Presence => Err(diagnostic(
            DiagnosticCode::MalformedDirective,
            format!("`{}` requires a colon and value", line.name),
            path,
            location,
        )
        .help(format!("write `//@ {}: value`", line.name))),
    }
}

fn expect_absolute_guest_path(
    path: &Path,
    line: &DirectiveLine,
    location: &Location,
    name: &str,
) -> Result<String, Diagnostic> {
    let value = expect_value(path, line, location)?;
    if value.starts_with('/') {
        Ok(value.to_owned())
    } else {
        Err(invalid_value(path, location, format!("`{name}` must be an absolute guest path")))
    }
}

fn parse_positive<T>(
    path: &Path,
    location: &Location,
    raw: &str,
    name: &str,
) -> Result<T, Diagnostic>
where
    T: TryFrom<u64>,
{
    let value = raw.parse::<u64>().map_err(|_| {
        invalid_value(path, location, format!("`{name}` must be a positive base-10 integer"))
    })?;
    if value == 0 {
        return Err(invalid_value(path, location, format!("`{name}` must be greater than zero")));
    }
    T::try_from(value).map_err(|_| {
        invalid_value(path, location, format!("`{name}` value is outside the supported range"))
    })
}

fn parse_integer<T>(
    path: &Path,
    location: &Location,
    raw: &str,
    name: &str,
) -> Result<T, Diagnostic>
where
    T: TryFrom<u64>,
{
    let value = raw.parse::<u64>().map_err(|_| {
        invalid_value(path, location, format!("`{name}` must be a base-10 integer"))
    })?;
    T::try_from(value).map_err(|_| {
        invalid_value(path, location, format!("`{name}` value is outside the supported range"))
    })
}

fn validate_environment_key(path: &Path, location: &Location, key: &str) -> Result<(), Diagnostic> {
    let key = key.trim();
    if key.is_empty() || key.contains(['=', '\0']) {
        Err(invalid_value(
            path,
            location,
            "environment key must be non-empty and contain neither `=` nor NUL",
        ))
    } else {
        Ok(())
    }
}

fn split_flags(flags: &str) -> Vec<String> {
    flags
        .split('\'')
        .enumerate()
        .flat_map(
            |(index, segment)| {
                if index % 2 == 1 { vec![segment] } else { segment.split_whitespace().collect() }
            },
        )
        .map(str::to_owned)
        .collect()
}

fn is_known_name(name: &str) -> bool {
    matches!(
        name,
        "ignore-test"
            | "needs-threads"
            | "needs-subprocess"
            | "needs-symlink"
            | "needs-dynamic-linking"
            | "needs-target-std"
            | "needs-unwind"
            | "run-flags"
            | "exec-env"
            | "unset-exec-env"
            | "image"
            | "from-snapshot"
            | "pull-policy"
            | "cpus"
            | "memory"
            | "root-disk"
            | "max-duration"
            | "user"
            | "workdir"
            | "shell"
            | "init"
            | "network"
            | "network-default-egress"
            | "network-default-ingress"
            | "network-rule"
            | "publish-port"
            | "dns-server"
            | "dns-query-timeout"
            | "no-dns-rebind-protection"
            | "tls-intercept"
            | "tls-intercept-port"
            | "tls-bypass"
            | "no-tls-verify-upstream"
            | "tls-verify-upstream-for"
            | "no-tls-block-quic"
            | "tls-upstream-ca-cert"
            | "tls-upstream-ca-cert-for"
            | "tls-intercept-ca-cert"
            | "tls-intercept-ca-key"
            | "tls-cert-cache-capacity"
            | "tls-cert-validity-hours"
            | "max-network-connections"
            | "trust-host-cas"
            | "network-mac"
            | "network-mtu"
            | "network-ipv4"
            | "network-ipv4-pool"
            | "network-ipv6"
            | "network-ipv6-pool"
            | "preserve-on-failure"
    ) || name.starts_with("only-")
        || name.starts_with("ignore-")
}

fn diagnostic(
    code: DiagnosticCode,
    message: impl Into<String>,
    path: &Path,
    location: &Location,
) -> Diagnostic {
    Diagnostic::new(code, message, path, location.label("directive declared here"))
}

fn invalid_value(path: &Path, location: &Location, message: impl Into<String>) -> Diagnostic {
    diagnostic(DiagnosticCode::InvalidValue, message, path, location)
}

#[cfg(test)]
mod tests {
    use super::{DirectiveForm, scan_line};

    #[test]
    fn ignores_an_ordinary_rust_line() {
        assert_eq!(scan_line(1, "fn main() {}"), None);
    }

    #[test]
    fn scans_presence_directive_after_indentation() {
        let directive = scan_line(7, "    //@ needs-threads").unwrap();

        assert_eq!(directive.name, "needs-threads");
        assert_eq!(directive.form, DirectiveForm::Presence);
        assert_eq!(directive.remark, None);
        assert_eq!(directive.span.line, 7);
        assert_eq!(directive.span.column, 9);
    }

    #[test]
    fn retains_a_presence_directive_remark() {
        let directive = scan_line(3, "//@ only-linux (uses epoll)").unwrap();

        assert_eq!(directive.name, "only-linux");
        assert_eq!(directive.form, DirectiveForm::Presence);
        assert_eq!(directive.remark.as_deref(), Some("(uses epoll)"));
    }

    #[test]
    fn splits_a_value_only_at_the_name_colon() {
        let directive =
            scan_line(4, "//@ image: registry.example:5000/team/app@sha256:deadbeef").unwrap();

        assert_eq!(directive.name, "image");
        assert_eq!(
            directive.form,
            DirectiveForm::Value("registry.example:5000/team/app@sha256:deadbeef".into())
        );
        assert_eq!(directive.remark, None);
    }

    #[test]
    fn normalizes_a_crlf_line_ending() {
        let directive = scan_line(2, "//@ memory: 512\r").unwrap();

        assert_eq!(directive.form, DirectiveForm::Value("512".into()));
    }

    #[test]
    fn retains_unicode_in_values_and_remarks() {
        let value = scan_line(1, "//@ user: oluwaseyi-测试").unwrap();
        let remark = scan_line(2, "//@ ignore-test (blocked by résumé fixture)").unwrap();

        assert_eq!(value.form, DirectiveForm::Value("oluwaseyi-测试".into()));
        assert_eq!(remark.remark.as_deref(), Some("(blocked by résumé fixture)"));
    }
}
