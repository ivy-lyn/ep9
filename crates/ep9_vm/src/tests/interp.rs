//! Dispatch: expansions, no Rust recursion for JS calls (gate "frames-no-rust-recursion"),
//! prefixed operands, dispatch slots and the code registry, random verified units, and the
//! sample unit of ep9_bytecode.

use ep9_bytecode::{FeedbackSlotKind, FunctionId, VerifiedUnit};

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

/// The environment variable that makes `scaled_steps_fit_a_small_stack` run its child part;
/// its value is the child's native stack size in bytes.
const SCALED_CHILD: &str = "EP9_SCALED_REENTRY_CHILD";

/// The native stack of the child of `scaled_steps_fit_a_small_stack`: 2 MiB (the main-thread
/// size of many embedders, and the size of the reproduction in the M4-P1 Codex finding).
const SCALED_STACK: usize = 2 << 20;

/// inner.js: `1.5 + 1000 + 100000` with a `Wide` and an `ExtraWide` AddSmi.
fn reenter_inner<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut s = Asm::script(0);
    s.lda_num(1.5);
    s.b.add_smi(1000, s.bin).add_smi(100_000, s.bin).return_();
    cx.run_script(unit("", vec![s.finish()]), "inner.js")
}

/// mid.js: `return reenter_inner()` through an `ExtraWide` CallUndefinedReceiver0.
fn reenter_mid<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.run_script(scaled_call_unit("reenter_inner", 1 << 16), "mid.js")
}

/// A script `return name()` whose call slot comes after `pad` other slots, so that the call
/// has a `Wide` (pad >= 256) or `ExtraWide` (pad >= 65536) prefix.
fn scaled_call_unit(name: &str, pad: usize) -> VerifiedUnit {
    let mut s = Asm::script(0);
    s.lda_global(name);
    s.b.star(s.f);
    for _ in 0..pad {
        s.b.feedback_slot(FeedbackSlotKind::BinaryOp);
    }
    let k = s.b.feedback_slot(FeedbackSlotKind::Call);
    s.b.call_undefined_receiver0(s.f, k).return_();
    unit("", vec![s.finish()])
}

/// The native frames of the prefixed steps (`step::<S2>`, `step::<S4>`) bound the native
/// stack that host re-entry needs: a `Wide` call to a host function that runs a script with
/// an `ExtraWide` call to a host function that runs a script with `Wide` and `ExtraWide`
/// arithmetic keeps three step frames live, each under a match loop frame. Under the match
/// loop (the debug default, unoptimized frames) this fits in a 2 MiB thread, with feedback
/// recording on (threshold 0). The type-feedback records of the steps go through the debug
/// out-of-line `Flow::recorded`, as in the match loop: inline, they tripled the debug frame
/// of `step::<S2>` (M4-P1 Codex finding, 279 KiB to 821 KiB on aarch64) and this run
/// overflowed the stack. An overflow aborts the process, so the run is a child process.
#[test]
fn scaled_steps_fit_a_small_stack() {
    if let Some(bytes) = std::env::var_os(SCALED_CHILD) {
        let bytes: usize = bytes.to_str().unwrap().parse().unwrap();
        let r = std::thread::Builder::new()
            .stack_size(bytes)
            .spawn(|| {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: 0,
                    ..config(Dispatch::Match, false)
                });
                iso.register_host_function("reenter_inner", 0, reenter_inner);
                iso.register_host_function("reenter_mid", 0, reenter_mid);
                iso.with_cx_checkpoint(|cx| {
                    cx.run_script(scaled_call_unit("reenter_mid", 256), "outer.js")
                        .map(|v| cx.display(v))
                })
            })
            .unwrap()
            .join()
            .unwrap();
        println!("SCALED_REENTRY {r:?}");
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::interp::scaled_steps_fit_a_small_stack",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCALED_CHILD, SCALED_STACK.to_string())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("SCALED_REENTRY Ok(\"101001.5\")"),
        "{:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
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

/// One case of [`prefixed_smi_ops_in_the_fast_body`]: an opcode, the int32 accumulator (or a
/// double, `None` for `LdaSmi`), the immediate, and the prefix the immediate needs.
type SmiCase = (
    ep9_bytecode::Opcode,
    Option<f64>,
    i32,
    ep9_bytecode::OperandScale,
);

