#![allow(dead_code)] // m4-stub
//! `cargo xtask m1-exit` (M1 exit check, docs/plan/09-milestones.md section 2.2, M1 exit 1-10):
//! the table of the 10 M1 exit items, PASS, FAIL, OPEN, N/A or NOT-RUN per item, for one host
//! and for `x86_64-pc-windows-msvc` from the jobs of a CI run.
//!
//! ```text
//! cargo xtask m1-exit --host <triple> --out m1-exit.md [--root <dir>]
//!     [--kernels <json>]      tools/bench/cross/kernels.mjs --check              item 1
//!     [--cross <json>]        tools/bench/cross run (ep9-cross/v1), exit2.mjs   item 2
//!     [--exec <json>]         ep9-test262 exec --json                           items 3, 4
//!     [--exec-gc <json>]...   ep9-test262 exec --gc-stress ... (S1 on) --json   item 4
//!     [--fuzz <json>]...      one record per fuzz campaign                      item 5
//!     [--base <rev>]          the adr-lint range start (default m0-exit)        item 6
//!     [--verdicts <md>]...    spike Verdict tables                              items 7, 10
//!     [--nesting <log>]       cargo test output of the nesting tests            item 8
//!     [--ci-run <id>] [--ci-run-json <file>]                                    the Windows column
//! ```
//!
//! **M4-P0: the skeleton** (plan v9 section 2.6). The flags are parsed and each given input is
//! read (a JSON input must parse); every item cell is NOT-RUN and the task exits 1. M4-P3 TOOLS
//! builds the items (plan v9 section 9.4). The extra rows (no exit-code effect) are built: the
//! carried M0 item-5 cell (SP-Dispatch, read from the newest `*-m0-spike-review.md`), the stub
//! markers (the `m2`, `m3` and `m4` markers, plan v9 rule 4 and V9-D37) and the ADR Counts
//! line. The tree root is `--root`, or this workspace (as m0-exit). The shared readers are in
//! `crate::exit`.
//!
//! Exit 0 when every cell is PASS or N/A, 1 otherwise, 2 on a usage error or an input that
//! cannot be read.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::exit::adr::counts_line;
use crate::exit::ci::{CI_TRIPLE, Ci};
use crate::exit::git;
use crate::exit::json::{Json, parse_json};
use crate::exit::read;
use crate::exit::report::{Cell, Status, all_ok, cell, stub_markers};
use crate::exit::verdict::{newest_spike_review, verdict_table};

/// The stub markers of plan v9 rule 4 (V9-D37). Written in two parts, so this file does not hold
/// the first two (m0-exit greps for them).
const STUB_MARKERS: [&str; 3] = [
    concat!("m2", "-stub"),
    concat!("m3", "-stub"),
    concat!("m4", "-stub"),
];

/// The 10 M1 exit items (09-milestones section 2.2, M1 exit), short titles.
const TITLES: [&str; 10] = [
    "All 8 v8-v7 kernels and Octane-7 run correctly",
    "Octane-7 >= 1.0x QuickJS; v8-v7 7-kernel geomean >= 0.75x node --jitless",
    "`language/` >= 85%",
    "Same pass set under GC-stress with S1 on; verifier clean",
    "Fuzzilli 100 CPU-h and front-end cargo-fuzz 24 CPU-h, 0 unique crashes",
    "No H-row change without an approved ADR amendment",
    "Spike C gate green; H2, H14 and H16 Frozen (H17-H19 at M1a)",
    "Conditional, assignment, unary, arrow and `**` chains pass cap-1 / cap+1",
    "H29-H34 Frozen",
    "SP-Facts green",
];

/// The parsed command line.
#[derive(Debug, Default)]
struct Options {
    root: PathBuf,
    host: String,
    out: PathBuf,
    kernels: Option<PathBuf>,
    cross: Option<PathBuf>,
    exec: Option<PathBuf>,
    exec_gc: Vec<PathBuf>,
    fuzz: Vec<PathBuf>,
    base: String,
    verdicts: Vec<PathBuf>,
    nesting: Option<PathBuf>,
    ci_run: Option<String>,
    ci_run_json: Option<PathBuf>,
}

