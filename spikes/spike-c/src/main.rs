//! Spike C driver. `gate.sh` runs it.
//!
//! ```text
//! spike-c                                   the linked contract versions
//! spike-c kernels                           the protocol kernels and their hot functions
//! spike-c run <file.js>                     run a script in T0 and print its output
//! spike-c t0 <kernel | all> [--n N]         run a kernel in T0: output, check, wall time (report)
//! spike-c facts <kernel> [--n N] [--out F]  the live facts of each hot function after a T0 run;
//!                                           --out writes them as a PROFILED file (SP-Facts form)
//! spike-c compile <kernel>                  run the pipeline on the kernel's first hot function
//! spike-c driver <kernel> [--n N]          print the driver script (node runs the same text)
//! spike-c structural [check... | all]       the checks of `gate.sh structural`
//! ```

use std::process::ExitCode;
use std::time::Instant;

use spike_b::harness::Config;
use spike_c::harness::Session;
use spike_c::kernels::{self, Kernel};
use spike_c::{facts, structural};

fn usage() -> ExitCode {
    eprintln!(
        "usage: spike-c [kernels | run <file.js> | t0 <kernel|all> [--n N] | facts <kernel> [--n N] [--out FILE] \
         | compile <kernel> | driver <kernel> [--n N] | structural [check...|all]]"
    );
    ExitCode::from(2)
}

fn kernel_arg(name: Option<&String>) -> Result<&'static Kernel, String> {
    let name = name.ok_or("a kernel name is needed")?;
    kernels::kernel(name).ok_or_else(|| {
        let known: Vec<&str> = kernels::KERNELS.iter().map(|k| k.name).collect();
        format!("unknown kernel {name} (known: {})", known.join(", "))
    })
}

/// `--n N` and `--out F` from `args`.
fn options(args: &[String]) -> Result<(Option<u32>, Option<String>), String> {
    let (mut n, mut out) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--n" => {
                let v = it.next().ok_or("--n needs a value")?;
                n = Some(v.parse().map_err(|_| format!("bad --n {v}"))?);
            }
            "--out" => out = Some(it.next().ok_or("--out needs a file")?.clone()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok((n, out))
}

fn list() -> Result<bool, String> {
    println!("v8-v7 files: {}", kernels::bench_dir().display());
    for k in &kernels::KERNELS {
        println!("{} ({}, {:?}, n = {})", k.name, k.file, k.role, k.n);
        for h in k.hot {
            println!(
                "  {:<32} lines {}-{}  node --jitless self time {:.1}% (est.)  {}",
                h.name,
                h.lines.0,
                h.lines.1,
                h.jitless_share,
                h.reach
                    .map_or("no JS path (inner closure)".to_string(), |r| format!(
                        "value: {r}"
                    ))
            );
        }
    }
    Ok(true)
}

/// Runs the driver of `k` in T0 on a new session: the session, the driver and the result.
fn run_t0(
    k: &Kernel,
    n: u32,
) -> Result<(Session, kernels::Driver, spike_b::harness::RunOutput, f64), String> {
    let d = kernels::driver(k, n)?;
    let mut s = Session::new(&Config::default())?;
    let t = Instant::now();
    let r = s.run(&d.text, "driver.js");
    let secs = t.elapsed().as_secs_f64();
    Ok((s, d, r, secs))
}

fn t0(args: &[String]) -> Result<bool, String> {
    let (n, _) = options(&args[1.min(args.len())..])?;
    let which: Vec<&Kernel> = match args.first().map(String::as_str) {
        Some("all") => kernels::KERNELS.iter().collect(),
        _ => vec![kernel_arg(args.first())?],
    };
    let mut ok = true;
    for k in which {
        let n = n.unwrap_or(k.n);
        let (_, d, r, secs) = run_t0(k, n)?;
        for l in &r.output {
            println!("{}: {l}", k.name);
        }
        let right = r.result.is_ok() && d.matches(&r.output);
        if let Err(e) = &r.result {
            println!("{}: uncaught {e}", k.name);
        }
        println!(
            "{}: T0, n = {n}, {:.3} s (one run; a report, not a class S reading): {}",
            k.name,
            secs,
            if right { "PASS" } else { "FAIL" }
        );
        ok &= right;
    }
    Ok(ok)
}

