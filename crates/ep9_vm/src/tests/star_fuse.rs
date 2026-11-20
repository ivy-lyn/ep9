//! The Star lookahead (M9-P0 STAR-FUSE, M10-P2 and M10-P3 STAR-LA2; `interp::tails::star_next`,
//! `interp::tails::star_done`): a producer that is followed by a one-byte-scale `Star r`
//! stores the accumulator to `r` and dispatches past the Star. Each program runs under every
//! dispatch expansion, with and without GC stress, with the lookahead on and off
//! (`tails::star_count::OFF`, test builds only): all runs agree (the result, the output, and
//! an exception with its message, location and stack), and the fused Stars
//! (`tails::star_count::FUSED`) show that the on runs fused and the off runs did not.
//!
//! The plain Star handler (Codex sl2-r1 F3): the runs put a counting handler in the `Star`
//! entry of the isolate's dispatch table ([`counting_star`]). It records the pc of each Star
//! that runs in its own handler and then runs the Star handler of the table. In each threaded
//! expansion the Stars that the on run fused plus the Stars that ran in the Star handler are,
//! pc for pc, the Stars that ran in the Star handler in the off run. So a fused Star that
//! still dispatched to the Star handler (the store done twice: the same result) fails the
//! test. The match loop does not read the table, so there this count is empty.
//!
//! The producers of the M10-P3 pick (DL M6-ORCH-1205): the four context-slot loads,
//! `LdaCurrentContext`, `LdaConstant`, `LdaZero`, `LdaNull` and `LdaFalse`. `LdaSmi` and the
//! `GetNamedProperty` IC hit are no producers: a Star after them runs in the Star handler
//! (the cases have them as such Stars). The cases (plan v14 §3.4, plan v15 §5.1; DL
//! M9-PRE-STARFUSE-KIND): every producer kind; a throw near fused Stars (no fusion in the
//! throwing instruction; the Star register keeps its value; the throw position is the
//! thrower's); a GC after a fused Star of a heap constant; a Star whose register is the
//! producer's input; a Star that is a jump target (a jump to it runs the Star handler; only
//! the fall-through fuses), for each producer kind; the debugger off-switch (a breakpoint at
//! the Star pc: the byte there is not a Star, so the producer does not fuse, and the
//! instruction at the Star pc runs and reads its own position), for each producer kind; and
//! the debug native-stack floor (sl2-r1 F1: below `native_limit` the Star runs in its own
//! handler, which checks the floor before its write).

use std::cell::{Cell, RefCell};

use ep9_bytecode::{
    FunctionCode, FunctionId, HandlerKind, Opcode, Register, RegisterList, RuntimeId, ScopeId,
    ScopeKind, VariableMode, VerifiedUnit,
};

use super::{
    Asm, Run, config, isolate, meta, on_big_stack, pos, take_output, unit, unit_with_scopes,
};
use crate::frame::{native_ptr, slot};
use crate::interp::Handler;
use crate::interp::tails::star_count;
use crate::unit::FunctionInfo;
use crate::{Dispatch, IsolateConfig};

/// A Star by its place: (the function id, the byte offset of the Star in its code).
type At = (u32, u32);

thread_local! {
    /// The Stars that ran in the Star handler on this thread ([`counting_star`]).
    static PLAIN: RefCell<Vec<At>> = const { RefCell::new(Vec::new()) };
    /// The Star handler of the table that [`counting_star`] replaced.
    static STAR: Cell<Option<Handler>> = const { Cell::new(None) };
}

crate::interp::dsl::handler! {
    /// The `Star` entry of the test runs: records the pc of the Star, then runs the Star
    /// handler that the table held ([`STAR`]).
    ///
    /// # Safety
    /// As for the Star handler: the dispatch table calls it, `pc` is a `Star` of verified code
    /// in the live top frame `fp`.
    fn counting_star(cx, pc, fp, acc) {
        // SAFETY: header slot 4 of the live frame `fp` holds its FunctionInfo native word.
        let fi = unsafe { &*native_ptr::<FunctionInfo>(*fp.offset(slot::FUNCTION_INFO)) };
        let at = (fi.function_id, (pc.addr() - fi.code.addr()) as u32);
        PLAIN.with(|p| p.borrow_mut().push(at));
        let star = STAR.with(|s| s.get()).expect("the Star handler of the table");
        // SAFETY: the handler of the same table entry, with the same state.
        unsafe { star(cx, pc, fp, acc) }
    }
}

