//! Dispatch: expansions, no Rust recursion for JS calls (gate "frames-no-rust-recursion"),
//! prefixed operands, dispatch slots and the code registry, random verified units, and the
//! sample unit of ep9_bytecode.

use ep9_bytecode::{FunctionId, VerifiedUnit};

use super::{Asm, common, config, isolate, meta, ok, on_big_stack, run_all, take_output, unit};
use crate::dispatch::{CodeRegistry, DispatchKind};
use crate::{Args, Cx, Dispatch, IsolateConfig, JsResult, Local};

#[test]
fn expansions_and_defaults() {
    assert!(Dispatch::ALL.contains(&Dispatch::Match));
    assert!(Dispatch::ALL.contains(&Dispatch::Sibling));
    #[cfg(feature = "become")]
    assert!(Dispatch::ALL.contains(&Dispatch::Become));
    // H15 (T0 result, 2026-10-21): sibling calls are the release expansion on every target.
    let release = Dispatch::Sibling;
    assert_eq!(Dispatch::RELEASE, release);
    let default = if cfg!(debug_assertions) {
        Dispatch::Match
    } else {
        release
    };
    assert_eq!(Dispatch::DEFAULT, default);
    // The handler count that `cargo xtask disasm-check --expect` checks per expansion.
    let n = crate::internals::THREADED_HANDLERS;
    assert_eq!(
        n,
        ep9_bytecode::OPCODE_COUNT + crate::internals::COLD_OPCODES + 3
    );
    eprintln!("threaded handlers per expansion: {n}");
}

fn sp<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::number(ep9_platform::stack_pointer() as f64))
}

/// f1: function deep(n) { return n === 0 ? sp() : deep(n - 1); }
/// script: return [deep(0) - deep(N), deep(N)]  (the native stack pointer at both depths).
fn deep_unit(n: i32) -> VerifiedUnit {
    let mut f = Asm::new(meta("deep", 1, 0, 0, 0));
    let p = f.b.parameter(0);
    let base = f.b.new_label();
    f.b.lda_zero()
        .test_equal_strict(p, f.cmp)
        .jump_if_true(base);
    f.lda_global("deep");
    f.b.star(f.f).ldar(p).sub_smi(1, f.bin).star(f.a.get(0));
    f.b.call_undefined_receiver1(f.f, f.a.get(0), f.call)
        .return_();
    f.b.bind(base);
    f.lda_global("sp");
    f.b.star(f.f)
        .call_undefined_receiver0(f.f, f.call)
        .return_();

    let mut s = Asm::script(0);
    let t = s.b.new_register();
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("deep");
    for (i, depth) in [0, n].into_iter().enumerate() {
        s.lda_global("deep");
        s.b.star(s.f).lda_smi(depth).star(s.a.get(0));
        s.b.call_undefined_receiver1(s.f, s.a.get(0), s.call);
        if i == 0 {
            s.b.star(t);
        } else {
            s.b.sub(t, s.bin);
        }
    }
    s.b.return_();
    unit("", vec![s.finish(), f.finish()])
}

