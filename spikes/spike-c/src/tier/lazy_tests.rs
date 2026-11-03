//! The short lazy-deopt probe tests (SPIKEC-10, H29-Q4; [`crate::lazy`]). macOS arm64 only:
//! the frame-record walk needs a frame record in every frame, which the platform ABI makes
//! sure of; elsewhere they print a skip line (the CFI walk is the gated part of H29-Q4).

use spike_b::harness::Config;

use super::{LazyRun, lazy_probe};

fn macos_aarch64() -> bool {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return true;
    }
    eprintln!("skip: the frame-record walk of the probe is checked on macOS arm64 only");
    false
}

use crate::lazy::{DEFS, MAIN, WARM};

/// The checks every probe run must pass: one record swapped, from the call's return to its
/// lazy exit; the walk after the swap goes through the swapped frame to the same outer
/// frames; T0's output; one lazy exit counted and no guard exit.
fn check_swap(r: &LazyRun) {
    assert_eq!(
        r.compiled, r.t0,
        "compiled {:?} vs T0 {:?}",
        r.compiled, r.t0
    );
    let swap = r.swap.as_ref().expect("lazyDeopt() ran armed");
    let calls = &r.report.emit.meta.calls;
    assert_eq!(calls.len(), 1, "one call site: {calls:?}");
    let (ret, stub) = (
        r.code.start + calls[0].ret as usize,
        r.code.start + calls[0].stub as usize,
    );
    assert_eq!(swap.swapped.len(), 1, "{swap:?}");
    assert_eq!(swap.swapped[0].1, ret);
    // The walks start in two places of one function; from the second return on they are the
    // same chain, but for the swapped address.
    let (b, a) = (&swap.before[1..], &swap.after[1..]);
    assert_eq!(
        b.len(),
        a.len(),
        "the walk ends in the same place: {b:x?} vs {a:x?}"
    );
    let at = b
        .iter()
        .position(|&x| x == ret)
        .expect("the call's return in the walk");
    assert_eq!(a[at], stub, "the swapped record holds the lazy exit");
    for (i, (x, y)) in b.iter().zip(a).enumerate() {
        if i != at {
            assert_eq!(x, y, "frame {i} changed");
        }
    }
    assert!(at + 1 < a.len(), "frames past the swapped one: {a:x?}");
    assert_eq!(r.call_counts(), [1], "{:?}", r.exits);
    assert!(
        r.exit_counts().iter().all(|&n| n == 0),
        "guard exits {:?}",
        r.exits
    );
}

/// The probe: the swap of one `Op::Call` frame under placement (b), a backtrace through it,
/// and T0 resumes after the call with the call's result and the live values rebuilt from
/// the call record. A later call (not armed) runs the compiled code again.
#[test]
fn a_swapped_call_frame_returns_into_its_lazy_exit() {
    if !macos_aarch64() {
        return;
    }
    let r = lazy_probe(&Config::default(), DEFS, WARM, MAIN, "f").expect("probe");
    check_swap(&r);
    assert_eq!(r.compiled.output, ["19.25", "21.375", "24.25"]);
    // The system unwinder (CFI only) is not asked to pass JIT code: no FDE is registered
    // for it. The probe reports what it printed; the frame-record walk above is the check.
    let s = r.swap.as_ref().expect("swap");
    eprintln!(
        "std::backtrace frames before {:?} after {:?}; frame records {}",
        s.std_before,
        s.std_after,
        s.after.len()
    );
    let g = &r.report.graph;
    assert!(g.contains("lazy -> @"), "{g}");
}

/// The callee throws after the swap: the lazy exit sees 0 (an exception pending) and unwinds
/// from the call, as the call site does; the caller's `catch` gets it as in T0.
#[test]
fn a_swapped_call_that_throws_unwinds_from_the_call() {
    if !macos_aarch64() {
        return;
    }
    let defs = "\
var boom = false;\n\
function g(x) { lazyDeopt(); if (boom) throw new Error('e' + x); return x * 2 + 0.5; }\n\
function f(a, b) { var s = a + 1; var t = b * 1.5; var r = g(s); return r + s + t + a; }\n";
    let main = "boom = true; try { print(f(3, 2.5)); } catch (e) { print('caught', e.message); } boom = false; print(f(1, 0.5));";
    let r = lazy_probe(&Config::default(), defs, WARM, main, "f").expect("probe");
    assert_eq!(r.compiled, r.t0, "{:?} vs {:?}", r.compiled, r.t0);
    assert_eq!(r.compiled.output, ["caught e4", "8.25"]);
    let s = r.swap.as_ref().expect("swap");
    assert_eq!(s.swapped.len(), 1);
    assert_eq!(
        r.call_counts(),
        [0],
        "an unwind is no lazy exit: {:?}",
        r.exits
    );
}

/// GC stress (a collection before every allocation, the heap verifier after): the callee
/// allocates after the swap (each allocation collects), and the rebuilt frame holds its
/// result and the live values.
#[test]
fn a_swapped_call_frame_under_gc_stress() {
    if !macos_aarch64() {
        return;
    }
    let defs = "\
function g(x) { lazyDeopt(); var o = { v: x, w: [x, 'k' + x] }; return o.v * 2 + o.w.length - 1.5; }\n\
function f(a, b) { var s = a + 1; var t = b * 1.5; var r = g(s); return r + s + t + a; }\n";
    let main = "print(f(3, 2.5)); print(f(4, 1.25));";
    let r = lazy_probe(&Config::stress(), defs, WARM, main, "f").expect("probe");
    check_swap(&r);
    assert_eq!(r.compiled.output, ["19.25", "21.375"]);
}
