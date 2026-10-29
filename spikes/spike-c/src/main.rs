//! Spike C driver. `gate.sh` runs it.
//!
//! ```text
//! spike-c                                   the linked contract versions
//! spike-c kernels                           the protocol kernels and their hot functions
//! spike-c run <file.js>                     run a script in T0 and print its output
//! spike-c t0 <kernel | all> [--n N]         run a kernel in T0: output, check, wall time (report)
//! spike-c facts <kernel> [--n N] [--out F]  the live facts of each hot function after a T0 run;
//!                                           --out writes them as a PROFILED file (SP-Facts form)
//! spike-c compile <kernel> [--graph]        compile the kernel's first hot function (live facts, n = 1)
//! spike-c graph <kernel> [hot index] [--graph]
//!                                           build, check and allocate the graph of a hot function
//!                                           (live facts, n = 1; by function id, so an inner
//!                                           closure too), then report what the emitter does
//! spike-c tier <kernel> [--n N] [--stress]  config C: that function compiled, output against T0
//!                                           (--stress: GC before every allocation, verifier on;
//!                                           SPIKE_C_FORCE_DEOPT=<seed>: a forced build)
//! spike-c deopt <kernel> [--n N] [--at A]   each guard forced once (WI-SC4, gate.sh deopt)
//! spike-c time <kernel> [--rounds R] [--n N] [--k K]
//!                                           class S timing (protocol §3): T0, C, H0, H1 in new
//!                                           processes, paired ratios; a report, never a reading
//! spike-c once <t0|c|h0|h1> <kernel> [--n N] [--k K]   one timed configuration (`time` runs it)
//! spike-c show <kernel> [hot index]         bytecode and live facts of a hot function
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
         | compile <kernel> [--graph] | graph <kernel> [i] [--graph] | tier <kernel> [--n N] [--stress] \
         | deopt <kernel> [--n N] [--at A] | time <kernel> [--rounds R] [--n N] [--k K] \
         | once <t0|c|h0|h1> <kernel> [--n N] [--k K] | show <kernel> [i] | driver <kernel> [--n N] \
         | structural [check...|all]]"
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
    let graph = args.iter().any(|a| a == "--graph");
    let h = k.hot.first().ok_or("no hot function")?;
    let (mut s, d, r, _) = run_t0(k, 1)?;
    r.result.map_err(|e| format!("{}: {e}", k.name))?;
    // An inner closure is found by its function id (WI-SC5).
    let f = match h.reach {
        Some(reach) => Some(s.function_value(reach)?),
        None => None,
    };
    let script = s.script("driver.js").ok_or("no driver script")?;
    let ids = script.functions_on_line(h.lines.0 + d.offset);
    let [id] = ids[..] else {
        return Err(format!(
            "{}: {} functions on line {}",
            h.name,
            ids.len(),
            h.lines.0
        ));
    };
    let live = facts::live(&s, script)?;
    let target = match f {
        Some(f) => spike_c::harness::Target::Value(f),
        None => spike_c::harness::Target::Id {
            file: "driver.js".into(),
            id,
        },
    };
    let input = s.input_of_target(&target)?;
    let ff = facts::for_function(&live, id);
    match spike_c::compile_input(&input, &ff) {
        Ok((code, report)) => {
            if graph {
                print!("{}", report.graph);
            }
            println!("{}", report.line());
            println!("{}", report.keyed_line());
            println!("{}: {}", report.name, report.emit.exits_line());
            // The movz form of the stubs, a report next to the default call form (MC-w15-14,
            // V10-D31).
            let (_, movz) =
                spike_c::compile_input_with(&input, &ff, spike_c::emit_a64::StubForm::Movz)
                    .map_err(|e| format!("{}: movz stubs: {e}", h.name))?;
            println!("{}: {}", report.name, movz.emit.exits_line());
            // The VM's reader takes the table the install registers (H29).
            let records = ep9_vm::NativeCx::check_deopt_meta(&report.emit.meta.encode())
                .map_err(|e| format!("{}: exit table: {e}", h.name))?;
            println!(
                "{}: exit table read by ep9_vm: {records} records",
                report.name
            );
            println!("{}: {}", report.name, report.emit.meta.line());
            let meta = report.emit.meta.encode();
            println!(
                "{}: code fnv1a64 {:016x}, exit metadata fnv1a64 {:016x} (no address in either)",
                report.name,
                fnv1a64(&code.code),
                fnv1a64(&meta)
            );
            Ok(true)
        }
        Err(e) => {
            println!("{}: {e}", h.name);
            Ok(false)
        }
    }
}

