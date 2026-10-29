//! `ep9-test262`: the test262 runner.
//!
//! Commands:
//! - `index`: the test262 front-matter indexer (JSON).
//! - `parse`: the parse-only runner through `ep9_frontend`, checked against
//!   `expectations/test262-parse.<target-triple>.txt`.
//! - `exec`: the execution runner (engine end to end, worker processes), checked against
//!   `expectations/test262.<target-triple>.jit.txt` (decision NAMING).
//! - `exec-worker`, `parse-worker`: internal; one worker process of `exec` or `parse`.
//!
//! Default paths are relative to the ep9 repo root (found at build time), so `cargo run` works
//! from any directory. Paths given as flags are used as given.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod exec;
mod expect;
mod frontmatter;
mod index;
mod json;
mod parse;
mod pool;
mod provenance;
mod suite;

/// The target triple this binary was built for (from build.rs).
const TRIPLE: &str = env!("EP9_TARGET_TRIPLE");

const USAGE: &str = "\
usage: ep9-test262 <command> [flags]

commands:
  index   Write the front matter of every test under test/ as JSON.
          --test262 <dir>   test262 checkout (default: <repo>/third_party/test262)
          --out <file>      write here instead of stdout
          --threads <n>     worker threads (default: all CPUs)
  parse   Parse every test under test/{language,built-ins,annexB,intl402} with ep9_frontend.
          Exit 0 if the failures equal the expectation file, 1 if not, 2 on errors.
          --test262 <dir>   test262 checkout (default: <repo>/third_party/test262)
          --expect <file>   expectation file (default: <repo>/expectations/test262-parse.<triple>.txt)
          --update-expectations   write the current failures to the expectation file
          --json <file>     also write a JSON summary
          --threads <n>     worker threads (default: all CPUs)
          --strict-by <prefix|opts>  strict runs: source prefix (default) or SourceOpts
          --verbose         print every failure
          [<path prefix>...] only tests whose path starts with one of these (e.g. test/annexB)
  exec    Run every test under test/ through the engine (fresh isolate per run, harness
          prelude, flags, negative phases) in worker processes.
          Exit 0 if the failures equal the expectation file, 1 if not, 2 on errors (an
          infrastructure error, panic, timeout or worker crash, is always exit 2).
          --test262 <dir>   test262 checkout (default: <repo>/third_party/test262)
          --expect <file>   expectation file (default: <repo>/expectations/test262.<triple>.jit.txt)
          --update-expectations   write the current failures to the expectation file
          --json <file>     also write a JSON summary with every failure
          --threads <n>     worker processes (default: all CPUs)
          --timeout <s>     per-run soft timeout in seconds (default 20)
          --gc-stress <n>   collect before every n-th allocation and verify the heap (default 0)
          --skip-list <file>  with --gc-stress only: neither run nor count the listed tests
                            (one path per line, as in an expectation file); prints the count
          --feedback-threshold <n>  calls before a function gets a feedback vector (default 8;
                            0 = at the first call, 4294967295 = never)
          --feedback <mode>  type-feedback mode of every isolate: full or ics_only (default:
                            the default of this build's preset; full for the jit preset)
          --causes <n>      failure causes in the report (default 20)
          --progress        print progress to stderr
          --verbose         print every failure
          [<path prefix>...] only tests whose path starts with one of these
";

fn repo_root() -> PathBuf {
    // tools/test262 -> repo root
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::canonicalize(&root).unwrap_or(root)
}

fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("ep9-test262: {msg}\n\n{USAGE}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        return usage_error("missing command");
    };
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "index" => match parse_index_args(&rest) {
            Ok(opts) => index::run(&opts),
            Err(e) => usage_error(&e),
        },
        "parse" => match parse_parse_args(&rest) {
            Ok(opts) => parse::run(&opts),
            Err(e) => usage_error(&e),
        },
        "exec" => match parse_exec_args(&rest) {
            Ok(opts) => exec::run(&opts),
            Err(e) => usage_error(&e),
        },
        "exec-worker" => match parse_worker_args(&rest) {
            Ok(opts) => exec::worker(opts),
            Err(e) => usage_error(&e),
        },
        "parse-worker" => match parse_parse_worker_args(&rest) {
            Ok(opts) => parse::worker(opts),
            Err(e) => usage_error(&e),
        },
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => usage_error(&format!("unknown command {other:?}")),
    }
}

/// Take the value of a `--flag <value>` pair.
fn value<'a>(flag: &str, it: &mut impl Iterator<Item = &'a String>) -> Result<&'a String, String> {
    it.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn threads(v: &str) -> Result<usize, String> {
    v.parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("bad --threads value {v:?}"))
}