/// One run: the run, the fused Stars and the Stars that ran in the Star handler.
struct One {
    run: Run,
    fused: Vec<At>,
    plain: Vec<At>,
}

/// `run_in` with [`counting_star`] in the `Star` entry of the isolate's table.
fn run_counted(c: IsolateConfig, unit: VerifiedUnit, on: bool) -> One {
    star_count::OFF.with(|c| c.set(!on));
    star_count::FUSED.with(|f| f.borrow_mut().clear());
    PLAIN.with(|p| p.borrow_mut().clear());
    let mut iso = isolate(c);
    let star = Opcode::Star as usize;
    STAR.with(|s| s.set(Some(iso.cx.table[star])));
    iso.cx.table[star] = counting_star as Handler;
    take_output();
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|v| cx.display(v)));
    let output = take_output();
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    One {
        run: Run { result: r, output },
        fused: star_count::FUSED.with(|f| f.take()),
        plain: PLAIN.with(|p| p.take()),
    }
}

/// One run of `make` (feedback from the first call, so the ICs of a function hit from its
/// second call) with the lookahead on or off.
fn run_one(d: Dispatch, stress: bool, on: bool, make: fn() -> VerifiedUnit) -> One {
    on_big_stack(move || {
        let mut c = config(d, stress);
        c.feedback_threshold = 0;
        run_counted(c, make(), on)
    })
}

/// The Stars of `v`, sorted (a multiset).
fn sorted(v: &[At]) -> Vec<At> {
    let mut v = v.to_vec();
    v.sort_unstable();
    v
}

/// Checks the plain Star handler against the fused Stars (sl2-r1 F3; the module doc): no off
/// run fused a Star; in a threaded expansion the fused and the plain Stars of the on run are
/// the plain Stars of the off run, pc for pc, and the on run fused at least one Star where
/// `fuses` is true.
fn check_plain(label: &str, d: Dispatch, on: &One, off: &One, fuses: bool) {
    assert!(
        off.fused.is_empty(),
        "{label}: a Star fused with the lookahead off"
    );
    if d == Dispatch::Match {
        assert!(
            on.plain.is_empty() && off.plain.is_empty(),
            "{label}: the match loop ran the table"
        );
        return;
    }
    let mut both = on.fused.clone();
    both.extend_from_slice(&on.plain);
    assert_eq!(
        sorted(&both),
        sorted(&off.plain),
        "{label}: a fused Star also ran in the Star handler, or a Star went missing"
    );
    if fuses {
        assert!(!on.fused.is_empty(), "{label}: no Star fused");
        for at in &on.fused {
            assert!(
                off.plain.contains(at),
                "{label}: {at:?} is no Star of the off run"
            );
        }
    }
}

/// Runs `make` under every expansion, with and without GC stress, with the lookahead on and
/// off, and checks that all runs agree and the plain Star handler ([`check_plain`]). Returns
/// the run and the count of fused Stars of each on run.
fn run_both(make: fn() -> VerifiedUnit) -> (Run, Vec<usize>) {
    let mut first: Option<(String, Run)> = None;
    let mut fused = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let on = run_one(d, stress, true, make);
            let off = run_one(d, stress, false, make);
            let label = format!("{} stress={stress}", d.name());
            check_plain(&label, d, &on, &off, false);
            fused.push(on.fused.len());
            for (r, which) in [(on.run, "on"), (off.run, "off")] {
                let label = format!("{label} lookahead={which}");
                match &first {
                    None => first = Some((label, r)),
                    Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
                }
            }
        }
    }
    (first.unwrap().1, fused)
}

/// `print(r)`: the value of register `r` (a `Ldar`, which is no producer, so the print adds
/// no fused Star).
fn print_reg(s: &mut Asm, r: Register) {
    s.b.ldar(r);
    s.print_acc();
}

/// The constant producers (and `LdaCurrentContextSlot`, a context-slot load) as the producer
/// of a case ([`emit`], [`shown`]). `SMI` (`LdaSmi`) is no producer: the A side of
/// [`star_at_a_jump_target`].
const CONSTANT: u8 = 0;
const ZERO: u8 = 1;
const SMI: u8 = 2;
const NULL: u8 = 3;
const FALSE: u8 = 4;
const CTX: u8 = 5;

