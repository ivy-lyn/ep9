//! `cargo xtask api-list` (H20-Q6) on the rustdoc JSON fixtures in `tests/api_list/` (README
//! there): the exit codes and the diff lines. No rustdoc runs (`--json`), so no nightly is
//! needed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("api_list")
        .join(name)
}

fn api_list(json: &str, extra: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("api-list")
        .arg("--json")
        .arg(fixture(json))
        .arg("--golden")
        .arg(fixture("golden.txt"))
        .args(extra)
        .output()
        .unwrap();
    eprintln!(
        "--- api-list {json} {extra:?}: {}\n--- stdout\n{}--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn the_golden_is_the_listing_of_the_base_json() {
    let out = api_list("base.json", &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("api-list: 4 lines, golden 4 lines: 0 new, 0 gone"));
    let p = api_list("base.json", &["--print"]);
    let golden = std::fs::read_to_string(fixture("golden.txt")).unwrap();
    let want: Vec<&str> = golden.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(stdout(&p).lines().collect::<Vec<_>>(), want);
}

/// The acceptance of H20-Q6 (plan v12 §8.1): a new `pub` item fails the golden diff.
#[test]
fn a_new_pub_item_fails_the_diff() {
    let out = api_list("new-pub-item.json", &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    let diff: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("api-list:"))
        .collect();
    assert_eq!(diff, ["+ k::probe_new_item :: fn probe_new_item() -> u32"]);
    assert!(text.contains("1 new, 0 gone"), "{text}");
}

#[test]
fn another_format_version_and_bad_arguments_exit_2() {
    let out = api_list("format-62.json", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("format_version 62"));
    assert_eq!(
        api_list("base.json", &["--bless", "--print"]).status.code(),
        Some(2)
    );
    assert_eq!(api_list("base.json", &["--nope"]).status.code(), Some(2));
}
