//! `$262.detachArrayBuffer`, `$262.agent` and the `[[CanBlock]]` flags of `ep9-test262 exec`
//! on a small fake test262 tree (TypedArray TA-2 TA-HARNESS; design
//! `docs/plan/notes/typedarray-design.md` §5.6, §8.2, §9.2):
//!
//! - an agent is an isolate on its own thread: it reports, runs its jobs, leaves; the main
//!   agent reads the reports in order of arrival;
//! - the end of a run stops every agent (a loop, a wait for a broadcast); an agent that does
//!   not stop, and a main agent that waits past the deadline, are infrastructure errors, never
//!   a pass;
//! - an agent that throws fails the test, with the agent's exception as the cause;
//! - a hook that needs an engine part this build lacks throws a TypeError (feature probe:
//!   with a `SharedArrayBuffer` or `ArrayBuffer` constructor the same file checks the real
//!   path);
//! - `CanBlockIsTrue` fails (the main agent cannot block), `CanBlockIsFalse` runs.
//!
//! Every test runs the runner binary with a soft timeout, and the runner kills a worker at its
//! hard timeout, so no test here can hang.

use std::path::{Path, PathBuf};
use std::process::Command;

const ASSERT: &str = r#"
function assert(c, m) { if (c !== true) throw new Test262Error(m || "assert"); }
assert.sameValue = function (a, b, m) {
  if (a !== b) throw new Test262Error((m || "") + " expected " + b + " got " + a);
};
assert.throws = function (C, f, m) {
  try { f(); } catch (e) {
    if (e.constructor !== C) throw new Test262Error((m || "") + " wrong error: " + e);
    return;
  }
  throw new Test262Error((m || "") + " no error");
};
"#;
const STA: &str = r#"
function Test262Error(m) { this.message = m || ""; }
Test262Error.prototype.toString = function () { return "Test262Error: " + this.message; };
"#;
const DONE: &str = r#"
function $DONE(error) {
  if (error) { print("Test262:AsyncTestFailure:" + error); } else { print("Test262:AsyncTestComplete"); }
}
"#;

/// A wait for the next report, as the test262 `atomicsHelper.js` override does.
const NEXT_REPORT: &str = r#"
function nextReport() {
  var r;
  while ((r = $262.agent.getReport()) == null) { $262.agent.sleep(1); }
  return r;
}
"#;

fn test(meta: &str, body: &str) -> String {
    format!("/*---\ndescription: x\n{meta}---*/\n{NEXT_REPORT}{body}\n")
}

