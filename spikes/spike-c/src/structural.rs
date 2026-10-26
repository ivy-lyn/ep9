//! `spike-c structural`: the checks of `gate.sh structural` that run in the built binary (the
//! release build, where T0 runs the threaded dispatch and placement (b) code really runs).
//!
//! | Check | What | Until |
//! |---|---|---|
//! | harness | an isolate with the builtins runs a script and `print` captures | - |
//! | install-b | Spike B's (b) emitter code for a loop function, installed by [`Session::install_b`], gives T0's output, from an Opt slot | - |
//! | facts | live facts of a run -> PROFILED file -> read back: the same records | - |
//! | pipeline | [`crate::compile`] refuses with "graph builder not built yet" | SPIKEC-1 replaces it with the `am3` compile check |
//! | kernels | each protocol kernel runs in T0 (n = 1) with its expected output; SKIP without the v8-v7 checkout | - |

use std::process::ExitCode;

use spike_b::emit::Emitter;
use spike_b::harness::Config;

use crate::facts;
use crate::harness::Session;
use crate::kernels;

type Check = fn() -> Result<String, String>;

const CHECKS: [(&str, Check); 5] = [
    ("harness", harness),
    ("install-b", install_b),
    ("facts", facts_round_trip),
    ("pipeline", pipeline),
    ("kernels", kernels_t0),
];

fn harness() -> Result<String, String> {
    let mut s = Session::new(&Config::default())?;
    let r = s.run("print(Math.max(2, 9), 'spike'.length);", "h.js");
    if r.output != ["9 5"] {
        return Err(format!("{r:?}"));
    }
    Ok("print captured \"9 5\"".into())
}

const LOOP: &str =
    "function sum(n) { let a = 0; for (let i = 0; i < n; i++) { a = a + i; } return a; }";
const LOOP_MAIN: &str = "print(sum(1000), sum(0), sum(70000));";

fn install_b() -> Result<String, String> {
    let Some(mut e) = spike_b::jit_b::BEmitter::host() else {
        return Ok("SKIP: no placement (b) emitter for this architecture".into());
    };
    let config = Config::default().threaded();
    let mut t0 = Session::new(&config)?;
    t0.run(LOOP, "defs.js").result?;
    let want = t0.run(LOOP_MAIN, "main.js");
    let mut s = Session::new(&config)?;
    s.run(LOOP, "defs.js").result?;
    let f = s.function_value("sum")?;
    let code = e.emit(&s.input_of(f)?).map_err(|r| r.to_string())?;
    let bytes = code.code.len();
    s.install_b(&[f], vec![code])?;
    if s.slot_kind(f) != Some(ep9_vm::DispatchKind::Opt) {
        return Err(format!("slot kind {:?}, not Opt", s.slot_kind(f)));
    }
    let got = s.run(LOOP_MAIN, "main.js");
    if got != want || got.output != ["499500 0 2449965000"] {
        return Err(format!("(b) {got:?} vs T0 {want:?}"));
    }
    Ok(format!(
        "sum: {bytes} bytes in an Opt slot, output equals T0"
    ))
}

const FACTS_SRC: &str = "\
function P(x) { this.x = x; }\n\
function get(ps) { var s = 0; for (var i = 0; i < ps.length; i++) s = (s + ps[i].x) | 0; return s; }\n\
var ps = []; for (var i = 0; i < 20; i++) ps.push(new P(i));\n\
for (var k = 0; k < 30; k++) get(ps);\n";

fn facts_round_trip() -> Result<String, String> {
    let mut s = Session::new(&Config::default())?;
    s.run(FACTS_SRC, "f.js").result?;
    let script = s.script("f.js").ok_or("no script")?;
    let get = *script.functions_named("get").first().ok_or("no get")?;
    let live = facts::live(&s, script)?;
    let n = facts::for_function(&live, get).records.len();
    if n == 0 {
        return Err("no live record for get".into());
    }
    let bytes = facts::to_profiled(&live)?.encode();
    let (back, dropped) = facts::read_profiled(&bytes, script.unit.unit())?;
    let m = facts::for_function(&back, get);
    if !dropped.is_empty() || m.records != facts::for_function(&live, get).records {
        return Err(format!(
            "records differ after the round trip; dropped {dropped:?}"
        ));
    }
    Ok(format!(
        "get: {n} records, {} bytes, read back equal",
        bytes.len()
    ))
}

fn pipeline() -> Result<String, String> {
    let mut s = Session::new(&Config::default())?;
    s.run(LOOP, "defs.js").result?;
    let f = facts::FunctionFacts::none(ep9_facts::FactSource::Runtime, 1);
    match crate::compile(&s, "sum", &f) {
        Ok(_) => Err("compiled, but the pipeline is a stub at M4-P0".into()),
        Err(e) if e.contains("graph builder not built yet") || e.contains("architecture") => {
            Ok(format!("refused as expected at M4-P0: {e}"))
        }
        Err(e) => Err(format!("unexpected refusal: {e}")),
    }
}

fn kernels_t0() -> Result<String, String> {
    if !kernels::bench_present() {
        return Ok(format!(
            "SKIP: no {} (bash scripts/sync-bench.sh bench-v8v7)",
            kernels::bench_dir().display()
        ));
    }
    let mut done = Vec::new();
    for k in &kernels::KERNELS {
        let d = kernels::driver(k, 1)?;
        let mut s = Session::new(&Config::default())?;
        let r = s.run(&d.text, "driver.js");
        r.result.map_err(|e| format!("{}: {e}", k.name))?;
        if !d.matches(&r.output) {
            return Err(format!(
                "{}: {:?}, expected {:?}",
                k.name, r.output, d.expect
            ));
        }
        done.push(k.name);
    }
    Ok(format!("T0, n = 1: {}", done.join(", ")))
}

/// Runs the checks named in `args` (`all` or none: every check).
pub fn main(args: &[String]) -> ExitCode {
    let all = args.is_empty() || args.iter().any(|a| a == "all");
    let mut failed = false;
    let mut ran = 0;
    for (name, check) in CHECKS {
        if !all && !args.iter().any(|a| a == name) {
            continue;
        }
        ran += 1;
        match check() {
            Ok(note) => println!("{name:<10} PASS  {note}"),
            Err(e) => {
                failed = true;
                println!("{name:<10} FAIL  {e}");
            }
        }
    }
    if ran == 0 {
        eprintln!(
            "spike-c structural: no check named {args:?} (known: {})",
            CHECKS.map(|c| c.0).join(", ")
        );
        return ExitCode::from(2);
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_check_passes() {
        // `kernels` runs in kernels::tests (with the hot-line checks).
        for (name, check) in CHECKS.into_iter().filter(|c| c.0 != "kernels") {
            let r = check();
            assert!(r.is_ok(), "{name}: {r:?}");
        }
    }
}
