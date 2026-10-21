//! `cargo xtask feature-matrix` (H27, docs/plan/08-repo.md section 7): build, clippy
//! `-D warnings` and test for each feature set, one package at a time, never with
//! `--workspace` (a workspace build unifies features and would hide a jitless leak):
//!
//! | Set | Packages and flags |
//! |---|---|
//! | default (`jit`) | `-p ep9_cli` |
//! | jitless | `-p ep9_cli --no-default-features` |
//! | aot | `-p ep9_unit --features aot` |
//!
//! Each set runs `cargo build`, `cargo clippy --all-targets -- -D warnings` and `cargo test`,
//! all with `--locked`. Every command runs even after a failure, so one run shows every broken
//! cell; the summary lists PASS or FAIL per cell. Exit 0 when all pass, 1 when any fails, 2 on
//! a usage error or when cargo cannot start.

use std::ffi::OsString;
use std::process::Command;

const USAGE: &str = "\
usage: cargo xtask feature-matrix [options]
  --set NAME        default, jitless or aot; repeat for more (default: all three)
  --step NAME       build, clippy or test; repeat for more (default: all three)
  --target TRIPLE   pass --target to every command (a cross target allows build and clippy
                    only, since its tests cannot run here)
  --dry-run         print the commands, run nothing";

/// The feature sets: name and the package flags.
pub(crate) const SETS: [(&str, &[&str]); 3] = [
    ("default", &["-p", "ep9_cli"]),
    ("jitless", &["-p", "ep9_cli", "--no-default-features"]),
    ("aot", &["-p", "ep9_unit", "--features", "aot"]),
];

/// The steps, in run order.
pub(crate) const STEPS: [&str; 3] = ["build", "clippy", "test"];

#[derive(Debug)]
struct Opts {
    sets: Vec<&'static str>,
    steps: Vec<&'static str>,
    target: Option<String>,
    dry_run: bool,
}

fn pick(all: &[&'static str], name: &str, what: &str) -> Result<&'static str, String> {
    all.iter()
        .find(|n| **n == name)
        .copied()
        .ok_or_else(|| format!("unknown {what} {name:?}\n{USAGE}"))
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let set_names: Vec<&'static str> = SETS.iter().map(|(n, _)| *n).collect();
    let mut o = Opts {
        sets: Vec::new(),
        steps: Vec::new(),
        target: None,
        dry_run: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--set" => add(&mut o.sets, pick(&set_names, &val()?, "set")?),
            "--step" => add(&mut o.steps, pick(&STEPS, &val()?, "step")?),
            "--target" => o.target = Some(val()?),
            "--dry-run" => o.dry_run = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    if o.sets.is_empty() {
        o.sets = set_names;
    }
    if o.steps.is_empty() {
        o.steps = STEPS.to_vec();
    }
    Ok(o)
}

fn add(list: &mut Vec<&'static str>, name: &'static str) {
    if !list.contains(&name) {
        list.push(name);
    }
}

/// The cargo arguments of one cell.
pub(crate) fn cargo_args(set: &str, step: &str, target: Option<&str>) -> Vec<String> {
    let flags = SETS
        .iter()
        .find(|(n, _)| *n == set)
        .map(|(_, f)| *f)
        .expect("known set");
    let mut a = vec![step.to_string(), "--locked".to_string()];
    a.extend(flags.iter().map(|s| s.to_string()));
    if let Some(t) = target {
        a.extend(["--target".to_string(), t.to_string()]);
    }
    if step == "clippy" {
        a.extend(["--all-targets", "--", "-D", "warnings"].map(String::from));
    }
    a
}

pub fn main(args: &[String]) -> Result<bool, String> {
    let o = parse_opts(args)?;
    if let Some(t) = &o.target
        && o.steps.contains(&"test")
        && *t != crate::disasm::host_triple()?
    {
        return Err(format!(
            "tests cannot run for {t} on this host; pass --step build --step clippy\n{USAGE}"
        ));
    }
    // `cargo xtask` runs under cargo, which sets CARGO to itself (the pinned toolchain).
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut results = Vec::new();
    for set in &o.sets {
        for step in &o.steps {
            let args = cargo_args(set, step, o.target.as_deref());
            println!("feature-matrix: {set} {step}: cargo {}", args.join(" "));
            if o.dry_run {
                continue;
            }
            let status = Command::new(&cargo)
                .args(&args)
                .current_dir(crate::disasm::repo_root())
                .status()
                .map_err(|e| format!("cannot run {cargo:?}: {e}"))?;
            results.push((*set, *step, status.success()));
        }
    }
    println!();
    for (set, step, ok) in &results {
        println!(
            "feature-matrix {}: {set} {step}",
            if *ok { "PASS" } else { "FAIL" }
        );
    }
    Ok(results.iter().all(|(_, _, ok)| *ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_run_is_nine_cells_per_package_never_workspace() {
        let o = parse_opts(&[]).unwrap();
        assert_eq!(o.sets, ["default", "jitless", "aot"]);
        assert_eq!(o.steps, ["build", "clippy", "test"]);
        let mut n = 0;
        for set in &o.sets {
            for step in &o.steps {
                let a = cargo_args(set, step, None);
                n += 1;
                assert_eq!(a[0], *step);
                assert!(a.contains(&"--locked".to_string()));
                assert!(a.contains(&"-p".to_string()));
                assert!(!a.iter().any(|s| s == "--workspace" || s == "--all"));
            }
        }
        assert_eq!(n, 9);
    }

    #[test]
    fn the_three_sets() {
        assert_eq!(
            cargo_args("default", "build", None).join(" "),
            "build --locked -p ep9_cli"
        );
        assert_eq!(
            cargo_args("jitless", "test", None).join(" "),
            "test --locked -p ep9_cli --no-default-features"
        );
        assert_eq!(
            cargo_args("aot", "clippy", Some("x86_64-pc-windows-msvc")).join(" "),
            "clippy --locked -p ep9_unit --features aot --target x86_64-pc-windows-msvc \
             --all-targets -- -D warnings"
        );
    }

    #[test]
    fn options() {
        let o = parse_opts(&args(&[
            "--set",
            "aot",
            "--step",
            "test",
            "--set",
            "jitless",
            "--set",
            "aot",
            "--dry-run",
        ]))
        .unwrap();
        assert_eq!(o.sets, ["aot", "jitless"]);
        assert_eq!(o.steps, ["test"]);
        assert!(o.dry_run);
        assert!(parse_opts(&args(&["--set", "jit"])).is_err());
        assert!(parse_opts(&args(&["--step", "fmt"])).is_err());
        assert!(parse_opts(&args(&["--set"])).is_err());
        assert!(parse_opts(&args(&["--workspace"])).is_err());
    }

    #[test]
    fn a_cross_target_cannot_run_tests() {
        let e = main(&args(&[
            "--target",
            "riscv64gc-unknown-none-elf",
            "--dry-run",
        ]))
        .unwrap_err();
        assert!(e.contains("tests cannot run"), "{e}");
        let ok = main(&args(&[
            "--target",
            "riscv64gc-unknown-none-elf",
            "--step",
            "build",
            "--dry-run",
        ]));
        assert_eq!(ok, Ok(true));
    }
}