fn run_deep(d: Dispatch, n: i32, native_stack: usize) -> String {
    std::thread::Builder::new()
        .stack_size(native_stack)
        .spawn(move || {
            let mut iso = isolate(IsolateConfig {
                vm_stack_slots: 1 << 22,
                ..config(d, false)
            });
            iso.register_host_function("sp", 0, sp);
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(deep_unit(n), "deep.js")
                    .map(|v| cx.display(v))
            });
            match r {
                Ok(v) => v,
                Err(e) => format!("exception: {e}"),
            }
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn js_calls_do_not_recurse_in_rust() {
    // `sp()` sees the same native stack pointer at JS depth 0 and at JS depth 100 000:
    // calls and returns between bytecode functions push VM frames only. The thread has a
    // 512 KiB native stack, far less than 100 000 Rust frames would need.
    assert_eq!(run_deep(Dispatch::DEFAULT, 100_000, 512 << 10), "0");
    // Without optimization the sibling expansion makes real calls (H15), so it joins in
    // release builds only.
    if !cfg!(debug_assertions) {
        for &d in Dispatch::ALL {
            assert_eq!(run_deep(d, 100_000, 512 << 10), "0", "{}", d.name());
        }
    }
}

#[test]
fn wide_and_extra_wide_operands() {
    fn make() -> VerifiedUnit {
        // f1: function id(x) { return x; }
        let mut id = Asm::new(meta("id", 1, 0, 0, 0));
        id.b.ldar(id.b.parameter(0)).return_();
        let mut s = Asm::script(0);
        // 300 registers and 300 constants: register and constant operands need `Wide`.
        let regs = s.b.new_register_list(300);
        let (last, fwide, awide) = (regs.get(299), regs.get(298), regs.get(297));
        let consts: Vec<_> = (0..300)
            .map(|i| s.b.number_constant(f64::from(i)))
            .collect();
        s.b.lda_smi(100_000).star(last); // ExtraWide immediate, Wide register
        s.b.lda_constant(consts[299]).add(last, s.bin).star(last); // 100299
        s.b.ldar(last).add_smi(-40_000, s.bin).star(last); // ExtraWide: 60299
        // A forward jump over more than 255 bytes needs a Wide offset.
        let over = s.b.new_label();
        s.b.jump(over);
        for _ in 0..300 {
            s.b.lda_zero();
        }
        s.b.bind(over);
        // A call through wide registers: the prefixed step switches frames.
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(fwide);
        s.b.ldar(last).star(awide);
        s.b.call_undefined_receiver1(fwide, awide, s.call)
            .star(last);
        s.b.ldar(last);
        s.print_acc();
        s.b.ldar(last).return_();
        unit("", vec![s.finish(), id.finish()])
    }
    let u = make();
    let code = &u.functions[0].bytecode;
    assert!(code.contains(&(ep9_bytecode::Opcode::Wide as u8)));
    assert!(code.contains(&(ep9_bytecode::Opcode::ExtraWide as u8)));
    let r = run_all(make);
    assert_eq!(ok(&r), "60299");
    assert_eq!(r.output, ["60299"]);
}

#[test]
fn dispatch_slots_and_code_registry() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    f.b.lda_smi(1).return_();
    let mut s = Asm::script(0);
    s.b.lda_undefined().return_();
    let u = unit("", vec![s.finish(), f.finish()]);
    iso.with_cx_checkpoint(|cx| cx.run_script(u, "r.js").map(|_| ()))
        .unwrap();
    let rt = &*iso.rt;
    let units = rt.units.borrow();
    let reg = rt.registry.borrow();
    assert_eq!(reg.len(), units[0].infos.len());
    for info in units[0].infos.iter() {
        assert_eq!(info.slot.kind(), Some(DispatchKind::Interp));
        assert_eq!(info.slot.entry(), info.code.addr());
        for pc in [
            info.code.addr(),
            info.code.addr() + info.code_len as usize - 1,
        ] {
            let e = reg.lookup(pc).unwrap();
            assert_eq!(e.kind, DispatchKind::Interp);
            assert_eq!(e.function, core::ptr::from_ref(info).addr());
        }
    }
    // Host functions and stubs have no code: their slot kind is NO_CODE.
    assert!(rt.infos.borrow().iter().all(|i| i.slot.kind().is_none()));
    assert!(rt.construct_stub.slot.kind().is_none());
}

#[test]
fn random_verified_units_never_misbehave() {
    // Every verified unit runs to a value or an exception: no panic, no UB, a valid heap.
    // The termination flag is set first, so loops end at the first interrupt poll.
    let seeds = if cfg!(debug_assertions) { 400 } else { 2000 };
    let mut ran = 0;
    on_big_stack(move || {
        for chunk in (0..seeds).collect::<Vec<u64>>().chunks(50) {
            let stress = chunk[0] < 100;
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            for &seed in chunk {
                let ru = common::random_unit(seed);
                let Ok(u) = ru.build_in_order() else {
                    continue;
                };
                iso.terminate_handle().terminate();
                let _ = iso.with_cx_checkpoint(|cx| cx.run_script(u, "random.js").map(|_| ()));
                ran += 1;
            }
            take_output();
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        }
        assert!(ran > seeds as usize / 2, "only {ran} units built");
    });
}

fn tag<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let strings = args.get(0);
    let cooked = cx.get_v(strings, "0")?;
    let raw = cx.get_v(strings, "raw")?;
    let raw0 = cx.get_v(raw, "0")?;
    let line = format!(
        "tag {} {} {}",
        cx.to_rust_string(cooked)?,
        cx.to_rust_string(raw0)?,
        cx.to_rust_string(args.get(1))?
    );
    RECORD.with(|r| r.borrow_mut().push(line));
    Ok(cx.new_string("t")?.value())
}

thread_local! {
    static RECORD: core::cell::RefCell<Vec<String>> = const { core::cell::RefCell::new(Vec::new()) };
}

#[test]
fn sample_unit_of_ep9_bytecode_runs() {
    // The hand-written sample of crates/ep9_bytecode/tests/common: globals, a loop, a call,
    // a switch, a template object, then a BigInt constant (not in v0) whose "not implemented"
    // error the script's own catch block handles (total = -1).
    for &d in Dispatch::ALL {
        let (r, rec, total) = on_big_stack(move || {
            let mut iso = isolate(config(d, true));
            iso.register_host_function("tag", 1, tag);
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(common::sample_script_unit(), "sample.js")
                    .map(|v| cx.display(v))
            });

            let total = iso.global_raw("total").map(|v| iso.display_raw(v));
            (
                r,
                RECORD.with(|x| core::mem::take(&mut *x.borrow_mut())),
                total,
            )
        });
        assert_eq!(r.as_deref(), Ok("undefined"), "{}", d.name());
        assert_eq!(rec, ["tag n= n= 3"], "{}", d.name());
        assert_eq!(total.as_deref(), Some("-1"), "{}", d.name());
    }
}
