//! `ep9-test262 exec` on a small fake test262 tree: pass and fail rules (flags, negative
//! phases, async, includes, raw), and infrastructure errors (timeout, a run that returns after
//! the timeout, panic, worker crash, stuck worker, internal BytecodeGen error) that exit 2 and
//! are never written to the expectation file.

use std::path::{Path, PathBuf};
use std::process::Command;

// The compiled paths of `runner_dirty` (gates5 high 1); this file uses `compiled_dirty` only.
#[allow(dead_code)]
#[path = "../runner_build.rs"]
mod runner_build;

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
        // A class auto-accessor: unsupported (async generators compile since M3-P2 ASYNCGEN).
        ("unsupported.js", test("", "class A { accessor x = 1 }")),
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
        summary.contains("\"cause\":\"unsupported: class auto-accessors\""),
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

/// The JSON names its source (`cargo xtask m1-exit` items 3 and 4, P3-ORCH-14): the commit of
/// the ep9 tree the runner was built from, its dirty flag and the host that ran it. In a git
/// work tree both come from git; in a copy without git (CX5-C6) from `EP9_SOURCE_COMMIT` and
/// `EP9_SOURCE_DIRTY` when the caller gives them, else from the `.sbx-commit` file of
/// `sbx push-git`, else the JSON names no commit. The test does not need git to work.
#[test]
fn the_json_names_its_commit_dirty_flag_and_host() {
    let root = tree("source");
    let expect = expect_file(&root, FAILING);
    let json = root.join("summary.json");
    let (code, text) = exec(&root, &expect, &["--json", json.to_str().unwrap()], None);
    assert_eq!(code, Some(0), "{text}");
    let summary = std::fs::read_to_string(&json).unwrap();
    let repo = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    // `git -C <repo> <args>`, or None when git cannot answer (no git, or no work tree).
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let top = git(&["rev-parse", "--show-toplevel"]).and_then(|t| std::fs::canonicalize(t).ok());
    let env = |k: &str| std::env::var(k).ok();
    // An explicit EP9_SOURCE_DIRTY other than 0 is dirty on every path (CX6-2).
    let declared = env("EP9_SOURCE_DIRTY").is_some_and(|d| d.trim() != "0");
    let want = if top.as_deref() == Some(repo.as_path()) {
        let head = git(&["rev-parse", "HEAD"]).unwrap();
        let dirty = declared
            || !git(&["status", "--porcelain", "--untracked-files=no"])
                .unwrap()
                .is_empty();
        format!(
            ",\"commit\":\"{head}\",\"dirty\":{dirty},\"commit_source\":\"git\",\"made_by\":\"host:"
        )
    } else if let Some(commit) = env("EP9_SOURCE_COMMIT") {
        let dirty = env("EP9_SOURCE_DIRTY").map(|d| d.trim() != "0");
        format!(
            ",\"commit\":\"{}\"{},\"commit_source\":\"env\",\"made_by\":\"host:",
            commit.trim().to_ascii_lowercase(),
            dirty.map_or(String::new(), |d| format!(",\"dirty\":{d}"))
        )
    } else if let Ok(sbx) = std::fs::read_to_string(repo.join(".sbx-commit")) {
        // CX6-2 (c): clean only with the export manifest (this test runs on an unedited copy);
        // without it the JSON does not say.
        let manifest = repo.join(".sbx-manifest").exists();
        let dirty = if declared {
            ",\"dirty\":true"
        } else if manifest {
            ",\"dirty\":false"
        } else {
            ""
        };
        format!(
            ",\"commit\":\"{}\"{dirty},\"commit_source\":\"sbx-commit\",\"source_manifest\":\"{}\",\"made_by\":\"host:",
            sbx.trim().to_ascii_lowercase(),
            if manifest { "checked" } else { "missing" }
        )
    } else {
        ",\"commit_source\":\"env\",\"made_by\":\"host:".to_string()
    };
    assert!(summary.contains(&want), "{want}: {summary}");
    std::fs::remove_dir_all(root).unwrap();
}