/// Tests that pass.
fn passing() -> Vec<(&'static str, String)> {
    vec![
        (
            "report.js",
            test(
                "",
                "var t0 = $262.agent.monotonicNow();\n\
                 $262.agent.start('$262.agent.report(\"hi \" + typeof $262.agent.receiveBroadcast); \
                 $262.agent.leaving();');\n\
                 assert.sameValue(nextReport(), 'hi function');\n\
                 assert.sameValue($262.agent.getReport(), null);\n\
                 var t1 = $262.agent.monotonicNow();\n\
                 assert(typeof t0 === 'number' && t1 >= t0, 'monotonicNow');",
            ),
        ),
        (
            "three.js",
            test(
                "",
                "for (var i = 0; i < 3; i++) {\n\
                   $262.agent.start('$262.agent.report(' + i + '); $262.agent.leaving();');\n\
                 }\n\
                 var got = [nextReport(), nextReport(), nextReport()].sort();\n\
                 assert.sameValue(got.join(), '0,1,2');",
            ),
        ),
        // The agent's jobs run after its script (an async receiveBroadcast callback needs it).
        (
            "job.js",
            test(
                "",
                "$262.agent.start('Promise.resolve(1).then(function (v) { \
                 $262.agent.report(\"job \" + v); $262.agent.leaving(); });');\n\
                 assert.sameValue(nextReport(), 'job 1');",
            ),
        ),
        // The agent never leaves; the end of the run stops it.
        (
            "agent-loop.js",
            test(
                "",
                "$262.agent.start('$262.agent.report(\"looping\"); while (true) {}');\n\
                 assert.sameValue(nextReport(), 'looping');",
            ),
        ),
        // The agent waits for a broadcast that never comes; the end of the run stops it.
        (
            "agent-waits.js",
            test(
                "",
                "$262.agent.start('$262.agent.report(\"waiting\"); \
                 $262.agent.receiveBroadcast(function () {});');\n\
                 assert.sameValue(nextReport(), 'waiting');",
            ),
        ),
        // Each side has its own hooks: the other side's hooks throw a TypeError.
        (
            "sides.js",
            test(
                "",
                "assert.sameValue(Object.keys($262.agent).sort().join(), \
                 'broadcast,getReport,monotonicNow,sleep,start');\n\
                 assert.throws(TypeError, function () { __ep9_agent_report('x'); });\n\
                 assert.throws(TypeError, function () { __ep9_agent_leaving(); });\n\
                 assert.throws(TypeError, function () { __ep9_agent_receive_broadcast(function () {}); });\n\
                 $262.agent.start([\n\
                   'var bad = [];',\n\
                   'if (Object.keys($262.agent).sort().join() !== \"leaving,monotonicNow,receiveBroadcast,report,sleep\") bad.push(\"keys\");',\n\
                   'try { __ep9_agent_start(\"0\"); bad.push(\"start\"); } catch (e) { if (!(e instanceof TypeError)) bad.push(\"start: \" + e); }',\n\
                   'try { __ep9_agent_get_report(); bad.push(\"getReport\"); } catch (e) { if (!(e instanceof TypeError)) bad.push(\"getReport: \" + e); }',\n\
                   'try { $262.agent.receiveBroadcast(1); bad.push(\"callable\"); } catch (e) { if (!(e instanceof TypeError)) bad.push(\"callable: \" + e); }',\n\
                   'if (typeof $262.detachArrayBuffer !== \"function\") bad.push(\"detach\");',\n\
                   '$262.agent.report(bad.length ? bad.join() : \"ok\");',\n\
                   '$262.agent.leaving();'\n\
                 ].join('\\n'));\n\
                 assert.sameValue(nextReport(), 'ok');",
            ),
        ),
        // A hook that needs an engine part this build lacks throws a TypeError; with the
        // constructor the same file checks the real path (a feature probe).
        (
            "clean.js",
            test(
                "",
                "assert.throws(TypeError, function () { $262.detachArrayBuffer({}); });\n\
                 assert.throws(TypeError, function () { $262.detachArrayBuffer(1); });\n\
                 assert.throws(TypeError, function () { $262.agent.broadcast({}); });\n\
                 if (typeof ArrayBuffer === 'function') {\n\
                   var b = new ArrayBuffer(8);\n\
                   $262.detachArrayBuffer(b);\n\
                   assert.sameValue(b.byteLength, 0, 'detached');\n\
                 }\n\
                 if (typeof SharedArrayBuffer === 'function') {\n\
                   assert.throws(TypeError, function () { $262.detachArrayBuffer(new SharedArrayBuffer(8)); });\n\
                   try { $262.agent.broadcast(new SharedArrayBuffer(8)); }\n\
                   catch (e) { assert(e instanceof TypeError && /cannot share/.test(e.message), String(e)); }\n\
                 }",
            ),
        ),
        // A new realm gets the hooks of its agent.
        (
            "realm.js",
            test(
                "",
                "var other = $262.createRealm();\n\
                 assert.sameValue(typeof other.detachArrayBuffer, 'function');\n\
                 assert.sameValue(typeof other.agent.start, 'function');\n\
                 other.agent.start('$262.agent.report(\"from a realm\"); $262.agent.leaving();');\n\
                 assert.sameValue(nextReport(), 'from a realm');",
            ),
        ),
        ("can-block-false.js", test("flags: [CanBlockIsFalse]\n", "")),
        (
            "async.js",
            test(
                "flags: [async]\n",
                "$262.agent.start('$262.agent.report(\"async\"); $262.agent.leaving();');\n\
                 Promise.resolve().then(function () { assert.sameValue(nextReport(), 'async'); })\n\
                 .then($DONE, $DONE);",
            ),
        ),
    ]
}

