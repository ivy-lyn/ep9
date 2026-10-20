//! sp-dispatch: run, check and benchmark the three expansions and the C baseline.
//!
//! ```text
//! sp-dispatch run <variant> <file.ep9d>     one timed run; prints one result line
//! sp-dispatch dump <kernel> <n|default> <out.ep9d>
//! sp-dispatch ops                           opcode table
//! sp-dispatch check [--cgoto EXE] [--become EXE]
//! sp-dispatch deep-loop                     threaded variants on a 64 KiB native stack
//! sp-dispatch bench [--rounds N] [--scale F] [--cgoto EXE] [--become EXE] [--gate]
//! ```
//!
//! Result line (cgoto.c prints the same format):
//! `result=<i64> interrupts=<u64> slow=<u64> error=<u32> ns=<u64>`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use sp_dispatch::OPS;
use sp_dispatch::kernels::{self, KERNELS};
use sp_dispatch::program::{Program, Verified};
use sp_dispatch::vm::{self, Outcome, Prepared, Variant};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("run") => cmd_run(&args[1..]),
        Some("dump") => cmd_dump(&args[1..]),
        Some("ops") => cmd_ops(),
        Some("check") => cmd_check(&args[1..]),
        Some("deep-loop") => cmd_deep_loop(),
        Some("bench") => cmd_bench(&args[1..]),
        _ => Err("usage: sp-dispatch run|dump|ops|check|deep-loop|bench (see src/main.rs)".into()),
    };
    match r {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("sp-dispatch: {e}");
            ExitCode::from(2)
        }
    }
}

type R = Result<bool, String>;

/// (result, interrupts, slow paths, error): what every variant must agree on.
type Key = (i64, u64, u64, u32);

/// A check case: name, program, and the reference (result, interrupts, slow paths) if known.
type Case = (String, Program, Option<(i64, u64, u64)>);

fn load(path: &Path) -> Result<Verified, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Program::from_bytes(&bytes)?.verify()
}

fn line(o: &Outcome, ns: u128) -> String {
    format!(
        "result={} interrupts={} slow={} error={} ns={ns}",
        o.result, o.interrupts, o.slow_paths, o.error
    )
}

fn cmd_run(a: &[String]) -> R {
    let [v, file] = a else {
        return Err("run <variant> <file.ep9d>".into());
    };
    let variant = Variant::parse(v).ok_or_else(|| format!("unknown or disabled variant {v}"))?;
    let prog = load(Path::new(file))?;
    let (o, ns) = Prepared::new(&prog).exec_timed(variant);
    println!("{}", line(&o, ns));
    Ok(true)
}

fn kernel_n(k: &kernels::Kernel, s: &str) -> Result<i64, String> {
    if s == "default" {
        Ok(k.default_n)
    } else {
        s.parse().map_err(|e| format!("n: {e}"))
    }
}