/// CX1-N1 (M3-P4 fix round 2): the `Wide` and `ExtraWide` prefix handlers run `LdaSmi` and the
/// int32 cases of `AddSmi`, `SubSmi`, `ModSmi`, `BitwiseOrSmi`, `BitwiseXorSmi` and
/// `BitwiseAndSmi` in their fast body (`interp/ops.rs` `scaled_fast`, M3-P4 FIXES). The unit
/// has each of them with a 16-bit and a 32-bit immediate (the decoded scale is checked), plus
/// the cases that leave the fast body for the cold stepper (an overflow, a negative `%`
/// dividend, a double accumulator). Every dispatch expansion, with and without GC stress,
/// must print the JS values: a fast body that gives a wrong value fails the test.
#[test]
fn prefixed_smi_ops_in_the_fast_body() {
    use ep9_bytecode::{Instructions, Opcode as O, OperandScale as S};
    const CASES: &[SmiCase] = &[
        (O::LdaSmi, None, 1000, S::Wide),
        (O::LdaSmi, None, -32768, S::Wide),
        (O::LdaSmi, None, 100_000, S::ExtraWide),
        (O::LdaSmi, None, i32::MIN, S::ExtraWide),
        (O::AddSmi, Some(5.0), 1000, S::Wide),
        (O::AddSmi, Some(-7.0), -40_000, S::ExtraWide),
        (O::SubSmi, Some(5.0), 1000, S::Wide),
        (O::SubSmi, Some(5.0), 100_000, S::ExtraWide),
        (O::ModSmi, Some(123_456.0), 1000, S::Wide),
        (O::ModSmi, Some(7_654_321.0), 100_003, S::ExtraWide),
        (O::BitwiseOrSmi, Some(1.0), 4096, S::Wide),
        (O::BitwiseOrSmi, Some(1.0), 65_536, S::ExtraWide),
        (O::BitwiseXorSmi, Some(255.0), 1000, S::Wide),
        (O::BitwiseXorSmi, Some(255.0), 70_000, S::ExtraWide),
        (O::BitwiseAndSmi, Some(123_456.0), 1023, S::Wide),
        (
            O::BitwiseAndSmi,
            Some(2_147_483_647.0),
            65_535,
            S::ExtraWide,
        ),
        // The cold stepper: overflow, a negative dividend, a double accumulator.
        (O::AddSmi, Some(2_147_483_647.0), 1000, S::Wide),
        (O::SubSmi, Some(-2_147_483_648.0), 100_000, S::ExtraWide),
        (O::ModSmi, Some(-7.0), 1000, S::Wide),
        (O::AddSmi, Some(1.5), 100_000, S::ExtraWide),
    ];
    // The JS value of each case (all are integers or exact halves).
    fn want(op: O, acc: Option<f64>, imm: i32) -> String {
        let (a, b) = (acc.unwrap_or(0.0), f64::from(imm));
        let i = |x: f64| x as i32;
        let v = match op {
            O::LdaSmi => b,
            O::AddSmi => a + b,
            O::SubSmi => a - b,
            O::ModSmi => a % b,
            O::BitwiseOrSmi => f64::from(i(a) | imm),
            O::BitwiseXorSmi => f64::from(i(a) ^ imm),
            O::BitwiseAndSmi => f64::from(i(a) & imm),
            _ => unreachable!(),
        };
        if v.fract() == 0.0 {
            format!("{}", v as i64)
        } else {
            format!("{v}")
        }
    }
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        for &(op, acc, imm, _) in CASES {
            match acc {
                None => {}
                Some(a) if a.fract() == 0.0 && i32::try_from(a as i64).is_ok() => {
                    // A one-byte or prefixed LdaSmi of the accumulator is fine: the case is
                    // the op after it.
                    s.b.lda_smi(a as i32);
                }
                Some(a) => {
                    let k = s.b.number_constant(a);
                    s.b.lda_constant(k);
                }
            }
            let bin = s.bin;
            match op {
                O::LdaSmi => s.b.lda_smi(imm),
                O::AddSmi => s.b.add_smi(imm, bin),
                O::SubSmi => s.b.sub_smi(imm, bin),
                O::ModSmi => s.b.mod_smi(imm, bin),
                O::BitwiseOrSmi => s.b.bitwise_or_smi(imm, bin),
                O::BitwiseXorSmi => s.b.bitwise_xor_smi(imm, bin),
                O::BitwiseAndSmi => s.b.bitwise_and_smi(imm, bin),
                _ => unreachable!(),
            };
            s.print_acc();
        }
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    // The bytecode has each case op with its prefix, in order (an accumulator load of an
    // int32 past i8 is a prefixed LdaSmi too).
    let scale = |v: i32| {
        if i8::try_from(v).is_ok() {
            S::Single
        } else if i16::try_from(v).is_ok() {
            S::Wide
        } else {
            S::ExtraWide
        }
    };
    let mut want_ops = Vec::new();
    for &(op, acc, imm, prefix) in CASES {
        assert_eq!(scale(imm), prefix, "{op:?} {imm}");
        if let Some(a) = acc
            && a.fract() == 0.0
            && let Ok(a) = i32::try_from(a as i64)
            && scale(a) != S::Single
        {
            want_ops.push((O::LdaSmi, scale(a)));
        }
        want_ops.push((op, prefix));
    }
    let u = make();
    let ops: Vec<(O, S)> = Instructions::new(&u.functions[0].bytecode)
        .map(|i| i.unwrap())
        .filter(|i| i.scale != S::Single && CASES.iter().any(|c| c.0 == i.opcode))
        .map(|i| (i.opcode, i.scale))
        .collect();
    assert_eq!(ops, want_ops);
    let r = run_all(make);
    ok(&r);
    let expect: Vec<String> = CASES.iter().map(|&(op, a, b, _)| want(op, a, b)).collect();
    assert_eq!(r.output, expect);
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
    // a switch, a template object, then `print(..., 1n, ...)`: the BigInt constant is a BIGINT
    // cell (spec-temporal B2), so the catch block does not run (total stays 3).
    for &d in Dispatch::ALL {
        let (r, rec, total, out) = on_big_stack(move || {
            let mut iso = isolate(config(d, true));
            iso.register_host_function("tag", 1, tag);
            take_output();
            let r = iso.with_cx_checkpoint(|cx| {
                cx.run_script(common::sample_script_unit(), "sample.js")
                    .map(|v| cx.display(v))
            });

            let total = iso.global_raw("total").map(|v| iso.display_raw(v));
            (
                r,
                RECORD.with(|x| core::mem::take(&mut *x.borrow_mut())),
                total,
                take_output(),
            )
        });
        assert_eq!(r.as_deref(), Ok("undefined"), "{}", d.name());
        assert_eq!(rec, ["tag n= n= 3"], "{}", d.name());
        assert_eq!(total.as_deref(), Some("3"), "{}", d.name());
        assert_eq!(out.len(), 1, "{}: {out:?}", d.name());
        assert_eq!(out[0].split(' ').nth(1), Some("1"), "{}: {out:?}", d.name());
    }
}