/// Writes the producer of kind `k` ([`CONSTANT`] .. [`CTX`]): `v` + 0.5 as a number constant,
/// `v` as a Smi, or the context slot `v`; the other kinds do not read `v`.
fn emit(s: &mut Asm, k: u8, v: i32) {
    match k {
        CONSTANT => {
            s.lda_num(f64::from(v) + 0.5);
        }
        ZERO => {
            s.b.lda_zero();
        }
        SMI => {
            s.b.lda_smi(v);
        }
        NULL => {
            s.b.lda_null();
        }
        FALSE => {
            s.b.lda_false();
        }
        _ => {
            s.b.lda_current_context_slot(v as u32);
        }
    }
}

/// What `print` shows for the value of [`emit`] `(k, v)`. The context slots 0 and 1 hold 5
/// and 9 ([`star_at_a_jump_target`]).
fn shown(k: u8, v: i32) -> String {
    match k {
        CONSTANT => format!("{}", f64::from(v) + 0.5),
        ZERO => "0".into(),
        SMI => v.to_string(),
        NULL => "null".into(),
        FALSE => "false".into(),
        _ => if v == 0 { "5" } else { "9" }.into(),
    }
}

/// f1: every producer kind (the four context-slot loads, `LdaCurrentContext`, `LdaConstant`,
/// `LdaZero`, `LdaNull`, `LdaFalse`) and the two loads that are no producers
/// (`GetNamedProperty`, `LdaSmi`), each followed by `Star r`, then a print of each register.
/// The script calls f1 three times (the GetNamedProperty site misses on the first call and
/// hits after it).
fn producers() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    let save = f.b.new_register();
    let o = f.b.new_register();
    let cur = f.b.new_register();
    let regs: Vec<_> = (0..11).map(|_| f.b.new_register()).collect();
    // o = {}; o.x = 7
    f.b.create_empty_object_literal().star(o);
    f.b.lda_smi(7);
    f.set_named(o, "x");
    // A function context with slots 0 = 11 and 1 = "s".
    let ks = f.b.scope_constant(ScopeId(0));
    f.b.create_function_context(ks, 2).push_context(save);
    f.b.lda_smi(11).sta_current_context_slot(0);
    f.lda_str("s");
    f.b.sta_current_context_slot(1);
    f.b.lda_current_context().star(cur);
    let k = f.b.number_constant(2.5);
    f.get_named(o, "x");
    f.b.star(regs[0]);
    f.b.lda_current_context_slot(0).star(regs[1]);
    f.b.lda_immutable_current_context_slot(1).star(regs[2]);
    f.b.lda_context_slot(cur, 0, 0).star(regs[3]);
    f.b.lda_immutable_context_slot(cur, 1, 0).star(regs[4]);
    f.b.lda_constant(k).star(regs[5]);
    f.b.lda_zero().star(regs[6]);
    f.b.lda_smi(-3).star(regs[7]);
    f.b.lda_null().star(regs[8]);
    f.b.lda_false().star(regs[9]);
    f.b.lda_current_context().star(regs[10]);
    f.b.pop_context(save);
    for &r in &regs[..10] {
        print_reg(&mut f, r);
    }
    // The fused context register holds the function context: slot 0 through it.
    f.b.lda_context_slot(regs[10], 0, 0).return_();

    let mut s = Asm::script(0);
    let kf = s.b.function_constant(FunctionId(1));
    s.b.create_closure(kf, s.closure, 0);
    s.sta_global("f");
    for _ in 0..3 {
        s.lda_global("f");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        s.print_acc();
    }
    s.b.lda_undefined().return_();
    unit_with_scopes(
        "",
        vec![(
            ScopeKind::Function,
            None,
            vec![("a", VariableMode::Let), ("b", VariableMode::Const)],
        )],
        vec![s.finish(), f.finish()],
    )
}

#[test]
fn every_producer_kind_fuses_and_gives_the_star_handler_result() {
    let (r, fused) = run_both(producers);
    assert_eq!(r.result, Ok("undefined".to_string()));
    let one = [
        "7", "11", "s", "11", "s", "2.5", "0", "-3", "null", "false", "11",
    ];
    let want: Vec<&str> = one.iter().copied().cycle().take(33).collect();
    assert_eq!(r.output, want);
    // Per call of f: the four context-slot loads, the two `LdaCurrentContext` and the four
    // constant loads (LdaConstant, LdaZero, LdaNull, LdaFalse) fuse (10); the Stars after
    // GetNamedProperty and LdaSmi run in the Star handler. So 3 x 10 in every on run.
    assert!(fused.iter().all(|&n| n == 30), "{fused:?}");
}

