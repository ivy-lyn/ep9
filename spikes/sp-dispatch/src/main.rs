//! sp-dispatch: run, check and benchmark the three expansions and the C baseline.
//!
//! ```text
//! sp-dispatch run <variant> <file.ep9d>     one timed run; prints one result line
//! sp-dispatch dump <kernel> <n|default> <out.ep9d>
//! sp-dispatch ops                           opcode table
//! sp-dispatch check [--cgoto EXE] [--become EXE]
//! sp-dispatch deep-loop                     threaded variants on a 64 KiB native stack
//! sp-dispatch cpu                           CPU model (the x64 speed ratios depend on it)
//! sp-dispatch bench [--rounds N] [--scale F] [--cgoto EXE] [--become EXE] [--json FILE] [--gate]
//! ```
//!
//! Result line (cgoto.c prints the same format):
//! `result=<i64> interrupts=<u64> slow=<u64> error=<u32> ns=<u64>`
//!
//! Speed gates (H15 ADR; docs/plan/09-milestones.md section 4, row SP-Dispatch): `bench`
//! prints one `GATE-RESULT <id> <pass|fail|marginal|void>` line per gate of this target, and
//! `REPORT-RESULT` for a comparison that is measured but not gated there (x86-64: sibling-vs-match).
//! The verdict follows the speed-gate rule of docs/plan/10-quality.md section 3 (PERF-NOISE):
//! paired ratios per round, median, order-statistic interval, void if a kernel IQR > 5%.

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
        Some("cpu") => {
            println!("{}", cpu_model());
            Ok(true)
        }
        Some("bench") => cmd_bench(&args[1..]),
        _ => Err(
            "usage: sp-dispatch run|dump|ops|check|deep-loop|cpu|bench (see src/main.rs)".into(),
        ),
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

/// CPU model. The x86-64 ratios depend on it: one hosted runner label gave AMD EPYC 7763,
/// 9V74, 9V45 and Intel Xeon 8573C, with match/sibling from 0.77 to 1.13
/// (docs/measurements/2026-10-21-sp-dispatch-x64.md).
fn cpu_model() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        // CPUID leaves 0x8000_0002..=0x8000_0004 hold the 48-byte brand string, on every OS.
        use std::arch::x86_64::__cpuid;
        if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
            let mut b = Vec::with_capacity(48);
            for leaf in 0x8000_0002u32..=0x8000_0004 {
                let r = __cpuid(leaf);
                for w in [r.eax, r.ebx, r.ecx, r.edx] {
                    b.extend_from_slice(&w.to_le_bytes());
                }
            }
            let s = String::from_utf8_lossy(&b);
            let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    // Linux aarch64: implementer and part number.
    if let Ok(s) = std::fs::read_to_string("/proc/cpuinfo") {
        let field = |k: &str| {
            s.lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        };
        if let (Some(imp), Some(part)) = (field("CPU implementer"), field("CPU part")) {
            let name = match (imp.as_str(), part.as_str()) {
                ("0x41", "0xd0c") => "Neoverse-N1 ",
                ("0x41", "0xd49") => "Neoverse-N2 ",
                ("0x41", "0xd40") => "Neoverse-V1 ",
                ("0x41", "0xd4f") => "Neoverse-V2 ",
                _ => "",
            };
            return format!("{name}(implementer {imp}, part {part})");
        }
        if let Some(m) = field("model name") {
            return m;
        }
    }
    // macOS.
    Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
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

/// Linear-interpolation quantile of sorted values.
fn quantile(s: &[f64], p: f64) -> f64 {
    let x = p * (s.len() - 1) as f64;
    let i = x.floor() as usize;
    match s.get(i + 1) {
        Some(&next) => s[i] + (next - s[i]) * (x - i as f64),
        None => s[i],
    }
}

