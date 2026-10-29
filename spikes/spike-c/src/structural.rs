//! `spike-c structural`: the checks of `gate.sh structural` that run in the built binary (the
//! release build, where T0 runs the threaded dispatch and placement (b) code really runs).
//!
//! | Check | What | Until |
//! |---|---|---|
//! | harness | an isolate with the builtins runs a script and `print` captures | - |
//! | install-b | Spike B's (b) emitter code for a loop function, installed by [`Session::install_b`], gives T0's output, from an Opt slot | - |
//! | facts | live facts of a run -> PROFILED file -> read back: the same records | - |
//! | layout | the `ep9_vm::layout` facts [`crate::vm_layout`] takes hold on live objects | - |
//! | pipeline | a loop function compiled from its live facts (graph builder, lattice check, linear scan, a64 emit), installed in an Opt slot: output equal to T0 | - |
//! | lattice | the protocol §6.1 checks on a graph with a named-load fact: one trust level per value, PROVEN only from the H30 list, every predicted value behind a guard of its fact, the five ref nodes only, every guard resumable (SPIKEC-3 guard placement); a graph changed to break each rule is refused | - |
//! | deopt | an input no guard admits deopts into T0 in the same frame (H29: the exit table installed with the code, the deopt entry rebuilds the frame): output equal to T0, the guard's exit counted | - |
//! | kernels | each protocol kernel (the pinned files) runs in T0 (n = 1) with its expected output, check line included (node's, where known); SKIP without the v8-v7 checkout | - |

use std::process::ExitCode;

use spike_b::emit::Emitter;
use spike_b::harness::Config;

use crate::facts;
use crate::harness::Session;
use crate::kernels;

type Check = fn() -> Result<String, String>;

const CHECKS: [(&str, Check); 8] = [
    ("harness", harness),
    ("install-b", install_b),
    ("facts", facts_round_trip),
    ("layout", layout),
    ("pipeline", pipeline),
    ("lattice", lattice),
    ("deopt", deopt),
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
    // SAFETY: `code` is placement (b) code of Spike B's `BEmitter` for `f`, made from
    // `input_of(f)` just above (no JS ran since) and not changed.
    unsafe { s.install_b(&[f], vec![code], vec![None]) }?;
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

fn layout() -> Result<String, String> {
    crate::vm_layout::check(&mut Session::new(&Config::default())?)
}

fn aarch64() -> bool {
    spike_b::emit::Arch::HOST == Some(spike_b::emit::Arch::Aarch64)
}

fn pipeline() -> Result<String, String> {
    if !aarch64() {
        return Ok("SKIP: the emitter is aarch64 only (protocol §1)".into());
    }
    let r = crate::tier::run_script(
        &Config::default(),
        LOOP,
        "for (var k = 0; k < 20; k++) sum(10);",
        // 60000 keeps every `a + i` an int32 (70000 does not: the overflow guard is unbacked
        // until the P1 profile gives BinaryOp facts).
        "print(sum(1000), sum(0), sum(60000));",
        "sum",
    )?;
    if !r.equal() || r.compiled.output != ["499500 0 1799970000"] {
        return Err(format!("compiled {:?} vs T0 {:?}", r.compiled, r.t0));
    }
    Ok(format!("{}; output equals T0", r.report.line()))
}

fn lattice() -> Result<String, String> {
    use crate::ir::{Op, Repr, Trust};
    let mut s = Session::new(&Config::default())?;
    s.run(FACTS_SRC, "f.js").result?;
    let script = s.script("f.js").ok_or("no script")?;
    let get = *script.functions_named("get").first().ok_or("no get")?;
    let live = facts::live(&s, script)?;
    let input = s.function_input("get")?;
    let (g, _) =
        crate::build::build(&input, &facts::for_function(&live, get)).map_err(|e| e.to_string())?;
    g.check()?;
    let (fact, unbacked) = g.guard_counts();
    if fact == 0 {
        return Err(format!("no fact guard in\n{g}"));
    }
    // Negative controls: each broken graph is refused.
    let field = g
        .order()
        .find(|&n| matches!(g.node(n).op, Op::LoadField { .. }))
        .ok_or("no field load")?;
    let shape = g
        .order()
        .find(|&n| matches!(g.node(n).op, Op::CheckShape { .. }))
        .ok_or("no shape check")?;
    let mut bad = g.clone();
    bad.node_mut(field).trust = Trust::Proven;
    let e1 = bad.check().err().ok_or("a PROVEN field load passed")?;
    let mut bad = g.clone();
    bad.node_mut(field).trust = Trust::Predicted(ep9_facts::FactSource::Runtime);
    let e2 = bad
        .check()
        .err()
        .ok_or("a predicted value without a guard passed")?;
    let mut bad = g.clone();
    bad.node_mut(shape).guard = None;
    let e3 = bad
        .check()
        .err()
        .ok_or("a shape check without a guard passed")?;
    let mut bad = g.clone();
    bad.refs.clear();
    let e4 = bad
        .check()
        .err()
        .ok_or("a shape guard without its ref nodes passed")?;
    // Guard placement (SPIKEC-3): a poll of the shape guard's own instruction placed before
    // it; a deopt there would resume T0 before the poll and run it again.
    let mut bad = g.clone();
    let (b, off) = (bad.node(shape).block, bad.node(shape).offset);
    let poll = bad.add(Op::Poll, vec![], Repr::None, Trust::Unknown, b, off);
    let blk = &mut bad.blocks[b.0 as usize].nodes;
    let at = blk
        .iter()
        .position(|&n| n == shape)
        .ok_or("shape not in its block")?;
    blk.insert(at, poll);
    let e5 = bad
        .check()
        .err()
        .ok_or("a guard after an effect of its own instruction passed")?;
    if !e5.contains("not replayable") {
        return Err(format!("placement: wrong refusal {e5}"));
    }
    let _ = (e1, e2, e3, e4);
    Ok(format!(
        "get: {} nodes, {fact} fact guards, {unbacked} unbacked; 5 broken graphs refused",
        g.order().count()
    ))
}

fn deopt() -> Result<String, String> {
    if !aarch64() {
        return Ok("SKIP: the emitter is aarch64 only (protocol §1)".into());
    }
    let defs = "function P(x) { this.x = x; }\n\
        function get(ps, n) { var s = 0; for (var i = 0; i < n; i++) s = (s + ps[i].x) | 0; return s; }";
    let warm = "var ps = []; for (var i = 0; i < 20; i++) ps.push(new P(i)); for (var k = 0; k < 30; k++) get(ps, 20);";
    let r = crate::tier::run_script(
        &Config::default(),
        defs,
        warm,
        "print(get(ps, 20)); print(get([{ y: 1, x: 2 }], 1));",
        "get",
    )?;
    let [g] = r.fired()[..] else {
        return Err(format!(
            "exits {:?}: compiled {:?} vs T0 {:?}",
            r.exits, r.compiled, r.t0
        ));
    };
    if !r.equal() || r.t0.output != ["190", "2"] {
        return Err(format!("{:?} vs T0 {:?}", r.compiled, r.t0));
    }
    Ok(format!(
        "a new shape at `ps[i].x` deopts at guard g{g} ({} exit); output equal to T0",
        r.exits.iter().sum::<u64>()
    ))
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