fn parse_index_args(args: &[String]) -> Result<index::Options, String> {
    let mut opts = index::Options {
        test262: repo_root().join("third_party/test262"),
        out: None,
        threads: default_threads(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--test262" => opts.test262 = PathBuf::from(value(arg, &mut it)?),
            "--out" => opts.out = Some(PathBuf::from(value(arg, &mut it)?)),
            "--threads" => opts.threads = threads(value(arg, &mut it)?)?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(opts)
}

fn parse_parse_args(args: &[String]) -> Result<parse::Options, String> {
    let mut opts = parse::Options {
        test262: repo_root().join("third_party/test262"),
        expect: repo_root().join(format!("expectations/test262-parse.{TRIPLE}.txt")),
        update: false,
        json: None,
        threads: default_threads(),
        strict_by: parse::StrictBy::Prefix,
        verbose: false,
        filters: Vec::new(),
        triple: TRIPLE,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--test262" => opts.test262 = PathBuf::from(value(arg, &mut it)?),
            "--expect" => opts.expect = PathBuf::from(value(arg, &mut it)?),
            "--update-expectations" => opts.update = true,
            "--json" => opts.json = Some(PathBuf::from(value(arg, &mut it)?)),
            "--threads" => opts.threads = threads(value(arg, &mut it)?)?,
            "--strict-by" => opts.strict_by = strict_by(value(arg, &mut it)?)?,
            "--verbose" => opts.verbose = true,
            other if other.starts_with("--") => return Err(format!("unknown flag {other:?}")),
            prefix => opts
                .filters
                .push(prefix.trim_start_matches("./").to_string()),
        }
    }
    Ok(opts)
}

fn strict_by(v: &str) -> Result<parse::StrictBy, String> {
    parse::StrictBy::from_name(v).ok_or_else(|| format!("bad --strict-by value {v:?}"))
}

/// The flags of `parse-worker` (the parent passes `--test262` and `--strict-by`).
fn parse_parse_worker_args(args: &[String]) -> Result<parse::WorkerOptions, String> {
    let mut opts = parse::WorkerOptions {
        test262: repo_root().join("third_party/test262"),
        strict_by: parse::StrictBy::Prefix,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--test262" => opts.test262 = PathBuf::from(value(arg, &mut it)?),
            "--strict-by" => opts.strict_by = strict_by(value(arg, &mut it)?)?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(opts)
}

fn seconds(v: &str) -> Result<std::time::Duration, String> {
    v.parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s > 0.0 && *s <= 3600.0)
        .map(std::time::Duration::from_secs_f64)
        .ok_or_else(|| format!("bad --timeout value {v:?}"))
}

fn count(flag: &str, v: &str) -> Result<u32, String> {
    v.parse::<u32>()
        .map_err(|_| format!("bad {flag} value {v:?}"))
}

/// A `--feedback` value: `full` or `ics_only` (ep9_vm `Feedback`, C-23).
fn feedback(v: &str) -> Result<ep9_vm::Feedback, String> {
    ep9_vm::Feedback::from_name(v)
        .ok_or_else(|| format!("bad --feedback value {v:?} (full or ics_only)"))
}

fn parse_exec_args(args: &[String]) -> Result<exec::Options, String> {
    let mut opts = exec::Options {
        test262: repo_root().join("third_party/test262"),
        expect: repo_root().join(format!("expectations/test262.{TRIPLE}.jit.txt")),
        update: false,
        json: None,
        threads: default_threads(),
        timeout: std::time::Duration::from_secs(20),
        gc_stress: 0,
        skip_list: None,
        feedback_threshold: ep9_vm::IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD,
        feedback: ep9_vm::ExecOptions::default().feedback_mode(),
        verbose: false,
        progress: false,
        causes: 20,
        filters: Vec::new(),
        triple: TRIPLE,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--test262" => opts.test262 = PathBuf::from(value(arg, &mut it)?),
            "--expect" => opts.expect = PathBuf::from(value(arg, &mut it)?),
            "--update-expectations" => opts.update = true,
            "--json" => opts.json = Some(PathBuf::from(value(arg, &mut it)?)),
            "--threads" => opts.threads = threads(value(arg, &mut it)?)?,
            "--timeout" => opts.timeout = seconds(value(arg, &mut it)?)?,
            "--gc-stress" => opts.gc_stress = count(arg, value(arg, &mut it)?)?,
            "--skip-list" => opts.skip_list = Some(PathBuf::from(value(arg, &mut it)?)),
            "--feedback-threshold" => opts.feedback_threshold = count(arg, value(arg, &mut it)?)?,
            "--feedback" => opts.feedback = feedback(value(arg, &mut it)?)?,
            "--causes" => opts.causes = count(arg, value(arg, &mut it)?)? as usize,
            "--verbose" => opts.verbose = true,
            "--progress" => opts.progress = true,
            other if other.starts_with("--") => return Err(format!("unknown flag {other:?}")),
            prefix => opts
                .filters
                .push(prefix.trim_start_matches("./").to_string()),
        }
    }
    if opts.skip_list.is_some() {
        // The list names the tests that only GC-stress makes too slow; a normal run skips
        // nothing.
        if opts.gc_stress == 0 {
            return Err("--skip-list needs --gc-stress".into());
        }
        if opts.update {
            return Err("--skip-list and --update-expectations do not go together".into());
        }
    }
    Ok(opts)
}