fn facts_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let (n, out) = options(&args[1..])?;
    let (s, d, r, _) = run_t0(k, n.unwrap_or(1))?;
    r.result.map_err(|e| format!("{}: {e}", k.name))?;
    let script = s.script("driver.js").ok_or("no driver script")?;
    let live = facts::live(&s, script)?;
    println!("{}: {} live records in the unit", k.name, live.len());
    for h in k.hot {
        let ids = script.functions_on_line(h.lines.0 + d.offset);
        let [id] = ids[..] else {
            return Err(format!(
                "{}: {} functions on line {}",
                h.name,
                ids.len(),
                h.lines.0
            ));
        };
        let f = facts::for_function(&live, id);
        let mut kinds: Vec<String> = f.records.iter().map(|(_, r)| facts::kind_name(r)).collect();
        kinds.sort();
        kinds.dedup_by(|a, b| a == b);
        println!(
            "  {:<32} function {id}: {} records ({})",
            h.name,
            f.records.len(),
            kinds.join(", ")
        );
    }
    if let Some(path) = out {
        let bytes = facts::to_profiled(&live)?.encode();
        std::fs::write(&path, &bytes).map_err(|e| format!("{path}: {e}"))?;
        println!("wrote {path}: {} bytes, source PROFILED", bytes.len());
    }
    Ok(true)
}

fn run_cmd(args: &[String]) -> Result<bool, String> {
    let file = args.first().ok_or("run needs <file.js>")?;
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let mut s = Session::new(&Config::default())?;
    let r = s.run(&src, file);
    print!("{}", r.text());
    Ok(r.result.is_ok())
}

fn driver_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let (n, _) = options(&args[1..])?;
    let d = kernels::driver(k, n.unwrap_or(k.n))?;
    print!("{}", d.text);
    Ok(true)
}

fn compile_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let h = k.hot.first().ok_or("no hot function")?;
    let (s, d, r, _) = run_t0(k, 1)?;
    r.result.map_err(|e| format!("{}: {e}", k.name))?;
    let script = s.script("driver.js").ok_or("no driver script")?;
    let live = facts::live(&s, script)?;
    let ids = script.functions_on_line(h.lines.0 + d.offset);
    let [id] = ids[..] else {
        return Err(format!(
            "{}: {} functions on line {}",
            h.name,
            ids.len(),
            h.lines.0
        ));
    };
    let Some(reach) = h.reach else {
        return Err(format!(
            "{}: an inner closure; no lookup by function id yet",
            h.name
        ));
    };
    if reach.contains('.') {
        return Err(format!(
            "{}: only a global function name is compiled for now",
            h.name
        ));
    }
    match spike_c::compile(&s, reach, &facts::for_function(&live, id)) {
        Ok(code) => {
            println!("{}: {} bytes", h.name, code.code.len());
            Ok(true)
        }
        Err(e) => {
            println!("{e}");
            Ok(false)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        println!("spike-c: {}", spike_c::contracts_line());
        return ExitCode::SUCCESS;
    };
    let rest = &args[1..];
    let r = match cmd.as_str() {
        "structural" => return structural::main(rest),
        "kernels" => list(),
        "t0" => t0(rest),
        "facts" => facts_cmd(rest),
        "compile" => compile_cmd(rest),
        "driver" => driver_cmd(rest),
        "run" => run_cmd(rest),
        _ => return usage(),
    };
    match r {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("spike-c: {e}");
            ExitCode::from(2)
        }
    }
}