/// Tests that fail, each with its cause.
fn failing() -> Vec<(&'static str, String, &'static str)> {
    vec![
        // The main agent waits for a report; the agent threw: getReport throws its error.
        (
            "agent-throws.js",
            test(
                "",
                "$262.agent.start('throw new RangeError(\"boom\")');\nnextReport();",
            ),
            "Error: $262.agent.getReport: an agent threw: agent 0: RangeError: boom",
        ),
        // The main agent does not look: an agent error still fails a passing run.
        (
            "agent-throws-late.js",
            test(
                "",
                "$262.agent.start('$262.agent.report(\"x\"); throw new TypeError(\"late\")');\n\
                 assert.sameValue(nextReport(), 'x');\n\
                 try { nextReport(); } catch (e) {}",
            ),
            "agent threw: agent 0: TypeError: late",
        ),
        (
            "agent-syntax.js",
            test("", "$262.agent.start('var = ;');\nnextReport();"),
            "Error: $262.agent.getReport: an agent threw: agent 0: the script does not compile: ",
        ),
        (
            "can-block-true.js",
            test("flags: [CanBlockIsTrue]\n", ""),
            "CanBlockIsTrue: the main agent cannot block",
        ),
    ]
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A fresh tree with the harness files and `tests` under `test/agent/`.
fn tree(name: &str, tests: &[(&str, String)]) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("ep9-test262-agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(&root, "harness/assert.js", ASSERT);
    write(&root, "harness/sta.js", STA);
    write(&root, "harness/doneprintHandle.js", DONE);
    for (rel, src) in tests {
        write(&root, &format!("test/agent/{rel}"), src);
    }
    root
}

/// The (passed, total) files of the `agent` row of the report.
fn counts(text: &str) -> Option<(usize, usize)> {
    let row = text
        .lines()
        .find(|l| l.trim_start().starts_with("agent "))?;
    let mut words = row.split_whitespace().skip(1);
    let passed = words.next()?.parse().ok()?;
    let _slash = words.next()?;
    let total = words.next()?.parse().ok()?;
    Some((passed, total))
}

fn expect_file(root: &Path, paths: &[String]) -> PathBuf {
    let path = root.join("expect.txt");
    let mut text = paths.join("\n");
    text.push('\n');
    std::fs::write(&path, text).unwrap();
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

/// Every test of [`passing`] passes and every test of [`failing`] fails with its cause, plain
/// and under GC stress (each agent isolate gets the main agent's config); `only` keeps the
/// tests of these names (GC stress makes each agent isolate slow to make).
fn pass_and_fail(gc_stress: &str, only: Option<&[&str]>) {
    let keep = |n: &str| only.is_none_or(|o| o.contains(&n));
    let mut tests: Vec<_> = passing().into_iter().filter(|(n, _)| keep(n)).collect();
    let pass = tests.len();
    let fails: Vec<_> = failing().into_iter().filter(|(n, ..)| keep(n)).collect();
    tests.extend(fails.iter().map(|(n, s, _)| (*n, s.clone())));
    let root = tree(&format!("rules-{gc_stress}"), &tests);
    let listed: Vec<String> = fails
        .iter()
        .map(|(n, ..)| format!("test/agent/{n}"))
        .collect();
    let expect = expect_file(&root, &listed);
    let json = root.join("summary.json");
    let (code, text) = exec(
        &root,
        &expect,
        &[
            "--gc-stress",
            gc_stress,
            "--timeout",
            "60",
            "--json",
            json.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(counts(&text), Some((pass, pass + fails.len())), "{text}");
    let summary = std::fs::read_to_string(&json).unwrap();
    for (name, _, cause) in &fails {
        let entry =
            format!("{{\"path\":\"test/agent/{name}\",\"status\":\"fail\",\"cause\":\"{cause}");
        assert!(summary.contains(&entry), "{entry}: {summary}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn agents_report_leave_stop_and_fail() {
    pass_and_fail("0", None);
}

/// The agent paths under GC stress 1: reports, jobs, a stop at the end of the run, an agent
/// error, a hook that fails cleanly.
#[test]
fn agents_under_gc_stress() {
    let only = [
        "three.js",
        "job.js",
        "agent-waits.js",
        "clean.js",
        "agent-throws-late.js",
    ];
    pass_and_fail("1", Some(&only));
}

/// A main agent that waits for a report that never comes passes its deadline: an
/// infrastructure error, never a pass or a failure.
#[test]
fn a_main_agent_that_waits_past_the_deadline_is_a_timeout() {
    let tests = [(
        "silent.js",
        test(
            "flags: [onlyStrict]\n",
            "$262.agent.start('$262.agent.leaving();');\nnextReport();",
        ),
    )];
    let root = tree("silent", &tests);
    let expect = expect_file(&root, &[]);
    let (code, text) = exec(&root, &expect, &["--timeout", "1"], None);
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/agent/silent.js: [strict] timeout"),
        "{text}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// An agent that does not stop when the run ends (the `agent-stuck` hook: deaf to its
/// termination) is an infrastructure error, although the main agent passed.
#[test]
fn an_agent_that_does_not_stop_is_an_infrastructure_error() {
    let tests = [
        (
            "stuck.js",
            test("flags: [onlyStrict]\n", "$262.agent.start('');"),
        ),
        (
            "fine.js",
            test("flags: [onlyStrict]\n", "$262.agent.start('');"),
        ),
    ];
    let root = tree("stuck", &tests);
    let expect = expect_file(&root, &[]);
    let (code, text) = exec(&root, &expect, &[], Some("agent-stuck:/stuck.js"));
    assert_eq!(code, Some(2), "{text}");
    assert!(
        text.contains("test/agent/stuck.js: [strict] timeout: 1 agent(s) still ran"),
        "{text}"
    );
    assert_eq!(counts(&text), Some((1, 2)), "{text}");
    std::fs::remove_dir_all(root).unwrap();
}
