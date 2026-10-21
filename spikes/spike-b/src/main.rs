//! Spike B driver. `gate.sh` runs it; P3 (`bench`) and P4 (`unit`) get their subcommands
//! through the dispatch below, in their own files.
//!
//! ```text
//! spike-b                                   the linked contract versions
//! spike-b run [--jit f,g] [--gc-stress] [--eager] <defs.js> [<main.js>]
//!                                           run defs, JIT f and g, run main; print the output
//! spike-b ops <file.js> <name>...           the opcode list each named function JITs with
//! spike-b structural <a..h | all>           the structural gate checks (gate.sh structural)
//! spike-b bench <args>                      src/bench.rs (P3-SPIKEB-BENCH)
//! spike-b unit <args>                       src/unit/mod.rs (P4-SP-UNIT)
//! ```

use std::process::ExitCode;

use spike_b::harness::{Config, Session};

fn usage() -> ExitCode {
    eprintln!(
        "usage: spike-b [run [--jit f,g] [--gc-stress] [--eager] <defs.js> [<main.js>] \
         | ops <file.js> <name>... | structural <a..h|all> | bench <args> | unit <args>]"
    );
    ExitCode::from(2)
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

/// `spike-b run`.
fn run(args: &[String]) -> Result<bool, String> {
    let mut config = Config::default();
    let mut jit: Vec<String> = Vec::new();
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--jit" => {
                let v = it.next().ok_or("--jit needs a list")?;
                jit.extend(v.split(',').filter(|s| !s.is_empty()).map(String::from));
            }
            "--gc-stress" => {
                config = Config {
                    feedback_threshold: config.feedback_threshold,
                    ..Config::stress()
                }
            }
            "--eager" => config.feedback_threshold = 0,
            f => files.push(f.to_string()),
        }
    }
    let (defs, main) = match files.as_slice() {
        [d] => (read(d)?, String::new()),
        [d, m] => (read(d)?, read(m)?),
        _ => return Err("run needs <defs.js> [<main.js>]".into()),
    };
    let mut s = Session::new(&config)?;
    let d = s.run(&defs, "defs.js");
    print!("{}", d.text());
    if d.result.is_err() {
        return Ok(false);
    }
    if !jit.is_empty() {
        let names: Vec<&str> = jit.iter().map(String::as_str).collect();
        for p in s.jit(&names)? {
            eprintln!(
                "spike-b: JIT {} ({} bytes): {}",
                p.name,
                p.code.len(),
                p.ops_line
            );
        }
    }
    if main.is_empty() {
        return Ok(true);
    }
    let r = s.run(&main, "main.js");
    print!("{}", r.text());
    Ok(r.result.is_ok())
}

/// `spike-b ops`: the opcode list of each named function, or why it stays in T0.
fn ops(args: &[String]) -> Result<bool, String> {
    let [file, names @ ..] = args else {
        return Err("ops needs <file.js> <name>...".into());
    };
    let mut s = Session::new(&Config::default())?;
    s.run(&read(file)?, file).result?;
    let mut all = true;
    for n in names {
        match s.compile_function(n) {
            Ok(code) => println!(
                "{n}: JIT {} ops, {} bytes: {}",
                code.ops.len(),
                code.code.len(),
                code.ops_line()
            ),
            Err(e) => {
                all = false;
                println!("{n}: T0 ({e})");
            }
        }
    }
    Ok(all)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        println!("spike-b: {}", spike_b::contracts_line());
        return ExitCode::SUCCESS;
    };
    let rest = &args[1..];
    let r = match cmd.as_str() {
        "bench" => return ExitCode::from(spike_b::bench::main(rest).clamp(0, 255) as u8),
        "unit" => return ExitCode::from(spike_b::unit::main(rest).clamp(0, 255) as u8),
        "structural" => return spike_b::structural::main(rest),
        "run" => run(rest),
        "ops" => ops(rest),
        _ => return usage(),
    };
    match r {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("spike-b: {e}");
            ExitCode::from(2)
        }
    }
}