/// f1(o): `try { t = 1; t = o.x } catch (e) { print(t); print(e.message) }; return t`, with
/// `t = 1` as `LdaSmi 1; Star t` and `o.x` as `GetNamedProperty; Star t` (no producers: both
/// Stars run in the Star handler), after a fused `LdaCurrentContext; Star ctx`; f2(o):
/// `return o.x` with no handler, at the same position. The script calls f1({x: 5}) twice,
/// then f1(undefined) (the load throws: `t` stays 1), then f2(undefined): the uncaught
/// TypeError has the position of the load.
const THROW_SRC: &str = "function f(o) {\n  t = o.x;\n}\n";

fn throwing_load() -> VerifiedUnit {
    let src = THROW_SRC;
    let mut f = Asm::new(meta("f", 1, 0, 0, src.len() as u32));
    let (ctx, t) = (f.b.new_register(), f.b.new_register());
    let o = Register::parameter(0);
    let (start, end, handler, done) = (
        f.b.new_label(),
        f.b.new_label(),
        f.b.new_label(),
        f.b.new_label(),
    );
    f.b.lda_current_context().star(ctx);
    f.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    f.b.bind(start);
    f.b.lda_smi(1).star(t);
    f.b.set_expression_position(pos(src, "o.x"));
    // A slot of its own: the handler's `e.message` load below has the shared `named` slot.
    let (kx, sx) = (
        f.b.string_constant("x"),
        f.b.feedback_slot(ep9_bytecode::FeedbackSlotKind::LoadNamed),
    );
    f.b.get_named_property(o, kx, sx);
    f.b.star(t);
    f.b.bind(end);
    f.b.jump(done);
    f.b.bind(handler);
    let e = f.a.get(2);
    f.b.star(e);
    print_reg(&mut f, t);
    f.get_named(e, "message");
    f.print_acc();
    f.b.bind(done);
    f.b.ldar(t).return_();

    // g(o): `return o.x` with no handler, at the same position.
    let mut g = Asm::new(meta("g", 1, 0, 0, src.len() as u32));
    let r = g.b.new_register();
    g.b.set_expression_position(pos(src, "o.x"));
    g.get_named(o, "x");
    g.b.star(r).ldar(r).return_();

    let mut s = Asm::script(src.len());
    for (i, name) in ["f", "g"].iter().enumerate() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    let arg = s.b.new_register();
    s.b.create_empty_object_literal().star(arg);
    s.b.lda_smi(5);
    s.set_named(arg, "x");
    for _ in 0..2 {
        s.lda_global("f");
        s.b.star(s.f).call_undefined_receiver1(s.f, arg, s.call);
        s.print_acc();
    }
    s.b.lda_undefined().star(arg);
    s.lda_global("f");
    s.b.star(s.f).call_undefined_receiver1(s.f, arg, s.call);
    s.print_acc();
    s.lda_global("g");
    s.b.star(s.f).call_undefined_receiver1(s.f, arg, s.call);
    s.b.return_();
    unit(src, vec![s.finish(), f.finish(), g.finish()])
}

#[test]
fn a_throw_after_a_fused_star_keeps_the_star_register_and_its_position() {
    let (r, fused) = run_both(throwing_load);
    assert_eq!(&r.output[..3], ["5", "5", "1"], "{:?}", r.output);
    assert_eq!(r.output.len(), 5, "{:?}", r.output);
    assert!(r.output[3].contains("undefined"), "{:?}", r.output);
    assert_eq!(r.output[4], "1");
    let e = r.result.unwrap_err();
    assert_eq!(e.constructor.as_deref(), Some("TypeError"), "{e:?}");
    // The position of the load (`o.x`, line 2, column 7), not of the Star after it.
    let loc = e.location.unwrap();
    assert_eq!((loc.line, loc.column), (2, 7));
    // f: `LdaCurrentContext; Star ctx` in each call (3).
    assert!(fused.iter().all(|&n| n == 3), "{fused:?}");
}

