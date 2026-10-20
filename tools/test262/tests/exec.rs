//! `ep9-test262 exec` on a small fake test262 tree: pass and fail rules (flags, negative
//! phases, async, includes, raw), and infrastructure errors (timeout, a run that returns after
//! the timeout, panic, worker crash, stuck worker, internal BytecodeGen error) that exit 2 and
//! are never written to the expectation file.

use std::path::{Path, PathBuf};
use std::process::Command;

const ASSERT: &str = r#"
function assert(c, m) { if (c !== true) throw new Test262Error(m || "assert"); }
assert.sameValue = function (a, b, m) {
  if (a !== b) throw new Test262Error((m || "") + " expected " + b + " got " + a);
};
"#;
const STA: &str = r#"
function Test262Error(m) { this.message = m || ""; }
Test262Error.prototype.toString = function () { return "Test262Error: " + this.message; };
function $DONOTEVALUATE() { throw "Test262: This statement should not be evaluated."; }
"#;
const DONE: &str = r#"
function $DONE(error) {
  if (error) { print("Test262:AsyncTestFailure:" + error); } else { print("Test262:AsyncTestComplete"); }
}
"#;

fn test(meta: &str, body: &str) -> String {
    format!("/*---\ndescription: x\n{meta}---*/\n{body}\n")
}

/// A fresh tree with tests that need no built-ins beyond the v0 realm.
fn tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ep9-test262-exec-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(&root, "harness/assert.js", ASSERT);
    write(&root, "harness/sta.js", STA);
    write(&root, "harness/doneprintHandle.js", DONE);
    write(
        &root,
        "harness/extra.js",
        "function extra() { return 42; }\n",
    );
    let tests: &[(&str, String)] = &[
        ("pass.js", test("", "assert.sameValue(1 + 1, 2);")),
        ("fail.js", test("", "assert.sameValue(1, 2, 'one');")),
        (
            "neg-parse.js",
            test(
                "negative:\n  phase: parse\n  type: SyntaxError\n",
                "$DONOTEVALUATE();\nvar = ;",
            ),
        ),
        (
            "neg-runtime.js",
            test(
                "negative:\n  phase: runtime\n  type: TypeError\n",
                "null.x;",
            ),
        ),
        (
            "neg-runtime-wrong.js",
            test(
                "negative:\n  phase: runtime\n  type: TypeError\n",
                "throw new RangeError('r');",
            ),
        ),
        // The constructor of the thrown value names the type, not its text.
        (
            "neg-runtime-string.js",
            test(
                "negative:\n  phase: runtime\n  type: TypeError\n",
                "throw 'TypeError: not an error object';",
            ),
        ),
        (
            "neg-runtime-object.js",
            test(
                "negative:\n  phase: runtime\n  type: TypeError\n",
                "throw { toString: function () { return 'TypeError: fake'; } };",
            ),
        ),
        (
            "neg-runtime-t262.js",
            test(
                "negative:\n  phase: runtime\n  type: Test262Error\n",
                "throw new Test262Error('x');",
            ),
        ),
        (
            "strict-only.js",
            test(
                "flags: [onlyStrict]\n",
                "assert.sameValue((function () { return this; })(), undefined);",
            ),
        ),
        (
            "sloppy-only.js",
            test(
                "flags: [noStrict]\n",
                "implicit = 1; assert.sameValue(implicit, 1);",
            ),
        ),
        (
            "raw.js",
            test(
                "flags: [raw]\n",
                "if (typeof assert !== 'undefined') throw new Error('harness loaded');",
            ),
        ),
        ("async-pass.js", test("flags: [async]\n", "$DONE();")),
        ("async-fail.js", test("flags: [async]\n", "$DONE('bad');")),
        ("async-no-done.js", test("flags: [async]\n", "var x = 1;")),
        // A failure marker fails the test even if a completion marker follows it.
        (
            "async-fail-then-done.js",
            test("flags: [async]\n", "$DONE('bad'); $DONE();"),
        ),
        // One completion signal: a second $DONE() is a fail.
        (
            "async-done-twice.js",
            test("flags: [async]\n", "$DONE(); $DONE();"),
        ),
        (
            "include.js",
            test("includes: [extra.js]\n", "assert.sameValue(extra(), 42);"),
        ),
        ("unsupported.js", test("", "function* g() {}")),
    ];
    for (name, src) in tests {
        write(&root, &format!("test/language/{name}"), src);
    }
    root
}

const FAILING: &[&str] = &[
    "test/language/async-done-twice.js",
    "test/language/async-fail-then-done.js",
    "test/language/async-fail.js",
    "test/language/async-no-done.js",
    "test/language/fail.js",
    "test/language/neg-runtime-object.js",
    "test/language/neg-runtime-string.js",
    "test/language/neg-runtime-wrong.js",
    "test/language/unsupported.js",
];

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn expect_file(root: &Path, paths: &[&str]) -> PathBuf {
    let path = root.join("expect.txt");
    std::fs::write(&path, paths.join("\n") + "\n").unwrap();
    path
}

