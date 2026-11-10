//! `cargo xtask extref-lint` on the evasion fixtures in `tests/extref_lint/` (README there;
//! V11IN-22: renames, raw names and macros) and on the tree.
//!
//! The fixture dir is a fake repo root. Its files are only parsed, never built, so these
//! tests need no tool and run on every runner. A fixture marks each line that must be a
//! finding with a trailing `// FLAG`; every other line must give none.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("extref_lint")
}

fn extref_lint(root: &Path, paths: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("extref-lint")
        .arg("--root")
        .arg(root)
        .args(paths)
        .output()
        .unwrap();
    eprintln!(
        "--- extref-lint {paths:?}: {}\n--- stdout\n{}--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Every `.rs` file under `crates/*/src` of the fixture root, as a `/` path from the root.
fn cases() -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        let mut es: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        es.sort_by_key(|e| e.file_name());
        for e in es {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = format!("{rel}/{name}");
            if e.path().is_dir() {
                walk(&e.path(), &r, out);
            } else if name.ends_with(".rs") {
                out.push(r);
            }
        }
    }
    let mut out = Vec::new();
    for c in ["ep9_value", "ep9_vm"] {
        walk(
            &fixtures().join("crates").join(c).join("src"),
            &format!("crates/{c}/src"),
            &mut out,
        );
    }
    out
}

/// The lines of `case` that end in `// FLAG`, in order.
fn flagged(case: &str) -> Vec<usize> {
    let mut p = fixtures();
    p.extend(case.split('/'));
    fs::read_to_string(&p)
        .unwrap()
        .lines()
        .enumerate()
        .filter(|(_, l)| l.trim_end().ends_with("// FLAG"))
        .map(|(i, _)| i + 1)
        .collect()
}

/// The distinct lines of `case` that the output reports as findings (`case:line: ...`).
fn reported(out: &str, case: &str) -> Vec<usize> {
    let prefix = format!("{case}:");
    let mut v: Vec<usize> = out
        .lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .map(|rest| rest.split(':').next().unwrap().parse().unwrap())
        .collect();
    v.dedup();
    v
}

/// One run over the whole fixture root: exit 1, and the findings of each file are exactly
/// its `// FLAG` lines (the clean file and the scanned `#[path]` file give none).
#[test]
fn each_evasion_form_is_a_finding_and_nothing_else_is() {
    let out = extref_lint(&fixtures(), &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let cases = cases();
    assert_eq!(cases.len(), 10, "{cases:?}");
    for case in &cases {
        assert_eq!(reported(&text, case), flagged(case), "{case}");
    }
    // Every finding line is in a fixture file (none from the hidden module file).
    for l in text.lines().filter(|l| !l.starts_with("extref-lint:")) {
        assert!(cases.iter().any(|c| l.starts_with(&format!("{c}:"))), "{l}");
    }
}

/// The rule of each form (the README table).
#[test]
fn each_form_has_its_rule() {
    let out = extref_lint(&fixtures(), &[]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let rule = |case: &str, line: usize| -> Vec<String> {
        let p = format!("crates/{case}:{line}: ");
        text.lines()
            .filter_map(|l| l.strip_prefix(&p))
            .map(|r| r.split(':').next().unwrap().to_string())
            .collect()
    };
    assert_eq!(rule("ep9_vm/src/layout.rs", 9), ["alias"]);
    assert_eq!(rule("ep9_value/src/encoding.rs", 7), ["alias"]);
    assert_eq!(rule("ep9_vm/src/frame.rs", 16), ["alias"]);
    assert_eq!(rule("ep9_vm/src/external_ref.rs", 13), ["alias"]);
    assert_eq!(rule("ep9_vm/src/kelvin.rs", 3), ["field"]);
    assert_eq!(rule("ep9_vm/src/macro_args.rs", 8), ["field"]);
    assert_eq!(rule("ep9_vm/src/macro_args.rs", 16), ["encoding"]);
    assert_eq!(rule("ep9_vm/src/hiding.rs", 3), ["macro"]);
    assert_eq!(rule("ep9_vm/src/path_mod.rs", 2), ["macro"]);
}

/// The `#[path]` rule reads the scanned set: with only `path_mod.rs` on the command line,
/// `inner/ok.rs` is not scanned, so its `#[path]` line is a finding too.
#[test]
fn a_path_module_is_checked_against_the_files_of_the_run() {
    let out = extref_lint(&fixtures(), &["crates/ep9_vm/src/path_mod.rs"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(reported(&text, "crates/ep9_vm/src/path_mod.rs"), [2, 4, 7]);
}
