//! The SPIKEC-10 speed tests (plan v11 §4.8 task 1, the speed branch of V11-D28): the d19
//! form of the Float64 ops (a chain of double ops reads the last result from d19, not
//! through a `fmov` from its register) and the loop-invariant loads and checks that move to
//! a loop's preheader (`build::hoist_array_checks`: context slot loads, int32 and number
//! checks of values from outside the loop). Each function runs in the builds of
//! `review_tests` and must equal T0:
//!
//! ```text
//!  unforced | int32 junk | forced seeds 1..8 | GC stress
//! ```

use spike_b::harness::Config;

use super::{FORCE_LEN, IntFacts, ScriptRun, run_script, run_script_full};
use crate::emit_a64::{EmitOptions, ForceTable};

/// A function compiled with a Mac-only emitter: skip elsewhere.
fn aarch64() -> bool {
    if spike_b::emit::Arch::HOST == Some(spike_b::emit::Arch::Aarch64) {
        return true;
    }
    eprintln!("skip: the emitter is aarch64 only");
    false
}

/// Runs `name` compiled in each build of the module doc; the unforced run and the list of
/// differences from T0 (and of errors).
fn check(tag: &str, defs: &str, warm: &str, main: &str, name: &str) -> (ScriptRun, Vec<String>) {
    let mut bad = Vec::new();
    let mut add = |what: String, r: Result<ScriptRun, String>| match r {
        Ok(r) if !r.equal() => bad.push(format!(
            "[{tag}] {what} differs: T0 {:?} {:?} vs C {:?} {:?}",
            r.t0.result, r.t0.output, r.compiled.result, r.compiled.output
        )),
        Ok(_) => {}
        Err(e) => bad.push(format!("[{tag}] {what}: error {e}")),
    };
    let d = Config::default();
    let junk = EmitOptions {
        int32_junk: true,
        ..Default::default()
    };
    add(
        "junk".into(),
        run_script_full(&d, defs, warm, main, name, IntFacts::Live, &junk),
    );
    for seed in 1..=8u64 {
        let t = ForceTable::seeded(seed, FORCE_LEN);
        let opts = EmitOptions {
            force: Some(&t),
            ..Default::default()
        };
        add(
            format!("forced seed {seed}"),
            run_script_full(&d, defs, warm, main, name, IntFacts::Live, &opts),
        );
    }
    add(
        "stress".into(),
        run_script(&Config::stress(), defs, warm, main, name),
    );
    let r = run_script(&d, defs, warm, main, name).unwrap_or_else(|e| panic!("[{tag}] {e}"));
    if !r.equal() {
        bad.push(format!(
            "[{tag}] unforced differs: T0 {:?} {:?} vs C {:?} {:?}",
            r.t0.result, r.t0.output, r.compiled.result, r.compiled.output
        ));
    }
    (r, bad)
}

fn assert_clean(bad: &[String]) {
    for b in bad {
        eprintln!("MISMATCH {b}");
    }
    assert!(bad.is_empty(), "{} mismatches: {bad:?}", bad.len());
}

/// The d19 form: chains where the last result is the left input, the right input, or both
/// (`-` and `/` are not commutative, so a wrong operand order shows), across an element
/// load (which keeps d19), and across a call (which must not: the callee may change d19)
/// and a block edge (a branch merge clears it). Doubles with NaN, -0 and infinities.
#[test]
fn double_chains_read_the_last_result_from_d19() {
    if !aarch64() {
        return;
    }
    let defs = "\
function g() { return 0; }\n\
function chain(a, b, c, n) {\n\
  var s = 0.5;\n\
  for (var i = 0; i < n; i++) {\n\
    s = a[i] - b[i] * s;\n\
    s = c / (s + 0.25);\n\
    s = (s - a[i]) / s;\n\
    var t = s * s;\n\
    t = t - s;\n\
    g();\n\
    s = t + s * 0.5;\n\
    if (a[i] < 1.5) s = s - 1.5; else s = 2.5 - s;\n\
    s = s * 1.25 - a[i];\n\
  }\n\
  return s;\n\
}\n";
    let warm = "for (var w = 0; w < 30; w++) chain([0.5, 1.5, 2.5], [0.25, 0.75, 1.25], 3.5, 3);";
    let main = "\
print(chain([0.5, 1.5, 2.5], [0.25, 0.75, 1.25], 3.5, 3), chain([1.5, 0.5], [2.5, 0.5], -0.5, 2));\n\
print(chain([0.5, NaN], [1.5, 0.5], 3.5, 2), chain([-0.5, 1e300], [1e300, 1e-300], 1e-300, 2));\n\
print(1 / chain([0.5], [0.5], -0, 1), chain([0.5, 1.5], [0.5, 1.5], Infinity, 2));\n";
    let (r, bad) = check("chain", defs, warm, main, "chain");
    assert_clean(&bad);
    let g = &r.report.graph;
    for op in [
        "Float64(Sub)",
        "Float64(Div)",
        "Float64(Mul)",
        "Float64(Add)",
        "Call",
    ] {
        assert!(g.contains(op), "no {op}\n{g}");
    }
    assert!(r.fired().is_empty(), "unforced exits {:?}", r.exits);
}