/// The getter `get x() { new Object(); new Object(); return 9 }` on `o`, then three times
/// `t = o.x` (`GetNamedProperty; Star t`), `u = "y"` (`LdaConstant; Star u`, fused: a heap
/// string in the register), `new Object()` (under GC stress a collection after the fused
/// Star: `u` is a root in its register) and `print(t)`, `print(u)`: under GC stress each
/// allocation of the getter collects.
fn gc_after_a_fused_star() -> VerifiedUnit {
    let mut getter = Asm::new(meta("get x", 0, 0, 0, 0));
    let none = RegisterList::new(getter.a.get(0).local_index().unwrap(), 0);
    for _ in 0..2 {
        getter.construct_global("Object", none);
    }
    getter.b.lda_smi(9).return_();

    let mut s = Asm::script(0);
    let (o, t, u) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    s.b.create_empty_object_literal().star(o);
    let args = s.b.new_register_list(4);
    s.b.mov(o, args.get(0));
    s.lda_str("x");
    s.b.star(args.get(1));
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0).star(args.get(2));
    s.b.lda_undefined().star(args.get(3));
    s.b.call_runtime(RuntimeId::DefineAccessorInLiteral, args, 4);
    let none = RegisterList::new(s.a.get(0).local_index().unwrap(), 0);
    for _ in 0..3 {
        s.get_named(o, "x");
        s.b.star(t);
        s.lda_str("y");
        s.b.star(u);
        s.construct_global("Object", none);
        print_reg(&mut s, t);
        print_reg(&mut s, u);
    }
    s.b.ldar(t).return_();
    unit("", vec![s.finish(), getter.finish()])
}

#[test]
fn a_gc_after_a_fused_star_gives_the_same_result() {
    let (r, fused) = run_both(gc_after_a_fused_star);
    assert_eq!(r.result, Ok("9".to_string()));
    assert_eq!(r.output, ["9", "y", "9", "y", "9", "y"]);
    // The fused Stars: the key `LdaConstant "x"; Star` (1) and `LdaConstant "y"; Star u` in
    // each turn (3).
    assert!(fused.iter().all(|&n| n == 4), "{fused:?}");
}

/// f1: `LdaContextSlot c, 0, 0; Star c` (the Star target is the context register that the
/// producer reads), three calls. The constant producers read no register: `LdaZero; Star z`
/// and `LdaFalse; Star z` store to the register that the instructions after them read.
fn star_into_the_input() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    let (save, r, c, z) = (
        f.b.new_register(),
        f.b.new_register(),
        f.b.new_register(),
        f.b.new_register(),
    );
    f.b.create_empty_object_literal().star(r);
    f.b.lda_smi(4);
    f.set_named(r, "x");
    let ks = f.b.scope_constant(ScopeId(0));
    f.b.create_function_context(ks, 1).push_context(save);
    f.b.lda_smi(6).sta_current_context_slot(0);
    f.b.lda_current_context().star(c);
    f.get_named(r, "x");
    f.b.star(r);
    f.b.lda_context_slot(c, 0, 0).star(c);
    f.b.pop_context(save);
    print_reg(&mut f, r);
    print_reg(&mut f, c);
    f.b.lda_false().star(z);
    print_reg(&mut f, z);
    f.b.lda_zero().star(z);
    f.b.lda_smi(2).add(z, f.bin).star(z);
    f.b.ldar(r).add(c, f.bin).add(z, f.bin).return_();

    let mut s = Asm::script(0);
    let kf = s.b.function_constant(FunctionId(1));
    s.b.create_closure(kf, s.closure, 0);
    s.sta_global("f");
    for _ in 0..3 {
        s.lda_global("f");
        s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
        s.print_acc();
    }
    s.b.lda_undefined().return_();
    unit_with_scopes(
        "",
        vec![(ScopeKind::Function, None, vec![("n", VariableMode::Let)])],
        vec![s.finish(), f.finish()],
    )
}

#[test]
fn a_star_into_the_producer_input_stores_after_the_read() {
    let (r, fused) = run_both(star_into_the_input);
    let one = ["4", "6", "false", "12"];
    let want: Vec<&str> = one.iter().copied().cycle().take(12).collect();
    assert_eq!(r.output, want);
    // Per call: `LdaCurrentContext; Star c`, `LdaContextSlot; Star c`, `LdaFalse; Star z` and
    // `LdaZero; Star z` (4).
    assert!(fused.iter().all(|&n| n == 3 * 4), "{fused:?}");
}

