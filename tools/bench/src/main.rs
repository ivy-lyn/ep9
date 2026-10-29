//! `ep9-bench`: perf CI v0 (docs/plan/09-milestones.md section 8 week 1; 10-quality section 2).
//!
//! It runs a command N times and records the instructions retired per run as JSON. Speed gates
//! use instruction counts, because wall time on shared hosts is too noisy \[R03\]\[R09\].
//!
//! ```text
//! ep9-bench [--runs N] [--warmup W] [--counter auto|time|perf|cachegrind]
//!           [--name NAME] [--out FILE] [--show-output | --expect-stdout FILE]
//!           -- <command> [args...]
//! ```
//!
//! A host with no instruction counter (Windows; a VM without PMC access) still gets the runs and
//! the JSON, with null counts and `"counter_status": "unavailable"`, and exit status 3.
//!
//! With `--expect-stdout`, every run (warm-up and measured) must print the expected output, so a
//! wrong answer never gets a count (W4-PERF-REFUSE): a run that does not stops ep9-bench with exit
//! status 4 and no JSON.
#![forbid(unsafe_code)]

mod counter;
mod report;

use counter::{Counter, RunError, Sample};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: ep9-bench [flags] -- <command> [args...]

Runs <command> N times and writes the instructions retired per run as JSON.

flags:
  --runs N          measured runs (default 5)
  --warmup W        unmeasured runs before them (default 1)
  --counter C       auto | time (macOS) | perf (Linux) | cachegrind (Linux) (default auto)
  --name NAME       benchmark name in the report (default: the command)
  --out FILE        write the JSON there (default: stdout)
  --show-output     keep the command's stdout and stderr (default: discarded; a failed run
                    prints the last 20 lines of its stderr)
  --expect-stdout FILE
                    every run must print what FILE holds (compared after CRLF -> LF and a
                    trim of leading and trailing whitespace), else exit 4. Not with --show-output

Before the runs, ep9-bench probes the counter: `true` under it must give a count above 0.

exit status:
  0  the JSON has the counts
  1  error: the command failed, a counted run gave no count, or the JSON was not written
  2  usage error
  3  no instruction counter on this host. The JSON is still written: \"counter\": null,
     \"counter_status\": \"unavailable\", \"counter_note\": <why>, null counts (wall time only)
  4  a run printed something other than --expect-stdout. No JSON is written

environment:
  EP9_BENCH_TIME    the `time -l` program (default /usr/bin/time; tests use a fake)
";

/// Exit status when the host has no instruction counter. The JSON is still written.
const EXIT_NO_COUNTER: u8 = 3;

/// Exit status when a run's stdout is not the `--expect-stdout` text. No JSON is written.
const EXIT_WRONG_OUTPUT: u8 = 4;