/// The block of the node `n<id>` in a graph text (`b<k> @...` header lines).
fn block_of(graph: &str, pat: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let (mut block, mut header) = (String::new(), false);
    for l in graph.lines() {
        if let Some(rest) = l.strip_prefix('b') {
            block = format!("b{}", rest.split(' ').next().unwrap_or(""));
            header = l.contains(" loop preds");
        } else if l.contains(pat) {
            out.push((block.clone(), header));
        }
    }
    out
}

/// A closure's loop reads an outer `var` (a context slot) in its condition and a number
/// parameter in its body: with no call and only number stores in the loop, the slot load,
/// its int32 check and the number check move to the preheader, once per loop entry (not
/// one per iteration). Equal to T0 in every build; then a call whose loop runs 0 times with
/// a slot that holds no int32 exits at the moved check (the loop entry) and T0 runs on.
#[test]
fn a_context_slot_load_and_its_checks_move_out_of_a_call_free_loop() {
    if !aarch64() {
        return;
    }
    let defs = "\
function mk(w) {\n\
  return [function fill(a, f) { for (var i = 0; i < w; i++) { a[i] = a[i] * f + i; } return a; },\n\
          function setw(v) { w = v; }];\n\
}\n\
var p = mk(4), fill = p[0], setw = p[1];\n";
    let warm = "for (var q = 0; q < 30; q++) fill([0.5, 1.5, 2.5, 3.5, 4.5], 0.5);";
    let main = "print(fill([0.5, 1.5, 2.5, 3.5, 4.5], 1.5).join(), fill([1, 2, 3, 4], 2).join());";
    let (r, bad) = check("fill", defs, warm, main, "fill");
    assert_clean(&bad);
    let g = &r.report.graph;
    let loads = block_of(g, "LoadContextSlot");
    assert!(!loads.is_empty(), "no slot load\n{g}");
    for (b, header) in &loads {
        assert!(!header, "a slot load in the loop header {b}\n{g}");
    }
    let header = g
        .lines()
        .find(|l| l.contains(" loop preds"))
        .expect("a loop");
    let pre = loads[0].0.clone();
    assert!(
        header.contains(&format!("preds [{pre} ")),
        "the slot load is not in the loop's preheader {pre}\n{g}"
    );
    for (b, _) in block_of(g, "CheckNumber(n2)") {
        assert_eq!(b, pre, "the number check of `f` stays in the loop\n{g}");
    }
    assert!(r.fired().is_empty(), "unforced exits {:?}", r.exits);
    // The slot holds a string, and the loop runs 0 times (`i < 'x'` is false): the moved
    // int32 check exits at the loop entry, and T0 runs the call to its end.
    let main =
        "setw('x'); print(fill([0.5], 2).join()); setw(2); print(fill([0.5, 1.5], 2).join());";
    let r = run_script(&Config::default(), defs, warm, main, "fill").expect("run");
    assert!(r.equal(), "{:?} vs {:?}", r.compiled, r.t0);
    assert_eq!(r.compiled.output, ["0.5", "1,4"]);
    assert_eq!(r.exits.iter().sum::<u64>(), 1, "{:?}", r.exits);
}