/// f1 for the producer kind `K` ([`emit`]): `t = A` reaches the Star by a jump (the Star
/// handler runs), then `t = B` reaches it by the fall-through from the producer (fused):
///
/// ```text
///     CreateFunctionContext; PushContext save; slot 0 = 5; slot 1 = 9
///     LdaFalse; Star flag            ; fused
///     <A>; Jump L                    ; no Star after this load
///  M: <B>                            ; falls through to the Star: fused
///  L: Star t                         ; a jump target
///     print(t)
///     Ldar flag; JumpIfTrue END
///     LdaTrue; Star flag             ; LdaTrue is no producer
///     JumpLoop M
///  END: PopContext save; Ldar t; Return
/// ```
fn star_at_a_jump_target<const K: u8>() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    let (save, flag, t) = (f.b.new_register(), f.b.new_register(), f.b.new_register());
    let (m, l, end) = (f.b.new_label(), f.b.new_label(), f.b.new_label());
    let ks = f.b.scope_constant(ScopeId(0));
    f.b.create_function_context(ks, 2).push_context(save);
    f.b.lda_smi(5).sta_current_context_slot(0);
    f.b.lda_smi(9).sta_current_context_slot(1);
    f.b.lda_false().star(flag);
    let (a, b) = if K == CTX { (0, 1) } else { (5, -3) };
    emit(&mut f, if K == CTX { CTX } else { SMI }, a);
    f.b.jump(l);
    f.b.bind(m);
    emit(&mut f, K, b);
    f.b.bind(l);
    f.b.star(t);
    print_reg(&mut f, t);
    f.b.ldar(flag).jump_if_true(end);
    f.b.lda_true().star(flag).jump_loop(m, 0, f.loop_);
    f.b.bind(end);
    f.b.pop_context(save);
    f.b.ldar(t).return_();

    let mut s = Asm::script(0);
    let kf = s.b.function_constant(FunctionId(1));
    s.b.create_closure(kf, s.closure, 0);
    s.sta_global("f");
    s.lda_global("f");
    s.b.star(s.f)
        .call_undefined_receiver0(s.f, s.call)
        .return_();
    unit_with_scopes(
        "",
        vec![(
            ScopeKind::Function,
            None,
            vec![("a", VariableMode::Let), ("b", VariableMode::Let)],
        )],
        vec![s.finish(), f.finish()],
    )
}

/// The printed `A` and `B` of [`star_at_a_jump_target`] for kind `k`.
fn jump_target_want(k: u8) -> [String; 2] {
    if k == CTX {
        [shown(CTX, 0), shown(CTX, 1)]
    } else {
        [shown(SMI, 5), shown(k, -3)]
    }
}

#[test]
fn a_star_at_a_jump_target_fuses_only_on_the_fall_through() {
    let cases: [(u8, fn() -> VerifiedUnit); 5] = [
        (CTX, star_at_a_jump_target::<CTX>),
        (CONSTANT, star_at_a_jump_target::<CONSTANT>),
        (ZERO, star_at_a_jump_target::<ZERO>),
        (NULL, star_at_a_jump_target::<NULL>),
        (FALSE, star_at_a_jump_target::<FALSE>),
    ];
    for (k, make) in cases {
        let (r, fused) = run_both(make);
        let want = jump_target_want(k);
        assert_eq!(r.output, want, "kind {k}");
        assert_eq!(r.result, Ok(want[1].clone()), "kind {k}");
        // `LdaFalse; Star flag` and `<B>; Star t` (the fall-through); the jump to the Star
        // runs the Star handler (`check_plain`: that Star pc is in the plain Stars of the on
        // run once and of the off run twice).
        assert!(fused.iter().all(|&n| n == 2), "kind {k}: {fused:?}");
    }
}

/// The debugger off-switch. A breakpoint at the Star pc puts a break byte at that pc of the
/// function's Interp debug copy (H24-Q3). This test stands in for that copy with the same
/// code and one byte changed at the Star pc: `ToObject t` in place of `Star t` (the same
/// operand; it reads its own pc for its throw position, as a break reads the pc it stops
/// at). For `LdaNull` the ToObject throws a TypeError; for the other producers it does not
/// throw (a wrapper), and the fused count shows the off-switch.
const BREAK_SRC: &str = "t = o.x;\nBREAK;\n";

fn at_a_breakpoint(k: u8, patched: bool) -> FunctionCode {
    let src = BREAK_SRC;
    let mut s = Asm::script(src.len());
    let t = s.b.new_register();
    // LdaTrue is no producer: no fused Star before the case.
    s.b.lda_true().star(t);
    s.b.set_statement_position(pos(src, "t = o.x"));
    emit(&mut s, k, 7);
    // The breakpoint is at this Star pc.
    s.b.set_statement_position(pos(src, "BREAK"));
    if patched {
        s.b.to_object(t);
    } else {
        s.b.star(t);
    }
    s.b.ldar(t).return_();
    s.finish()
}

fn breakpoint<const K: u8, const PATCHED: bool>() -> VerifiedUnit {
    unit(BREAK_SRC, vec![at_a_breakpoint(K, PATCHED)])
}

