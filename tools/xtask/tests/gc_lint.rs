//! `cargo xtask gc-lint` on the cases in `tests/gc_lint/` (README there) and on the tree.
//!
//! The cases are only parsed, never built, so these tests need no tool and run on every
//! runner. A case marks each line it expects as a finding with a trailing `// FLAG`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("gc_lint")
}

fn gc_lint(root: Option<&Path>, paths: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_xtask"));
    c.arg("gc-lint");
    if let Some(r) = root {
        c.arg("--root").arg(r);
    }
    let out = c.args(paths).output().unwrap();
    eprintln!(
        "--- gc-lint {paths:?}: {}\n--- stdout\n{}--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
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

/// The lines of `case` that the output reports as findings (`case:line: ...`), in order and
/// with repeats: two findings on one line show twice.
fn reported(out: &str, case: &str) -> Vec<usize> {
    let prefix = format!("{case}:");
    out.lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .map(|rest| rest.split(':').next().unwrap().parse().unwrap())
        .collect()
}

/// Runs gc-lint on one case: exit 1 with exactly its `// FLAG` lines, or exit 0 if it has none.
fn check_case(case: &str) {
    let want = flagged(case);
    let out = gc_lint(Some(&fixtures()), &[case]);
    let code = if want.is_empty() { 0 } else { 1 };
    assert_eq!(out.status.code(), Some(code), "{case}");
    assert_eq!(reported(&stdout(&out), case), want, "{case}");
}

#[test]
fn vec_of_locals() {
    check_case("vec_local.rs");
}

#[test]
fn hash_map_of_property_descriptors() {
    check_case("hashmap_descriptor.rs");
}

#[test]
fn boxed_slice_of_js_str() {
    check_case("box_jsstr.rs");
}

#[test]
fn struct_holding_a_local_in_a_vec() {
    check_case("holder_in_vec.rs");
}

#[test]
fn static_local() {
    check_case("static_local.rs");
}

#[test]
fn thread_local_local() {
    check_case("thread_local_local.rs");
}

#[test]
fn closure_capture_in_a_box() {
    check_case("boxed_closure.rs");
}

/// An alias in one file (`use std::alloc::Layout as Local`) hides no `Local` in another, also
/// when two files give one alias name different targets.
#[test]
fn an_alias_never_hides_a_stack_only_type() {
    let out = gc_lint(Some(&fixtures()), &["alias_collision"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    for case in ["alias_collision/store.rs", "alias_collision/unrelated.rs"] {
        assert_eq!(reported(&text, case), flagged(case), "{case}");
    }
}

/// `Holder<'s, Local<'s>>` where `Holder` holds a `Local` and puts `T` in a `Vec`.
#[test]
fn a_derived_type_walks_its_type_arguments() {
    check_case("generic_holder.rs");
}

#[test]
fn a_boxing_call_through_an_alias() {
    check_case("boxed_closure_alias.rs");
}

#[test]
fn a_shadow_hides_a_capture_only_where_it_is_in_scope() {
    check_case("closure_shadowing.rs");
}

/// `IteratorRecord` (H20 ABI-3) is stack-only because it holds `Local`s: next to the real
/// `crates/ep9_vm/src/abi/iter.rs`, `Vec<IteratorRecord>` is a finding. The same case alone is
/// clean, so the finding comes from the real definition, not from a name list.
#[test]
fn a_vec_of_iterator_records_is_a_finding() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gc_lint_iterator_record");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let case = "iterator_record/records_in_vec.rs";
    fs::copy(fixtures().join(case), root.join("records_in_vec.rs")).unwrap();
    let alone = gc_lint(Some(&root), &["records_in_vec.rs"]);
    assert_eq!(alone.status.code(), Some(0));
    let iter_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/ep9_vm/src/abi/iter.rs");
    fs::copy(iter_rs, root.join("iter.rs")).unwrap();
    let out = gc_lint(Some(&root), &["records_in_vec.rs", "iter.rs"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert_eq!(reported(&text, "records_in_vec.rs"), flagged(case));
    assert!(reported(&text, "iter.rs").is_empty(), "{text}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn clean_patterns_pass() {
    assert!(flagged("clean.rs").is_empty());
    check_case("clean.rs");
}

#[test]
fn paths_print_with_slashes() {
    check_case("nested/deep/vec_in_mod.rs");
    // A directory argument: the files under it print relative to the root, with `/`.
    let out = gc_lint(Some(&fixtures()), &["nested"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert_eq!(
        reported(&text, "nested/deep/vec_in_mod.rs"),
        flagged("nested/deep/vec_in_mod.rs")
    );
    assert!(!text.contains('\\'), "{text}");
}

#[test]
fn an_allow_skips_its_item_and_is_listed() {
    let out = gc_lint(Some(&fixtures()), &["allowed.rs"]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(reported(&text, "allowed.rs").is_empty(), "{text}");
    assert!(
        text.contains(
            "gc-lint: allow allowed.rs:5: fixture: a reviewed exception (lines 6-8, 1 finding)"
        ),
        "{text}"
    );
}

#[test]
fn wrong_allows_are_findings() {
    let out = gc_lint(Some(&fixtures()), &["bad_allows.rs"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert_eq!(reported(&text, "bad_allows.rs"), [6, 9, 12, 13]);
    assert!(text.contains("bad_allows.rs:6: this gc-lint allow covers no finding"));
    assert!(text.contains("bad_allows.rs:9: this gc-lint allow is not followed by"));
    assert!(text.contains("bad_allows.rs:13: malformed gc-lint comment"));
}

#[test]
fn usage_and_parse_errors_exit_2() {
    let out = gc_lint(Some(&fixtures()), &["does_not_parse.txt"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("does_not_parse.txt:3:"), "{err}");
    assert_eq!(
        gc_lint(Some(&fixtures()), &["no_such_case.rs"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(gc_lint(None, &["--no-such-flag"]).status.code(), Some(2));
}

/// The engine tree passes, and the one allow (the debug test of the on-stack check) is listed.
#[test]
fn the_tree_is_clean() {
    let out = gc_lint(None, &[]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(
        text.contains("gc-lint: allow crates/ep9_vm/src/abi/local.rs:"),
        "{text}"
    );
}