/// `mov64` takes `movn` when more halfwords are 0xFFFF than 0 (the DOUBLE hole: 2
/// instructions), else `movz` as before; each form makes the same value.
#[test]
fn mov64_takes_movn_for_mostly_ones() {
    let enc = crate::emit_a64::mov64_words;
    let mut want = dynasmrt::VecAssembler::<dynasmrt::aarch64::Aarch64Relocation>::new(0);
    dynasmrt::dynasm!(want
        ; .arch aarch64
        ; movn x15, 0x8, lsl 16
        ; movk x15, 0xfff7, lsl 48
    );
    let want = want.finalize().expect("code");
    assert_eq!(enc(15, 0xFFF7_FFFF_FFF7_FFFF), want);
    assert_eq!(enc(15, u64::MAX).len(), 4, "movn x15, #0");
    assert_eq!(enc(15, 0).len(), 4, "movz x15, #0");
    assert_eq!(enc(15, 0xFFFE_0000_0000_0000).len(), 4, "movz, as before");
    // The value each form makes, decoded from the instruction words (bits 31-23: 64-bit
    // movz 0b110100101, movn 0b100100101, movk 0b111100101).
    for v in [
        0xFFF7_FFFF_FFF7_FFFF_u64,
        u64::MAX,
        0,
        0xFFFF_FFFF_0000_FFFF,
        0x0000_FFFF_FFFF_FFFF,
        0x1234_FFFF_FFFF_5678,
        0xFFFE_0000_0000_0002,
        0x0002_0000_0000_0000,
    ] {
        let mut x = 0u64;
        for w in enc(15, v).chunks(4) {
            let w = u32::from_le_bytes(w.try_into().expect("a word"));
            assert_eq!(w & 31, 15, "{v:#x}: the target register");
            let (imm, s) = (u64::from((w >> 5) & 0xFFFF), 16 * ((w >> 21) & 3));
            x = match w >> 23 {
                0b1_1010_0101 => imm << s,
                0b1_0010_0101 => !(imm << s),
                0b1_1110_0101 => (x & !(0xFFFF << s)) | imm << s,
                op => panic!("{v:#x}: not a mov word {w:#010x} (bits 31-23 {op:#b})"),
            };
        }
        assert_eq!(x, v, "{v:#x}");
    }
}

/// MC-P1-JIT3 and MC-w25-6 (SPIKEC-10). `a[i] = a[i] * f + i`: the load site has no kind yet
/// (its IC recorded none), the store site saw DOUBLE. One DOUBLE site and no other kind is
/// enough for the DoubleElements of the moved kind guard (before: every site needed a
/// DOUBLE-only set, so this loop took the full element path). The moved `CheckArray`'s exit
/// record names the keyed slot of its element site. Equal to T0 in every build.
#[test]
fn a_site_with_no_kind_yet_keeps_the_double_cell_and_the_kind_guard_names_its_site() {
    if !aarch64() {
        return;
    }
    let defs = "function fill(a, f, n) { for (var i = 0; i < n; i++) { a[i] = a[i] * f + i; } return a; }\n";
    let warm = "for (var q = 0; q < 30; q++) fill([0.5, 1.5, 2.5, 3.5, 4.5], 0.5, 5);";
    let main = "print(fill([0.5, 1.5, 2.5, 3.5, 4.5], 1.5, 5).join(), fill([1, 2, 3, 4], 2, 4).join());\n\
print(fill([0.5, , 2.5], 2, 3).join(), fill([0.5, 1.5], 2, 4).join(), fill([], 2, 0).join());";
    let (r, bad) = check("jit3", defs, warm, main, "fill");
    assert_clean(&bad);
    let g = &r.report.graph;
    assert!(g.contains("DoubleElements("), "no DoubleElements\n{g}");
    let kind_guards: Vec<u32> = g
        .lines()
        .filter(|l| l.contains("= CheckArray("))
        .filter_map(|l| l.split(" guard g").nth(1))
        .filter_map(|t| t.split(' ').next()?.parse().ok())
        .collect();
    assert!(!kind_guards.is_empty(), "no kind guard\n{g}");
    for k in kind_guards {
        let e = r.report.emit.meta.exits.iter().find(|e| e.guard == k);
        let e = e.unwrap_or_else(|| panic!("no exit record for g{k}"));
        assert!(
            e.feedback.is_some(),
            "g{k}: the kind guard names no slot: {e:?}"
        );
    }
}