/// Rank k (1-based) of the order-statistic interval [x(k), x(n+1-k)] of the median: the
/// largest k with P(Bin(n, 1/2) <= k - 1) <= 0.025, so the interval covers >= 95%.
/// 10-quality section 3: n = 7 -> 1, n = 10 -> 2, n = 15 -> 4, n = 31 -> 10.
/// For n < 6 no k reaches 95%; it returns 1 (min and max).
fn median_rank(n: usize) -> usize {
    let mut pmf = 0.5f64.powi(n as i32);
    let mut cdf = 0.0;
    let mut k = 1;
    for i in 0..n {
        cdf += pmf; // P(B <= i)
        if cdf > 0.025 {
            break;
        }
        k = i + 1;
        pmf *= (n - i) as f64 / (i + 1) as f64;
    }
    k.min(n.div_ceil(2))
}

/// One statistic of paired ratios with its verdict against a limit.
struct Stat {
    what: String,
    limit: f64,
    /// true: pass needs ratio <= limit; false: pass needs ratio >= limit.
    at_most: bool,
    n: usize,
    median: f64,
    lo: f64,
    hi: f64,
    /// Interquartile range in % of the median.
    iqr_pct: f64,
    result: &'static str,
}

/// PERF-NOISE verdict for the paired ratios `v` (10-quality section 3): pass if the whole
/// interval of the median is on the pass side, fail if it is on the fail side, else marginal.
fn stat(what: &str, v: &[f64], limit: f64, at_most: bool) -> Stat {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let k = median_rank(n);
    let (lo, hi) = (s[k - 1], s[n - k]);
    let median = quantile(&s, 0.5);
    let iqr_pct = (quantile(&s, 0.75) - quantile(&s, 0.25)) / median * 100.0;
    let (pass, fail) = if at_most {
        (hi <= limit, lo > limit)
    } else {
        (lo >= limit, hi < limit)
    };
    let result = if pass {
        "pass"
    } else if fail {
        "fail"
    } else {
        "marginal"
    };
    Stat {
        what: what.into(),
        limit,
        at_most,
        n,
        median,
        lo,
        hi,
        iqr_pct,
        result,
    }
}

/// A speed gate (or a report with the same form): a geomean statistic plus per-kernel guards.
struct Gate {
    id: String,
    /// "gate" counts for the target; "report" is printed and stored only.
    kind: &'static str,
    rule: String,
    stats: Vec<Stat>,
}

impl Gate {
    /// void if a kernel's IQR > 5% (host not quiet; 10-quality section 3), else the worst
    /// verdict: fail > marginal > pass.
    fn result(&self) -> &'static str {
        if self.stats.iter().skip(1).any(|s| s.iqr_pct > 5.0) {
            return "void";
        }
        self.interval_result()
    }

    /// The worst interval verdict, IQR ignored.
    fn interval_result(&self) -> &'static str {
        let rank = |r: &str| match r {
            "fail" => 2,
            "marginal" => 1,
            _ => 0,
        };
        self.stats
            .iter()
            .map(|s| s.result)
            .max_by_key(|r| rank(r))
            .unwrap_or("fail")
    }
}