fn cmd_dump(a: &[String]) -> R {
    let [name, n, out] = a else {
        return Err("dump <kernel> <n|default> <out>".into());
    };
    let k = kernels::find(name).ok_or_else(|| format!("unknown kernel {name}"))?;
    let prog = (k.build)(kernel_n(k, n)?);
    let bytes = prog.to_bytes();
    prog.verify()?;
    std::fs::write(out, bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

fn cmd_ops() -> R {
    for (i, info) in OPS.iter().enumerate() {
        let class = if info.exit { "exit" } else { "tail" };
        println!(
            "{i:2} {:<16} {class} len={} {:?}",
            info.name,
            info.insn_len(),
            info.operands
        );
    }
    Ok(true)
}

/// Parses a result line into (result, interrupts, slow, error) and ns.
fn parse_line(s: &str) -> Option<(Key, u64)> {
    let mut m = BTreeMap::new();
    for kv in s.split_whitespace() {
        let (k, v) = kv.split_once('=')?;
        m.insert(k, v);
    }
    Some((
        (
            m.get("result")?.parse().ok()?,
            m.get("interrupts")?.parse().ok()?,
            m.get("slow")?.parse().ok()?,
            m.get("error")?.parse().ok()?,
        ),
        m.get("ns")?.parse().ok()?,
    ))
}

/// Runs an external variant process (`exe args.. file`) and parses its result line.
fn run_external(exe: &Path, args: &[&str], file: &Path) -> Result<(Key, u64), String> {
    let out = Command::new(exe)
        .args(args)
        .arg(file)
        .output()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(format!(
            "{} {args:?} {}: {} {stdout}{}",
            exe.display(),
            file.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_line(stdout.trim()).ok_or_else(|| format!("{}: bad output {stdout:?}", exe.display()))
}

fn opt(a: &[String], name: &str) -> Option<String> {
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn work_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let d = exe.parent().unwrap().join("sp-dispatch-programs");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn key(o: &Outcome) -> Key {
    (o.result, o.interrupts, o.slow_paths, o.error)
}

/// Small inputs: every variant, the C baseline and the reference model must agree.
fn cmd_check(a: &[String]) -> R {
    let cgoto = opt(a, "--cgoto").map(PathBuf::from);
    let become_exe = opt(a, "--become").map(PathBuf::from);
    let dir = work_dir()?;
    let mut ok = true;
    let mut cases: Vec<Case> = Vec::new();
    for k in KERNELS {
        let sizes: &[i64] = if k.name == "call" {
            &[0, 1, 2, 10, 25]
        } else {
            &[0, 1, 2, 25, 20_001]
        };
        for &n in sizes {
            cases.push((
                format!("{}({n})", k.name),
                (k.build)(n),
                Some((k.reference)(n)),
            ));
        }
    }
    cases.push(("all-ops(1000)".into(), kernels::build_all_ops(1000), None));
    cases.push((
        "overflow".into(),
        kernels::build_overflow(),
        Some((-2, 0, 3)),
    ));
    cases.push(("runaway".into(), kernels::build_runaway(), None));
    for (name, prog, want) in cases {
        let file = dir.join(format!("check-{}.ep9d", name.replace(['(', ')'], "_")));
        std::fs::write(&file, prog.to_bytes()).map_err(|e| e.to_string())?;
        let prog = prog.verify()?;
        let mut got: Vec<(String, Key)> = Vec::new();
        for &v in Variant::ALL.iter().chain([&Variant::MatchCounting]) {
            got.push((v.name().into(), key(&vm::run(&prog, v))));
        }
        if let Some(c) = &cgoto {
            got.push(("cgoto".into(), run_external(c, &[], &file)?.0));
        }
        if let Some(b) = &become_exe {
            got.push((
                "become".into(),
                run_external(b, &["run", "become"], &file)?.0,
            ));
        }
        let first = got[0].1;
        let mut same = got.iter().all(|(_, g)| *g == first);
        match want {
            Some((r, i, s)) => same &= first == (r, i, s, 0),
            // Runaway recursion must end in the stack-overflow error with the throw result.
            None if name == "runaway" => same &= first == (i64::MIN, 0, 0, vm::ERR_STACK_OVERFLOW),
            // No reference model: all variants agree and there is no error.
            None => same &= first.3 == 0,
        }
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        println!(
            "{} {name:<16} result={} interrupts={} slow={} error={}  [{}]",
            if same { "ok  " } else { "FAIL" },
            first.0,
            first.1,
            first.2,
            first.3,
            names.join(" ")
        );
        if !same {
            println!("     want {want:?}, got {got:?}");
            ok = false;
        }
    }
    Ok(ok)
}

/// Runs the threaded variants on a 64 KiB thread with about 10^8 dispatches each. A handler
/// that lost its tail call adds a native frame per dispatch and overflows this stack (the
/// process aborts). `all-ops` runs every opcode in its loop, so this covers each handler.
/// VM calls (fib) must not use the native stack either. Results must equal the match loop.
fn cmd_deep_loop() -> R {
    let jobs: Vec<(&str, Program)> = vec![
        ("all-ops", kernels::build_all_ops(2_000_000)),
        ("loop", (kernels::find("loop").unwrap().build)(5_000_000)),
        (
            "branchy",
            (kernels::find("branchy").unwrap().build)(3_000_000),
        ),
        ("call", (kernels::find("call").unwrap().build)(27)),
    ];
    let mut ok = true;
    for &v in Variant::ALL.iter().filter(|v| **v != Variant::Match) {
        for (name, prog) in &jobs {
            let prog = prog.clone().verify()?;
            let w = vm::run(&prog, Variant::Match);
            let want = (w.result, w.interrupts, w.slow_paths);
            let t = Instant::now();
            let o = std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || vm::run(&prog, v))
                .map_err(|e| e.to_string())?
                .join()
                .map_err(|_| "thread panicked".to_string())?;
            let pass =
                (o.result, o.interrupts, o.slow_paths, o.error) == (want.0, want.1, want.2, 0);
            ok &= pass;
            println!(
                "{} deep-loop {:<8} {name:<8} result={} ({:.2} s, 64 KiB native stack)",
                if pass { "ok  " } else { "FAIL" },
                v.name(),
                o.result,
                t.elapsed().as_secs_f64()
            );
        }
    }
    Ok(ok)
}

fn uptime() -> String {
    Command::new("uptime")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("uptime failed: {e}"))
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn geomean(v: &[f64]) -> f64 {
    (v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp()
}

struct Runner {
    name: String,
    exe: PathBuf,
    args: Vec<String>,
}

/// Interleaved benchmark. Each sample is a fresh process that times only the interpreter
/// run. Each round runs every kernel x variant once; the variant order rotates per round.
fn cmd_bench(a: &[String]) -> R {
    let rounds: usize = opt(a, "--rounds")
        .map_or(Ok(15), |s| s.parse())
        .map_err(|e| format!("{e}"))?;
    let scale: f64 = opt(a, "--scale")
        .map_or(Ok(1.0), |s| s.parse())
        .map_err(|e| format!("{e}"))?;
    let gate = a.iter().any(|x| x == "--gate");
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let run = |v: &str| vec!["run".to_string(), v.to_string()];
    let mut runners = vec![
        Runner {
            name: "match".into(),
            exe: me.clone(),
            args: run("match"),
        },
        Runner {
            name: "sibling".into(),
            exe: me.clone(),
            args: run("sibling"),
        },
    ];
    if let Some(b) = opt(a, "--become") {
        runners.push(Runner {
            name: "become".into(),
            exe: b.into(),
            args: run("become"),
        });
    }
    let Some(c) = opt(a, "--cgoto") else {
        return Err("bench needs --cgoto EXE".into());
    };
    runners.push(Runner {
        name: "cgoto".into(),
        exe: c.into(),
        args: vec![],
    });
    // --also NAME=EXE:VARIANT adds a measured variant that the gate ignores.
    for (i, x) in a.iter().enumerate().filter(|(_, x)| *x == "--also") {
        let bad = || format!("{x} NAME=EXE:VARIANT");
        let spec = a.get(i + 1).ok_or_else(bad)?;
        let (name, rest) = spec.split_once('=').ok_or_else(bad)?;
        let (exe, v) = rest.rsplit_once(':').ok_or_else(bad)?;
        runners.push(Runner {
            name: name.into(),
            exe: exe.into(),
            args: run(v),
        });
    }

    let dir = work_dir()?;
    println!("host: {} {}", std::env::consts::ARCH, std::env::consts::OS);
    println!("uptime before: {}", uptime());
    let mut files = Vec::new();
    let mut dispatches = Vec::new();
    for k in KERNELS {
        let n = if k.name == "call" {
            k.default_n + scale.log2().round() as i64
        } else {
            (k.default_n as f64 * scale) as i64
        };
        let prog = (k.build)(n);
        let file = dir.join(format!("bench-{}.ep9d", k.name));
        std::fs::write(&file, prog.to_bytes()).map_err(|e| e.to_string())?;
        let o = vm::run(&prog.verify()?, Variant::MatchCounting);
        println!(
            "kernel {:<8} n={n:<10} dispatches={:<11} {}",
            k.name, o.dispatches, k.about
        );
        files.push((k.name, file, n));
        dispatches.push(o.dispatches);
    }

    // samples[kernel][runner] = ns per run; results[kernel] = the agreed outcome.
    let mut samples = vec![vec![Vec::<f64>::new(); runners.len()]; files.len()];
    let mut results: Vec<Option<Key>> = vec![None; files.len()];
    let mut ok = true;
    let warmup = 1;
    for round in 0..warmup + rounds {
        for (ki, (kname, file, _)) in files.iter().enumerate() {
            for j in 0..runners.len() {
                let ri = (j + round + ki) % runners.len();
                let r = &runners[ri];
                let args: Vec<&str> = r.args.iter().map(String::as_str).collect();
                let (res, ns) = run_external(&r.exe, &args, file)?;
                match results[ki] {
                    None => results[ki] = Some(res),
                    Some(want) if want != res => {
                        println!(
                            "FAIL result mismatch: {kname} {}: {res:?} != {want:?}",
                            r.name
                        );
                        ok = false;
                    }
                    _ => {}
                }
                if round >= warmup {
                    samples[ki][ri].push(ns as f64);
                }
            }
        }
        eprint!("\rround {}/{}", round + 1, warmup + rounds);
    }
    eprintln!();
    println!("uptime after:  {}", uptime());
    println!();
    println!("Median of {rounds} runs per cell (warm-up round dropped). ms, then ns per dispatch:");
    print!("| kernel | result |");
    for r in &runners {
        print!(" {} ms | {} ns/d |", r.name, r.name);
    }
    println!();
    let mut med = vec![vec![0.0; runners.len()]; files.len()];
    for (ki, (kname, _, _)) in files.iter().enumerate() {
        print!("| {kname} | {} |", results[ki].unwrap().0);
        for ri in 0..runners.len() {
            let mut v = samples[ki][ri].clone();
            let m = median(&mut v);
            med[ki][ri] = m;
            print!(" {:.1} | {:.3} |", m / 1e6, m / dispatches[ki] as f64);
        }
        println!();
    }
    println!();
    println!("Spread per cell: min..max ms");
    for (ki, (kname, _, _)) in files.iter().enumerate() {
        let cells: Vec<String> = runners
            .iter()
            .enumerate()
            .map(|(ri, r)| {
                let v = &samples[ki][ri];
                let lo = v.iter().cloned().fold(f64::INFINITY, f64::min);
                let hi = v.iter().cloned().fold(0.0, f64::max);
                format!("{} {:.1}..{:.1}", r.name, lo / 1e6, hi / 1e6)
            })
            .collect();
        println!("  {kname:<8} {}", cells.join(" | "));
    }

    let idx = |n: &str| runners.iter().position(|r| r.name == n).unwrap();
    let (im, is, ic) = (idx("match"), idx("sibling"), idx("cgoto"));
    let mut sc = Vec::new();
    let mut ms = Vec::new();
    println!();
    println!(
        "| kernel | sibling / cgoto | match / sibling | paired sibling / cgoto | paired match / sibling |"
    );
    for (ki, (kname, _, _)) in files.iter().enumerate() {
        let a = med[ki][is] / med[ki][ic];
        let b = med[ki][im] / med[ki][is];
        // Paired: ratio inside each round, then the median. Robust to load drift.
        let mut pa: Vec<f64> = (0..rounds)
            .map(|i| samples[ki][is][i] / samples[ki][ic][i])
            .collect();
        let mut pb: Vec<f64> = (0..rounds)
            .map(|i| samples[ki][im][i] / samples[ki][is][i])
            .collect();
        println!(
            "| {kname} | {a:.3} | {b:.3} | {:.3} | {:.3} |",
            median(&mut pa),
            median(&mut pb)
        );
        sc.push(a);
        ms.push(b);
    }
    let (gsc, gms) = (geomean(&sc), geomean(&ms));
    println!("| geomean | {gsc:.3} | {gms:.3} | | |");
    for (ib, r) in runners.iter().enumerate() {
        if matches!(r.name.as_str(), "match" | "sibling" | "cgoto") {
            continue;
        }
        let bs: Vec<f64> = (0..files.len())
            .map(|ki| med[ki][ib] / med[ki][is])
            .collect();
        println!(
            "{} / sibling per kernel: {bs:.3?}, geomean {:.3} (not gated)",
            r.name,
            geomean(&bs)
        );
    }
    let g1 = gsc <= 1.25;
    let g2 = gms >= 1.2;
    println!();
    println!(
        "GATE sibling <= 1.25x cgoto (geomean): {gsc:.3} {}",
        if g1 { "PASS" } else { "FAIL" }
    );
    println!(
        "GATE sibling >= 1.2x faster than match (geomean): {gms:.3} {}",
        if g2 { "PASS" } else { "FAIL" }
    );
    for (ki, (kname, _, _)) in files.iter().enumerate() {
        println!(
            "  per kernel {kname:<8} sibling/cgoto {:.3} {}  match/sibling {:.3} {}",
            sc[ki],
            if sc[ki] <= 1.25 { "ok" } else { "over" },
            ms[ki],
            if ms[ki] >= 1.2 { "ok" } else { "under" }
        );
    }
    println!(
        "results equal across variants: {}",
        if ok { "yes" } else { "NO" }
    );
    Ok(ok && (!gate || (g1 && g2)))
}