// ---- MC-w34-6: prefixed jumps and their feedback on each arch (M7-P5 Q6 DISPATCH) ----------

/// The runs of `interp::scaled::step` and `step_cold` on this thread so far.
fn scaled_runs() -> [u64; 2] {
    crate::interp::dsl::SCALED_RUNS.with(|c| c.get())
}

/// `let i = 0; while (i < 3) { if (false) { <far bytes> } i++; } print(i); return i;` with the
/// three jumps over the `far` bytes (one-byte `LdaZero` instructions that never run):
/// `JumpIfFalse` (to the loop exit), `Jump` (over the bytes) and `JumpLoop` (back to the loop
/// header).
fn far_jumps_unit(far: usize) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let header = s.b.bound_label();
    let (exit, skip) = (s.b.new_label(), s.b.new_label());
    s.b.lda_smi(3).test_less_than(i, s.cmp).jump_if_false(exit);
    s.b.jump(skip);
    for _ in 0..far {
        s.b.lda_zero();
    }
    s.b.bind(skip);
    s.b.ldar(i).inc(s.bin).star(i);
    s.b.jump_loop(header, 0, s.loop_);
    s.b.bind(exit);
    s.b.ldar(i);
    s.print_acc();
    s.b.ldar(i).return_();
    unit("", vec![s.finish()])
}

