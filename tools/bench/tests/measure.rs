//! Runs the built `ep9-bench` on a tiny command. macOS must always give a count. Other hosts
//! skip when they have no instruction counter (no perf access, no valgrind, Windows).

use std::process::Command;

fn true_cmd() -> &'static str {
    if cfg!(target_os = "macos") {
        "/usr/bin/true"
    } else {
        "/bin/true"
    }
}

#[test]
fn counts_instructions() {
    let out = Command::new(env!("CARGO_BIN_EXE_ep9-bench"))
        .args(["--runs", "2", "--warmup", "0", "--name", "true", "--"])
        .arg(true_cmd())
        .output()
        .expect("run ep9-bench");
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success()
        && !cfg!(target_os = "macos")
        && stderr.contains("no instruction counter")
    {
        eprintln!("skipped: {stderr}");
        return;
    }
    assert!(out.status.success(), "ep9-bench failed: {stderr}");
    let json = String::from_utf8(out.stdout).expect("utf-8 output");
    assert!(json.contains("\"schema\": \"ep9-bench/v0\""), "{json}");
    assert_eq!(json.matches("\"instructions\": ").count(), 3, "{json}");
    // Every run of a real process retires instructions.
    assert!(!json.contains("\"instructions\": 0,"), "{json}");
}

#[test]
fn failing_command_is_an_error() {
    if !cfg!(target_os = "macos") {
        return; // needs a counter; macOS always has one
    }
    let out = Command::new(env!("CARGO_BIN_EXE_ep9-bench"))
        .args(["--runs", "1", "--", "/usr/bin/false"])
        .output()
        .expect("run ep9-bench");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("exited with"));
}
