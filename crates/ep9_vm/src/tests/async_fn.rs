//! Tests of async functions (runtime ids 32-35, `interp/slow/async_fn.rs`) (owner M3-P1
//! ASYNC). ep9_vm tests cannot compile JS and have no `%Promise%` (ep9_builtins makes it), so
//! these hand-built units check the routing and the checks of each id; the JS-level tests
//! (await chains, job order, GC stress) are in `crates/ep9_builtins/tests/async_fn.rs` and the
//! e2e programs in `crates/ep9_cli/tests/e2e/async/`.

use ep9_bytecode::{FunctionFlags, Register, RegisterList, RuntimeId, VerifiedUnit};

use super::{Asm, config, meta, on_big_stack, run_in, unit};
use crate::Dispatch;
use crate::exotic::probe;

/// Runs the unit from `make` under every dispatch expansion, with and without GC stress;
/// returns each run's label, error text and probe hits.
fn runs(
    make: impl Fn() -> VerifiedUnit + Copy + Send + 'static,
) -> Vec<(String, String, Vec<&'static str>)> {
    let mut out = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, hits) = on_big_stack(move || {
                probe::take();
                let r = run_in(config(d, stress), make());
                (r, probe::take())
            });
            let e = r.result.expect_err("throws").to_string();
            out.push((format!("{} stress={stress}", d.name()), e, hits));
        }
    }
    out
}

/// `CallRuntime` of ids 32-35 reaches `slow/async_fn.rs` (P0-VM3 routing); outside an async
/// function body each id refuses its operands with a catchable internal error (a
/// BytecodeGen bug, never UB).
#[test]
fn ids_32_to_35_reach_async_fn_and_check_their_operands() {
    const IDS: [(RuntimeId, &str); 4] = [
        (
            RuntimeId::AsyncFunctionEnter,
            "AsyncFunctionEnter outside its async function",
        ),
        (
            RuntimeId::AsyncFunctionAwait,
            "AsyncFunctionAwait of a non-async-function object",
        ),
        (
            RuntimeId::AsyncFunctionResolve,
            "AsyncFunctionResolve of a non-async-function object",
        ),
        (
            RuntimeId::AsyncFunctionReject,
            "AsyncFunctionReject of a non-async-function object",
        ),
    ];
    let ids: Vec<u16> = IDS.iter().map(|&(id, _)| id as u16).collect();
    assert_eq!(ids, [32, 33, 34, 35]);
    for (id, text) in IDS {
        let info = id.info();
        let argc = info.arity.expect("fixed arity");
        let make = move || {
            let mut s = Asm::script(0);
            let args = s.b.new_register_list(argc);
            for i in 0..argc {
                s.b.lda_smi(i as i32 + 1).star(args.get(i));
            }
            s.b.call_runtime(id, args, argc).return_();
            unit("", vec![s.finish()])
        };
        let hook = format!("async_fn {}", info.name);
        for (label, e, hits) in runs(make) {
            assert_eq!(
                e,
                format!("test.js:1:1: Error: ep9 internal error: {text}"),
                "{} {label}",
                info.name
            );
            assert_eq!(hits, [hook.as_str()], "{} {label}", info.name);
        }
    }
}

/// An async function body (flag ASYNC) passes the checks of `AsyncFunctionEnter`; without
/// `%Promise%` (no ep9_builtins tables) it is not implemented, as a catchable error.
#[test]
fn enter_in_an_async_function_needs_promise() {
    fn make() -> VerifiedUnit {
        // async function f() {}: AsyncFunctionEnter(closure, this) only.
        let mut m = meta("f", 0, 0, 0, 0);
        m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::ASYNC;
        let mut f = Asm::new(m);
        let t = f.b.new_register_list(2);
        f.b.lda_current_function().star(t.get(0));
        f.b.mov(Register::RECEIVER, t.get(1));
        f.b.call_runtime(RuntimeId::AsyncFunctionEnter, t, 2)
            .return_();
        let mut s = Asm::script(0);
        let k = s.b.function_constant(ep9_bytecode::FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.b.star(s.f);
        s.b.call_undefined_receiver(s.f, RegisterList::EMPTY, 0, s.call);
        s.b.return_();
        unit("", vec![s.finish(), f.finish()])
    }
    for (label, e, hits) in runs(make) {
        assert_eq!(
            e, "test.js:1:1: Error: not implemented in ep9 v0: async functions without %Promise%",
            "{label}"
        );
        assert_eq!(hits, ["async_fn AsyncFunctionEnter"], "{label}");
    }
}