#[test]
fn a_breakpoint_at_the_star_pc_turns_the_lookahead_off() {
    type Make = fn() -> VerifiedUnit;
    let cases: [(u8, Make, Make); 4] = [
        (
            CONSTANT,
            breakpoint::<CONSTANT, false>,
            breakpoint::<CONSTANT, true>,
        ),
        (ZERO, breakpoint::<ZERO, false>, breakpoint::<ZERO, true>),
        (NULL, breakpoint::<NULL, false>, breakpoint::<NULL, true>),
        (FALSE, breakpoint::<FALSE, false>, breakpoint::<FALSE, true>),
    ];
    for (k, off, on) in cases {
        // The two forms differ in the opcode byte at the Star pc only.
        let (a, b) = (at_a_breakpoint(k, false), at_a_breakpoint(k, true));
        let diff: Vec<usize> = (0..a.bytecode().len())
            .filter(|&i| a.bytecode()[i] != b.bytecode()[i])
            .collect();
        assert_eq!(a.bytecode().len(), b.bytecode().len());
        assert_eq!(diff.len(), 1, "kind {k}: {diff:?}");
        for &i in &diff {
            assert_eq!(a.bytecode()[i], Opcode::Star as u8);
            assert_eq!(b.bytecode()[i], Opcode::ToObject as u8);
        }
        // No breakpoint: the producer before the Star pc fuses it.
        let (r, fused) = run_both(off);
        assert!(r.result.is_ok(), "kind {k}: {r:?}");
        assert!(fused.iter().all(|&n| n == 1), "kind {k}: {fused:?}");
        // A breakpoint at the Star pc: the producer does not fuse past it, and the instruction
        // at the Star pc runs: its throw has the position of the Star pc (line 2), not of the
        // producer (line 1).
        let (r, fused) = run_both(on);
        assert!(fused.iter().all(|&n| n == 0), "kind {k}: {fused:?}");
        if k == NULL {
            let e = r.result.unwrap_err();
            assert_eq!(
                e.constructor.as_deref(),
                Some("TypeError"),
                "kind {k}: {e:?}"
            );
            let loc = e.location.unwrap();
            assert_eq!((loc.line, loc.column), (2, 1), "kind {k}");
        } else {
            assert!(r.result.is_ok(), "kind {k}: {r:?}");
        }
    }
}

/// The turns of [`near_the_floor`]: more than a debug sibling run on a [`FLOOR_STACK`] thread
/// can take before its host call fails (each turn keeps the native frames of its
/// instructions there, well over 200 bytes), and few for the other expansions.
const FLOOR_TURNS: i32 = 20_000;
/// The native stack of the [`near_the_floor`] runs (`native_limit` is a quarter of it above
/// its low end, `isolate::native_margin`).
const FLOOR_STACK: usize = 4 << 20;

/// sl2-r1 F1, the debug native-stack floor:
///
/// ```text
///     LdaZero; Star n                    ; fused
///  L: try { noop() }                     ; a host call: a RangeError once the native stack
///                                        ; is below native_limit
///     catch { LdaNull; Star u; Ldar u; Return }   ; below native_limit: no fusion
///     LdaFalse; Star t                   ; fused
///     n = n + 1; if (n < FLOOR_TURNS) JumpLoop L
///     LdaUndefined; Return
/// ```
///
/// A debug sibling run (no tail calls: each instruction keeps a native frame) reaches
/// `native_limit` in a few thousand turns: the host call throws, and the catch runs below
/// `native_limit` and above the floor (`DEBUG_SLACK` below it). There the lookahead does not
/// fuse (`tails::star_room`): the Star runs in its own handler, which checks the floor before
/// its write. The other expansions keep the native stack flat and run all turns.
///
/// Become is not run here: [`counting_star`] calls the Star handler with an ordinary call, so
/// under become each plain Star keeps one wrapper frame (208 bytes, aarch64 debug), and
/// 20,000 turns would reach native_limit on this thread; its handlers keep the stack flat, so
/// it has no floor case.
fn near_the_floor_code() -> FunctionCode {
    let mut s = Asm::script(0);
    let (n, t, u, ctx, one) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    let (top, start, end, handler) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.lda_current_context().star(ctx);
    s.b.lda_zero().star(n);
    s.b.bind(top);
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    s.lda_global("noop");
    s.b.star(s.f).call_undefined_receiver0(s.f, s.call);
    s.b.bind(end);
    s.b.lda_false().star(t);
    s.b.lda_smi(1).star(one);
    s.b.ldar(n).add(one, s.bin).star(n);
    // acc = n < FLOOR_TURNS
    s.b.lda_smi(FLOOR_TURNS).test_less_than(n, s.cmp);
    let done = s.b.new_label();
    s.b.jump_if_false(done);
    s.b.jump_loop(top, 0, s.loop_);
    s.b.bind(done);
    s.b.lda_undefined().return_();
    s.b.bind(handler);
    s.b.lda_null().star(u);
    s.b.ldar(u).return_();
    s.finish()
}

