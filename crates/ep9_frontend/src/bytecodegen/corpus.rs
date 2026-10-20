//! Corpus runs over test262 and the benchmark pins (needs the third_party checkouts):
//! verifier-clean counts per result class, and determinism (each file compiled twice).
//!
//! ```text
//! cargo test -p ep9_frontend --release --lib bytecodegen::corpus -- --ignored --nocapture
//! ```
//!
//! A test262 file runs in the modes the harness uses: module, strict only, sloppy only (or
//! `raw`), or both strict and sloppy. Every compile goes through [`super::check_determinism`]:
//! two builds on two threads, bytes and content hash compared. Any `Internal` error (builder or
//! verifier rejection) or any determinism mismatch fails the test.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{CodegenError, CompileOptions, Determinism, check_determinism, compile};
use crate::{CompileError, SourceOpts, Strictness};

/// (path, `.mjs`, from test262).
pub(crate) fn corpus_files() -> Vec<(PathBuf, bool, bool)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party");
    let mut out = Vec::new();
    for (dir, test262) in [
        ("test262/test", true),
        ("test262/harness", false),
        ("bench-octane", false),
        ("bench-v8v7", false),
        ("bench-jetstream", false),
    ] {
        let before = out.len();
        let mut stack = vec![root.join(dir)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    if path
                        .file_name()
                        .is_some_and(|n| n == ".git" || n == "node_modules")
                    {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "js" || e == "mjs") {
                    let module = path.extension().is_some_and(|e| e == "mjs");
                    out.push((path, module, test262));
                }
            }
        }
        crate::probe::assert_checkout(dir, out.len() - before);
    }
    out
}

/// The modes a test262 harness runs a file in.
pub(crate) fn test262_modes(src: &str, ext_module: bool) -> Vec<SourceOpts> {
    let front = src
        .split("/*---")
        .nth(1)
        .and_then(|rest| rest.split("---*/").next())
        .unwrap_or("");
    let flags = front
        .lines()
        .find(|l| l.trim_start().starts_with("flags:"))
        .unwrap_or("");
    if ext_module || flags.contains("module") {
        vec![SourceOpts::module()]
    } else if flags.contains("onlyStrict") {
        vec![SourceOpts::script(Strictness::Strict)]
    } else if flags.contains("noStrict") || flags.contains("raw") {
        vec![SourceOpts::script(Strictness::Sloppy)]
    } else {
        vec![
            SourceOpts::script(Strictness::Sloppy),
            SourceOpts::script(Strictness::Strict),
        ]
    }
}

/// A result class for the report.
fn class_of(r: &Result<Determinism, CodegenError>) -> String {
    match r {
        Ok(Determinism::Same(_)) => "ok".into(),
        Ok(Determinism::Different(..)) => "DETERMINISM MISMATCH".into(),
        Err(CodegenError::Compile(CompileError::Syntax(_))) => "early error (front end)".into(),
        Err(CodegenError::Compile(CompileError::Range(_))) => "range error (front end)".into(),
        Err(CodegenError::Unsupported(u)) => format!("unsupported: {}", u.feature),
        Err(CodegenError::Limit(m)) => format!("limit: {m}"),
        Err(CodegenError::Internal(m)) => {
            // Group by the message without numbers.
            let key: String = m.chars().filter(|c| !c.is_ascii_digit()).take(80).collect();
            format!("INTERNAL: {key}")
        }
    }
}

#[derive(Default)]
struct Report {
    files: usize,
    units: usize,
    classes: BTreeMap<String, usize>,
    examples: BTreeMap<String, String>,
    /// (path under third_party, mode, content hash) of every unit that compiled.
    hashes: Vec<(String, String, [u8; 32])>,
}

impl Report {
    /// SHA-256 over the sorted (path, mode, unit hash) lines: one value to compare across
    /// hosts and targets.
    fn digest(&mut self) -> String {
        self.hashes.sort();
        let mut h = ep9_bytecode::sha256::Sha256::default();
        for (path, mode, hash) in &self.hashes {
            h.update(path.as_bytes());
            h.update(b"\0");
            h.update(mode.as_bytes());
            h.update(b"\0");
            h.update(hash);
        }
        ep9_bytecode::sha256::to_hex(&h.finish())
    }
}

/// The path under `third_party/`, with `/` separators.
fn short_path(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    p.split("third_party/").nth(1).unwrap_or(&p).to_owned()
}