struct Options {
    runs: usize,
    warmup: usize,
    counter: Option<Counter>,
    name: Option<String>,
    out: Option<PathBuf>,
    show_output: bool,
    expect_stdout: Option<PathBuf>,
    command: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        runs: 5,
        warmup: 1,
        counter: None,
        name: None,
        out: None,
        show_output: false,
        expect_stdout: None,
        command: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--runs" => o.runs = value()?.parse().map_err(|_| "--runs needs a number")?,
            "--warmup" => o.warmup = value()?.parse().map_err(|_| "--warmup needs a number")?,
            "--counter" => {
                let v = value()?;
                if v != "auto" {
                    o.counter = Some(Counter::parse(&v).ok_or(format!("unknown counter {v}"))?);
                }
            }
            "--name" => o.name = Some(value()?),
            "--out" => o.out = Some(PathBuf::from(value()?)),
            "--show-output" => o.show_output = true,
            "--expect-stdout" => o.expect_stdout = Some(PathBuf::from(value()?)),
            "--" => {
                o.command = it.cloned().collect();
                break;
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if o.command.is_empty() {
        return Err("no command after --".into());
    }
    if o.runs == 0 {
        return Err("--runs must be at least 1".into());
    }
    if o.show_output && o.expect_stdout.is_some() {
        return Err("--show-output and --expect-stdout cannot be used together".into());
    }
    Ok(o)
}

/// The JSON, and why the host has no counter (`None` when it has one).
struct Outcome {
    json: String,
    no_counter: Option<String>,
}

fn run(o: &Options) -> Result<Outcome, RunError> {
    let want = match &o.expect_stdout {
        Some(p) => Some(
            std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?,
        ),
        None => None,
    };
    let counter = match o.counter {
        Some(c) => c.probe().map(|()| c),
        None => Counter::detect(),
    };
    let wrap = counter.as_ref().ok().copied();
    let program = PathBuf::from(&o.command[0]);
    let args = &o.command[1..];
    let one = || counter::measure(wrap, &program, args, o.show_output, want.as_deref());
    for _ in 0..o.warmup {
        one()?;
    }
    let samples: Vec<Sample> = (0..o.runs).map(|_| one()).collect::<Result<_, _>>()?;
    let name = o.name.clone().unwrap_or_else(|| o.command.join(" "));
    let build = engine_build(&program);
    Ok(Outcome {
        json: report::json(&name, &o.command, &counter, build.as_deref(), &samples),
        no_counter: counter.err(),
    })
}

/// The build line of an ep9 binary (`"engine_build"`, report.rs): when the program's file name
/// starts with `ep9`, the first line of its `--version` output that starts with `features `
/// (`features <list>; preset <name>`, M5-P0). None for another program, a failed run, or a
/// binary that prints no such line.
fn engine_build(program: &std::path::Path) -> Option<String> {
    let name = program.file_name()?.to_str()?;
    if !name.starts_with("ep9") {
        return None;
    }
    let out = std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    build_line(&String::from_utf8_lossy(&out.stdout))
}

/// The `features ...` line of a `--version` text, trimmed.
fn build_line(version: &str) -> Option<String> {
    version
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("features "))
        .map(str::to_string)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Only flags before `--`: the command's own `--help` goes to the command.
    let flags = args.iter().take_while(|a| *a != "--");
    if flags.into_iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ep9-bench: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&opts) {
        Ok(Outcome { json, no_counter }) => {
            if let Some(path) = &opts.out {
                if let Err(e) = std::fs::write(path, &json) {
                    eprintln!("ep9-bench: cannot write {}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            } else {
                print!("{json}");
            }
            match no_counter {
                None => ExitCode::SUCCESS,
                Some(why) => {
                    eprintln!(
                        "ep9-bench: no instruction counter on this host: {why}\n\
                         ep9-bench: the JSON has \"counter_status\": \"unavailable\" and null \
                         counts (wall time only); exit {EXIT_NO_COUNTER}"
                    );
                    ExitCode::from(EXIT_NO_COUNTER)
                }
            }
        }
        Err(e) => {
            eprintln!("ep9-bench: {e}");
            match e {
                RunError::Failed(_) => ExitCode::FAILURE,
                RunError::WrongOutput(_) => ExitCode::from(EXIT_WRONG_OUTPUT),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// The build line is the `features ...` line of `--version`; an older binary has none.
    #[test]
    fn build_line_of_a_version_text() {
        assert_eq!(
            build_line("ep9 0.0.0\r\nfeatures jit,become; preset jit\r\n").as_deref(),
            Some("features jit,become; preset jit")
        );
        assert_eq!(build_line("ep9 0.0.0\n"), None);
        assert_eq!(engine_build(std::path::Path::new("true")), None);
    }

    /// A program whose file name starts with `ep9` is asked for `--version`; one that does not is
    /// not, whatever it prints.
    #[cfg(unix)]
    #[test]
    fn engine_build_asks_an_ep9_binary_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ep9-bench-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = "#!/bin/sh\n[ \"$1\" = --version ] || exit 9\n\
                      echo 'ep9 0.0.0'\necho 'features none; preset jitless'\n";
        for name in ["ep9-fake", "other-fake"] {
            let p = dir.join(name);
            std::fs::write(&p, script).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            engine_build(&dir.join("ep9-fake")).as_deref(),
            Some("features none; preset jitless")
        );
        assert_eq!(engine_build(&dir.join("other-fake")), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn args() {
        let o = parse_args(&strings(&[
            "--runs",
            "3",
            "--counter",
            "perf",
            "--",
            "ep9",
            "-e",
            "0",
        ]))
        .unwrap();
        assert_eq!(o.runs, 3);
        assert_eq!(o.warmup, 1);
        assert_eq!(o.counter, Some(Counter::Perf));
        assert_eq!(o.command, strings(&["ep9", "-e", "0"]));
        assert!(parse_args(&strings(&["--runs", "3"])).is_err());
        assert!(parse_args(&strings(&["--runs", "0", "--", "x"])).is_err());
        assert!(parse_args(&strings(&["--counter", "cycles", "--", "x"])).is_err());
        assert_eq!(
            parse_args(&strings(&["--counter", "auto", "--", "x"]))
                .unwrap()
                .counter,
            None
        );
        let o = parse_args(&strings(&["--expect-stdout", "want.txt", "--", "x"])).unwrap();
        assert_eq!(o.expect_stdout, Some(PathBuf::from("want.txt")));
        assert!(
            parse_args(&strings(&[
                "--expect-stdout",
                "want.txt",
                "--show-output",
                "--",
                "x"
            ]))
            .is_err()
        );
    }
}