fn exec(root: &Path, expect: &Path, extra: &[&str], fault: Option<&str>) -> (Option<i32>, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ep9-test262"));
    cmd.arg("exec")
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
fn pass_and_fail_rules() {
    let root = tree("rules");
    let expect = expect_file(&root, FAILING);
    let json = root.join("summary.json");
    let (code, text) = exec(&root, &expect, &["--json", json.to_str().unwrap()], None);
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("language             9 / 18"), "{text}");
    let summary = std::fs::read_to_string(&json).unwrap();
    assert!(
        summary.contains("\"cause\":\"unsupported: generator functions\""),
        "{summary}"
    );
    assert!(
        summary.contains("\"cause\":\"async test: $DONE was not called\""),
        "{summary}"
    );
    assert!(
        summary.contains("\"cause\":\"negative runtime test: wrong error\""),
        "{summary}"
    );
    assert!(
        summary.contains("\"cause\":\"async test: $DONE was called more than once\""),
        "{summary}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn update_writes_only_failures() {
    let root = tree("update");
    let expect = root.join("new-expect.txt");
    let (code, text) = exec(&root, &expect, &["--update-expectations"], None);
    assert_eq!(code, Some(0), "{text}");
    let written = std::fs::read_to_string(&expect).unwrap();
    let listed: Vec<&str> = written.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(listed, FAILING);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_failure_and_now_passing_exit_1() {
    let root = tree("diff");
    let mut listed: Vec<&str> = FAILING
        .iter()
        .copied()
        .filter(|p| !p.ends_with("/fail.js"))
        .collect();
    listed.push("test/language/pass.js");
    let expect = expect_file(&root, &listed);
    let (code, text) = exec(&root, &expect, &[], None);
    assert_eq!(code, Some(1), "{text}");
    assert!(text.contains("NEW FAILURE test/language/fail.js"), "{text}");
    assert!(text.contains("test/language/pass.js"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn timeout_is_an_infra_error() {
    let root = tree("timeout");
    write(&root, "test/language/loop.js", &test("", "while (true) {}"));
    let mut listed = FAILING.to_vec();
    listed.push("test/language/loop.js");
    let expect = expect_file(&root, &listed);
    let before = std::fs::read_to_string(&expect).unwrap();
    let (code, text) = exec(
        &root,
        &expect,
        &["--timeout", "0.5", "--update-expectations"],
        None,
    );
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/loop.js: [sloppy] timeout"),
        "{text}"
    );
    assert!(text.contains("timeout 1"), "{text}");
    // An expectation file cannot hide it, and --update-expectations does not write it.
    assert_eq!(std::fs::read_to_string(&expect).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn panic_is_an_infra_error() {
    let root = tree("panic");
    let expect = expect_file(&root, FAILING);
    let (code, text) = exec(&root, &expect, &[], Some("panic:/pass.js"));
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/pass.js: [sloppy] panic: injected panic"),
        "{text}"
    );
    assert!(text.contains("panic 1"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn worker_crash_is_an_infra_error_and_the_run_goes_on() {
    let root = tree("crash");
    let expect = expect_file(&root, FAILING);
    let json = root.join("summary.json");
    let (code, text) = exec(
        &root,
        &expect,
        &["--json", json.to_str().unwrap()],
        Some("abort:/neg-parse.js"),
    );
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/neg-parse.js: the worker exited"),
        "{text}"
    );
    assert!(text.contains("worker crash 1"), "{text}");
    // Every other test still ran in a new worker.
    assert!(text.contains("language             8 / 18"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stuck_worker_is_killed() {
    let root = tree("stuck");
    let expect = expect_file(&root, FAILING);
    let (code, text) = exec(
        &root,
        &expect,
        &["--timeout", "0.5"],
        Some("hang:/include.js"),
    );
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/include.js: hard timeout"),
        "{text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_that_returns_after_the_deadline_is_a_timeout() {
    // A native loop that does not poll termination can outlast the deadline and then return
    // normally: still a timeout, never a pass.
    let root = tree("late");
    let expect = expect_file(&root, FAILING);
    let before = std::fs::read_to_string(&expect).unwrap();
    let (code, text) = exec(
        &root,
        &expect,
        &["--timeout", "0.5", "--update-expectations"],
        Some("late:/pass.js"),
    );
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/pass.js: [sloppy] timeout"),
        "{text}"
    );
    assert!(text.contains("timeout 1"), "{text}");
    assert_eq!(std::fs::read_to_string(&expect).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn internal_codegen_error_is_an_infra_error() {
    // A BytecodeGen bug (builder or verifier rejection) in a test, a negative parse test or a
    // harness include: never a test failure, so no expectation file can list it.
    let root = tree("internal");
    let expect = expect_file(&root, FAILING);
    let before = std::fs::read_to_string(&expect).unwrap();
    let (code, text) = exec(
        &root,
        &expect,
        &["--update-expectations"],
        Some("internal:/pass.js,internal:/neg-parse.js,internal:harness/extra.js"),
    );
    assert_eq!(code, Some(2), "{text}");
    for path in ["pass.js", "neg-parse.js", "include.js"] {
        assert!(
            text.contains(&format!("test/language/{path}: [sloppy] ")),
            "{path}: {text}"
        );
    }
    assert!(
        text.contains("internal BytecodeGen error: injected"),
        "{text}"
    );
    assert!(text.contains("infrastructure errors: 3"), "{text}");
    assert_eq!(std::fs::read_to_string(&expect).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}
