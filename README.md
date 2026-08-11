# cargo-xtest

`cargo-xtest` runs each Rust integration-test target in its own isolated
[Microsandbox](https://github.com/superradcompany/microsandbox) virtual machine
(VM).

At a high level:

- cargo-xtest discovers integration-test targets in Cargo's default workspace
  members.
- It reads `//@` directives from each test file.
- It compiles each file as one self-contained Linux-musl libtest binary.
- It starts one ephemeral VM, copies the binary into it, and lets libtest run
  every `#[test]` function in that binary.
- It stops the VM before moving to the next test file.

Multiple tests in one file share a binary and a VM. Tests in different files do
not.

> [!IMPORTANT]
> cargo-xtest isolates the finished test binary, not compilation. Cargo, build
> scripts, procedural macros, and the compiler run on your host. Run only source
> code you trust.

## Requirements

To install cargo-xtest from source, you need Git, rustup, and a nightly Rust
toolchain with the `rust-src` component.

To run test binaries, you also need:

- An `x86_64` or `aarch64` host.
- Linux with readable and writable KVM access, or macOS on Apple Silicon.
- The Linux-musl target that matches your host architecture.

On Linux, confirm that your user can open `/dev/kvm` before you run cargo-xtest.

The pinned Microsandbox SDK downloads its matching runtime during the first
build on Linux and macOS. It stores the runtime in `$MSB_HOME` when that variable
is set, or in `~/.microsandbox` otherwise.

### Install the Linux-musl target

Install the target for the toolchain that your project uses:

```sh
# x86_64 host
rustup target add x86_64-unknown-linux-musl

# aarch64 host
rustup target add aarch64-unknown-linux-musl
```

Run the matching command from your project directory when the project selects a
toolchain with `rust-toolchain.toml` or `rust-toolchain`.

## Install from source

```sh
rustup toolchain install nightly --component rust-src
git clone https://github.com/qaijuang/cargo-xtest.git
cd cargo-xtest
cargo +nightly install --path . --locked
```

This project does not publish cargo-xtest on crates.io yet.

## Run your first test

Create an integration-test file in a Rust package:

```rust
// tests/isolation.rs
//@ memory: 768
//@ max-duration: 120
//@ exec-env: CARGO_XTEST_VM=1

#[test]
fn runs_with_the_requested_environment() {
    assert_eq!(std::env::var("CARGO_XTEST_VM").unwrap(), "1");
}
```

Run all discovered integration-test targets from the workspace root:

```sh
cargo xtest
```

libtest still controls test discovery, output capture, and the final test
result inside each binary.

## Check a test before you run it

Use `explain` to see the effective settings for one test file:

```sh
cargo xtest explain tests/isolation.rs
```

The command marks each setting as a default or shows the source line that set
it. It validates directives without compiling the test or starting a VM.

## Understand the execution flow

For each `cargo xtest` invocation, cargo-xtest:

1. Uses Cargo metadata to find integration-test targets in the default
   workspace members.
2. Sorts the targets by source path.
3. Reads and validates the directives in the first target.
4. Skips the target when an applicability rule excludes its Linux-musl guest.
5. Uses the Cargo executable that started cargo-xtest to compile that target.
6. Creates an ephemeral VM from the selected OCI image or snapshot.
7. Copies the compiled test binary into the VM. When color is enabled, it also
   writes a private terminal description into the VM's ephemeral filesystem.
8. Runs the binary with the configured libtest arguments and environment.
9. Stops the VM and continues to the next target.

cargo-xtest stops at the first directive, compilation, execution, test, or
cleanup failure. A skipped target does not stop the run.

### Know the trust boundary

cargo-xtest isolates the compiled test binary. It does not isolate Cargo
compilation. Cargo, build scripts, procedural macros, and the compiler run on
the host before cargo-xtest creates the VM. Run cargo-xtest only on source code
you trust.

The VM receives the test binary but no host directory mount. Its network is
disabled unless the test file contains a `network` directive. cargo-xtest
destroys the VM after the test file finishes.

## Write directives

A directive starts with `//@`. You may indent directives.

Use a presence directive when the directive has no value:

```rust
//@ needs-threads
```

Use a colon for a value directive:

```rust
//@ memory: 1024
```

Put only one directive on each line. A value continues to the end of its line,
so OCI references may contain more colons. Ordinary Rust lines and comments do
not affect the configuration.

Presence directives may include a remark in parentheses:

```rust
//@ only-linux (uses Linux APIs)
```

`ignore-test` requires a nonempty remark; put it in parentheses as shown above.
Other remarks explain intent to readers but do not change execution.

Revision-qualified forms such as `//@[fast]` are not supported. Use separate
Cargo test targets or library-level test cases instead.

## Select tests

### `ignore-test`

Skip the test file unconditionally. Include the reason in parentheses.

```rust
//@ ignore-test (blocked by issue 123)
```

### `only-<predicate>` and `ignore-<predicate>`

Run or skip a file according to its Linux-musl guest. These are presence
directives.

```rust
//@ only-linux
//@ ignore-aarch64
```

All `only-<predicate>` directives must match. Any matching
`ignore-<predicate>` directive skips the file.

The current profile recognizes these predicates:

| Predicate                                                     | Match rule                     |
| ------------------------------------------------------------- | ------------------------------ |
| `test`, `linux`, `musl`, `linux-musl`, `unix`, `elf`, `64bit` | Always matches                 |
| `windows`, `macos`, `gnu`, `msvc`, `32bit`                    | Never matches                  |
| `x86_64`, `aarch64`                                           | Matches the guest architecture |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`     | Matches the full guest target  |

An unknown predicate skips the file and reports the predicate in the reason.

## Declare required capabilities

Capability directives describe what the test binary needs. They are presence
directives.

| Directive               | Current behavior                                                      |
| ----------------------- | --------------------------------------------------------------------- |
| `needs-threads`         | The self-contained profile treats threads as available.               |
| `needs-subprocess`      | The profile treats subprocesses as available.                         |
| `needs-symlink`         | The profile treats symbolic links as available.                       |
| `needs-target-std`      | The profile treats the target standard library as available.          |
| `needs-unwind`          | The profile treats unwinding as available.                            |
| `needs-dynamic-linking` | The profile skips the file because the test binary is self-contained. |

Declare each capability at most once.

## Configure libtest and the environment

### `run-flags`

Pass arguments to the libtest binary. You may repeat this directive. cargo-xtest
appends arguments in source order.

```rust
//@ run-flags: --nocapture
//@ run-flags: --exact 'module::named test'
```

Whitespace separates arguments. Single quotes group text that contains spaces.
Double-quote and backslash escape rules are not supported.

### Color output

cargo-xtest follows `CARGO_TERM_COLOR`. `always` enables color, `never`
disables it, and `auto` uses color when cargo-xtest's host standard output is a
terminal. A `--color` value in `run-flags` takes precedence.

When color is enabled, cargo-xtest writes its own small terminal description to
the ephemeral VM and sets `TERM` and `TERMINFO` for libtest. This works with the
built-in image, an `image` directive, or a snapshot without changing that root
filesystem. If the test explicitly sets or unsets `TERM` or `TERMINFO`,
cargo-xtest leaves terminal setup to the test and its image.

### `exec-env`

Set an environment variable inside the VM:

```rust
//@ exec-env: RUST_BACKTRACE=1
```

The value may contain additional `=` characters. Each environment key may
appear only once.

### `unset-exec-env`

Remove an environment variable before libtest starts:

```rust
//@ unset-exec-env: RUST_LOG
```

The key must start with an ASCII letter or `_`. Its remaining characters must
be ASCII letters, digits, or `_`. Do not set and unset the same key.

## Choose the root filesystem

### `image`

Start the VM from an OCI image reference:

```rust
//@ image: alpine:3.22
```

Without this directive, cargo-xtest uses its pinned built-in Alpine image.

### `from-snapshot`

Start the VM from an existing Microsandbox snapshot:

```rust
//@ from-snapshot: prepared-database
```

`image` and `from-snapshot` conflict. Declare only one root filesystem source.

### `pull-policy`

Control when Microsandbox pulls an OCI image:

```rust
//@ pull-policy: never
```

Choose `if-missing`, `always`, or `never`. This directive applies only to image
root filesystems and conflicts with `from-snapshot`.

### `root-disk`

Set the image root disk size in mebibytes:

```rust
//@ root-disk: 8192
```

Use a positive base-10 integer. This directive applies only to image root
filesystems and conflicts with `from-snapshot`.

## Set VM resources

### `cpus`

Set the number of virtual CPUs. The value must be a positive integer that fits
in an unsigned 8-bit value.

```rust
//@ cpus: 2
```

### `memory`

Set memory in mebibytes. Use a positive integer.

```rust
//@ memory: 1024
```

### `max-duration`

Set the maximum VM lifetime in seconds. Use a positive integer.

```rust
//@ max-duration: 900
```

## Configure the guest process

### `user`

Run the test binary as the named guest user:

```rust
//@ user: tester
```

### `workdir`

Set the guest working directory. Use an absolute guest path.

```rust
//@ workdir: /workspace
```

### `shell`

Set the guest shell. Use an absolute guest path. cargo-xtest uses this shell
when it must remove environment variables before it starts the test binary.

```rust
//@ shell: /bin/bash
```

### `init`

Set the guest init program to `auto` or an absolute guest path:

```rust
//@ init: auto
```

## Configure the network

Network access is disabled when a test file has no `network` directive. Use the
presence form to allow DNS and public internet addresses:

```rust
//@ network
```

This default blocks routed access to private, link-local, metadata, multicast,
loopback, and host addresses. The guest's own loopback interface stays inside
the guest. This mode does not publish a guest port.

### Choose a network policy

Combine the `public`, `private`, and `host` profiles when a test needs more
than public access:

```rust
//@ network: public,private,host
```

`host` means the sandbox host through its gateway address and
`host.microsandbox.internal`. It does not mean guest loopback.

These modes cannot be combined with profiles:

| Value       | Behavior                                                         |
| ----------- | ---------------------------------------------------------------- |
| `none`      | Create the network interface but deny ingress and egress.        |
| `allow-all` | Allow every ingress and egress destination.                      |
| `custom`    | Use ordered rules and deny unmatched traffic in both directions. |

`allow-all` removes the destination protections provided by the default
profile. Use it only when the test requires unrestricted traffic.

### Write a custom policy

A custom policy denies unmatched ingress and egress by default. You may change
either default:

```rust
//@ network: custom
//@ network-default-egress: deny
//@ network-default-ingress: deny
```

Add rules in evaluation order. The first matching rule wins:

```rust
//@ network-rule: egress allow domain-suffix=example.com protocols=tcp ports=443
//@ network-rule: ingress allow group=public protocols=tcp ports=8080
```

Use this grammar:

```text
DIRECTION ACTION DESTINATION [protocols=LIST] [ports=LIST]
```

- Direction: `egress`, `ingress`, or `any`.
- Action: `allow` or `deny`.
- Destination: `any`, `ip=ADDRESS`, `cidr=CIDR`, `domain=NAME`,
  `domain-suffix=NAME`, or `group=NAME`.
- Destination group: `public`, `loopback`, `private`, `link-local`,
  `metadata`, `multicast`, or `host`.
- Protocol list: a comma-separated list of `tcp`, `udp`, `icmpv4`, or
  `icmpv6`. Omit it to match every protocol.
- Port list: comma-separated ports or inclusive ranges such as
  `80,443,8000-8100`. Omit it to match every guest-side port.

Ingress and `any` rules cannot use ICMP because Microsandbox has no inbound
ICMP path.

### Publish guest ports

Map a host port to a guest port with TCP or UDP:

```rust
//@ network
//@ publish-port: tcp 18080:8080
//@ publish-port: udp 127.0.0.1:15353:5353
```

The shorter form binds to host loopback. Add a host IP address before the host
port to choose another bind address. Put an IPv6 bind address in brackets.
Publishing on a non-loopback address can expose the guest service to other
machines.

### Configure DNS

By default, Microsandbox uses the host's configured resolvers, applies a
5,000-millisecond query timeout, and blocks DNS rebinding. Override the
resolvers or timeout when needed:

```rust
//@ network
//@ dns-server: 1.1.1.1
//@ dns-server: dns.google:53
//@ dns-query-timeout: 2500
```

`dns-server` accepts an IP address or host name with an optional port. Repeat
it to set more than one resolver. Use this presence directive only when the
test must accept private addresses returned by public DNS:

```rust
//@ no-dns-rebind-protection
```

### Configure TLS interception

Enable TLS interception explicitly:

```rust
//@ network
//@ tls-intercept
```

It intercepts TCP port 443, verifies upstream certificates, and blocks QUIC on
intercepted ports by default. If you add any `tls-intercept-port` directive,
the explicit port list replaces the default. Repeat the directive to intercept
more than one port. These directives refine that behavior:

| Directive                  | Value or effect                                    |
| -------------------------- | -------------------------------------------------- |
| `tls-intercept-port`       | Set a TCP port in the explicit interception list.  |
| `tls-bypass`               | Bypass an exact host or `*.suffix` pattern.        |
| `no-tls-verify-upstream`   | Stop verifying every upstream certificate.         |
| `tls-verify-upstream-for`  | Set `HOST-PATTERN yes\|no` for one host pattern.   |
| `no-tls-block-quic`        | Allow UDP on intercepted ports.                    |
| `tls-upstream-ca-cert`     | Trust a host CA certificate for every upstream.    |
| `tls-upstream-ca-cert-for` | Trust `HOST-PATTERN=HOST-PATH` for matching hosts. |
| `tls-intercept-ca-cert`    | Use a specific interception CA certificate.        |
| `tls-intercept-ca-key`     | Use the matching interception CA private key.      |
| `tls-cert-cache-capacity`  | Set the positive certificate-cache entry limit.    |
| `tls-cert-validity-hours`  | Set generated certificate validity in hours.       |

Provide both interception CA files or neither. Without them, Microsandbox
generates and persists an interception CA in its runtime directory.

CA paths must be absolute host paths or begin with `{{src-base}}`, which means
the directory containing the test file:

```rust
//@ tls-upstream-ca-cert: {{src-base}}/certificates/test-ca.pem
```

The CA files remain on the host. cargo-xtest passes their resolved paths to
Microsandbox and does not mount the test directory into the VM.

### Set connection and interface options

Use these directives only with `network`:

| Directive                 | Effect                                                   |
| ------------------------- | -------------------------------------------------------- |
| `max-network-connections` | Set the concurrent guest connection limit. Default: 256. |
| `trust-host-cas`          | Copy the host's trusted root CAs into the guest.         |
| `network-mac`             | Set six hexadecimal MAC octets.                          |
| `network-mtu`             | Set the interface MTU. Default: 1500.                    |
| `network-ipv4`            | Set the guest IPv4 address.                              |
| `network-ipv4-pool`       | Set the IPv4 pool; its prefix must be `/30` or shorter.  |
| `network-ipv6`            | Set the guest IPv6 address.                              |
| `network-ipv6-pool`       | Set the IPv6 pool; its prefix must be `/64` or shorter.  |

Microsandbox derives unset addresses, pools, and the MAC from the sandbox
slot. `trust-host-cas` expands the guest's trust store; use it only when the
test must trust certificates accepted by the host.

### `preserve-on-failure`

The parser accepts `preserve-on-failure`, but the current one-shot execution
path cannot preserve a VM. `cargo xtest` reports an error before it creates the
VM. Do not use this directive yet.

## Review the defaults

`cargo xtest explain <test-file>` is the authoritative view of effective
settings. A test file with no directives uses these defaults:

| Setting           | Default                      |
| ----------------- | ---------------------------- |
| Execution profile | Self-contained Linux-musl    |
| Libtest arguments | None                         |
| Environment       | Unchanged                    |
| Root filesystem   | Pinned built-in Alpine image |
| Image pull policy | `if-missing`                 |
| Root disk         | 4096 MiB                     |
| Virtual CPUs      | 1                            |
| Memory            | 512 MiB                      |
| Maximum duration  | 600 seconds                  |
| Lifecycle         | Ephemeral                    |
| Network           | Disabled                     |
| Failure retention | Destroy                      |
| User              | Image default                |
| Working directory | Image default                |
| Shell             | `/bin/sh`                    |
| Init              | None                         |

## Avoid duplicate and conflicting directives

Most value directives and presence settings may appear only once. cargo-xtest
reports the duplicate and points to the first declaration.

These directives may repeat:

- `run-flags`, which appends more libtest arguments.
- `only-<predicate>` and `ignore-<predicate>`, when each predicate is unique.
- `exec-env` and `unset-exec-env`, when each key is unique.
- `network-rule`, whose source order controls first-match evaluation.
- `publish-port`, `dns-server`, `tls-intercept-port`, `tls-bypass`,
  `tls-verify-upstream-for`, `tls-upstream-ca-cert`, and
  `tls-upstream-ca-cert-for`.

The root filesystem rules add two specific conflicts:

- `image` conflicts with `from-snapshot`.
- `pull-policy` and `root-disk` conflict with `from-snapshot`.

## Read diagnostics and exit statuses

Directive diagnostics include a stable `XT` code, the test file and source
location, the relevant source line, and help when cargo-xtest can suggest a
correction.

```text
error[XT005]: `from-snapshot` conflicts with the earlier `image` directive
 --> tests/database.rs:2:5
  |
2 | //@ from-snapshot: prepared-database
  |     ^^^^^^^^^^^^^ conflicting rootfs source
  |
  = note: first rootfs source declared at tests/database.rs:1:5
```

cargo-xtest keeps compiler and libtest output visible. It uses these main exit
statuses:

| Status | Meaning                                                                             |
| ------ | ----------------------------------------------------------------------------------- |
| 0      | Every eligible test target passed, or cargo-xtest skipped it.                       |
| 1      | cargo-xtest could not complete discovery, configuration, VM execution, or cleanup.  |
| 2      | The command line was invalid.                                                       |
| 101    | A libtest binary failed. Cargo also commonly uses this status for its own failures. |

Cargo metadata, compilation, and test-process failures preserve their nonzero
status when it fits in an 8-bit exit status. cargo-xtest uses status 1 otherwise.

## Current limitations

- cargo-xtest runs Cargo integration-test targets only. It does not run unit
  tests, documentation tests, or benchmark targets.
- It uses only the default workspace members and stops at the first failure.
- Every guest is Linux-musl and matches the host architecture. macOS and Windows
  guest binaries are not supported.
- Dynamic linking is unavailable in the self-contained profile.
- cargo-xtest parses `preserve-on-failure` for explainability but cannot preserve
  the VM during one-shot execution.
- The pinned Microsandbox SDK describes Windows host support as preview. Its
  prebuilt feature does not install the required Windows runtime artifacts, and
  this project does not run real-VM tests on Windows.
- cargo-xtest does not provide service-specific orchestration. Prepare the
  selected image or snapshot with the services that your test needs.
