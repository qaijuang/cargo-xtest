# cargo-xtest

`cargo-xtest` runs each Rust integration-test target inside its own isolated [Microsandbox](https://github.com/superradcompany/microsandbox) virtual machine (VM).

The basic idea is simple:

- Cargo still decides which packages, features, profiles, and integration-test targets you want.
- `cargo-xtest` asks Cargo to compile those test artifacts for the Linux-musl guest target in a single build.
- It reads the `//@` directives you put in each test file.
- For each test file, it starts a temporary VM, copies the compiled test binary into it, and lets libtest run every `#[test]` function in that binary.
- When that test file is done, the VM is destroyed before `cargo-xtest` moves on to the next file.

If you want the exact step-by-step sequence, see [Understand the execution flow][execution-flow].

One important thing to keep in mind: tests inside the **same test file** share one binary and one VM. Tests from **different files** do not.

> [!IMPORTANT]
> `cargo-xtest` isolates the code that runs inside the guest VM, but compilation still happens on your host machine. Read [Know the trust boundary][trust-boundary] before using it, and only run source code you trust.

## Contents

- **Getting started**
  - [Requirements](#requirements)
  - [Install from source](#install-from-source)
  - [Run your first test](#run-your-first-test)

- **Running tests**
  - [Select packages and tests](#select-packages-and-tests)
  - [Check a test before you run it](#check-a-test-before-you-run-it)
  - [Understand the execution flow][execution-flow]
  - [Interrupt a run](#interrupt-a-run)

- **Configuring tests**
  - [Write directives](#write-directives)
  - [Select tests](#select-tests)
  - [Declare required capabilities](#declare-required-capabilities)
  - [Configure libtest and the environment](#configure-libtest-and-the-environment)
  - [Choose the root filesystem][root-filesystem]
  - [Set VM resources](#set-vm-resources)
  - [Configure the guest process](#configure-the-guest-process)
  - [Configure the network](#configure-the-network)

- **References**
  - [Review the defaults][defaults]
  - [Avoid duplicate and conflicting directives][directive-repetition]
  - [Read diagnostics and exit statuses](#read-diagnostics-and-exit-statuses)
  - [Current limitations](#current-limitations)

## Requirements

If you're installing `cargo-xtest` from source, you'll need:

- Git
- `rustup`
- A nightly Rust toolchain

To actually run test binaries, you'll also need:

- An `x86_64` or `aarch64` host.
- Either:
  - Linux with readable and writable KVM access, or
  - macOS on Apple Silicon.

- The Linux-musl Rust target that matches your host architecture.

If you're on Linux, make sure your user can open `/dev/kvm` before running `cargo-xtest`.

The pinned Microsandbox SDK downloads the matching Microsandbox runtime the first time you build on Linux or macOS.

It stores that runtime in:

- `$MSB_HOME`, if you have that environment variable set, or
- `~/.microsandbox` otherwise.

### Install the Linux-musl target

Install the target that matches both your host architecture and the Rust toolchain your project uses:

```sh
# x86_64 host
rustup target add x86_64-unknown-linux-musl

# aarch64 host
rustup target add aarch64-unknown-linux-musl
```

If your project selects a toolchain through `rust-toolchain.toml` or `rust-toolchain`, run the matching command from the project directory so you're installing the target for the right toolchain.

## Install from source

Install the nightly toolchain, clone the repository, and install the project:

```sh
rustup toolchain install nightly
git clone https://github.com/qaijuang/cargo-xtest.git
cd cargo-xtest
cargo +nightly install --path . --locked
```

`cargo-xtest` is not published on crates.io yet.

## Run your first test

Start by creating a normal Rust integration-test file:

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

Then, from the workspace root, run:

```sh
cargo xtest
```

That runs all integration-test targets `cargo-xtest` discovers.

For integration-test targets that use Cargo's default test harness, libtest still handles:

- test discovery,
- output capture, and
- the final test result.

You don't have to add directives to every file. If a test file has no `//@` directives, `cargo-xtest` runs it using the [safe defaults][defaults].

## Select packages and tests

For package selection, features, integration-test targets, profiles, and build settings, use the same Cargo options you already know from `cargo test`.

For example:

```sh
cargo xtest --workspace --exclude slow-service
cargo xtest --package api --features postgres --test database
cargo xtest --release --jobs 4
```

`cargo-xtest` passes those options back to the same Cargo executable that invoked it. Cargo is still responsible for validating their values and reporting command-line errors.

To see the full supported option set, run:

```sh
cargo xtest --help
```

By default, `cargo-xtest` asks Cargo to build test targets using `--tests`.

Cargo can also produce unit-test executables for libraries or binaries as part of that build. `cargo-xtest` intentionally does **not** run those executables. When that happens, it tells you why:

```text
skipped src/lib.rs: cargo-xtest runs integration-test targets only
```

Selectors that fall outside the integration-test boundary are rejected with a policy error. That includes:

- `--lib`
- `--bins`
- `--bin`
- `--examples`
- `--example`
- `--benches`
- `--bench`
- `--all-targets`
- `--doc`

`cargo-xtest` also reserves these options for itself:

- `--tests`
- `--target`
- `--message-format`
- `--unit-graph`

It needs control over test-target selection, the guest compilation target, and Cargo's artifact stream, so those options cannot be supplied by the caller.

If you only want to compile the selected tests, use:

```sh
cargo xtest --no-run
```

With `--no-run`, `cargo-xtest` stops after Cargo finishes compiling. It does **not** read test directives and does **not** start Microsandbox.

### Pass arguments to libtest

If you want to filter tests by name, put the test-name filter after the `cargo-xtest` options:

```sh
cargo xtest connection
```

For other libtest arguments, put them after `--`:

```sh
cargo xtest connection -- --exact --show-output
```

For each test file, `cargo-xtest` appends those command-line arguments to any `run-flags` directives declared inside that file.

The libtest binary then validates the final combined argument list.

By default, `cargo-xtest` stops as soon as a test binary fails.

If you want the remaining integration-test binaries to keep running, use:

```sh
cargo xtest --no-fail-fast
```

With `--no-fail-fast`, `cargo-xtest` continues after a **test-binary failure** and returns a failing status after the last test binary has run.

It does **not** keep going after:

- directive errors,
- Cargo errors,
- Microsandbox errors,
- signal handling errors, or
- cleanup errors.

Those still stop the run immediately.

To reduce progress output, use:

```sh
cargo xtest --quiet
```

`--quiet` hides Cargo progress along with `cargo-xtest`'s `running` and `skipped` messages.

Test output and error messages are still shown.

## Check a test before you run it

If you want to know exactly how `cargo-xtest` will interpret one test file before compiling or launching anything, use `explain`:

```sh
cargo xtest explain tests/isolation.rs
```

The output shows the effective configuration for that file.

For each setting, it either:

- tells you the value came from a default, or
- points to the source line that configured it.

`explain` also validates the directives.

It does **not**:

- compile the test, or
- start a VM.

## Understand the execution flow

Every time you run `cargo xtest`, this is what happens:

1. `cargo-xtest` passes the supported package, feature, profile, and build options to Cargo.
2. Cargo compiles the selected test artifacts for the Linux-musl guest target without running them.
3. `cargo-xtest` collects Cargo's integration-test executables and sorts all test artifacts by source path.
4. Test artifacts outside the integration-test boundary are reported as skipped.
5. `cargo-xtest` reads and validates the directives from the next integration-test file.
6. If an applicability rule says that file does not apply to its Linux-musl guest, the file is skipped.
7. `cargo-xtest` creates an ephemeral VM from the selected OCI image or Microsandbox snapshot.
8. It copies the compiled test binary into the VM. If color output is enabled, it also writes a private terminal description into the VM's ephemeral filesystem.
9. It runs the test binary with the configured libtest arguments and environment, forwarding Cargo diagnostics and guest output as they arrive.
10. It stops the VM, then moves on to the next integration-test target.

Normally, `cargo-xtest` stops on the first:

- directive failure,
- compilation failure,
- execution failure,
- test failure, or
- cleanup failure.

`--no-fail-fast` changes that behavior **only** for test-binary failures.

A skipped target does not stop the run.

### Know the trust boundary

The VM isolates the **compiled test binary**.

It does **not** isolate the compilation process.

Before `cargo-xtest` creates a VM, all of these still run directly on your host:

- Cargo,
- build scripts,
- procedural macros, and
- the Rust compiler.

That means you should only run `cargo-xtest` on source code you trust.

Once the VM starts, it receives the compiled test binary, but it does **not** receive a mounted host directory.

Networking is disabled unless the test file contains a `network` directive.

After that test file finishes, `cargo-xtest` destroys the VM.

## Interrupt a run

Press Ctrl-C to stop the current run.

What happens depends on where the run currently is:

- During compilation, `cargo-xtest` stops the Cargo process.
- During guest execution, it forwards the interrupt to the test binary and then stops the VM.

On Unix, `SIGTERM` follows the same shutdown path.

The resulting statuses are:

- Ctrl-C → status `130`
- `SIGTERM` → status `143`

`cargo-xtest` also stops the VM if guest output can no longer be written or another execution error occurs.

Cargo diagnostics and guest stdout/stderr are forwarded as raw bytes while the process is running.

That means a long-running test can keep printing progress without `cargo-xtest` having to retain the entire output in memory.

## Write directives

A `cargo-xtest` directive starts with:

```text
//@
```

You can indent directives if that makes the test file easier to read.

If a directive is just a yes/no capability or setting and doesn't need a value, use the presence form:

```rust
//@ needs-threads
```

If the directive needs a value, put a colon after its name:

```rust
//@ memory: 1024
```

Keep one directive per line.

For value directives, everything after the directive's separator belongs to the value until the end of that line. Because of that, values such as OCI image references can contain additional colons.

Normal Rust code and ordinary comments don't affect `cargo-xtest` configuration.

Presence directives can also include a human-readable remark in parentheses:

```rust
//@ only-linux (uses Linux APIs)
```

For `ignore-test`, that remark is required and must not be empty.

For other presence directives, remarks are optional. They're there to explain the intent to people reading the test, and they do not change how the test runs.

## Select tests

### `ignore-test`

Use `ignore-test` when you want to skip a test file every time.

Include the reason in parentheses:

```rust
//@ ignore-test (blocked by issue 123)
```

### `only-<predicate>` and `ignore-<predicate>`

These directives decide whether a test file applies to the Linux-musl guest it's about to run on.

They're presence directives:

```rust
//@ only-linux
//@ ignore-aarch64
```

When you use several `only-<predicate>` directives, **all of them must match**.

If **any** matching `ignore-<predicate>` directive is present, the file is skipped.

See the [directive repetition rules][directive-repetition] for the rules around repeating predicates.

The current execution profile understands these predicates:

| Predicate                                                     | Match rule                     |
| ------------------------------------------------------------- | ------------------------------ |
| `test`, `linux`, `musl`, `linux-musl`, `unix`, `elf`, `64bit` | Always matches                 |
| `windows`, `macos`, `gnu`, `msvc`, `32bit`                    | Never matches                  |
| `x86_64`, `aarch64`                                           | Matches the guest architecture |
| `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`     | Matches the full guest target  |

If you use an unknown predicate, `cargo-xtest` skips the file and includes that predicate in the reason it reports.

## Declare required capabilities

Capability directives tell `cargo-xtest` what the test binary needs from its environment.

These are presence directives.

| Directive               | Current behavior                                                      |
| ----------------------- | --------------------------------------------------------------------- |
| `needs-threads`         | The self-contained profile treats threads as available.               |
| `needs-subprocess`      | The profile treats subprocesses as available.                         |
| `needs-symlink`         | The profile treats symbolic links as available.                       |
| `needs-target-std`      | The profile treats the target standard library as available.          |
| `needs-unwind`          | The profile treats unwinding as available.                            |
| `needs-dynamic-linking` | The profile skips the file because the test binary is self-contained. |

The [directive repetition rules][directive-repetition] also apply to capability directives.

## Configure libtest and the environment

### `run-flags`

Use `run-flags` to pass arguments to the libtest binary:

```rust
//@ run-flags: --nocapture
//@ run-flags: --exact 'module::named test'
```

If you use `run-flags` more than once, `cargo-xtest` appends the arguments in source order.

See the [directive repetition rules][directive-repetition] for the exact repetition behavior.

Arguments are split on whitespace.

Single quotes let you group text that contains spaces.

Double-quote escaping and backslash escaping are **not** supported.

### Color output

If you pass `--color`, `cargo-xtest` follows that setting.

Otherwise, it follows `CARGO_TERM_COLOR`.

The supported behaviors are:

- `always` — always use color.
- `never` — never use color.
- `auto` — use color when the corresponding host output stream is a terminal.

Cargo build output goes to standard error.

Test output goes to standard output.

If a test file includes a `--color` value through `run-flags`, that value takes precedence for that specific libtest binary.

When color is enabled, `cargo-xtest` writes a small terminal description of its own into the ephemeral VM and sets `TERM` and `TERMINFO` for libtest.

That works whether the guest starts from:

- the built-in image,
- an `image` directive, or
- a snapshot.

It does this without modifying the selected root filesystem.

If the test itself explicitly sets or unsets `TERM` or `TERMINFO`, `cargo-xtest` leaves terminal setup to that test and its selected image.

### `exec-env`

Use `exec-env` to set an environment variable inside the VM:

```rust
//@ exec-env: RUST_BACKTRACE=1
```

The value may contain additional `=` characters.

See the [directive repetition rules][directive-repetition] for the rules around repeating environment keys.

### `unset-exec-env`

Use `unset-exec-env` when you want an environment variable removed before libtest starts:

```rust
//@ unset-exec-env: RUST_LOG
```

The environment-variable key must:

- start with an ASCII letter or `_`, and
- contain only ASCII letters, digits, or `_` after that.

`exec-env` and `unset-exec-env` share the same [environment-key rule][directive-repetition].

## Choose the root filesystem

### `image`

Use `image` to start the VM from a particular OCI image reference:

```rust
//@ image: alpine:3.22
```

If you don't provide this directive, `cargo-xtest` uses its pinned built-in Alpine image.

### `from-snapshot`

If you've already prepared a Microsandbox snapshot, you can start the VM from it instead:

```rust
//@ from-snapshot: prepared-database
```

See the [directive conflict rules][directive-repetition] for which root-filesystem settings can and cannot be combined.

### `pull-policy`

Use `pull-policy` to decide when Microsandbox should pull an OCI image:

```rust
//@ pull-policy: never
```

Supported values are:

- `if-missing`
- `always`
- `never`

This directive applies only when you're using an image-based root filesystem.

See the [directive conflict rules][directive-repetition] for related conflicts.

### `root-disk`

Use `root-disk` to set the image root-disk size in mebibytes:

```rust
//@ root-disk: 8192
```

The value must be a positive base-10 integer.

Like `pull-policy`, `root-disk` applies only to image-based root filesystems.

See the [directive conflict rules][directive-repetition] for combinations that aren't allowed.

## Set VM resources

### `cpus`

Use `cpus` to choose how many virtual CPUs the VM receives:

```rust
//@ cpus: 2
```

The value must be:

- a positive integer, and
- small enough to fit in an unsigned 8-bit value.

### `memory`

Use `memory` to set VM memory in mebibytes:

```rust
//@ memory: 1024
```

The value must be a positive integer.

### `max-duration`

Use `max-duration` to limit the VM's maximum lifetime, in seconds:

```rust
//@ max-duration: 900
```

The value must be a positive integer.

## Configure the guest process

### `user`

Use `user` to run the test binary as a particular user inside the guest:

```rust
//@ user: tester
```

### `workdir`

Use `workdir` to set the test process's working directory inside the guest:

```rust
//@ workdir: /workspace
```

The path must be an absolute guest path.

### `shell`

Use `shell` to set the guest shell:

```rust
//@ shell: /bin/bash
```

The shell path must be an absolute guest path.

`cargo-xtest` uses this shell when it needs to remove environment variables before launching the test binary.

### `init`

Use `init` to choose the guest init program.

You can set it to `auto`:

```rust
//@ init: auto
```

or provide an absolute guest path.

## Configure the network

If a test file has no `network` directive, networking is disabled.

For a test that only needs DNS and public internet addresses, use the presence form:

```rust
//@ network
```

This public-access mode still blocks routed access to:

- private addresses,
- link-local addresses,
- metadata addresses,
- multicast addresses,
- loopback addresses, and
- host addresses.

The guest's own loopback interface still works inside the guest.

This mode does **not** publish any guest ports.

### Choose a network policy

If public access isn't enough, you can combine these profiles:

- `public`
- `private`
- `host`

For example:

```rust
//@ network: public,private,host
```

Here, `host` means the sandbox host through:

- its gateway address, and
- `host.microsandbox.internal`.

It does **not** mean the guest's own loopback interface.

The following modes stand on their own and cannot be combined with the profile names above:

| Value       | Behavior                                                         |
| ----------- | ---------------------------------------------------------------- |
| `none`      | Create the network interface but deny ingress and egress.        |
| `allow-all` | Allow every ingress and egress destination.                      |
| `custom`    | Use ordered rules and deny unmatched traffic in both directions. |

`allow-all` removes the destination protections you get from the default public-access profile.

Only use it when the test genuinely needs unrestricted traffic.

### Write a custom policy

With a custom policy, unmatched ingress and egress are denied by default.

You can set those defaults explicitly:

```rust
//@ network: custom
//@ network-default-egress: deny
//@ network-default-ingress: deny
```

Then add rules in the order you want them evaluated:

```rust
//@ network-rule: egress allow domain-suffix=example.com protocols=tcp ports=443
//@ network-rule: ingress allow group=public protocols=tcp ports=8080
```

Rules use this grammar:

```text
DIRECTION ACTION DESTINATION [protocols=LIST] [ports=LIST]
```

The fields work like this:

- **Direction:** `egress`, `ingress`, or `any`.
- **Action:** `allow` or `deny`.
- **Destination:** `any`, `ip=ADDRESS`, `cidr=CIDR`, `domain=NAME`, `domain-suffix=NAME`, or `group=NAME`.
- **Destination group:** `public`, `loopback`, `private`, `link-local`, `metadata`, `multicast`, or `host`.
- **Protocol list:** a comma-separated list of `tcp`, `udp`, `icmpv4`, or `icmpv6`. Leave it out to match every protocol.
- **Port list:** comma-separated guest-side ports or inclusive ranges such as `80,443,8000-8100`. Leave it out to match every guest-side port.

Rules are checked in source order, and the **first matching rule wins**.

Ingress rules and `any` rules cannot use ICMP because Microsandbox has no inbound ICMP path.

### Publish guest ports

Use `publish-port` when you need to map a host port to a TCP or UDP port inside the guest:

```rust
//@ network
//@ publish-port: tcp 18080:8080
//@ publish-port: udp 127.0.0.1:15353:5353
```

The shorter form, such as:

```text
18080:8080
```

binds the host side to loopback.

If you want another host bind address, put the host IP before the host port.

For IPv6 host addresses, put the address in brackets.

Be careful with non-loopback bind addresses: publishing there can make the guest service reachable from other machines.

### Configure DNS

By default, Microsandbox:

- uses the host's configured DNS resolvers,
- uses a 5,000-millisecond DNS query timeout, and
- enables DNS-rebinding protection.

You can override the DNS servers or query timeout when needed:

```rust
//@ network
//@ dns-server: 1.1.1.1
//@ dns-server: dns.google:53
//@ dns-query-timeout: 2500
```

`dns-server` accepts:

- an IP address, or
- a hostname,

with an optional port.

You can provide more than one resolver. See the [directive repetition rules][directive-repetition] for the exact repetition behavior.

If your test specifically needs to accept private addresses returned by public DNS, add:

```rust
//@ no-dns-rebind-protection
```

Only use that directive when the test actually needs that behavior.

### Configure TLS interception

TLS interception has to be enabled explicitly:

```rust
//@ network
//@ tls-intercept
```

When enabled, Microsandbox:

- intercepts TCP port `443`,
- verifies upstream TLS certificates, and
- blocks QUIC on intercepted ports by default.

If you provide **any** `tls-intercept-port` directive, your explicit list replaces the default intercepted-port list.

See the [directive repetition rules][directive-repetition] for TLS options that support repeated declarations.

These directives let you fine-tune the behavior:

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

If you provide your own interception CA, you must provide **both**:

- the CA certificate, and
- its matching private key.

Provide both or neither.

If you don't provide them, Microsandbox generates an interception CA and persists it in the Microsandbox runtime directory.

CA paths must either:

- be absolute host paths, or
- start with `{{src-base}}`.

`{{src-base}}` means the directory that contains the test file.

For example:

```rust
//@ tls-upstream-ca-cert: {{src-base}}/certificates/test-ca.pem
```

Those CA files stay on the host.

`cargo-xtest` resolves their paths and passes those paths to Microsandbox. It does **not** mount the test directory into the guest VM.

### Set connection and interface options

The following directives only make sense when `network` is enabled:

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

If you don't set addresses, address pools, or the MAC manually, Microsandbox derives them from the sandbox slot.

`trust-host-cas` expands the guest's trust store.

Only use it when the test needs to trust certificates that the host already accepts.

## Review the defaults

The most reliable way to see the effective configuration for a specific file is:

```sh
cargo xtest explain <test-file>
```

For a test file with no directives, these are the defaults:

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
| User              | Image default                |
| Working directory | Image default                |
| Shell             | `/bin/sh`                    |
| Init              | None                         |

## Avoid duplicate and conflicting directives

Most value directives and presence settings can only be declared once.

If you declare one of them twice, `cargo-xtest` reports the duplicate and points you back to the first declaration.

The following directives are allowed to repeat:

- `run-flags` — each declaration appends more libtest arguments.
- `only-<predicate>` and `ignore-<predicate>` — as long as every repeated predicate is unique.
- `exec-env` and `unset-exec-env` — as long as a given environment key appears only once across both kinds of directive.
- `network-rule` — repetition is expected, and source order controls first-match evaluation.
- `publish-port`
- `dns-server`
- `tls-intercept-port`
- `tls-bypass`
- `tls-verify-upstream-for`
- `tls-upstream-ca-cert`
- `tls-upstream-ca-cert-for`

The root-filesystem directives also have these specific conflicts:

- `image` conflicts with `from-snapshot`.
- `pull-policy` conflicts with `from-snapshot`.
- `root-disk` conflicts with `from-snapshot`.

See [Choose the root filesystem][root-filesystem] for the behavior and examples for those options.

## Read diagnostics and exit statuses

When a directive is invalid, `cargo-xtest` reports a diagnostic that includes:

- a stable `XT` error code,
- the test filename and source location,
- the relevant source line, and
- help text when `cargo-xtest` can suggest a correction.

For example:

```text
error[XT005]: `from-snapshot` conflicts with the earlier `image` directive
 --> tests/database.rs:2:5
  |
2 | //@ from-snapshot: prepared-database
  |     ^^^^^^^^^^^^^ conflicting rootfs source
  |
  = note: first rootfs source declared at tests/database.rs:1:5
```

Compiler output and libtest output stay visible during a run.

The main exit statuses are:

| Status | Meaning                                                                              |
| ------ | ------------------------------------------------------------------------------------ |
| `0`    | Every eligible test target passed, or `cargo-xtest` skipped it.                      |
| `1`    | `cargo-xtest` could not complete discovery, configuration, VM execution, or cleanup. |
| `2`    | The command line was invalid.                                                        |
| `101`  | A libtest binary failed. Cargo also commonly uses this status for its own failures.  |

If Cargo compilation fails, `cargo-xtest` preserves Cargo's nonzero status when that status fits in an 8-bit exit code.

Any nonzero exit from the guest test process makes `cargo-xtest` return status `101`.

## Current limitations

A few boundaries are worth knowing before you depend on `cargo-xtest`:

- `cargo-xtest` runs **Cargo integration-test targets only**. It does not run unit tests, documentation tests, or benchmark targets.
- Every guest uses Linux-musl and matches the host's architecture. macOS and Windows guest binaries are not supported.
- Dynamic linking is not available in the self-contained execution profile.
- The pinned Microsandbox SDK describes Windows host support as preview. Its prebuilt feature does not install the Windows runtime artifacts that are required, and this project does not run real-VM tests on Windows.
- `cargo-xtest` does not provide service-specific orchestration. If your test needs a database, daemon, or some other service, prepare the selected image or snapshot with those services yourself.

[defaults]: #review-the-defaults
[directive-repetition]: #avoid-duplicate-and-conflicting-directives
[execution-flow]: #understand-the-execution-flow
[root-filesystem]: #choose-the-root-filesystem
[trust-boundary]: #know-the-trust-boundary