fn parse_worker_args(args: &[String]) -> Result<exec::WorkerOptions, String> {
    let mut opts = exec::WorkerOptions {
        test262: repo_root().join("third_party/test262"),
        timeout: std::time::Duration::from_secs(20),
        gc_stress: 0,
        feedback_threshold: ep9_vm::IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD,
        feedback: ep9_vm::ExecOptions::default().feedback_mode(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--test262" => opts.test262 = PathBuf::from(value(arg, &mut it)?),
            "--timeout" => {
                let ms = value(arg, &mut it)?;
                let ms = ms
                    .parse::<u64>()
                    .ok()
                    .filter(|m| *m > 0)
                    .ok_or_else(|| format!("bad --timeout value {ms:?} (milliseconds)"))?;
                opts.timeout = std::time::Duration::from_millis(ms);
            }
            "--gc-stress" => opts.gc_stress = count(arg, value(arg, &mut it)?)?,
            "--feedback-threshold" => opts.feedback_threshold = count(arg, value(arg, &mut it)?)?,
            "--feedback" => opts.feedback = feedback(value(arg, &mut it)?)?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn skip_list_only_with_gc_stress() {
        let exec = |a: &[&str]| parse_exec_args(&strings(a));
        assert!(exec(&["--skip-list", "skip.txt"]).is_err());
        assert!(exec(&["--gc-stress", "0", "--skip-list", "skip.txt"]).is_err());
        assert!(
            exec(&[
                "--gc-stress",
                "1",
                "--skip-list",
                "skip.txt",
                "--update-expectations"
            ])
            .is_err()
        );
        let o = exec(&["--gc-stress", "1", "--skip-list", "skip.txt"]).unwrap();
        assert_eq!(o.skip_list, Some(PathBuf::from("skip.txt")));
        assert_eq!(exec(&[]).unwrap().skip_list, None);
    }

    #[test]
    fn feedback_threshold_reaches_exec_and_worker() {
        let default = ep9_vm::IsolateConfig::default().feedback_threshold;
        assert_eq!(parse_exec_args(&[]).unwrap().feedback_threshold, default);
        let o = parse_exec_args(&strings(&["--feedback-threshold", "0"])).unwrap();
        assert_eq!(o.feedback_threshold, 0);
        assert!(parse_exec_args(&strings(&["--feedback-threshold", "-1"])).is_err());
        let w = parse_worker_args(&strings(&["--feedback-threshold", "4294967295"])).unwrap();
        assert_eq!(w.feedback_threshold, u32::MAX);
        assert_eq!(parse_worker_args(&[]).unwrap().feedback_threshold, default);
    }

    /// `--feedback` (C-23): full or ics_only, for the parent and its workers; the default is
    /// the default of this build's preset (with the runner's default `jit` feature: `full`).
    #[test]
    fn feedback_reaches_exec_and_worker() {
        use ep9_vm::Feedback;
        let default = ep9_vm::ExecOptions::default().feedback_mode();
        #[cfg(feature = "jit")]
        assert_eq!(default, Feedback::FULL);
        assert_eq!(parse_exec_args(&[]).unwrap().feedback, default);
        assert_eq!(parse_worker_args(&[]).unwrap().feedback, default);
        let o = parse_exec_args(&strings(&["--feedback", "ics_only"])).unwrap();
        assert_eq!(o.feedback, Feedback::ICS_ONLY);
        let w = parse_worker_args(&strings(&["--feedback", "full"])).unwrap();
        assert_eq!(w.feedback, Feedback::FULL);
        assert!(parse_exec_args(&strings(&["--feedback", "jit"])).is_err());
        assert!(parse_worker_args(&strings(&["--feedback"])).is_err());
    }

    #[test]
    fn strict_by_reaches_parse_and_its_worker() {
        let p = parse_parse_args(&strings(&["--strict-by", "opts"])).unwrap();
        assert_eq!(p.strict_by, parse::StrictBy::Opts);
        assert!(parse_parse_args(&strings(&["--strict-by", "x"])).is_err());
        let w =
            parse_parse_worker_args(&strings(&["--strict-by", "opts", "--test262", "/t"])).unwrap();
        assert_eq!(w.strict_by, parse::StrictBy::Opts);
        assert_eq!(w.test262, PathBuf::from("/t"));
        assert_eq!(
            parse_parse_worker_args(&[]).unwrap().strict_by,
            parse::StrictBy::Prefix
        );
        assert!(parse_parse_worker_args(&strings(&["--threads", "2"])).is_err());
    }
}
