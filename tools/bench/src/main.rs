//! `ep9-bench`: perf CI v0 (docs/plan/09-milestones.md section 8 week 1; 10-quality section 2).
//!
//! It runs a command N times and records the instructions retired per run as JSON. Speed gates
//! use instruction counts, because wall time on shared hosts is too noisy [R03][R09].
//!
//! ```text
//! ep9-bench [--runs N] [--warmup W] [--counter auto|time|perf|cachegrind]
//!           [--name NAME] [--out FILE] [--show-output] -- <command> [args...]
//! ```
//!
//! A host with no instruction counter (Windows; a VM without PMC access) still gets the runs and
//! the JSON, with null counts and `"counter_status": "unavailable"`, and exit status 3.
#![forbid(unsafe_code)]

mod counter;
mod report;

use counter::{Counter, Sample};
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

Before the runs, ep9-bench probes the counter: `true` under it must give a count above 0.

exit status:
  0  the JSON has the counts
  1  error: the command failed, a counted run gave no count, or the JSON was not written
  2  usage error
  3  no instruction counter on this host. The JSON is still written: \"counter\": null,
     \"counter_status\": \"unavailable\", \"counter_note\": <why>, null counts (wall time only)

environment:
  EP9_BENCH_TIME    the `time -l` program (default /usr/bin/time; tests use a fake)
";

/// Exit status when the host has no instruction counter. The JSON is still written.
const EXIT_NO_COUNTER: u8 = 3;

struct Options {
    runs: usize,
    warmup: usize,
    counter: Option<Counter>,
    name: Option<String>,
    out: Option<PathBuf>,
    show_output: bool,
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
    Ok(o)
}

/// The JSON, and why the host has no counter (`None` when it has one).
struct Outcome {
    json: String,
    no_counter: Option<String>,
}

fn run(o: &Options) -> Result<Outcome, String> {
    let counter = match o.counter {
        Some(c) => c.probe().map(|()| c),
        None => Counter::detect(),
    };
    let wrap = counter.as_ref().ok().copied();
    let program = PathBuf::from(&o.command[0]);
    let args = &o.command[1..];
    for _ in 0..o.warmup {
        counter::measure(wrap, &program, args, o.show_output)?;
    }
    let samples: Vec<Sample> = (0..o.runs)
        .map(|_| counter::measure(wrap, &program, args, o.show_output))
        .collect::<Result<_, _>>()?;
    let name = o.name.clone().unwrap_or_else(|| o.command.join(" "));
    Ok(Outcome {
        json: report::json(&name, &o.command, &counter, &samples),
        no_counter: counter.err(),
    })
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
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
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
    }
}