fn near_the_floor() -> VerifiedUnit {
    unit("", vec![near_the_floor_code()])
}

/// The byte offset of the `Star` after the only `LdaNull` of [`near_the_floor`].
fn floor_star_at() -> u32 {
    let code = near_the_floor_code();
    let bytes = code.bytecode();
    let hits: Vec<usize> = (0..bytes.len() - 1)
        .filter(|&i| bytes[i] == Opcode::LdaNull as u8 && bytes[i + 1] == Opcode::Star as u8)
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    hits[0] as u32 + 1
}

/// `noop()`: a host function that returns undefined.
fn noop<'s>(
    _cx: &mut crate::Cx<'s>,
    _this: crate::Local<'s>,
    _args: crate::Args<'s>,
    _nt: crate::Local<'s>,
) -> crate::JsResult<crate::Local<'s>> {
    Ok(crate::Local::undefined())
}

#[test]
fn below_the_native_limit_the_star_runs_in_its_own_handler() {
    let at = (0, floor_star_at());
    for d in [Dispatch::Match, Dispatch::Sibling] {
        for stress in [false, true] {
            let run = |on: bool| {
                std::thread::Builder::new()
                    .stack_size(FLOOR_STACK)
                    .spawn(move || {
                        let mut c = config(d, stress);
                        c.feedback_threshold = 0;
                        // `isolate` with `noop`: the program calls no print.
                        star_count::OFF.with(|c| c.set(!on));
                        star_count::FUSED.with(|f| f.borrow_mut().clear());
                        PLAIN.with(|p| p.borrow_mut().clear());
                        let mut iso = isolate(c);
                        iso.register_host_function("noop", 0, noop);
                        let star = Opcode::Star as usize;
                        STAR.with(|s| s.set(Some(iso.cx.table[star])));
                        iso.cx.table[star] = counting_star as Handler;
                        let r = iso.with_cx_checkpoint(|cx| {
                            cx.run_script(near_the_floor(), "floor.js")
                                .map(|v| cx.display(v))
                        });
                        One {
                            run: Run {
                                result: r,
                                output: Vec::new(),
                            },
                            fused: star_count::FUSED.with(|f| f.take()),
                            plain: PLAIN.with(|p| p.take()),
                        }
                    })
                    .unwrap()
                    .join()
                    .unwrap()
            };
            let (on, off) = (run(true), run(false));
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(on.run, off.run, "{label}");
            let floor = cfg!(debug_assertions) && d == Dispatch::Sibling;
            if floor {
                // The catch ran below native_limit: its Star ran in the Star handler, in both
                // runs, and never fused. (The on run takes more turns before the host call
                // fails: a fused Star keeps no Star handler frame. So the turns of the two
                // runs differ, and `check_plain` does not apply.)
                assert_eq!(on.run.result, Ok("null".to_string()), "{label}");
                assert!(off.fused.is_empty(), "{label}");
                assert!(
                    !on.fused.contains(&at),
                    "{label}: the Star below native_limit fused"
                );
                assert_eq!(on.plain.iter().filter(|&&p| p == at).count(), 1, "{label}");
                assert_eq!(off.plain.iter().filter(|&&p| p == at).count(), 1, "{label}");
                // Above native_limit the lookahead fused: `LdaCurrentContext; Star ctx` and
                // `LdaZero; Star n` once, and `LdaFalse; Star t` in each turn before the host
                // call failed.
                assert!(on.fused.len() > 2, "{label}: {}", on.fused.len());
            } else {
                check_plain(&label, d, &on, &off, true);
                assert_eq!(on.run.result, Ok("undefined".to_string()), "{label}");
                // `LdaCurrentContext; Star ctx`, `LdaZero; Star n` and `LdaFalse; Star t` in
                // each turn.
                assert_eq!(on.fused.len(), 2 + FLOOR_TURNS as usize, "{label}");
            }
        }
    }
}
