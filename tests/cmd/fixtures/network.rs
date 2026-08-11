//@ network: custom
//@ network-default-egress: deny
//@ network-default-ingress: deny
//@ network-rule: egress allow domain-suffix=example.com protocols=tcp,tcp ports=443
//@ network-rule: ingress allow group=public protocols=tcp ports=8080
//@ publish-port: tcp 127.0.0.1:18080:8080
//@ publish-port: udp 127.0.0.1:15353:5353
//@ publish-port: tcp [::1]:18081:8081
//@ dns-server: 1.1.1.1
//@ dns-server: dns.google:53
//@ dns-query-timeout: 2500
//@ no-dns-rebind-protection
//@ tls-intercept
//@ tls-intercept-port: 443
//@ tls-intercept-port: 8443
//@ tls-bypass: *.internal.example
//@ no-tls-verify-upstream
//@ tls-verify-upstream-for: api.example.com yes
//@ no-tls-block-quic
//@ tls-upstream-ca-cert: {{src-base}}/certificates/upstream.pem
//@ tls-upstream-ca-cert-for: *.internal.example={{src-base}}/certificates/internal.pem
//@ tls-intercept-ca-cert: {{src-base}}/certificates/intercept.pem
//@ tls-intercept-ca-key: {{src-base}}/certificates/intercept-key.pem
//@ tls-cert-cache-capacity: 128
//@ tls-cert-validity-hours: 12
//@ max-network-connections: 64
//@ trust-host-cas
//@ network-mac: 02:00:00:00:00:2a
//@ network-mtu: 1400
//@ network-ipv4: 172.20.0.2
//@ network-ipv4-pool: 172.20.0.0/16
//@ network-ipv6: fd42:6d73:62::2
//@ network-ipv6-pool: fd42:6d73:62::/48

#[test]
fn uses_network_configuration() {}
