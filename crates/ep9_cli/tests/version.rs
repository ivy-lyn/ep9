//! Runs the built `ep9` binary and checks its version output.

use std::process::Command;

#[test]
fn prints_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
        .arg("--version")
        .output()
        .expect("run ep9");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    // Line 1 is the version; line 2 the build line `features <list>; preset <name>` (M5-P0
    // GATES5, MC-w17-5).
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some(concat!("ep9 ", env!("CARGO_PKG_VERSION")))
    );
    let build = lines.next().expect("the build line");
    assert!(
        build.starts_with("features ") && build.contains("; preset "),
        "{stdout}"
    );
    assert_eq!(lines.next(), None, "{stdout}");
}
