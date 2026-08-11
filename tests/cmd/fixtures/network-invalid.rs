//@ network: custom
//@ tls-intercept
//@ network-rule: sideways allow any
//@ network-rule: egress allow any ports=9000-8000
//@ publish-port: sctp 8080:80
//@ dns-server: dns google
//@ tls-verify-upstream-for: example.com maybe
//@ tls-upstream-ca-cert: relative/ca.pem
//@ network-mac: 02:00:00:00:00
//@ network-ipv4: 999.0.0.1
//@ network-ipv4-pool: 172.20.0.0/31
//@ network-ipv6: not-an-address
//@ network-ipv6-pool: fd42:6d73:62::/65

#[test]
fn rejects_invalid_network_configuration() {}
