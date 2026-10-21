//! `ep9-test262 parse` on a small fake test262 tree: a file that the runner cannot check (read
//! error, bad front matter, missing include, a parser panic, a worker crash) is an error (exit
//! 2). An expectation file must not hide it, and `--update-expectations` must not write it.
//! The parse-side test hooks (`EP9_TEST262_FAULT=parse-*`) make the parse workers fail.

use std::path::{Path, PathBuf};
use std::process::Command;

const OK: &str = "/*---\ndescription: parses\n---*/\nvar a;\n";
/// A real parse failure: a positive test with a syntax error.
const FAILS: &str = "/*---\ndescription: does not parse\n---*/\nvar = ;\n";

/// A fresh tree: harness/{assert,sta}.js, test/language/{ok,fails}.js, the other test dirs empty.
fn tree(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("ep9-test262-infra-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["harness", "test/built-ins", "test/annexB", "test/intl402"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    write(&root, "harness/assert.js", b"var assert;\n");
    write(&root, "harness/sta.js", b"var Test262Error;\n");
    write(&root, "test/language/ok.js", OK.as_bytes());
    write(&root, "test/language/fails.js", FAILS.as_bytes());
    root
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn expect_file(root: &Path, paths: &[&str]) -> PathBuf {
    let path = root.join("expect.txt");
    std::fs::write(&path, paths.join("\n") + "\n").unwrap();
    path
}

/// Run `parse` on `root` and return the exit code and stdout + stderr.
fn parse(root: &Path, expect: &Path, extra: &[&str]) -> (Option<i32>, String) {
    parse_with_fault(root, expect, extra, None)
}

/// As [`parse`], with `EP9_TEST262_FAULT` set to `fault` (or unset).
fn parse_with_fault(
    root: &Path,
    expect: &Path,
    extra: &[&str],
    fault: Option<&str>,
) -> (Option<i32>, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ep9-test262"));
    cmd.arg("parse")
        .arg("--test262")
        .arg(root)
        .arg("--expect")
        .arg(expect)
        .args(["--threads", "2"])
        .args(extra);
    match fault {
        Some(f) => cmd.env("EP9_TEST262_FAULT", f),
        None => cmd.env_remove("EP9_TEST262_FAULT"),
    };
    let out = cmd.output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code(), text)
}

#[test]
fn listed_parse_failure_matches() {
    let root = tree("control");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse(&root, &expect, &[]);
    assert_eq!(code, Some(0), "{text}");
    assert!(!text.contains("could not be checked"), "{text}");
    assert!(
        text.contains("harness include files          2 / 2"),
        "{text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parse_panic_is_an_infra_error() {
    // Caught in the worker today (a `[mode] panic:` run); under `panic = "abort"` it ends the
    // worker and the text comes from its stderr. Either way: exit 2 with the panic text.
    let root = tree("panic");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse_with_fault(&root, &expect, &[], Some("parse-panic:/ok.js"));
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("injected parse panic for test/language/ok.js"),
        "{text}"
    );
    assert!(text.contains("1 of 4 files could not be checked"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parse_worker_crash_is_an_infra_error_and_the_run_goes_on() {
    // One worker: it aborts on a test, then a new one aborts on an include (the second pass),
    // and the other files are still checked.
    let root = tree("abort");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse_with_fault(
        &root,
        &expect,
        &["--threads", "1"],
        Some("parse-abort:/ok.js,parse-abort:harness/sta.js"),
    );
    assert_eq!(code, Some(2), "{text}");
    for path in ["test/language/ok.js", "harness/sta.js"] {
        assert!(
            text.contains(&format!("{path}: worker crash: the worker exited")),
            "{path}: {text}"
        );
    }
    assert!(text.contains("2 of 4 files could not be checked"), "{text}");
    assert!(!text.contains("test/language/fails.js:"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_panic_that_aborts_the_worker_reaches_the_report() {
    // A panic that cannot unwind ends the worker after the panic hook ran, as every panic does
    // under `panic = "abort"`: the infrastructure line carries the panic text.
    let root = tree("nounwind");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse_with_fault(&root, &expect, &[], Some("parse-nounwind-panic:/ok.js"));
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/ok.js: worker crash: the worker exited"),
        "{text}"
    );
    assert!(
        text.contains("panicked: injected parse panic for test/language/ok.js"),
        "{text}"
    );
    assert!(text.contains("1 of 4 files could not be checked"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_crash_that_does_not_repeat_alone_is_still_an_infra_error() {
    // The worker gets fails.js and ok.js together and aborts on fails.js only while ok.js
    // waits: run again one at a time, neither file crashes. The crash still counts (exit 2),
    // on the first file of the group.
    let root = tree("group");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse_with_fault(
        &root,
        &expect,
        &["--threads", "1"],
        Some("parse-abort-in-group:/fails.js"),
    );
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/fails.js: worker crash: the worker exited"),
        "{text}"
    );
    assert!(text.contains("none stopped it again"), "{text}");
    assert!(text.contains("1 of 4 files could not be checked"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn listed_test_with_invalid_utf8_is_an_error() {
    let root = tree("utf8");
    write(
        &root,
        "test/language/fails.js",
        &[FAILS.as_bytes(), b"\xff"].concat(),
    );
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse(&root, &expect, &[]);
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/fails.js: file is not valid UTF-8"),
        "{text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn listed_test_with_bad_front_matter_is_an_error() {
    let root = tree("frontmatter");
    write(
        &root,
        "test/language/fails.js",
        FAILS.replace("/*---", "/*--x").as_bytes(),
    );
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse(&root, &expect, &[]);
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/fails.js: front matter:"),
        "{text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn listed_missing_include_is_an_error() {
    let root = tree("include");
    std::fs::remove_file(root.join("harness/sta.js")).unwrap();
    let expect = expect_file(&root, &["harness/sta.js", "test/language/fails.js"]);
    let (code, text) = parse(&root, &expect, &[]);
    assert_eq!(code, Some(2), "{text}");
    assert!(text.contains("harness/sta.js: cannot read:"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unlisted_unreadable_test_is_an_error() {
    let root = tree("unlisted");
    write(&root, "test/language/bad.js", b"\xff");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let (code, text) = parse(&root, &expect, &[]);
    assert_eq!(code, Some(2), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn update_does_not_write_infra_errors() {
    let root = tree("update");
    write(&root, "test/language/bad.js", b"\xff");
    let expect = expect_file(&root, &["test/language/fails.js"]);
    let before = std::fs::read_to_string(&expect).unwrap();
    let (code, text) = parse(&root, &expect, &["--update-expectations"]);
    assert_eq!(code, Some(2), "{text}");
    assert_eq!(std::fs::read_to_string(&expect).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}
