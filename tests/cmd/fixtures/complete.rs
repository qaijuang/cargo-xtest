//@ ignore-test (temporarily blocked)
//@ only-linux (VM guests are Linux)
//@ ignore-aarch64 (fixture is x86_64-only)
//@ needs-threads
//@ needs-subprocess
//@ needs-symlink
//@ needs-dynamic-linking
//@ needs-target-std
//@ needs-unwind
//@ run-flags: --exact 'named case'
//@ exec-env: RUST_BACKTRACE=1
//@ unset-exec-env: RUST_LOG
//@ image: ghcr.io/example/test@sha256:abcd
//@ pull-policy: never
//@ cpus: 2
//@ memory: 1024
//@ root-disk: 8192
//@ max-duration: 900
//@ user: tester
//@ workdir: /workspace
//@ shell: /bin/bash
//@ init: auto
//@ preserve-on-failure

#[test]
fn uses_complete_configuration() {}
