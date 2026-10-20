//! End-to-end tests: every `tests/e2e/**/*.js` program runs through the built `ep9` binary,
//! once as is and once with `--gc-stress` (a collection before every allocation, the heap
//! verifier after each one). Both runs must print the expected output.
//!
//! ```text
//! tests/e2e/<group>/<name>.js    the program; leading `// key: value` lines are directives
//! tests/e2e/<group>/<name>.out   expected stdout, written from node (EP9_E2E_BLESS=1)
//!
//! directives   exit: N        expected exit status (default 0)
//!              stderr: TEXT   stderr contains TEXT (repeatable)
//!              error-at: L:C  the uncaught error is reported at line L, UTF-16 column C
//! ```
//!
//! `expected_outputs_match_node` checks every `.out` file, exit status and `error-at` against
//! node (`tests/node_driver.cjs`), so the expectations are right by construction. It skips
//! with a message when node is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// M0 gate: at least this many programs.
const MIN_PROGRAMS: usize = 150;

fn e2e_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e")
}

fn programs() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("read tests/e2e")
            .map(|e| e.expect("dir entry").path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "js") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&e2e_dir(), &mut out);
    out
}

#[derive(Debug, Default)]
struct Directives {
    exit: i32,
    stderr: Vec<String>,
    error_at: Option<String>,
}

