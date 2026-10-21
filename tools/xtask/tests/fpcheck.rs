//! `cargo xtask fp-check` on the fixtures in `tests/fpcheck/` (README there).
//!
//! The fixture tests build objects, so they need llvm-tools and the x86_64-unknown-linux-gnu and
//! x86_64-apple-darwin rustup targets: they are `#[ignore]`, and the fp-check host runs them with
//! `cargo test -p xtask --test fpcheck -- --ignored` [I14]. The other tests need no tool and run
//! on every runner.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The target the fixtures build for; with `@x86-64-v3` every crate may use FMA instructions.
const LINUX_X64: &str = "x86_64-unknown-linux-gnu";
/// x86-64 Mach-O, which keeps jump tables in `__text` (the `jump-table-fma` case).
const MACOS_X64: &str = "x86_64-apple-darwin";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fpcheck")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            fs::copy(e.path(), &dst).unwrap();
        }
    }
}

/// A temp copy of `common/` plus one case; removed on drop.
struct Fixture(PathBuf);

impl Fixture {
    fn new(case: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("ep9-fpcheck-{case}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        copy_dir(&fixtures().join("common"), &dir);
        copy_dir(&fixtures().join(case), &dir);
        // The repo's toolchain pin, so the copy outside the repo builds with the same rustc.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        fs::copy(
            repo.join("rust-toolchain.toml"),
            dir.join("rust-toolchain.toml"),
        )
        .unwrap();
        Fixture(dir)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fp_check(args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("fp-check")
        .args(args)
        .output()
        .unwrap();
    eprintln!(
        "--- fp-check {args:?}: {}\n--- stdout\n{}--- stderr (tail)\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(15)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    out
}

/// Runs fp-check on a fixture copy for `targets` and returns (exit code, stdout).
fn run_case(fx: &Fixture, targets: &[&str], extra: &[&str]) -> (i32, String) {
    let root = fx.0.to_str().unwrap();
    let target_dir = fx.0.join("target");
    let target_dir = target_dir.to_str().unwrap();
    // Its own target dir, also when CARGO_TARGET_DIR is set: the tests run in parallel.
    let mut args = vec![
        "--root",
        root,
        "-p",
        "fx_cli",
        "--bin",
        "fx",
        "--target-dir",
        target_dir,
    ];
    for t in targets {
        args.extend(["--target", t]);
    }
    args.extend(extra);
    let out = fp_check(&args);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn fail_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|l| l.starts_with("fp-check FAIL "))
        .collect()
}

#[test]
#[ignore = "needs llvm-tools and the x86_64-unknown-linux-gnu target (fp-check host)"]
fn float_rem_in_engine_crate_fails_and_names_the_function() {
    let fx = Fixture::new("float-rem");
    let (code, out) = run_case(&fx, &[LINUX_X64], &[]);
    assert_eq!(code, 1);
    assert!(
        out.contains(&format!(
            "fp-check FAIL {LINUX_X64}: fmod in fx_engine::js_remainder: banned libm symbol"
        )),
        "{out}"
    );
    // The same object passes when the allow list names that function.
    let allow = fx.0.join("allow.toml");
    fs::write(
        &allow,
        "[[allow]]\nsymbols = [\"fmod\"]\nfunction = \"fx_engine::js_remainder\"\n\
         targets = [\"x86_64-unknown-linux-gnu\"]\nreopen = \"test\"\n",
    )
    .unwrap();
    let (code, out) = run_case(&fx, &[LINUX_X64], &["--allow", allow.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("fmod in fx_engine::js_remainder: allowed by entry 1"));
}

#[test]
#[ignore = "needs llvm-tools and the x86_64-unknown-linux-gnu target (fp-check host)"]
fn mul_add_outside_ep9_math_fails_under_x86_64_v3() {
    let fx = Fixture::new("mul-add");
    let v3 = format!("{LINUX_X64}@x86-64-v3");
    let (code, out) = run_case(&fx, &[&v3, LINUX_X64], &[]);
    assert_eq!(code, 1);
    let fails = fail_lines(&out);
    // x86-64-v3: an inline FMA instruction in the function.
    assert!(
        fails.iter().any(|l| l
            .starts_with(&format!("fp-check FAIL {v3}: FMA instruction `vfmadd"))
            && l.contains("in fx_engine::fused_step")),
        "{out}"
    );
    // Baseline x86-64 (no FMA unit assumed): a call to the C fma.
    assert!(
        fails.contains(&format!(
            "fp-check FAIL {LINUX_X64}: fma in fx_engine::fused_step: banned libm symbol (H36), not in the allow list"
        )
        .as_str()),
        "{out}"
    );
}

#[test]
#[ignore = "needs llvm-tools and the x86_64-unknown-linux-gnu target (fp-check host)"]
fn libm_import_fails_and_names_the_function() {
    let fx = Fixture::new("libm-import");
    let (code, out) = run_case(&fx, &[LINUX_X64], &[]);
    assert_eq!(code, 1);
    assert!(
        out.contains(&format!(
            "fp-check FAIL {LINUX_X64}: cbrt in fx_engine::fake_libm_cbrt: banned libm symbol"
        )),
        "{out}"
    );
}

#[test]
#[ignore = "needs llvm-tools and the x86_64-unknown-linux-gnu target (fp-check host)"]
fn disallowed_methods_allow_outside_exact_rs_fails() {
    let fx = Fixture::new("allow-site");
    let (code, out) = run_case(&fx, &[LINUX_X64], &[]);
    assert_eq!(code, 1);
    let fails = fail_lines(&out);
    assert_eq!(
        fails,
        [
            "fp-check FAIL source: crates/fx_engine/src/lib.rs:4: names clippy::disallowed_methods in fx_engine::allowed_sqrt (only crates/ep9_math/src/exact.rs may)"
        ],
        "{out}"
    );
}

/// x86-64 Mach-O: an FMA right after a jump table, which `llvm-objdump -d` decodes as code
/// across the table end, is still found (fp-check decodes again from the table end).
#[test]
#[ignore = "needs llvm-tools and the x86_64-apple-darwin target (fp-check host)"]
fn fma_after_macho_jump_table_fails() {
    let fx = Fixture::new("jump-table-fma");
    let (code, out) = run_case(&fx, &[MACOS_X64], &[]);
    assert_eq!(code, 1, "{out}");
    assert_eq!(
        fail_lines(&out),
        [
            format!(
                "fp-check FAIL {MACOS_X64}: FMA instruction `vfmadd213sd %xmm2, %xmm1, %xmm0` in _fx_table_fma: only ep9_math:: functions may use FMA (H36)"
            )
            .as_str()
        ],
        "{out}"
    );
}

/// FMA through ep9_math passes, and fp-check sees it inside `ep9_math::`: on the x86-64 baseline
/// only in the dispatch's FMA copy (`::hw`), with no call to the C `fma`.
#[test]
#[ignore = "needs llvm-tools and the x86_64-unknown-linux-gnu target (fp-check host)"]
fn fma_inside_ep9_math_passes() {
    let fx = Fixture::new("ep9-math-user");
    let manifest = fx.0.join("crates/fx_engine/Cargo.toml");
    let ep9_math = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/ep9_math")
        .canonicalize()
        .unwrap();
    let text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("@EP9_MATH@", &ep9_math.to_string_lossy().replace('\\', "/"));
    fs::write(&manifest, text).unwrap();
    let v3 = format!("{LINUX_X64}@x86-64-v3");
    let (code, out) = run_case(&fx, &[LINUX_X64, &v3], &["-v"]);
    assert_eq!(code, 0, "{out}");
    for spec in [LINUX_X64, v3.as_str()] {
        assert!(
            out.contains(&format!(
                "fp-check {spec}: 0 banned symbol references, FMA instructions: 0 outside ep9_math, 2 inside"
            )),
            "{out}"
        );
    }
    assert!(
        out.lines().any(
            |l| l.starts_with(&format!("fp-check {LINUX_X64}: FMA `vfmadd"))
                && l.ends_with("in ep9_math::exact::fma_exact::hw (ep9_math: allowed)")
        ),
        "{out}"
    );
}

/// The control: a fixture with plain IEEE arithmetic passes, on both x86-64 builds.
#[test]
#[ignore = "needs llvm-tools and the x86_64-unknown-linux-gnu target (fp-check host)"]
fn clean_fixture_passes() {
    let fx = Fixture::new("clean");
    let v3 = format!("{LINUX_X64}@x86-64-v3");
    let (code, out) = run_case(&fx, &[LINUX_X64, &v3], &[]);
    assert_eq!(code, 0, "{out}");
    assert!(fail_lines(&out).is_empty());
}

/// A missing tool is a tool error (exit 2), before any build. No tool needed.
#[test]
fn missing_tool_exits_2() {
    let missing = std::env::temp_dir().join("ep9-fpcheck-no-such-tool/llvm-objdump");
    let out = fp_check(&["--objdump", missing.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("llvm-objdump does not run"), "{err}");
}

/// A usage error exits 2.
#[test]
fn usage_error_exits_2() {
    assert_eq!(fp_check(&["--no-such-option"]).status.code(), Some(2));
    assert_eq!(fp_check(&["--target", "x@"]).status.code(), Some(2));
}