/// MC-w34-6 (2) (V8 0b4d4612c8: an ExtraWide `JumpLoop` offset that the interpreter and the
/// verifier read with different signs). The jumps of [`far_jumps_unit`] with 70,000 bytes
/// have offsets past 64 KiB, so each is an `ExtraWide` instruction (a 4-byte offset); with
/// 40,000 bytes each is a `Wide` instruction whose 2-byte offset has its top bit set, so a
/// decoder that sign-extends it jumps to the wrong place. Node prints 3 for the same script.
/// On aarch64 the `Wide` and `ExtraWide` fast bodies run all three (`ops.rs`
/// `prefix_runs_fast`): no run of `interp::scaled::step`. Elsewhere each runs in `step`: 3 per
/// loop turn and the exit `JumpIfFalse`. Every expansion, with and without GC stress.
#[test]
fn extra_wide_jumps_over_64_kib() {
    use ep9_bytecode::{Instructions, Opcode as O, OperandScale as S};
    for (far, scale) in [(70_000, S::ExtraWide), (40_000, S::Wide)] {
        let u = far_jumps_unit(far);
        let code = &u.functions[0].bytecode;
        let prefixed: Vec<(O, S)> = Instructions::new(code)
            .map(|i| i.unwrap())
            .filter(|i| i.scale != S::Single)
            .map(|i| (i.opcode, i.scale))
            .collect();
        assert_eq!(
            prefixed,
            [
                (O::JumpIfFalse, scale),
                (O::Jump, scale),
                (O::JumpLoop, scale)
            ]
        );
        // Each offset is past 32 KiB: a 2-byte offset has its top bit set.
        for i in Instructions::new(code).map(|i| i.unwrap()) {
            if i.scale != S::Single {
                assert!(i.operand(0) > 0x8000, "{:?} {}", i.opcode, i.operand(0));
            }
        }
        let steps = if crate::interp::ops::PREFIXED_FAST {
            0
        } else {
            3 * 3 + 1
        };
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let (r, n) = on_big_stack(move || {
                    let before = scaled_runs();
                    let r = super::run_in(config(d, stress), far_jumps_unit(far));
                    (r, scaled_runs()[0] - before[0])
                });
                let label = format!("{far} bytes: {} stress={stress}", d.name());
                assert_eq!(ok(&r), "3", "{label}");
                assert_eq!(r.output, ["3"], "{label}");
                assert_eq!(n, steps, "{label}: the runs of interp::scaled::step");
            }
        }
    }
}

/// The script of `prefixed_feedback_is_the_same_on_every_arch`: 300 number constants, so the
/// name `x` (constant 300) needs a `Wide` operand, and a loop body of more than 255 bytes, so
/// its jumps need one too.
///
/// ```text
/// o = {}; o.x = 2; sum = 0;
/// for (i = 0; i < 20; i++) {
///   t = o.x;                       // Wide GetNamedProperty
///   if (t) {                       // Wide JumpIfToBooleanFalse
///     sum = sum + t;
///     d = 0.5 + 1000;              // Wide AddSmi of a double (a NUMBER record)
///     <300 one-byte instructions>
///   }
/// }                                // Wide JumpIfFalse (the exit), Wide JumpLoop
/// print(sum, d); return sum;        // Wide LdaConstant (0.5) and LdaGlobal (print)
/// ```
fn prefixed_feedback_unit() -> VerifiedUnit {
    let mut s = Asm::script(0);
    for i in 0..300 {
        s.b.number_constant(f64::from(i));
    }
    let kx = s.b.string_constant("x");
    let half = s.b.number_constant(0.5);
    let (o, sum, i, t, d) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    let add = s.b.feedback_slot(ep9_bytecode::FeedbackSlotKind::BinaryOp);
    let inc = s.b.feedback_slot(ep9_bytecode::FeedbackSlotKind::BinaryOp);
    s.b.create_empty_object_literal().star(o);
    s.b.lda_smi(2).set_named_property(o, kx, s.set_named);
    s.b.lda_zero().star(sum).lda_zero().star(i);
    let header = s.b.bound_label();
    let (exit, skip) = (s.b.new_label(), s.b.new_label());
    s.b.lda_smi(20).test_less_than(i, s.cmp).jump_if_false(exit);
    s.b.get_named_property(o, kx, s.named).star(t);
    s.b.jump_if_to_boolean_false(skip);
    s.b.ldar(t).add(sum, s.bin).star(sum);
    s.b.lda_constant(half).add_smi(1000, add).star(d);
    for _ in 0..300 {
        s.b.lda_zero();
    }
    s.b.bind(skip);
    s.b.ldar(i).inc(inc).star(i);
    s.b.jump_loop(header, 0, s.loop_);
    s.b.bind(exit);
    s.b.ldar(sum).star(s.a.get(0));
    s.b.ldar(d).star(s.a.get(1));
    s.lda_global("print");
    s.b.star(s.f)
        .call_undefined_receiver2(s.f, s.a.get(0), s.a.get(1), s.call);
    s.b.ldar(sum).return_();
    unit("", vec![s.finish()])
}