/// MC-w17-5 (plan v10 section 2.6): the JSON names the build: `mode` and `preset` are the
/// preset the isolates ran (`jit` with the runner's default `jit` feature, V10-A12), `features`
/// the ep9_vm features, `feedback` the `--feedback` mode (default `full` for `jit`). The summary
/// line says the same.
#[test]
fn the_json_names_its_build() {
    let root = tree("build");
    let expect = expect_file(&root, FAILING);
    let json = root.join("summary.json");
    let preset = if cfg!(feature = "jit") {
        "jit"
    } else {
        "jitless"
    };
    for (extra, feedback) in [
        (
            &[][..],
            if cfg!(feature = "jit") {
                "full"
            } else {
                "ics_only"
            },
        ),
        (&["--feedback", "ics_only"][..], "ics_only"),
        (&["--feedback", "full"][..], "full"),
    ] {
        let mut args = vec!["--json", json.to_str().unwrap()];
        args.extend_from_slice(extra);
        let (code, text) = exec(&root, &expect, &args, None);
        assert_eq!(code, Some(0), "{text}");
        assert!(
            text.contains(&format!(", mode {preset}, feedback {feedback}, timeout ")),
            "{text}"
        );
        let summary = std::fs::read_to_string(&json).unwrap();
        let head = format!(
            "{{\"command\":\"exec\",\"mode\":\"{preset}\",\"preset\":\"{preset}\",\"features\":["
        );
        assert!(summary.starts_with(&head), "{summary}");
        assert!(
            summary.contains(&format!("],\"feedback\":\"{feedback}\",\"target\":")),
            "{summary}"
        );
        if cfg!(feature = "jit") {
            assert!(summary.contains("\"features\":[\"jit\""), "{summary}");
        }
    }
    let (code, text) = exec(&root, &expect, &["--feedback", "some"], None);
    assert_eq!(code, Some(2), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

/// C2 and item 8 (plan v10 section 2.6): the JSON names the commit the runner was built at
/// (in a git work tree: HEAD, as cargo rebuilt it for this test) and the HEAD of the test262
/// checkout it ran; a test262 directory that is no git work tree of its own names none.
#[test]
fn the_json_names_the_runner_commit_and_test262_head() {
    let root = tree("runner");
    let expect = expect_file(&root, FAILING);
    let json = root.join("summary.json");
    let run = || {
        let (code, text) = exec(&root, &expect, &["--json", json.to_str().unwrap()], None);
        assert_eq!(code, Some(0), "{text}");
        std::fs::read_to_string(&json).unwrap()
    };
    let git = |dir: &Path, args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@test.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let summary = run();
    assert!(!summary.contains("\"test262_head\""), "{summary}");
    let repo = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let top =
        git(&repo, &["rev-parse", "--show-toplevel"]).and_then(|t| std::fs::canonicalize(t).ok());
    if top.as_deref() == Some(repo.as_path()) {
        let head = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        // gates5 high 1: and whether its compiled sources had uncommitted changes (cargo
        // rebuilt it for this test, so the flag is the state of the tree now).
        let dirty = runner_build::compiled_dirty(&repo).unwrap();
        assert!(
            summary.contains(&format!(
                ",\"runner_commit\":\"{head}\",\"runner_dirty\":{dirty}"
            )),
            "{head} {dirty}: {summary}"
        );
    }
    // The fake tree as a git checkout of its own.
    if git(&root, &["init", "-q"]).is_some() {
        // The stamp file of scripts/sync-lib.sh is not a change of the checkout.
        std::fs::write(root.join(".ep9-stamp"), "pin patches\n").unwrap();
        git(&root, &["add", "-A", ":!.ep9-stamp"]).unwrap();
        git(&root, &["commit", "-q", "-m", "t262"]).unwrap();
        let t262 = git(&root, &["rev-parse", "HEAD"]).unwrap();
        let summary = run();
        // gates5 high 2: the HEAD before and after the run, and no change in the checkout.
        assert!(
            summary.contains(&format!(
                ",\"test262_head\":\"{t262}\",\"test262_head_end\":\"{t262}\",\"test262_dirty\":false"
            )),
            "{t262}: {summary}"
        );
        // An edited test, or a new one the run picks up: dirty.
        git(&root, &["commit", "-q", "-am", "summary"]).unwrap();
        let t262 = git(&root, &["rev-parse", "HEAD"]).unwrap();
        let pass = std::fs::read_to_string(root.join("test/language/pass.js")).unwrap();
        for (file, text) in [
            ("test/language/pass.js", format!("{pass}// edited\n")),
            ("test/language/new.js", pass.clone()),
        ] {
            std::fs::write(root.join(file), text).unwrap();
            let summary = run();
            assert!(
                summary.contains(&format!(
                    ",\"test262_head\":\"{t262}\",\"test262_head_end\":\"{t262}\",\"test262_dirty\":true"
                )),
                "{file}: {summary}"
            );
            git(&root, &["checkout", "-q", "--", "."]).unwrap();
            git(&root, &["clean", "-q", "-f", "test"]).unwrap();
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// The JSON names the skipped tests, not only their count: `cargo xtask m0-exit` item 6 checks
/// that a GC-stress run skipped exactly the committed skip list (M2-P2-RF2-I8).
#[test]
fn skip_list_paths_reach_the_json() {
    let root = tree("skip");
    let listed: Vec<&str> = FAILING
        .iter()
        .copied()
        .filter(|p| !p.ends_with("/fail.js"))
        .collect();
    let expect = expect_file(&root, &listed);
    let skip = root.join("skip.txt");
    std::fs::write(
        &skip,
        "# s\ntest/language/pass.js\ntest/language/fail.js\ntest/language/none.js\n",
    )
    .unwrap();
    let json = root.join("summary.json");
    let (code, text) = exec(
        &root,
        &expect,
        &[
            "--gc-stress",
            "1",
            "--skip-list",
            skip.to_str().unwrap(),
            "--json",
            json.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(code, Some(0), "{text}");
    assert!(text.contains("skipped: 2"), "{text}");
    let summary = std::fs::read_to_string(&json).unwrap();
    // Sorted; the entry that names no test is not in it.
    assert!(
        summary.contains(
            "\"skipped\":2,\"skipped_paths\":[\"test/language/fail.js\",\"test/language/pass.js\"]"
        ),
        "{summary}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A run with path filters (a GC-stress shard) names every file it ran, sorted, in
/// `run_paths`: `cargo xtask m0-exit` item 6 checks that each shard ran exactly the files of its
/// shard (M3-P3 fix round 2, C-G1). A full run leaves the list out.
#[test]
fn filtered_runs_name_their_files() {
    let root = tree("run-paths");
    let expect = expect_file(&root, FAILING);
    let skip = root.join("skip.txt");
    std::fs::write(&skip, "# s\ntest/language/neg-runtime-t262.js\n").unwrap();
    let json = root.join("summary.json");
    let (code, text) = exec(
        &root,
        &expect,
        &[
            "--gc-stress",
            "1",
            "--skip-list",
            skip.to_str().unwrap(),
            "--json",
            json.to_str().unwrap(),
            "test/language/raw.js",
            "test/language/neg-runtime-",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{text}");
    let summary = std::fs::read_to_string(&json).unwrap();
    assert!(
        summary.contains(
            "\"skipped_paths\":[\"test/language/neg-runtime-t262.js\"],\"run_paths\":[\"test/language/neg-runtime-object.js\",\
             \"test/language/neg-runtime-string.js\",\"test/language/neg-runtime-wrong.js\",\
             \"test/language/raw.js\"]"
        ),
        "{summary}"
    );
    assert!(
        summary.contains("\"files\":{\"passed\":1,\"total\":4}"),
        "{summary}"
    );
    let (code, text) = exec(&root, &expect, &["--json", json.to_str().unwrap()], None);
    assert_eq!(code, Some(0), "{text}");
    let summary = std::fs::read_to_string(&json).unwrap();
    assert!(!summary.contains("run_paths"), "{summary}");
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

/// A panic in a run is an infrastructure error in both builds of the binary. The dev profile
/// (`cargo test`, as CI runs it) unwinds: the worker catches the panic and reports `panic`.
/// The release profile has `panic = "abort"` (H23), and `cargo test --release` builds this
/// binary with it: the worker ends after its panic hook, and the report line is a
/// `worker crash` whose stderr tail carries the panic text.
#[test]
fn panic_is_an_infra_error() {
    let root = tree("panic");
    let expect = expect_file(&root, FAILING);
    let (code, text) = exec(&root, &expect, &[], Some("panic:/pass.js"));
    assert_eq!(code, Some(2), "{text}");
    let caught = text.contains("test/language/pass.js: [sloppy] panic: injected panic")
        && text.contains("(panic 1, timeout 0, worker crash 0, other 0)");
    let aborted = text.contains("test/language/pass.js: the worker exited")
        && text.contains("ep9-test262 worker panicked: injected panic for test/language/pass.js")
        && text.contains("(panic 0, timeout 0, worker crash 1, other 0)");
    assert!(caught || aborted, "{text}");
    // Every other test still ran.
    assert!(text.contains("language             8 / 18"), "{text}");
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
fn a_panic_that_aborts_the_worker_reaches_the_report() {
    // A panic that cannot unwind ends the worker after the panic hook ran, as every panic does
    // under `panic = "abort"`: a worker crash whose line carries the panic text.
    let root = tree("nounwind");
    let expect = expect_file(&root, FAILING);
    let (code, text) = exec(&root, &expect, &[], Some("nounwind-panic:/pass.js"));
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/language/pass.js: the worker exited"),
        "{text}"
    );
    assert!(
        text.contains("panicked: injected panic for test/language/pass.js"),
        "{text}"
    );
    assert!(text.contains("worker crash 1"), "{text}");
    assert!(text.contains("language             8 / 18"), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_fault_no_infrastructure_error() {
    let root = tree("clean");
    let expect = expect_file(&root, FAILING);
    let (code, text) = exec(&root, &expect, &[], None);
    assert_eq!(code, Some(0), "{text}");
    assert!(
        text.contains("infrastructure errors: 0 (panic 0, timeout 0, worker crash 0, other 0)"),
        "{text}"
    );
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
