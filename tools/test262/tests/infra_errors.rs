//! `ep9-test262 parse` on a small fake test262 tree: a file that the runner cannot check (read
//! error, bad front matter, missing include) is an error (exit 2). An expectation file must not
//! hide it, and `--update-expectations` must not write it.

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
    let out = Command::new(env!("CARGO_BIN_EXE_ep9-test262"))
        .arg("parse")
        .arg("--test262")
        .arg(root)
        .arg("--expect")
        .arg(expect)
        .args(["--threads", "2"])
        .args(extra)
        .output()
        .unwrap();
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