const USAGE: &str = "usage: cargo xtask m1-exit --host <triple> --out <file.md> [--root <dir>] \
     [--kernels <json>] [--cross <json>] [--exec <json>] [--exec-gc <json>]... [--fuzz <json>]... \
     [--base <rev>] [--verdicts <md>]... [--nesting <log>] [--ci-run <id>] [--ci-run-json <file>]";

fn parse_args(args: &[String], root: PathBuf) -> Result<Options, String> {
    let mut o = Options {
        root,
        base: "m0-exit".into(),
        ..Options::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n{USAGE}"))
        };
        match a.as_str() {
            "--root" => o.root = PathBuf::from(val()?),
            "--host" => o.host = val()?,
            "--out" => o.out = PathBuf::from(val()?),
            "--kernels" => o.kernels = Some(PathBuf::from(val()?)),
            "--cross" => o.cross = Some(PathBuf::from(val()?)),
            "--exec" => o.exec = Some(PathBuf::from(val()?)),
            "--exec-gc" => o.exec_gc.push(PathBuf::from(val()?)),
            "--fuzz" => o.fuzz.push(PathBuf::from(val()?)),
            "--base" => o.base = val()?,
            "--verdicts" => o.verdicts.push(PathBuf::from(val()?)),
            "--nesting" => o.nesting = Some(PathBuf::from(val()?)),
            "--ci-run" => o.ci_run = Some(val()?),
            "--ci-run-json" => o.ci_run_json = Some(PathBuf::from(val()?)),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if o.host.is_empty() || o.out.as_os_str().is_empty() {
        return Err(format!("--host and --out are required\n{USAGE}"));
    }
    if let Some(id) = &o.ci_run
        && (id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!("--ci-run needs a numeric run id, not {id:?}"));
    }
    if o.base.is_empty() {
        return Err(format!("--base needs a revision\n{USAGE}"));
    }
    Ok(o)
}

/// Entry point: writes the report to `--out` and prints the table.
pub fn main(args: &[String]) -> Result<bool, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    let opts = parse_args(args, root)?;
    let report = run(&opts)?;
    std::fs::write(&opts.out, &report.markdown)
        .map_err(|e| format!("cannot write {}: {e}", opts.out.display()))?;
    print!("{}", report.table);
    println!("m1-exit: wrote {}", opts.out.display());
    Ok(all_ok(&report.cells))
}

