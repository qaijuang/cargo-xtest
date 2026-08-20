#![cfg(not(miri))]

mod support;

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use microsandbox::sandbox::SandboxStatus;
#[cfg(unix)]
use microsandbox::{MicrosandboxError, Sandbox};
use support::is_vm_tests_enabled;

const MARKER: &[u8] = b"guest output is live\n";

#[test]
fn streams_guest_stdout_before_the_guest_exits() {
    if !is_vm_tests_enabled() {
        return;
    }

    let (mut child, received_lines, reader) = spawn_case("streaming");
    wait_for_marker(&received_lines, MARKER);
    let marker_received = Instant::now();
    let status = child.wait().unwrap();
    let output = reader.join().unwrap();

    assert!(status.success(), "cargo-xtest failed with {status}: {output:?}");
    assert!(
        marker_received.elapsed() >= Duration::from_secs(2),
        "guest output was buffered until the guest exited: {output:?}"
    );
}

#[test]
fn streams_guest_stderr_before_the_guest_exits() {
    if !is_vm_tests_enabled() {
        return;
    }

    let (mut child, received_lines, reader) = spawn_case_stream("streaming", true);
    wait_for_marker(&received_lines, b"guest stderr is live\n");
    let marker_received = Instant::now();
    let status = child.wait().unwrap();
    let output = reader.join().unwrap();

    assert!(status.success(), "cargo-xtest failed with {status}: {output:?}");
    assert!(
        marker_received.elapsed() >= Duration::from_secs(2),
        "guest stderr was buffered until the guest exited: {output:?}"
    );
}

#[test]
fn streams_cargo_stderr_before_cargo_exits() {
    if !is_vm_tests_enabled() {
        return;
    }

    let (mut child, received_lines, reader) = spawn_case_stream("compiler-streaming", true);
    wait_for_marker(&received_lines, b"cargo compiler output is live\n");
    let marker_received = Instant::now();
    let status = child.wait().unwrap();
    let output = reader.join().unwrap();

    assert_eq!(status.code(), Some(101), "unexpected status {status}: {output:?}");
    assert!(
        marker_received.elapsed() >= Duration::from_secs(2),
        "Cargo output was buffered until Cargo exited: {output:?}"
    );
}

#[test]
#[cfg(unix)]
fn interrupts_cargo_compilation() {
    if !is_vm_tests_enabled() {
        return;
    }

    let (mut child, received_lines, reader) = spawn_case_stream("compiler-streaming", true);
    wait_for_marker(&received_lines, b"cargo compiler output is live\n");
    send_signal(&child, "INT");

    let status = child.wait().unwrap();
    let output = reader.join().unwrap();
    assert_eq!(status.code(), Some(130), "unexpected status {status}: {output:?}");
}

#[test]
#[cfg(unix)]
fn interrupts_sandbox_setup() {
    if !is_vm_tests_enabled() {
        return;
    }

    let (mut child, received_lines, reader) = spawn_case("streaming");
    wait_for_marker(&received_lines, b"running ");
    send_signal(&child, "INT");

    let status = child.wait().unwrap();
    let output = reader.join().unwrap();
    assert_eq!(status.code(), Some(130), "unexpected status {status}: {output:?}");
    assert_sandbox_stopped(&format!("cargo-xtest-{}-0", child.id()));
}

#[test]
#[cfg(unix)]
fn forwards_interrupt_and_stops_the_sandbox() {
    if !is_vm_tests_enabled() {
        return;
    }

    assert_signal("INT", 130);
}

#[test]
#[cfg(unix)]
fn forwards_termination_and_stops_the_sandbox() {
    if !is_vm_tests_enabled() {
        return;
    }

    assert_signal("TERM", 143);
}

#[cfg(unix)]
fn assert_signal(signal_name: &str, expected_status: i32) {
    let (mut child, received_lines, reader) = spawn_case("interrupt");
    let sandbox_name = format!("cargo-xtest-{}-0", child.id());
    wait_for_marker(&received_lines, b"guest is waiting for an interrupt\n");

    send_signal(&child, signal_name);

    let status = child.wait().unwrap();
    let output = reader.join().unwrap();
    assert_eq!(status.code(), Some(expected_status), "unexpected status {status}: {output:?}");
    assert_sandbox_stopped(&sandbox_name);
}

#[cfg(unix)]
fn send_signal(child: &Child, signal_name: &str) {
    let signal = Command::new("kill")
        .args([format!("-{signal_name}"), child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal.success(), "could not send SIG{signal_name}: {signal}");
}

#[test]
fn preserves_non_utf8_guest_stdout() {
    if !is_vm_tests_enabled() {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-xtest"))
        .args(["xtest"])
        .current_dir(format!("{}/tests/runtime/non-utf8", env!("CARGO_MANIFEST_DIR")))
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .unwrap();

    assert!(output.status.success(), "cargo-xtest failed: {output:?}");
    assert!(
        output
            .stdout
            .windows(b"guest byte: \xff\n".len())
            .any(|bytes| bytes == b"guest byte: \xff\n"),
        "guest stdout was not preserved: {:?}",
        output.stdout
    );
}

fn spawn_case(case: &str) -> (Child, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<Vec<u8>>) {
    spawn_case_stream(case, false)
}

fn spawn_case_stream(
    case: &str,
    read_stderr: bool,
) -> (Child, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<Vec<u8>>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-xtest"));
    command
        .args(["xtest"])
        .current_dir(format!("{}/tests/runtime/{case}", env!("CARGO_MANIFEST_DIR")))
        .env("CARGO_TERM_COLOR", "never");
    if read_stderr {
        command.stdout(Stdio::inherit()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::inherit());
    }
    let mut child = command.spawn().unwrap();
    let stream: Box<dyn std::io::Read + Send> = if read_stderr {
        Box::new(child.stderr.take().unwrap())
    } else {
        Box::new(child.stdout.take().unwrap())
    };
    let (lines, received_lines) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut stream = BufReader::new(stream);
        let mut output = Vec::new();
        loop {
            let mut line = Vec::new();
            if stream.read_until(b'\n', &mut line).unwrap() == 0 {
                break;
            }
            output.extend_from_slice(&line);
            lines.send(line).unwrap();
        }
        output
    });
    (child, received_lines, reader)
}

fn wait_for_marker(received_lines: &mpsc::Receiver<Vec<u8>>, marker: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = received_lines.recv_timeout(remaining).unwrap();
        if line.windows(marker.len()).any(|bytes| bytes == marker) {
            return;
        }
    }
}

#[cfg(unix)]
fn assert_sandbox_stopped(name: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    match runtime.block_on(Sandbox::get(name)) {
        Ok(sandbox) => assert_eq!(sandbox.status_snapshot(), SandboxStatus::Stopped),
        Err(MicrosandboxError::SandboxNotFound(_)) => {}
        Err(error) => panic!("could not inspect sandbox `{name}`: {error}"),
    }
}