/// The records of the FactSnapshot of [`prefixed_feedback_unit`] (`FactSnapshot::dump` without
/// its header and unit lines, which name the bytecode ABI and the unit hash).
const PREFIXED_FACTS: &str = r#"shapes 2
  s0 proto=intrinsic#0 kind=ordinary elements=packed-int32 flags={extensible} capacity=4 keys=[]
  s1 proto=intrinsic#0 kind=ordinary elements=packed-int32 flags={extensible} capacity=4 keys=["x" {w,e,c} int32 const]
records 9
  f0 slot 0 Call flags={}: count=0 uninitialized [] result={}
  f0 slot 1 BinaryOp flags={}: {int32}
  f0 slot 2 CompareOp flags={}: {int32}
  f0 slot 3 LoadGlobal flags={}: bindings={data-property} values={}
  f0 slot 5 LoadNamed flags={}: monomorphic [s1 own-field 0] values={}
  f0 slot 6 StoreNamed flags={}: monomorphic [s0 add-field s1 0] values={}
  f0 slot 11 JumpLoop flags={}: back-edges=1
  f0 slot 12 BinaryOp flags={}: {int32,number}
  f0 slot 13 BinaryOp flags={}: {int32}
"#;

/// MC-w34-6 (1): the feedback of a prefixed instruction must not depend on the path that runs
/// it. On aarch64 the `Wide` fast body runs the prefixed GetNamedProperty and jumps
/// (`ops.rs` `prefix_runs_fast`); x86-64 runs them in `interp::scaled::step`. FASTDIFF compares
/// printed output only, so this test compares the FactSnapshot records (the slot kinds and
/// their facts) with one text, [`PREFIXED_FACTS`], on every target that runs it (CI: 4). Every
/// expansion, with and without GC stress.
#[test]
fn prefixed_feedback_is_the_same_on_every_arch() {
    use ep9_bytecode::{Instructions, Opcode as O, OperandScale as S};
    let u = prefixed_feedback_unit();
    let wide: Vec<O> = Instructions::new(&u.functions[0].bytecode)
        .map(|i| i.unwrap())
        .filter(|i| i.scale == S::Wide)
        .map(|i| i.opcode)
        .collect();
    assert_eq!(
        wide,
        [
            O::SetNamedProperty,
            O::JumpIfFalse,
            O::GetNamedProperty,
            O::JumpIfToBooleanFalse,
            O::LdaConstant,
            O::AddSmi,
            O::JumpLoop,
            O::LdaGlobal,
        ]
    );
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, out, facts) = on_big_stack(move || {
                // The type feedback `full`: the exporter keeps the record kinds too.
                let mut iso = super::profile::isolate(IsolateConfig {
                    feedback_threshold: 0,
                    ..config(d, stress)
                });
                let u = prefixed_feedback_unit();
                let hash = u.content_hash();
                // The budget runs out at a back edge of the loop: the sampled JumpLoop record
                // (`profile::record_jump_loop`) counts it, in the fast body that runs the
                // prefixed JumpLoop.
                iso.cx.budget = 5;
                take_output();
                let r = iso
                    .with_cx_checkpoint(|cx| cx.run_script(u, "facts.js").map(|v| cx.display(v)));
                let out = take_output();
                let dump = crate::abi::export_facts(&iso, &hash)
                    .expect("the unit is loaded")
                    .dump();
                let facts: String = dump.lines().skip(2).map(|l| format!("{l}\n")).collect();
                (r, out, facts)
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(r.as_deref(), Ok("40"), "{label}");
            assert_eq!(out, ["40 1000.5"], "{label}");
            assert_eq!(facts, PREFIXED_FACTS, "{label}");
        }
    }
}