/// `spike-c graph <kernel> [hot index] [--graph]`: the graph build of a hot function from the
/// live facts of an n = 1 T0 run, found by its function id (SPIKEC-4: kernel 2's `lin_solve`
/// is an inner closure, so no JS value reaches it). Exit 0 if the graph builds, checks and
/// allocates; the emitter's result is a report line.
fn graph_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let show = args.iter().any(|a| a == "--graph");
    let which: usize = match args.get(1).filter(|a| !a.starts_with("--")) {
        Some(v) => v.parse().map_err(|_| format!("bad hot index {v}"))?,
        None => 0,
    };
    let h = k.hot.get(which).ok_or("no such hot function")?;
    let (s, d, r, _) = run_t0(k, 1)?;
    r.result.map_err(|e| format!("{}: {e}", k.name))?;
    let script = s.script("driver.js").ok_or("no driver script")?;
    let ids = script.functions_on_line(h.lines.0 + d.offset);
    let [id] = ids[..] else {
        return Err(format!(
            "{}: {} functions on line {}",
            h.name,
            ids.len(),
            h.lines.0
        ));
    };
    let live = facts::live(&s, script)?;
    let input = script.unit_input(id, h.name)?;
    match spike_c::graph_report(&input, &facts::for_function(&live, id)) {
        Ok(g) => {
            if show {
                print!("{}", g.graph);
            }
            for l in g.lines() {
                println!("{l}");
            }
            Ok(true)
        }
        Err(e) => {
            println!("{}: graph build refused: {e}", h.name);
            Ok(false)
        }
    }
}

/// FNV-1a, 64 bits (the report's code and metadata digests).
fn fnv1a64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| {
        (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// `spike-c tier <kernel> [--n N]`: config C (the first hot function compiled from the live
/// facts of a warm-up run) against T0, output compared line for line.
fn tier_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let stress = args.iter().any(|a| a == "--stress");
    let rest: Vec<String> = args[1..]
        .iter()
        .filter(|a| *a != "--stress")
        .cloned()
        .collect();
    let (n, _) = options(&rest)?;
    let config = if stress {
        Config::stress()
    } else {
        Config::default()
    };
    // `SPIKE_C_FORCE_DEOPT=<seed>` (WI-SC4): a forced build, every guard armed from the seed.
    let force = match std::env::var("SPIKE_C_FORCE_DEOPT") {
        Ok(v) => {
            let seed: u64 = v
                .parse()
                .map_err(|_| format!("SPIKE_C_FORCE_DEOPT={v}: not a number"))?;
            Some(spike_c::emit_a64::ForceTable::seeded(
                seed,
                spike_c::tier::FORCE_LEN,
            ))
        }
        Err(_) => None,
    };
    let opts = spike_c::emit_a64::EmitOptions {
        force: force.as_ref(),
        ..Default::default()
    };
    let run = spike_c::tier::run_full(
        k,
        n.unwrap_or(k.n),
        &config,
        spike_c::tier::IntFacts::Live,
        &opts,
    )?;
    println!("{}", run.report.line());
    println!("{}", run.line());
    let mut ok = run.equal && (run.probe_exit.is_some() || run.function != "am3");
    if let Some(t) = &force {
        // Per guard: the VM counted at least the forced exit (a natural exit adds to it,
        // as GC stress does), and with no natural exit exactly that.
        let mut guards: Vec<u32> = run.report.emit.meta.exits.iter().map(|e| e.guard).collect();
        guards.sort_unstable();
        guards.dedup();
        let mut forced = 0;
        for &g in &guards {
            let f = u64::from(t.get(g) == 0);
            let c: u64 = run
                .report
                .emit
                .meta
                .exits
                .iter()
                .zip(&run.exits)
                .filter(|(r, _)| r.guard == g)
                .map(|(_, c)| c)
                .sum();
            forced += f;
            if c < f || (!stress && c != f) {
                println!("{}: g{g}: forced {f}, counted {c}", run.function);
                ok = false;
            }
        }
        println!(
            "{}: forced build: {forced} of {} guards forced once, the exit counters agree, heap verifier clean",
            run.function,
            guards.len()
        );
    }
    Ok(ok)
}

/// `spike-c deopt <kernel> [--n N] [--at A]` (`gate.sh deopt`, WI-SC4): each guard of the
/// first hot function forced once, alone, at its `A`-th pass (default 3), in a config C run
/// of loop(n) (default n = 1): the output equals T0, the heap verifier finds nothing, and the
/// VM's exit counters hold exactly that exit.
fn deopt_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let n = flag(args, "--n")?.unwrap_or(1u32);
    let at = flag(args, "--at")?.unwrap_or(3u32);
    let runs = spike_c::tier::force_each_guard(k, n, &Config::default(), at)?;
    let mut right = 0;
    for r in &runs {
        if r.right() {
            right += 1;
        } else {
            println!(
                "{}: g{}: forced {}, counted {} (other exits {}), output {}",
                k.name,
                r.guard,
                r.forced,
                r.counted,
                r.others,
                if r.equal { "equal to T0" } else { "differs" }
            );
        }
    }
    println!(
        "{}: {} guards, each forced once at pass {at} (n = {n}): {right} right (output equal to T0, one exit counted, heap verifier clean)",
        k.name,
        runs.len()
    );
    Ok(!runs.is_empty() && right == runs.len())
}

