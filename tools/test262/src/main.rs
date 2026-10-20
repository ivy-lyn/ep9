//! `ep9-test262`: the test262 runner.
//!
//! M0 week 1 has two commands:
//! - `index`: the test262 front-matter indexer (JSON).
//! - `parse`: the parse-only runner through `ep9_frontend`, checked against
//!   `expectations/test262-parse.<target-triple>.txt`.
//!
//! Default paths are relative to the ep9 repo root (found at build time), so `cargo run` works
//! from any directory. Paths given as flags are used as given.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod expect;
mod frontmatter;
mod index;
mod json;
mod parse;
mod pool;
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
            "--strict-by" => {
                opts.strict_by = match value(arg, &mut it)?.as_str() {
                    "prefix" => parse::StrictBy::Prefix,
                    "opts" => parse::StrictBy::Opts,
                    other => return Err(format!("bad --strict-by value {other:?}")),
                }
            }
            "--verbose" => opts.verbose = true,
            other if other.starts_with("--") => return Err(format!("unknown flag {other:?}")),
            prefix => opts
                .filters
                .push(prefix.trim_start_matches("./").to_string()),
        }
    }
    Ok(opts)
}
