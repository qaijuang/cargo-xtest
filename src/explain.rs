use std::fmt::{self, Write as _};
use std::net::SocketAddr;

use microsandbox_network::config::PortProtocol;
use microsandbox_network::policy::{
    Action, Destination, DestinationGroup, Direction, NetworkProfile, Protocol, Rule,
};

use crate::directive::NetworkMode;
use crate::helpers::AsStr;
use crate::model::{
    EffectiveRootfs, EffectiveSpecification, EnvironmentChange, FailureRetention, GuestProcess,
    ImageSource, NetworkAccess, NetworkConfiguration, Origin, Sandbox, Setting,
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
    render_network(output, &sandbox.network)?;
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

#[allow(clippy::too_many_lines)]
fn render_network(output: &mut String, network: &Setting<NetworkAccess>) -> fmt::Result {
    let NetworkAccess::Enabled(configuration) = &network.value else {
        return writeln!(output, "  network: disabled {}", origin(network.origin));
    };

    writeln!(output, "  network: enabled {}", origin(network.origin))?;
    match &configuration.mode {
        NetworkMode::Profiles(profiles) => {
            let profiles =
                profiles.iter().copied().map(network_profile).collect::<Vec<_>>().join(",");
            writeln!(output, "    policy: profiles {profiles}")?;
        }
        NetworkMode::None => writeln!(output, "    policy: none")?,
        NetworkMode::AllowAll => writeln!(output, "    policy: allow-all")?,
        NetworkMode::Custom => {
            writeln!(output, "    policy: custom")?;
            render_nested_setting(
                output,
                "    default-egress",
                network_action(configuration.default_egress.value),
                configuration.default_egress.origin,
            )?;
            render_nested_setting(
                output,
                "    default-ingress",
                network_action(configuration.default_ingress.value),
                configuration.default_ingress.origin,
            )?;
            render_network_rules(output, configuration)?;
        }
    }
    render_published_ports(output, configuration)?;
    render_dns(output, configuration)?;
    render_tls(output, configuration)?;
    render_nested_setting(
        output,
        "    max-connections",
        configuration.max_connections.value.unwrap_or(256),
        configuration.max_connections.origin,
    )?;
    render_nested_setting(
        output,
        "    trust-host-cas",
        yes_no(configuration.trust_host_cas.value),
        configuration.trust_host_cas.origin,
    )?;
    render_network_interface(output, configuration)
}

fn render_network_rules(output: &mut String, configuration: &NetworkConfiguration) -> fmt::Result {
    if configuration.rules.is_empty() {
        return writeln!(output, "    rules: none");
    }
    writeln!(output, "    rules:")?;
    for rule in &configuration.rules {
        writeln!(output, "      - {} {}", network_rule(&rule.value), origin(rule.origin))?;
    }
    Ok(())
}

fn network_rule(rule: &Rule) -> String {
    let direction = match rule.direction {
        Direction::Egress => "egress",
        Direction::Ingress => "ingress",
        Direction::Any => "any",
    };
    let action = network_action(rule.action);
    let destination = match &rule.destination {
        Destination::Any => "any".to_owned(),
        Destination::Cidr(value) => format!("cidr={value}"),
        Destination::Domain(value) => format!("domain={value}"),
        Destination::DomainSuffix(value) => format!("domain-suffix={value}"),
        Destination::Group(value) => format!("group={}", destination_group(*value)),
    };
    let protocols = if rule.protocols.is_empty() {
        String::new()
    } else {
        format!(
            " protocols={}",
            rule.protocols.iter().copied().map(network_protocol).collect::<Vec<_>>().join(",")
        )
    };
    let ports = if rule.ports.is_empty() {
        String::new()
    } else {
        let ports = rule
            .ports
            .iter()
            .map(|range| {
                if range.start == range.end {
                    range.start.to_string()
                } else {
                    format!("{}-{}", range.start, range.end)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(" ports={ports}")
    };
    format!("{direction} {action} {destination}{protocols}{ports}")
}

fn render_published_ports(
    output: &mut String,
    configuration: &NetworkConfiguration,
) -> fmt::Result {
    if configuration.ports.is_empty() {
        writeln!(output, "    published-ports: none")?;
    } else {
        writeln!(output, "    published-ports:")?;
        for port in &configuration.ports {
            let protocol = match port.value.protocol {
                PortProtocol::Tcp => "tcp",
                PortProtocol::Udp => "udp",
            };
            let host = SocketAddr::new(port.value.host_bind, port.value.host_port);
            writeln!(
                output,
                "      - {protocol} {host} -> {} {}",
                port.value.guest_port,
                origin(port.origin)
            )?;
        }
    }
    Ok(())
}

fn render_dns(output: &mut String, configuration: &NetworkConfiguration) -> fmt::Result {
    writeln!(output, "    dns:")?;
    if configuration.dns.servers.is_empty() {
        writeln!(output, "      servers: host-default")?;
    } else {
        writeln!(output, "      servers:")?;
        for server in &configuration.dns.servers {
            writeln!(output, "        - {} {}", server.value, origin(server.origin))?;
        }
    }
    render_nested_setting(
        output,
        "      query-timeout-ms",
        configuration.dns.query_timeout_ms.value,
        configuration.dns.query_timeout_ms.origin,
    )?;
    render_nested_setting(
        output,
        "      rebind-protection",
        yes_no(configuration.dns.rebind_protection.value),
        configuration.dns.rebind_protection.origin,
    )
}

fn render_tls(output: &mut String, configuration: &NetworkConfiguration) -> fmt::Result {
    render_nested_setting(
        output,
        "    tls-intercept",
        yes_no(configuration.tls.enabled.value),
        configuration.tls.enabled.origin,
    )?;
    if !configuration.tls.enabled.value {
        return Ok(());
    }
    for port in &configuration.tls.intercepted_ports {
        render_nested_setting(output, "      port", port.value, port.origin)?;
    }
    render_string_settings(output, "      bypass", &configuration.tls.bypass)?;
    render_nested_setting(
        output,
        "      verify-upstream",
        yes_no(configuration.tls.verify_upstream.value),
        configuration.tls.verify_upstream.origin,
    )?;
    for verification in &configuration.tls.scoped_verification {
        writeln!(
            output,
            "      verify-upstream-for: {} {} {}",
            verification.value.pattern,
            yes_no(verification.value.verify),
            origin(verification.origin)
        )?;
    }
    render_nested_setting(
        output,
        "      block-quic",
        yes_no(configuration.tls.block_quic.value),
        configuration.tls.block_quic.origin,
    )?;
    render_string_settings(
        output,
        "      upstream-ca-cert",
        &configuration.tls.upstream_ca_certificates,
    )?;
    for certificate in &configuration.tls.scoped_upstream_ca_certificates {
        writeln!(
            output,
            "      upstream-ca-cert-for: {}={} {}",
            certificate.value.pattern,
            certificate.value.path,
            origin(certificate.origin)
        )?;
    }
    render_nested_setting(
        output,
        "      intercept-ca-cert",
        configuration.tls.intercept_ca_certificate.value.as_deref().unwrap_or("generated"),
        configuration.tls.intercept_ca_certificate.origin,
    )?;
    render_nested_setting(
        output,
        "      intercept-ca-key",
        configuration.tls.intercept_ca_key.value.as_deref().unwrap_or("generated"),
        configuration.tls.intercept_ca_key.origin,
    )?;
    render_nested_setting(
        output,
        "      cert-cache-capacity",
        configuration.tls.cache_capacity.value,
        configuration.tls.cache_capacity.origin,
    )?;
    render_nested_setting(
        output,
        "      cert-validity-hours",
        configuration.tls.validity_hours.value,
        configuration.tls.validity_hours.origin,
    )
}

fn render_string_settings(
    output: &mut String,
    name: &str,
    settings: &[Setting<String>],
) -> fmt::Result {
    if settings.is_empty() {
        writeln!(output, "{name}: none")
    } else {
        for setting in settings {
            render_nested_setting(output, name, &setting.value, setting.origin)?;
        }
        Ok(())
    }
}

fn render_network_interface(
    output: &mut String,
    configuration: &NetworkConfiguration,
) -> fmt::Result {
    writeln!(output, "    interface:")?;
    let mac = configuration.interface.mac.value.map_or_else(
        || "derived".to_owned(),
        |mac| mac.iter().map(|octet| format!("{octet:02x}")).collect::<Vec<_>>().join(":"),
    );
    render_nested_setting(output, "      mac", mac, configuration.interface.mac.origin)?;
    render_nested_setting(
        output,
        "      mtu",
        configuration.interface.mtu.value.map_or_else(|| "1500".to_owned(), |v| v.to_string()),
        configuration.interface.mtu.origin,
    )?;
    render_nested_setting(
        output,
        "      ipv4",
        configuration
            .interface
            .ipv4
            .value
            .map_or_else(|| "derived".to_owned(), |value| value.to_string()),
        configuration.interface.ipv4.origin,
    )?;
    render_nested_setting(
        output,
        "      ipv4-pool",
        configuration.interface.ipv4_pool.value.as_deref().unwrap_or("derived"),
        configuration.interface.ipv4_pool.origin,
    )?;
    render_nested_setting(
        output,
        "      ipv6",
        configuration
            .interface
            .ipv6
            .value
            .map_or_else(|| "derived".to_owned(), |value| value.to_string()),
        configuration.interface.ipv6.origin,
    )?;
    render_nested_setting(
        output,
        "      ipv6-pool",
        configuration.interface.ipv6_pool.value.as_deref().unwrap_or("derived"),
        configuration.interface.ipv6_pool.origin,
    )
}

fn network_profile(profile: NetworkProfile) -> &'static str {
    match profile {
        NetworkProfile::Public => "public",
        NetworkProfile::Private => "private",
        NetworkProfile::Host => "host",
    }
}

fn network_action(action: Action) -> &'static str {
    match action {
        Action::Allow => "allow",
        Action::Deny => "deny",
    }
}

fn destination_group(group: DestinationGroup) -> &'static str {
    match group {
        DestinationGroup::Public => "public",
        DestinationGroup::Loopback => "loopback",
        DestinationGroup::Private => "private",
        DestinationGroup::LinkLocal => "link-local",
        DestinationGroup::Metadata => "metadata",
        DestinationGroup::Multicast => "multicast",
        DestinationGroup::Host => "host",
    }
}

fn network_protocol(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
        Protocol::Icmpv4 => "icmpv4",
        Protocol::Icmpv6 => "icmpv6",
    }
}

const fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
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

fn render_nested_setting(
    output: &mut String,
    name: &str,
    value: impl fmt::Display,
    source: Origin,
) -> fmt::Result {
    writeln!(output, "{name}: {value} {}", origin(source))
}

fn origin(origin: Origin) -> String {
    origin.line().map_or_else(|| "[default]".to_owned(), |line| format!("[line {line}]"))
}