fn run(files: &[(PathBuf, bool, bool)]) -> Report {
    let next = AtomicUsize::new(0);
    let report = Mutex::new(Report::default());
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(12);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, ext_module, test262)) = files.get(i) else {
                        break;
                    };
                    let Ok(src) = std::fs::read_to_string(path) else {
                        continue;
                    };
                    let modes = if *test262 {
                        test262_modes(&src, *ext_module)
                    } else if *ext_module {
                        vec![SourceOpts::module()]
                    } else {
                        vec![SourceOpts::script(Strictness::Sloppy)]
                    };
                    // A negative parse/early test must fail in the front end.
                    let negative_early = *test262
                        && src.contains("negative:")
                        && (src.contains("phase: parse") || src.contains("phase: early"));
                    let mut local = Vec::new();
                    let mut hashes = Vec::new();
                    for opts in modes {
                        // G10 on a separate build: throwing instructions have positions.
                        if let Ok(Ok(unit)) = std::panic::catch_unwind(|| {
                            compile(&src, opts, CompileOptions::default())
                        }) {
                            let missing = super::tests::unpositioned_throws(&unit);
                            if !missing.is_empty() {
                                local.push((
                                    "NO POSITION on a throwing instruction".to_owned(),
                                    Some(missing.join(", ")),
                                ));
                            }
                            let bad_context = super::ctx_check::context_errors(&unit);
                            if !bad_context.is_empty() {
                                local.push((
                                    "CONTEXT chain error".to_owned(),
                                    Some(bad_context.join(", ")),
                                ));
                            }
                        }
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            check_determinism(&src, opts)
                        }));
                        let mode = format!("{opts:?}");
                        local.push(match r {
                            Ok(r) => {
                                if let Ok(Determinism::Same(h)) = &r {
                                    hashes.push((short_path(path), mode, *h));
                                }
                                let mut class = class_of(&r);
                                if negative_early && class.starts_with("early error") {
                                    class = "early error (negative test, expected)".into();
                                }
                                (class, r.err().map(|e| e.to_string()))
                            }
                            Err(p) => {
                                let msg = p
                                    .downcast_ref::<String>()
                                    .cloned()
                                    .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                                    .unwrap_or_default();
                                let key: String = msg
                                    .chars()
                                    .filter(|c| !c.is_ascii_digit())
                                    .take(80)
                                    .collect();
                                (format!("PANIC: {key}"), Some(msg))
                            }
                        });
                    }
                    let mut rep = report.lock().unwrap();
                    rep.files += 1;
                    rep.hashes.append(&mut hashes);
                    for (class, err) in local {
                        rep.units += usize::from(
                            !class.starts_with("NO POSITION") && !class.starts_with("CONTEXT"),
                        );
                        *rep.classes.entry(class.clone()).or_insert(0) += 1;
                        if let Some(err) = err {
                            rep.examples
                                .entry(class)
                                .or_insert_with(|| format!("{}: {err}", short_path(path)));
                        }
                    }
                }
            });
        }
    });
    report.into_inner().unwrap()
}

fn print(report: &mut Report) {
    println!(
        "files {}, compiles {} (each built twice for determinism)",
        report.files, report.units
    );
    println!(
        "corpus digest {} over {} units (compare across hosts and targets)",
        report.digest(),
        report.hashes.len()
    );
    for (class, n) in &report.classes {
        println!("  {n:>7}  {class}");
        if let Some(e) = report.examples.get(class) {
            let e: String = e.chars().take(200).collect();
            println!("           e.g. {e}");
        }
    }
}

/// All corpus files: no builder or verifier rejection, no determinism mismatch.
#[test]
#[ignore = "corpus check; needs third_party checkouts; run on demand"]
fn corpus_verifier_clean_and_deterministic() {
    let files = corpus_files();
    let mut report = run(&files);
    print(&mut report);
    let bad: usize = report
        .classes
        .iter()
        .filter(|(c, _)| {
            c.starts_with("INTERNAL")
                || c.starts_with("DETERMINISM")
                || c.starts_with("PANIC")
                || c.starts_with("NO POSITION")
                || c.starts_with("CONTEXT")
        })
        .map(|(_, n)| n)
        .sum();
    assert_eq!(
        bad, 0,
        "internal errors, panics, determinism mismatches, missing positions or context errors"
    );
    // Coverage at the pinned corpus (decisions-log BCG-CORPUS): an unreadable file or a partial
    // checkout drops out without a failure above.
    assert_eq!(
        (report.files, report.units),
        (54_522, 104_375),
        "corpus coverage changed: check the checkouts, then update these counts and BCG-CORPUS"
    );
}