fn directives(src: &str) -> Directives {
    let mut d = Directives::default();
    for line in src.lines() {
        let Some(rest) = line.strip_prefix("// ") else {
            break;
        };
        let Some((key, value)) = rest.split_once(':') else {
            continue;
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        match key {
            "exit" => d.exit = value.trim().parse().expect("exit: <number>"),
            "stderr" => d.stderr.push(value.to_string()),
            "error-at" => d.error_at = Some(value.trim().to_string()),
            _ => {}
        }
    }
    d
}

fn rel(p: &Path) -> String {
    p.strip_prefix(e2e_dir()).unwrap_or(p).display().to_string()
}

/// Runs `f` over `items` on a few threads; returns the failure texts.
fn par_check<T: Sync>(items: &[T], f: impl Fn(&T) -> Vec<String> + Sync) -> Vec<String> {
    let next = AtomicUsize::new(0);
    let failures = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .min(8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { return };
                    let errs = f(item);
                    if !errs.is_empty() {
                        failures.lock().expect("lock").extend(errs);
                    }
                }
            });
        }
    });
    let mut f = failures.into_inner().expect("lock");
    f.sort();
    f
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_ep9(args: &[&str], file: &Path) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
        .args(args)
        .arg(file)
        .output()
        .expect("run ep9");
    Output {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn check_run(file: &Path, mode: &str, args: &[&str]) -> (Vec<String>, Output) {
    let src = std::fs::read_to_string(file).expect("read program");
    let d = directives(&src);
    let expected = std::fs::read_to_string(file.with_extension("out")).unwrap_or_else(|_| {
        panic!(
            "{}: no .out file (run with EP9_E2E_BLESS=1 and node installed)",
            rel(file)
        )
    });
    let out = run_ep9(args, file);
    let name = format!("{} [{mode}]", rel(file));
    let mut errs = Vec::new();
    if out.code != Some(d.exit) {
        errs.push(format!(
            "{name}: exit {:?}, expected {}; stderr: {}",
            out.code, d.exit, out.stderr
        ));
    }
    if out.stdout != expected {
        errs.push(format!(
            "{name}: stdout differs\n--- expected\n{expected}--- got\n{}--- stderr\n{}",
            out.stdout, out.stderr
        ));
    }
    for want in &d.stderr {
        if !out.stderr.contains(want.as_str()) {
            errs.push(format!(
                "{name}: stderr does not contain {want:?}: {}",
                out.stderr
            ));
        }
    }
    if let Some(at) = &d.error_at {
        let first = out.stderr.lines().next().unwrap_or("");
        let want = format!("{}:{at}: ", file.display());
        if !first.starts_with(&want) {
            errs.push(format!(
                "{name}: first stderr line {first:?} does not start with {want:?}"
            ));
        }
    }
    for bad in ["panicked", "internal error", "heap verifier failed"] {
        if out.stderr.contains(bad) {
            errs.push(format!("{name}: stderr has {bad:?}: {}", out.stderr));
        }
    }
    (errs, out)
}

#[test]
fn programs_print_the_expected_output() {
    let files = programs();
    assert!(
        files.len() >= MIN_PROGRAMS,
        "{} programs, want at least {MIN_PROGRAMS}",
        files.len()
    );
    let failures = par_check(&files, |f| check_run(f, "plain", &[]).0);
    assert!(
        failures.is_empty(),
        "{} of {} programs failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
    println!("{} programs passed", files.len());
}

/// The collection count that `--gc-stats` prints, if any.
fn collections(stderr: &str) -> Option<u64> {
    let line = stderr
        .lines()
        .find(|l| l.starts_with("ep9: gc collections="))?;
    line["ep9: gc collections=".len()..]
        .split(' ')
        .next()?
        .parse()
        .ok()
}

#[test]
fn programs_pass_under_gc_stress() {
    let files = programs();
    let total = AtomicUsize::new(0);
    let failures = par_check(&files, |f| {
        let (mut errs, out) = check_run(f, "gc-stress", &["--gc-stress", "--gc-stats"]);
        // A program with an early error never gets an isolate, so it has no GC stats.
        let early_error = std::fs::read_to_string(f)
            .is_ok_and(|src| directives(&src).stderr.iter().any(|s| s == "SyntaxError"));
        match collections(&out.stderr) {
            Some(n) if n > 0 => {
                total.fetch_add(n as usize, Ordering::Relaxed);
            }
            None if early_error => {}
            other => errs.push(format!("{}: --gc-stress ran {other:?} collections", rel(f))),
        }
        errs
    });
    assert!(
        failures.is_empty(),
        "{} of {} programs failed under --gc-stress:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
    println!(
        "{} programs passed under --gc-stress, {} collections in all",
        files.len(),
        total.load(Ordering::Relaxed)
    );
}

/// node's location of an uncaught Error thrown by `file` (from its stack text).
fn node_error_at(stderr: &str, file: &Path) -> Option<String> {
    let needle = format!("{}:", file.display());
    for line in stderr.lines() {
        if !line.trim_start().starts_with("at ") {
            continue;
        }
        if let Some(i) = line.find(&needle) {
            let rest = &line[i + needle.len()..];
            let mut parts = rest.split(|c: char| !c.is_ascii_digit());
            if let (Some(l), Some(c)) = (parts.next(), parts.next())
                && !l.is_empty()
                && !c.is_empty()
            {
                return Some(format!("{l}:{c}"));
            }
        }
    }
    None
}

#[test]
fn expected_outputs_match_node() {
    let node = std::env::var("EP9_NODE").unwrap_or_else(|_| "node".into());
    let Ok(version) = Command::new(&node).arg("--version").output() else {
        eprintln!("skip: node is not installed (set EP9_NODE to its path)");
        return;
    };
    let bless = std::env::var("EP9_E2E_BLESS").is_ok_and(|v| v == "1");
    let driver = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/node_driver.cjs");
    let files = programs();
    let failures = par_check(&files, |file| {
        let src = std::fs::read_to_string(file).expect("read program");
        let d = directives(&src);
        let out = Command::new(&node)
            .arg(&driver)
            .arg(file)
            .output()
            .expect("run node");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let name = rel(file);
        let mut errs = Vec::new();
        if out.status.code() != Some(d.exit) {
            errs.push(format!(
                "{name}: node exit {:?}, directive {}; stderr: {stderr}",
                out.status.code(),
                d.exit
            ));
        }
        if d.exit == 0 && stdout.is_empty() {
            errs.push(format!("{name}: node prints nothing"));
        }
        if let Some(at) = &d.error_at {
            let got = node_error_at(&stderr, file);
            if got.as_deref() != Some(at.as_str()) {
                errs.push(format!(
                    "{name}: node reports the error at {got:?}, directive {at}"
                ));
            }
        }
        let out_file = file.with_extension("out");
        if bless {
            std::fs::write(&out_file, &stdout).expect("write .out");
        } else {
            let expected = std::fs::read_to_string(&out_file).unwrap_or_default();
            if expected != stdout {
                errs.push(format!(
                    "{name}: .out differs from node\n--- .out\n{expected}--- node\n{stdout}"
                ));
            }
        }
        errs
    });
    assert!(
        failures.is_empty(),
        "node ({}) disagrees with {} expectations:\n{}",
        String::from_utf8_lossy(&version.stdout).trim(),
        failures.len(),
        failures.join("\n")
    );
    println!(
        "node {} agrees with all {} expectations{}",
        String::from_utf8_lossy(&version.stdout).trim(),
        files.len(),
        if bless { " (blessed)" } else { "" }
    );
}

#[test]
fn directive_parsing() {
    let d = directives("// exit: 1\n// stderr: a: b\n// error-at: 3:4\nprint(1);\n// exit: 9\n");
    assert_eq!(d.exit, 1);
    assert_eq!(d.stderr, ["a: b"]);
    assert_eq!(d.error_at.as_deref(), Some("3:4"));
    assert_eq!(
        node_error_at(
            "Uncaught Error: x\n/t/a.js:7\n\nError: x\n    at f (/t/a.js:7:9)\n",
            Path::new("/t/a.js")
        )
        .as_deref(),
        Some("7:9")
    );
}
