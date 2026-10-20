//! The `ep9` binary: run, repl, eval, check, snapshot build, test262 host, Fuzzilli REPRL and
//! the JSC-shell API, with the flags `--jitless`, `--unit` and `--profile-out`.
//!
//! M0 (week 3): `ep9 <file.js>` and `ep9 -e <code>` run one script through the whole engine.
//!
//! ```text
//! source --ep9_frontend::compile--> VerifiedUnit --Isolate::run_script--> result
//!   compile error   -> "file:line:col: SyntaxError: ..." / "RangeError: ..."   exit 1
//!   uncaught throw  -> "Uncaught file:line:col: Name: message" + stack lines  exit 1
//!   bad flag, I/O   -> usage text                                             exit 2
//! ```
//!
//! The isolate runs on its own thread with a fixed native stack (`--stack-size`, default
//! 64 MiB), so the stack-limit RangeError behaves the same on every target (the main thread
//! of a Windows process has only 1 MiB).
#![forbid(unsafe_code)]

mod args;
mod run;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let opts = match args::parse(&argv) {
        Ok(args::Command::Run(opts)) => opts,
        Ok(args::Command::Version) => {
            println!("{}", args::VERSION_LINE);
            return ExitCode::SUCCESS;
        }
        Ok(args::Command::Help) => {
            print!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(msg) => {
            eprintln!("ep9: {msg}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    ExitCode::from(run::main(opts))
}