/// Builds a gate from per-round samples. `num` / `den` index the runners; the statistic per
/// round is num/den for each kernel and their geomean over the kernels.
fn paired_gate(
    g: (&str, &'static str, String),
    samples: &[Vec<Vec<f64>>],
    kernels: &[&str],
    (num, den): (usize, usize),
    (geo_limit, kernel_limit, at_most): (f64, f64, bool),
) -> Gate {
    let rounds = samples[0][num].len();
    let per_kernel: Vec<Vec<f64>> = samples
        .iter()
        .map(|k| (0..rounds).map(|i| k[num][i] / k[den][i]).collect())
        .collect();
    let per_round: Vec<f64> = (0..rounds)
        .map(|i| geomean(&per_kernel.iter().map(|k| k[i]).collect::<Vec<_>>()))
        .collect();
    let mut stats = vec![stat("geomean", &per_round, geo_limit, at_most)];
    for (ki, name) in kernels.iter().enumerate() {
        stats.push(stat(name, &per_kernel[ki], kernel_limit, at_most));
    }
    Gate {
        id: g.0.into(),
        kind: g.1,
        rule: g.2,
        stats,
    }
}

fn print_gate(g: &Gate) {
    let tag = if g.kind == "gate" { "GATE" } else { "REPORT" };
    println!("{tag} {}: {}", g.id, g.rule);
    for s in &g.stats {
        println!(
            "  {:<8} n={} median {:.3} interval [{:.3}, {:.3}] IQR {:.1}% ({} {:.2}) -> {}",
            s.what,
            s.n,
            s.median,
            s.lo,
            s.hi,
            s.iqr_pct,
            if s.at_most { "<=" } else { ">=" },
            s.limit,
            s.result
        );
    }
    let (r, ir) = (g.result(), g.interval_result());
    if r == "void" {
        println!(
            "{tag}-RESULT {} void (a kernel IQR > 5%; interval verdict: {ir})",
            g.id
        );
    } else {
        println!("{tag}-RESULT {} {r}", g.id);
    }
}

/// JSON string literal.
fn js(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// JSON number list.
fn jn(v: &[f64]) -> String {
    let items: Vec<String> = v.iter().map(|x| format!("{x}")).collect();
    format!("[{}]", items.join(","))
}

fn gate_json(g: &Gate) -> String {
    let stats: Vec<String> = g
        .stats
        .iter()
        .map(|s| {
            format!(
                "{{\"what\":{},\"limit\":{},\"at_most\":{},\"n\":{},\"median\":{},\"lo\":{},\"hi\":{},\"iqr_pct\":{},\"result\":{}}}",
                js(&s.what),
                s.limit,
                s.at_most,
                s.n,
                s.median,
                s.lo,
                s.hi,
                s.iqr_pct,
                js(s.result)
            )
        })
        .collect();
    format!(
        "{{\"id\":{},\"kind\":{},\"rule\":{},\"result\":{},\"interval_result\":{},\"stats\":[{}]}}",
        js(&g.id),
        js(g.kind),
        js(&g.rule),
        js(g.result()),
        js(g.interval_result()),
        stats.join(",")
    )
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
    let cpu = cpu_model();
    let up_before = uptime();
    println!("host: {} {}", std::env::consts::ARCH, std::env::consts::OS);
    println!("cpu: {cpu}");
    println!("release expansion: {}", Variant::RELEASE.name());
    println!("uptime before: {up_before}");
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
    let up_after = uptime();
    println!("uptime after:  {up_after}");
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
    let release = Variant::RELEASE.name();
    let ir = idx(release);
    println!();
    println!("Ratio of medians (for reading; the gates below use paired ratios):");
    println!("| kernel | {release} (release) / cgoto | sibling / cgoto | match / sibling |");
    let mut cols = [Vec::new(), Vec::new(), Vec::new()];
    for (ki, (kname, _, _)) in files.iter().enumerate() {
        let r = [
            med[ki][ir] / med[ki][ic],
            med[ki][is] / med[ki][ic],
            med[ki][im] / med[ki][is],
        ];
        println!("| {kname} | {:.3} | {:.3} | {:.3} |", r[0], r[1], r[2]);
        for (c, x) in cols.iter_mut().zip(r) {
            c.push(x);
        }
    }
    println!(
        "| geomean | {:.3} | {:.3} | {:.3} |",
        geomean(&cols[0]),
        geomean(&cols[1]),
        geomean(&cols[2])
    );
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

    // Speed gates per target (H15 ADR, 09-milestones section 4). The release expansion must
    // stay near C computed goto. On aarch64 sibling calls must also beat the match loop; on
    // x86-64 the match loop is the release expansion, so that comparison is a report only.
    let kernels: Vec<&str> = files.iter().map(|f| f.0).collect();
    let sib_rel = Variant::RELEASE == Variant::Sibling;
    let gates = [
        paired_gate(
            (
                &format!("{release}-vs-cgoto"),
                "gate",
                format!("{release} (release) time <= 1.25x cgoto.c (geomean), <= 1.5x per kernel"),
            ),
            &samples,
            &kernels,
            (ir, ic),
            (1.25, 1.5, true),
        ),
        paired_gate(
            (
                "sibling-vs-match",
                if sib_rel { "gate" } else { "report" },
                format!(
                    "match time >= 1.2x sibling time (geomean), >= 1.0x per kernel{}",
                    if sib_rel {
                        ""
                    } else {
                        " (release is match on x86-64: a report for the M2 re-check, not a gate)"
                    }
                ),
            ),
            &samples,
            &kernels,
            (im, is),
            (1.2, 1.0, false),
        ),
    ];
    println!();
    for g in &gates {
        print_gate(g);
    }
    println!(
        "results equal across variants: {}",
        if ok { "yes" } else { "NO" }
    );

    if let Some(path) = opt(a, "--json") {
        let names: Vec<&str> = runners.iter().map(|r| r.name.as_str()).collect();
        let ks: Vec<String> = files
            .iter()
            .enumerate()
            .map(|(ki, (kname, _, n))| {
                let med_ns: Vec<String> = names
                    .iter()
                    .enumerate()
                    .map(|(ri, r)| format!("{}:{}", js(r), med[ki][ri]))
                    .collect();
                let raw: Vec<String> = names
                    .iter()
                    .enumerate()
                    .map(|(ri, r)| format!("{}:{}", js(r), jn(&samples[ki][ri])))
                    .collect();
                format!(
                    "{{\"name\":{},\"n\":{n},\"dispatches\":{},\"result\":{},\"median_ns\":{{{}}},\"samples_ns\":{{{}}}}}",
                    js(kname),
                    dispatches[ki],
                    results[ki].map_or(0, |r| r.0),
                    med_ns.join(","),
                    raw.join(",")
                )
            })
            .collect();
        let gs: Vec<String> = gates.iter().map(gate_json).collect();
        let j = format!(
            "{{\"schema\":\"sp-dispatch-bench/1\",\"arch\":{},\"os\":{},\"cpu\":{},\"release\":{},\"rounds\":{rounds},\"warmup\":{warmup},\"scale\":{scale},\"uptime_before\":{},\"uptime_after\":{},\"results_equal\":{ok},\"kernels\":[{}],\"gates\":[{}]}}\n",
            js(std::env::consts::ARCH),
            js(std::env::consts::OS),
            js(&cpu),
            js(release),
            js(&up_before),
            js(&up_after),
            ks.join(","),
            gs.join(",")
        );
        std::fs::write(&path, j).map_err(|e| format!("{path}: {e}"))?;
        println!("json: {path}");
    }
    let gates_pass = gates
        .iter()
        .filter(|g| g.kind == "gate")
        .all(|g| g.result() == "pass");
    Ok(ok && (!gate || gates_pass))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_rank_matches_the_speed_gate_rule() {
        // 10-quality section 3: n = 7 -> min and max; 10 -> 2, 9; 15 -> 4, 12; 31 -> 10, 22.
        for (n, k) in [(5, 1), (7, 1), (10, 2), (15, 4), (31, 10)] {
            assert_eq!(median_rank(n), k, "n = {n}");
        }
    }

    #[test]
    fn verdict_pass_fail_marginal_void() {
        let v: Vec<f64> = (0..15).map(|i| 1.0 + i as f64 * 0.001).collect();
        assert_eq!(stat("k", &v, 1.25, true).result, "pass");
        assert_eq!(stat("k", &v, 0.9, true).result, "fail");
        assert_eq!(stat("k", &v, 1.007, true).result, "marginal");
        assert_eq!(stat("k", &v, 0.9, false).result, "pass");
        assert_eq!(stat("k", &v, 1.2, false).result, "fail");
        // One sample per round for 2 kernels x 2 runners; kernel 1 is noisy (IQR > 5%).
        let quiet: Vec<f64> = vec![100.0; 15];
        let noisy: Vec<f64> = (0..15).map(|i| 100.0 + (i % 5) as f64 * 10.0).collect();
        let samples = vec![
            vec![quiet.clone(), quiet.clone()],
            vec![noisy, quiet.clone()],
        ];
        let g = paired_gate(
            ("t", "gate", String::new()),
            &samples,
            &["a", "b"],
            (0, 1),
            (1.25, 1.5, true),
        );
        assert_eq!(g.result(), "void");
        assert_eq!(g.interval_result(), "pass");
    }
}
