//! Runs the built `ep9` binary and checks its output.

use std::process::Command;

#[test]
fn prints_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
        .output()
        .expect("run ep9");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    assert_eq!(
        stdout.trim_end(),
        concat!("ep9 ", env!("CARGO_PKG_VERSION"))
    );
}
