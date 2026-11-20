//! The lock guard (`lock-guard.sh`, dep-guards check 27; H26:42) on the two synced locks and on
//! planted copies of the ep9-typecheck lock. Each case runs the script with bash.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn here() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn tsrs_lock() -> PathBuf {
    here().join("../../third_party/tsrs/Cargo.lock")
}

fn own_lock() -> PathBuf {
    here().join("Cargo.lock")
}

fn own_text() -> String {
    std::fs::read_to_string(own_lock()).unwrap()
}

/// Runs the guard on the 2 locks, with `CI=true` or with no `CI`.
fn guard(tsrs: &Path, own: &Path, ci: bool) -> Output {
    let mut c = Command::new("bash");
    c.arg(here().join("lock-guard.sh")).arg(tsrs).arg(own);
    c.env_remove("CI");
    if ci {
        c.env("CI", "true");
    }
    c.output().unwrap()
}

/// A copy of the ep9-typecheck lock with other text.
fn planted(name: &str, text: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lock-guard-{name}.lock"));
    std::fs::write(&p, text).unwrap();
    p
}

/// The guard on the tsrs lock and `own`: its exit status and its stdout.
fn check(own: &Path) -> (Option<i32>, String) {
    let o = guard(&tsrs_lock(), own, false);
    (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned())
}

#[test]
fn the_synced_locks_pass() {
    let (code, out) = check(&own_lock());
    assert_eq!(code, Some(0), "{out}");
    assert!(
        out.starts_with("PASS: (27) the ep9-typecheck lock ("),
        "{out}"
    );
}

#[test]
fn an_extra_package_fails() {
    let text = own_text()
        + "\n[[package]]\nname = \"evil\"\nversion = \"1.0.0\"\n\
           source = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"00\"\n";
    let (code, out) = check(&planted("extra", &text));
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("FAIL: (27) extra: evil 1.0.0"), "{out}");
}

#[test]
fn a_changed_version_fails() {
    let text = own_text();
    let at = text.find("name = \"sha2\"\nversion = \"").expect("sha2 is in the lock");
    let start = at + "name = \"sha2\"\nversion = \"".len();
    let end = start + text[start..].find('"').unwrap();
    let text = format!("{}0.0.1{}", &text[..start], &text[end..]);
    let (code, out) = check(&planted("version", &text));
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("FAIL: (27) changed: sha2 0.0.1"), "{out}");
}

#[test]
fn a_package_header_with_a_comment_fails() {
    // Valid TOML: the package must not be hidden from the comparison.
    let text = own_text() + "\n[[package]] # planted\nname = \"evil\"\nversion = \"1.0.0\"\n";
    let (code, out) = check(&planted("comment", &text));
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("is not in the Cargo lock form: [[package]] # planted"), "{out}");
}

#[test]
fn a_key_with_no_spaces_fails() {
    let text = own_text() + "\n[[package]]\nname=\"evil\"\nversion = \"1.0.0\"\n";
    let (code, out) = check(&planted("nospace", &text));
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("is not in the Cargo lock form: name=\"evil\""), "{out}");
}
