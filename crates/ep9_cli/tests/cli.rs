//! The command line of the `ep9` binary: sources, flags, exit status and error reports.

use std::process::Command;

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn ep9(args: &[&str]) -> Out {
    ep9_env(args, &[])
}

fn ep9_env(args: &[&str], env: &[(&str, &str)]) -> Out {
    let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
        .args(args)
        .env_remove("EP9_CLI_FAULT")
        .env_remove("RUST_BACKTRACE")
        .envs(env.iter().copied())
        .output()
        .expect("run ep9");
    Out {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[test]
fn eval_prints_and_exits_0() {
    let o = ep9(&[
        "-e",
        "print(1 + 2, 'a' + 'b', [1, 2], null); gc(); print(typeof gc)",
    ]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(o.stdout, "3 ab 1,2 null\nfunction\n");
    assert_eq!(o.stderr, "");
}

#[test]
fn uncaught_error_reports_file_line_utf16_column() {
    // "😀" is 2 UTF-16 units, so `new` is at UTF-16 column 20 of line 2 (byte column 24).
    let o = ep9(&[
        "-e",
        "print('x');\n/* 😀😀 */ var e = new TypeError('bad');\nthrow e;",
    ]);
    assert_eq!(o.code, Some(1));
    assert_eq!(o.stdout, "x\n");
    let first = o.stderr.lines().next().unwrap_or("");
    assert_eq!(
        first, "[eval]:2:20: Uncaught TypeError: bad",
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("    at [eval]:2:20"), "{}", o.stderr);
}

#[test]
fn uncaught_non_error_values() {
    let o = ep9(&["-e", "throw 42"]);
    assert_eq!(o.code, Some(1));
    assert_eq!(o.stderr.trim_end(), "[eval]:1:1: Uncaught 42");
    let o = ep9(&["-e", "throw { toString() { return 'custom'; } }"]);
    assert_eq!(o.code, Some(1));
    assert!(o.stderr.contains("Uncaught custom"), "{}", o.stderr);
}

#[test]
fn compile_errors_exit_1_with_a_position() {
    let o = ep9(&["-e", "var ok = 1;\nvar = ;"]);
    assert_eq!(o.code, Some(1));
    assert!(o.stderr.starts_with("[eval]:2:"), "{}", o.stderr);
    assert!(o.stderr.contains("SyntaxError"), "{}", o.stderr);
    // A class auto-accessor: unsupported (async generators compile since M3-P2 ASYNCGEN).
    let o = ep9(&["-e", "class A { accessor x = 1 }"]);
    assert_eq!(o.code, Some(1));
    assert!(
        o.stderr
            .contains("[eval]:1:11: SyntaxError: ep9 BytecodeGen v0 does not support"),
        "{}",
        o.stderr
    );
}

#[test]
fn a_panic_is_an_internal_error_with_exit_70() {
    // The panic hook prints one ep9 line and exits 70 on the isolate thread, before Rust's
    // default report; under `panic = "abort"` the status stays 70 (not 134).
    let o = ep9_env(&["-e", "print('never')"], &[("EP9_CLI_FAULT", "panic")]);
    assert_eq!(o.code, Some(70), "{}", o.stderr);
    assert_eq!(o.stdout, "");
    let mut lines = o.stderr.lines();
    assert_eq!(
        lines.next(),
        Some("ep9: internal error: injected panic (EP9_CLI_FAULT=panic) (this is an ep9 bug)"),
        "{}",
        o.stderr
    );
    let at = lines.next().unwrap_or("");
    // The path separator is the host's (`crates\ep9_cli\...` on Windows).
    assert!(
        at.starts_with("ep9: panicked at crates")
            && at.contains("run.rs:")
            && at.ends_with(" on thread 'ep9-isolate'"),
        "{}",
        o.stderr
    );
    assert!(
        !o.stderr.contains("thread 'ep9-isolate' panicked"),
        "{}",
        o.stderr
    );
    // Without the hook variable the same script runs.
    let o = ep9(&["-e", "print('ran')"]);
    assert_eq!(
        (o.code, o.stdout.as_str()),
        (Some(0), "ran\n"),
        "{}",
        o.stderr
    );
}

#[test]
fn deep_recursion_is_a_catchable_range_error() {
    let o = ep9(&[
        "-e",
        "function f(n) { return f(n + 1) + 1; }\n\
         try { f(0); } catch (e) { print(e instanceof RangeError, e.message); }\n\
         print('alive');",
    ]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(o.stdout, "true Maximum call stack size exceeded\nalive\n");
}

#[test]
fn strict_and_module_flags() {
    let o = ep9(&[
        "--strict",
        "-e",
        "try { undeclared = 1 } catch (e) { print(e.name) }",
    ]);
    assert_eq!(o.stdout, "ReferenceError\n", "{}", o.stderr);
    let o = ep9(&["-e", "undeclared = 1; print(undeclared)"]);
    assert_eq!(o.stdout, "1\n", "{}", o.stderr);
    let o = ep9(&["--module", "-e", "print(typeof this)"]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(o.stdout, "undefined\n");
}

#[test]
fn gc_stress_and_stats() {
    let o = ep9(&[
        "--gc-stress",
        "--gc-stats",
        "-e",
        "var a = []; for (var i = 0; i < 100; i++) a.push({ i }); print(a[99].i)",
    ]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(o.stdout, "99\n");
    let n: u64 = o
        .stderr
        .lines()
        .find_map(|l| l.strip_prefix("ep9: gc collections="))
        .and_then(|r| r.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("a stats line");
    assert!(n >= 100, "only {n} collections");
}

#[test]
fn dispatch_kinds_agree() {
    for d in ["match", "sibling"] {
        let o = ep9(&[
            "--dispatch",
            d,
            "-e",
            "var s = 0; for (var i = 0; i < 1000; i++) s += i; print(s)",
        ]);
        assert_eq!(o.stdout, "499500\n", "{d}: {}", o.stderr);
    }
}

#[test]
fn print_bytecode() {
    let o = ep9(&["--print-bytecode", "-e", "print(1)"]);
    assert_eq!(o.code, Some(0));
    assert!(o.stdout.contains("; ep9 bytecode"), "{}", o.stdout);
    assert!(o.stdout.ends_with("1\n"));
}

#[test]
fn usage_and_io_errors_exit_2() {
    for args in [
        &[][..],
        &["--bogus", "x.js"][..],
        &["-e"][..],
        &["a.js", "b.js"][..],
        &["--gc-stress=0", "-e", "1"][..],
    ] {
        let o = ep9(args);
        assert_eq!(o.code, Some(2), "{args:?}: {}", o.stderr);
        assert!(o.stderr.contains("usage: ep9"), "{args:?}: {}", o.stderr);
    }
    let o = ep9(&["/nonexistent/ep9/file.js"]);
    assert_eq!(o.code, Some(2));
    assert!(o.stderr.contains("cannot read"), "{}", o.stderr);
}

#[test]
fn file_source_names_the_file() {
    let dir = std::env::temp_dir().join(format!("ep9-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("thrower.js");
    std::fs::write(
        &path,
        "print('one');\nfunction f() {\n  throw new Error('boom');\n}\nf();\n",
    )
    .expect("write");
    let o = ep9(&[path.to_str().expect("utf-8 path")]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(o.code, Some(1));
    assert_eq!(o.stdout, "one\n");
    let want = format!("{}:3:9: Uncaught Error: boom", path.display());
    assert_eq!(o.stderr.lines().next(), Some(want.as_str()), "{}", o.stderr);
    assert!(
        o.stderr
            .contains(&format!("    at f ({}:3:9)", path.display()))
    );
}