/// The value after `flag` in `args`, parsed.
fn flag<T: std::str::FromStr>(args: &[String], flag: &str) -> Result<Option<T>, String> {
    match args.iter().position(|a| a == flag) {
        None => Ok(None),
        Some(i) => {
            let v = args
                .get(i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            v.parse()
                .map(Some)
                .map_err(|_| format!("bad {flag} value {v}"))
        }
    }
}

/// `spike-c time <kernel> [--rounds R] [--n N] [--k K]`: the class S report (exit 1 on a wrong
/// run).
fn time_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let rounds = flag(args, "--rounds")?.unwrap_or(15usize);
    let n = flag(args, "--n")?.unwrap_or(k.n);
    let hot_k = flag(args, "--k")?.unwrap_or(spike_c::timing::default_k(k));
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let (report, ok) = spike_c::timing::time(&exe, k, rounds, n, hot_k)?;
    print!("{report}");
    Ok(ok)
}

/// `spike-c once <t0|c|h0|h1> <kernel> [--n N] [--k K]`: one timed configuration, in the
/// line form `timing::Once::parse` reads.
fn once_cmd(args: &[String]) -> Result<bool, String> {
    let which = args
        .first()
        .and_then(|w| spike_c::timing::Which::parse(w))
        .ok_or("once: t0, c, h0 or h1")?;
    let k = kernel_arg(args.get(1))?;
    let n = flag(&args[1..], "--n")?.unwrap_or(k.n);
    let hot_k = flag(&args[1..], "--k")?.unwrap_or(spike_c::timing::default_k(k));
    let o = spike_c::timing::once(which, k, n, hot_k)?;
    print!("{}", o.text());
    Ok(true)
}

/// `spike-c show <kernel> [hot index]`: the bytecode and the live facts (n = 1) of a hot
/// function (a report for the graph builder's input).
fn show_cmd(args: &[String]) -> Result<bool, String> {
    let k = kernel_arg(args.first())?;
    let which: usize = match args.get(1) {
        Some(v) => v.parse().map_err(|_| format!("bad hot index {v}"))?,
        None => 0,
    };
    let h = k.hot.get(which).ok_or("no such hot function")?;
    let (s, d, r, _) = run_t0(k, 1)?;
    r.result.map_err(|e| format!("{}: {e}", k.name))?;
    let script = s.script("driver.js").ok_or("no driver script")?;
    let ids = script.functions_on_line(h.lines.0 + d.offset);
    let [id] = ids[..] else {
        return Err(format!(
            "{}: {} functions on line {}",
            h.name,
            ids.len(),
            h.lines.0
        ));
    };
    let unit = script.unit.unit();
    print!(
        "{}",
        ep9_bytecode::disassemble_function(unit, ep9_bytecode::FunctionId(id))
    );
    let live = facts::live(&s, script)?;
    for (slot, f) in facts::for_function(&live, id).records {
        println!("slot {slot}: {f:?}");
    }
    Ok(true)
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
        "graph" => graph_cmd(rest),
        "tier" => tier_cmd(rest),
        "deopt" => deopt_cmd(rest),
        "time" => time_cmd(rest),
        "once" => once_cmd(rest),
        "driver" => driver_cmd(rest),
        "show" => show_cmd(rest),
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