/// The rendered result.
#[derive(Debug)]
struct Report {
    table: String,
    markdown: String,
    /// Per item (1-10): the host cell and the Windows cell.
    cells: Vec<(Cell, Cell)>,
    /// The extra rows: (what, cell). No exit-code effect.
    extra: Vec<(&'static str, Cell)>,
}

/// One given input, read: `<flag> <path>` and what was found.
fn input(flag: &str, p: &Path, json: bool) -> Result<String, String> {
    let text = read(p)?;
    if json {
        let j = parse_json(&text).map_err(|e| format!("{flag} {}: {e}", p.display()))?;
        let what = j
            .get("schema")
            .or_else(|| j.get("command"))
            .and_then(Json::str)
            .map_or(String::new(), |s| format!(" ({s})"));
        Ok(format!("{flag} {}{what}", p.display()))
    } else {
        Ok(format!("{flag} {}", p.display()))
    }
}

fn run(o: &Options) -> Result<Report, String> {
    // Every given input, read once (an input that cannot be read is a usage error, exit 2).
    let one = |flag: &str, p: &Option<PathBuf>, json: bool| -> Result<Vec<String>, String> {
        p.iter().map(|p| input(flag, p, json)).collect()
    };
    let many = |flag: &str, ps: &[PathBuf], json: bool| -> Result<Vec<String>, String> {
        ps.iter().map(|p| input(flag, p, json)).collect()
    };
    let kernels = one("--kernels", &o.kernels, true)?;
    let cross = one("--cross", &o.cross, true)?;
    let exec = one("--exec", &o.exec, true)?;
    let gc = many("--exec-gc", &o.exec_gc, true)?;
    let fuzz = many("--fuzz", &o.fuzz, true)?;
    let verdicts = many("--verdicts", &o.verdicts, false)?;
    let nesting = one("--nesting", &o.nesting, false)?;
    let ci = match &o.ci_run_json {
        Some(file) => {
            let ci = Ci::from_json(
                &parse_json(&read(file)?).map_err(|e| format!("{}: {e}", file.display()))?,
            )?;
            if let (Some(want), Some(got)) = (&o.ci_run, &ci.id)
                && want != got
            {
                return Err(format!(
                    "--ci-run {want} but {} holds run {got}",
                    file.display()
                ));
            }
            format!(
                "CI run {} on commit `{}` ({} jobs), from {}",
                ci.id.as_deref().unwrap_or("(no id)"),
                ci.head.as_deref().unwrap_or("unknown"),
                ci.jobs.len(),
                file.display()
            )
        }
        None => match &o.ci_run {
            Some(id) => format!("CI run {id} (--ci-run; not fetched by the skeleton)"),
            None => "no CI run given (--ci-run <id> or --ci-run-json <file>)".into(),
        },
    };
    let base = format!("--base {} (adr-lint range {}..HEAD)", o.base, o.base);
    let inputs: [Vec<String>; 10] = [
        kernels,
        cross,
        exec.clone(),
        gc.into_iter().chain(exec).collect(),
        fuzz,
        vec![base],
        verdicts.clone(),
        nesting,
        vec!["docs/adr/README.md (the ADR index)".into()],
        verdicts,
    ];
    let cells: Vec<(Cell, Cell)> = inputs
        .iter()
        .map(|given| {
            let given = if given.is_empty() {
                "no input given".to_string()
            } else {
                format!("input: {}", given.join(", "))
            };
            (
                cell(
                    Status::NotRun,
                    format!(
                        "not built yet (M4-P0 skeleton; M4-P3 TOOLS builds the check); {given}"
                    ),
                ),
                cell(
                    Status::NotRun,
                    format!("not built yet (M4-P0 skeleton; M4-P3 TOOLS builds the check); {ci}"),
                ),
            )
        })
        .collect();
    Ok(render(o, cells, extra_rows(&o.root)))
}

/// The extra rows (no exit-code effect): the carried M0 item-5 cell, the stub markers, the ADR
/// Counts line.
fn extra_rows(root: &Path) -> Vec<(&'static str, Cell)> {
    let m0_item5 = match newest_spike_review(root) {
        None => cell(
            Status::NotRun,
            "no docs/measurements/*-m0-spike-review.md in this tree",
        ),
        Some(f) => match read(&f) {
            Err(e) => cell(Status::NotRun, e),
            Ok(text) => sp_dispatch_cell(&text, &f),
        },
    };
    let counts = counts_line(root);
    let counts = if counts.is_empty() {
        cell(Status::NotRun, "docs/adr/README.md has no Counts line")
    } else {
        cell(Status::Pass, format!("docs/adr/README.md: {counts}"))
    };
    vec![
        ("carried M0 item 5 (SP-Dispatch condition 2)", m0_item5),
        (
            "stub markers (plan v9 rule 4)",
            stub_markers(root, &STUB_MARKERS),
        ),
        ("ADR Counts line", counts),
    ]
}

/// The carried M0 item-5 cell: the SP-Dispatch verdict of a spike review, by the word rule of
/// m0-exit's `spike_status` (a verdict with "condition" is OPEN). The last row wins, as in m0-exit
/// (a later table of the file overrides an earlier one).
fn sp_dispatch_cell(review: &str, file: &Path) -> Cell {
    let name = file
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let Some((_, v)) = verdict_table(review)
        .into_iter()
        .rfind(|(s, _)| s == "sp-dispatch")
    else {
        return cell(Status::Open, format!("sp-dispatch: no verdict ({name})"));
    };
    let l = v.to_ascii_lowercase();
    let st = if l.contains("no-go") || l.contains("fail") {
        Status::Fail
    } else if l.starts_with("go") && !l.contains("condition") {
        Status::Pass
    } else {
        Status::Open
    };
    cell(
        st,
        format!(
            "sp-dispatch: {v} ({name}); SP-Dispatch condition 2 on AMD EPYC 7763 (H15-Q5), carried from M0 (M3-P5-OPEN item 1)"
        ),
    )
}

fn render(o: &Options, cells: Vec<(Cell, Cell)>, extra: Vec<(&'static str, Cell)>) -> Report {
    let head = git(&o.root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let mut table = String::new();
    let _ = writeln!(
        table,
        "| # | M1 exit item | {} | {CI_TRIPLE} (CI) |",
        o.host
    );
    let _ = writeln!(table, "|---|---|---|---|");
    for (i, (t, (h, w))) in TITLES.iter().zip(&cells).enumerate() {
        let _ = writeln!(
            table,
            "| {} | {t} | {} | {} |",
            i + 1,
            h.status.name(),
            w.status.name()
        );
    }
    let all_pass = all_ok(&cells);
    let mut md = String::from("# M1 exit check (`cargo xtask m1-exit`)\n\n");
    let _ = writeln!(
        md,
        "- Items: docs/plan/09-milestones.md section 2.2, M1 exit 1-10 (plan v9 section 9.4)."
    );
    let _ = writeln!(md, "- Host column: {}; tree at commit `{head}`.", o.host);
    let _ = writeln!(
        md,
        "- Build state: the M4-P0 skeleton. Every item is NOT-RUN until M4-P3 TOOLS builds it."
    );
    let _ = writeln!(
        md,
        "- Result: {}.\n",
        if all_pass {
            "every item PASS on both columns (N/A cells are by design): the M1 exit holds"
        } else {
            "not every cell is PASS"
        }
    );
    md.push_str(&table);
    md.push_str("\n## Evidence\n");
    for (i, (t, (h, w))) in TITLES.iter().zip(&cells).enumerate() {
        let _ = write!(
            md,
            "\n### {}. {t}\n\n- {} {}: {}\n- {CI_TRIPLE} {}: {}\n",
            i + 1,
            o.host,
            h.status.name(),
            h.evidence,
            w.status.name(),
            w.evidence
        );
    }
    md.push_str("\n## Extra rows\n\nNot exit items: they do not change the result above.\n\n");
    for (what, c) in &extra {
        let _ = writeln!(md, "- {what}: {} {}", c.status.name(), c.evidence);
    }
    Report {
        table,
        markdown: md,
        cells,
        extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const HOST: &str = "aarch64-apple-darwin";

    /// The fixtures (plan v9 rule 24: no live ADR README or expectation file is read).
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/m1_exit")
            .join(name)
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// A fake repo (a git work tree) with the fixture ADR README and spike review.
    fn fake_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ep9-m1-exit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (from, to) in [
            ("adr-README.md", "docs/adr/README.md"),
            (
                "m0-spike-review.md",
                "docs/measurements/2026-10-26-m0-spike-review.md",
            ),
        ] {
            write(&root, to, &read(&fixture(from)).unwrap());
        }
        write(&root, "crates/ep9_vm/src/lib.rs", "// no marker\n");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args([
                    "-c",
                    "user.name=m1-exit test",
                    "-c",
                    "user.email=m1-exit@test.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        };
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "--no-verify", "-m", "fake"]);
        root
    }

    fn opts(root: &Path) -> Options {
        Options {
            root: root.to_path_buf(),
            host: HOST.into(),
            out: root.join("out.md"),
            base: "m0-exit".into(),
            ..Options::default()
        }
    }

    /// The P0 acceptance: 10 rows, every cell NOT-RUN, not a pass (exit 1).
    #[test]
    fn the_skeleton_prints_ten_not_run_rows() {
        let root = fake_repo("skeleton");
        let r = run(&opts(&root)).unwrap();
        assert_eq!(r.cells.len(), 10);
        assert!(
            r.cells
                .iter()
                .all(|(h, w)| h.status == Status::NotRun && w.status == Status::NotRun),
            "{}",
            r.markdown
        );
        assert!(!all_ok(&r.cells));
        // A header, the separator and 10 rows, each NOT-RUN in both columns.
        assert_eq!(r.table.lines().count(), 12, "{}", r.table);
        assert_eq!(
            r.table
                .lines()
                .filter(|l| l.ends_with("| NOT-RUN | NOT-RUN |"))
                .count(),
            10
        );
        assert!(r.cells[0].0.evidence.contains("no input given"));
        // The extra rows: SP-Dispatch is GO with a condition (OPEN), no stub marker, the Counts
        // line of the fixture README.
        assert_eq!(r.extra.len(), 3);
        assert_eq!(r.extra[0].1.status, Status::Open, "{:?}", r.extra[0]);
        assert!(r.extra[0].1.evidence.contains("condition 2"));
        assert_eq!(r.extra[1].1.status, Status::Pass, "{:?}", r.extra[1]);
        assert_eq!(r.extra[2].1.status, Status::Pass, "{:?}", r.extra[2]);
        assert!(r.extra[2].1.evidence.contains("Counts on 2026-10-26"));
        assert!(r.markdown.contains("## Extra rows"));
        // main() writes the file and returns "not a pass".
        let out = root.join("main.md");
        let args: Vec<String> = [
            "--root",
            root.to_str().unwrap(),
            "--host",
            HOST,
            "--out",
            out.to_str().unwrap(),
        ]
        .map(str::to_string)
        .to_vec();
        assert!(!main(&args).unwrap());
        assert!(read(&out).unwrap().starts_with("# M1 exit check"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Task 4 (V9-A4): the stub-marker row reads the m2, m3 and m4 markers.
    #[test]
    fn the_stub_row_reads_three_markers() {
        let root = fake_repo("stubs");
        let r = run(&opts(&root)).unwrap();
        assert_eq!(
            r.extra[1].1.evidence,
            format!(
                "no `{}`, `{}` or `{}` line",
                STUB_MARKERS[0], STUB_MARKERS[1], STUB_MARKERS[2]
            )
        );
        for (i, f) in ["crates/a.rs", "spikes/b.rs", "tools/c.rs"]
            .iter()
            .enumerate()
        {
            write(
                &root,
                f,
                &format!("#![allow(dead_code)] // {}\n", STUB_MARKERS[i]),
            );
        }
        let r = run(&opts(&root)).unwrap();
        let c = &r.extra[1].1;
        assert_eq!(c.status, Status::Open, "{c:?}");
        for f in ["crates/a.rs:1", "spikes/b.rs:1", "tools/c.rs:1"] {
            assert!(c.evidence.contains(f), "{f}: {}", c.evidence);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Given inputs are read and named; one that cannot be read or parsed is an error (exit 2).
    #[test]
    fn inputs_are_read() {
        let root = fake_repo("inputs");
        let mut o = opts(&root);
        o.exec = Some(fixture("exec.json"));
        o.exec_gc = vec![fixture("exec.json"), fixture("exec.json")];
        o.ci_run_json = Some(fixture("ci-run.json"));
        let r = run(&o).unwrap();
        assert!(
            r.cells[2].0.evidence.contains("exec.json (exec)"),
            "{}",
            r.cells[2].0.evidence
        );
        // Item 4 reads the GC shards and the plain exec run.
        assert_eq!(r.cells[3].0.evidence.matches("exec.json").count(), 3);
        assert!(
            r.cells[0].1.evidence.contains("CI run 123 on commit `abc`"),
            "{}",
            r.cells[0].1.evidence
        );
        o.ci_run = Some("7".into());
        assert!(run(&o).unwrap_err().contains("holds run 123"));
        o.ci_run = None;
        o.cross = Some(fixture("adr-README.md"));
        assert!(run(&o).unwrap_err().contains("JSON"));
        o.cross = Some(root.join("no-such.json"));
        assert!(run(&o).unwrap_err().contains("cannot read"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn arguments() {
        let a = |s: &[&str]| {
            parse_args(
                &s.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                PathBuf::new(),
            )
        };
        let o = a(&[
            "--host",
            "x",
            "--out",
            "o",
            "--exec-gc",
            "a",
            "--exec-gc",
            "b",
            "--fuzz",
            "f",
            "--fuzz",
            "g",
            "--verdicts",
            "v",
            "--kernels",
            "k",
            "--cross",
            "c",
            "--exec",
            "e",
            "--nesting",
            "n",
            "--ci-run",
            "12",
            "--ci-run-json",
            "j",
            "--root",
            "r",
        ])
        .unwrap();
        assert_eq!((o.exec_gc.len(), o.fuzz.len(), o.verdicts.len()), (2, 2, 1));
        assert_eq!(o.base, "m0-exit");
        assert_eq!(o.root, PathBuf::from("r"));
        assert_eq!(
            a(&["--host", "x", "--out", "o", "--base", "d7cc797"])
                .unwrap()
                .base,
            "d7cc797"
        );
        assert!(a(&["--host", "x"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--ci-run", "12a"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--what"]).is_err());
        assert!(a(&["--host", "x", "--out", "o", "--base", ""]).is_err());
        assert!(a(&["--host"]).is_err());
    }
}
