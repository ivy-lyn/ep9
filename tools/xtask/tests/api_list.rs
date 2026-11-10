//! `cargo xtask api-list` (H20-Q6) on the rustdoc JSON fixtures in `tests/api_list/` (README
//! there): the exit codes and the diff lines. No rustdoc runs (`--json`), so no nightly is
//! needed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("api_list")
        .join(name)
}

fn api_list(json: &str, extra: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("api-list")
        .arg("--json")
        .arg(fixture(json))
        .arg("--golden")
        .arg(fixture("golden.txt"))
        .args(extra)
        .output()
        .unwrap();
    eprintln!(
        "--- api-list {json} {extra:?}: {}\n--- stdout\n{}--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn the_golden_is_the_listing_of_the_base_json() {
    let out = api_list("base.json", &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("api-list: 4 lines, golden 4 lines: 0 new, 0 gone"));
    let p = api_list("base.json", &["--print"]);
    let golden = std::fs::read_to_string(fixture("golden.txt")).unwrap();
    let want: Vec<&str> = golden.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(stdout(&p).lines().collect::<Vec<_>>(), want);
}

/// The acceptance of H20-Q6 (plan v12 §8.1): a new `pub` item fails the golden diff.
#[test]
fn a_new_pub_item_fails_the_diff() {
    let out = api_list("new-pub-item.json", &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    let diff: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("api-list:"))
        .collect();
    assert_eq!(diff, ["+ k::probe_new_item :: fn probe_new_item() -> u32"]);
    assert!(text.contains("1 new, 0 gone"), "{text}");
}

#[test]
fn another_format_version_and_bad_arguments_exit_2() {
    let out = api_list("format-62.json", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("format_version 62"));
    assert_eq!(
        api_list("base.json", &["--bless", "--print"]).status.code(),
        Some(2)
    );
    assert_eq!(api_list("base.json", &["--nope"]).status.code(), Some(2));
}

/// `api-list --json <variant> --golden forms-golden.txt` (the listing of `forms-base.json`):
/// the diff lines of a variant.
fn forms_diff(json: &str) -> (Option<i32>, Vec<String>) {
    golden_diff(json, "forms-golden.txt")
}

/// `api-list --json <variant> --golden <golden>`: the exit code and the diff lines.
fn golden_diff(json: &str, golden: &str) -> (Option<i32>, Vec<String>) {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("api-list")
        .arg("--json")
        .arg(fixture(json))
        .arg("--golden")
        .arg(fixture(golden))
        .output()
        .unwrap();
    let text = stdout(&out);
    eprintln!("--- api-list {json}: {}\n{text}", out.status);
    let diff = text
        .lines()
        .filter(|l| !l.starts_with("api-list:"))
        .map(str::to_string)
        .collect();
    (out.status.code(), diff)
}

#[test]
fn the_forms_golden_is_the_listing_of_the_forms_base() {
    assert_eq!(forms_diff("forms-base.json"), (Some(0), vec![]));
}

/// Codex T1 (M7-P5): a callable item of a local private supertrait (the sealed form of
/// `Downcast: Sealed`) is API: a change of its signature fails the diff.
#[test]
fn a_private_supertrait_item_change_fails_the_diff() {
    assert_eq!(
        forms_diff("forms-supertrait.json"),
        (
            Some(1),
            vec![
                "+ fx::Downcast :: supertrait Sealed :: fn test(&self)".to_string(),
                "- fx::Downcast :: supertrait Sealed :: fn test(&self, Token)".to_string(),
            ]
        )
    );
}

/// Codex T3 (M7-P5): `pub use hidden::E::*` of a local enum lists each variant at its path,
/// so a variant change fails the diff.
#[test]
fn a_variant_change_behind_an_enum_glob_fails_the_diff() {
    assert_eq!(
        forms_diff("forms-enum-glob.json"),
        (
            Some(1),
            vec![
                "- fx::A :: variant A".to_string(),
                "+ fx::A :: variant A(u32)".to_string(),
                "+ fx::B :: variant B { x: u64 } = 7".to_string(),
            ]
        )
    );
}

/// Codex T4 (M7-P5): `extern "C"` -> `extern "C-unwind"` on a fn and a fn pointer type
/// fails the diff (rustdoc's ABI object `{"C": {"unwind": true}}`).
#[test]
fn an_unwind_abi_change_fails_the_diff() {
    assert_eq!(
        forms_diff("forms-unwind.json"),
        (
            Some(1),
            vec![
                "- fx::Fp :: type Fp = extern \"C\" fn()".to_string(),
                "+ fx::Fp :: type Fp = extern \"C-unwind\" fn()".to_string(),
                "- fx::f :: extern \"C\" fn f()".to_string(),
                "+ fx::f :: extern \"C-unwind\" fn f()".to_string(),
            ]
        )
    );
}

#[test]
fn the_where_golden_is_the_listing_of_the_where_base() {
    assert_eq!(
        golden_diff("where-base.json", "where-golden.txt"),
        (Some(0), vec![])
    );
}

/// M7-P5 Codex R3 finding 2 (fixer r5 item 2): the sealed supertrait in a where clause
/// (`trait Downcast where Self: sealed::Sealed`; rustdoc puts it in
/// `generics.where_predicates`, not in `bounds`) is walked as the bound form is, also one
/// level down (`Sealed where Self: Inner`): a change of an item's signature fails the diff.
#[test]
fn a_where_clause_supertrait_item_change_fails_the_diff() {
    assert_eq!(
        golden_diff("where-w64.json", "where-golden.txt"),
        (
            Some(1),
            vec![
                "- fx::Downcast :: supertrait Sealed :: fn test(&self) -> u32".to_string(),
                "+ fx::Downcast :: supertrait Sealed :: fn test(&self) -> u64".to_string(),
            ]
        )
    );
}
