//! The Star lookahead (M9-P0 STAR-FUSE, M10-P2 STAR-LA2; `interp::tails::star_next`,
//! `interp::tails::star_done`): a producer that is followed by a one-byte-scale `Star r`
//! stores the accumulator to `r` and dispatches past the Star. Each program runs under every
//! dispatch expansion, with and without GC stress, with the lookahead on and off
//! (`tails::star_count::OFF`, test builds only): all runs agree (the result, the output, and
//! an exception with its message, location and stack), and the count of fused Stars
//! (`tails::star_count::FUSED`) shows that the on runs fused and the off runs did not.
//!
//! The producers: the `GetNamedProperty` IC hit, the four context-slot loads,
//! `LdaCurrentContext` (M9-P0), and `LdaConstant`, `LdaZero`, `LdaSmi`, `LdaNull`,
//! `LdaFalse` (M10-P2). The cases (plan v14 §3.4, plan v15 §5.1; DL M9-PRE-STARFUSE-KIND):
//! every producer kind; a throw in the producer (no fusion; the Star register keeps its value;
//! the throw position is the producer's); a GC in the producer (a getter that allocates,
//! under GC stress) and a GC after a fused Star of a heap constant; a Star whose register is
//! the producer's input; a Star that is a jump target (a jump to it runs the Star handler;
//! only the fall-through fuses), for each producer kind; and the debugger off-switch (a
//! breakpoint at the Star pc: the byte there is not a Star, so the producer does not fuse, and
//! the instruction at the Star pc runs and reads its own position), for each producer kind.
//! The 5 constant producers read no register and do not throw, allocate or call, so the
//! throw and input cases have them only as fused Stars beside the producer under test.

use ep9_bytecode::{
    FeedbackSlotKind, FunctionCode, FunctionId, HandlerKind, Opcode, Register, RegisterList,
    RuntimeId, ScopeId, ScopeKind, VariableMode, VerifiedUnit,
};

use super::{Asm, Run, config, meta, on_big_stack, pos, run_in, unit, unit_with_scopes};
use crate::Dispatch;
use crate::interp::tails::star_count;

/// One run of `make` (feedback from the first call, so the ICs of a function hit from its
/// second call) with the lookahead on or off: the run and the Stars that a producer fused.
fn run_one(d: Dispatch, stress: bool, on: bool, make: fn() -> VerifiedUnit) -> (Run, u64) {
    on_big_stack(move || {
        star_count::OFF.with(|c| c.set(!on));
        star_count::FUSED.with(|c| c.set(0));
        let mut c = config(d, stress);
        c.feedback_threshold = 0;
        let r = run_in(c, make());
        (r, star_count::FUSED.with(|c| c.get()))
    })
}

/// Runs `make` under every expansion, with and without GC stress, with the lookahead on and
/// off, and checks that all runs agree and that no off run fused a Star. Returns the run and
/// the fused Stars of each on run.
fn run_both(make: fn() -> VerifiedUnit) -> (Run, Vec<u64>) {
    let mut first: Option<(String, Run)> = None;
    let mut fused = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            for on in [true, false] {
                let (r, n) = run_one(d, stress, on, make);
                let label = format!("{} stress={stress} lookahead={on}", d.name());
                if on {
                    fused.push(n);
                } else {
                    assert_eq!(n, 0, "{label}: a Star fused with the lookahead off");
                }
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

/// The 5 constant producers of M10-P2 (and `LdaCurrentContextSlot`, a producer of M9-P0) as
/// the producer of a case ([`emit`], [`shown`]).
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

/// f1: every producer kind (`GetNamedProperty`, the four context-slot loads,
/// `LdaCurrentContext`, `LdaConstant`, `LdaZero`, `LdaSmi`, `LdaNull`, `LdaFalse`), each
/// followed by `Star r`, then a print of each register. The script calls f1 three times (the
/// GetNamedProperty site misses on the first call and hits after it).
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
    // Per call of f: the four context-slot loads, the two `LdaCurrentContext` and the five
    // constant loads (LdaConstant, LdaZero, LdaSmi, LdaNull, LdaFalse) fuse (11);
    // GetNamedProperty fuses on its IC hits (calls 2 and 3). So 3 x 11 + 2 = 35 in every on
    // run.
    assert!(fused.iter().all(|&n| n == 35), "{fused:?}");
}

/// f1(o): `try { t = 1; t = o.x } catch (e) { print(t); print(e.message) }; return t`, with
/// `t = 1` as `LdaSmi 1; Star t` (fused) and `o.x` as `GetNamedProperty; Star t`; f2(o):
/// `return o.x` with no handler, at the same position. The script calls f1({x: 5}) twice (the
/// second call hits the IC and fuses), then f1(undefined) (the producer throws: no fusion, `t`
/// stays 1, the value of the fused `LdaSmi` Star), then f2(undefined): the uncaught TypeError
/// has the position of the producer.
const THROW_SRC: &str = "function f(o) {\n  t = o.x;\n}\n";

fn throwing_producer() -> VerifiedUnit {
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
        f.b.feedback_slot(FeedbackSlotKind::LoadNamed),
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
fn a_throwing_producer_does_not_fuse_and_keeps_the_star_register() {
    let (r, fused) = run_both(throwing_producer);
    assert_eq!(&r.output[..3], ["5", "5", "1"], "{:?}", r.output);
    assert_eq!(r.output.len(), 5, "{:?}", r.output);
    assert!(r.output[3].contains("undefined"), "{:?}", r.output);
    assert_eq!(r.output[4], "1");
    let e = r.result.unwrap_err();
    assert_eq!(e.constructor.as_deref(), Some("TypeError"), "{e:?}");
    // The position of the producer (`o.x`, line 2, column 7), not of the Star after it.
    let loc = e.location.unwrap();
    assert_eq!((loc.line, loc.column), (2, 7));
    // f: `LdaCurrentContext; Star ctx` and `LdaSmi 1; Star t` in each call (3 x 2), and
    // `GetNamedProperty; Star t` on the IC hit of the second call (1). The throwing loads of f
    // and g fuse nothing.
    assert!(fused.iter().all(|&n| n == 7), "{fused:?}");
}

/// The getter `get x() { new Object(); new Object(); return 9 }` on `o`, then three times
/// `t = o.x` (`GetNamedProperty; Star t`), `u = "y"` (`LdaConstant; Star u`, fused: a heap
/// string in the register), `new Object()` (under GC stress a collection after the fused
/// Star: `u` is a root in its register) and `print(t)`, `print(u)`: under GC stress each
/// allocation of the getter collects, inside the producer.
fn gc_in_the_producer() -> VerifiedUnit {
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
fn a_gc_in_the_producer_gives_the_same_result() {
    let (r, fused) = run_both(gc_in_the_producer);
    assert_eq!(r.result, Ok("9".to_string()));
    assert_eq!(r.output, ["9", "y", "9", "y", "9", "y"]);
    // A getter load is no IC hit of the fast body, so it does not fuse. The fused Stars: the
    // key `LdaConstant "x"; Star` (1) and `LdaConstant "y"; Star u` in each turn (3).
    assert!(fused.iter().all(|&n| n == 4), "{fused:?}");
}

/// f1: `GetNamedProperty r, "x"; Star r` (the Star target is the object register the producer
/// reads) and `LdaContextSlot c, 0, 0; Star c` (the context register), three calls. The
/// constant producers read no register: `LdaZero; Star z` and `LdaSmi 2; Star z` store to the
/// register that the `Add` after them reads.
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
    f.b.lda_zero().star(z);
    f.b.lda_smi(2).star(z).add(z, f.bin).star(z);
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
    assert_eq!(r.output, ["4", "6", "14", "4", "6", "14", "4", "6", "14"]);
    // Per call: `LdaCurrentContext; Star c`, `LdaContextSlot; Star c`, `LdaZero; Star z` and
    // `LdaSmi 2; Star z` (4), and `GetNamedProperty; Star r` on the IC hits of calls 2 and 3.
    assert!(fused.iter().all(|&n| n == 3 * 4 + 2), "{fused:?}");
}

/// f1 for the producer kind `K` ([`emit`]): `t = A` reaches the Star by a jump (the Star
/// handler runs), then `t = B` reaches it by the fall-through from the producer (fused):
///
/// ```text
///     CreateFunctionContext; PushContext save; slot 0 = 5; slot 1 = 9
///     LdaFalse; Star flag            ; fused
///     <A>; Jump L                    ; no Star after this producer
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
    let cases: [(u8, fn() -> VerifiedUnit); 6] = [
        (CTX, star_at_a_jump_target::<CTX>),
        (CONSTANT, star_at_a_jump_target::<CONSTANT>),
        (ZERO, star_at_a_jump_target::<ZERO>),
        (SMI, star_at_a_jump_target::<SMI>),
        (NULL, star_at_a_jump_target::<NULL>),
        (FALSE, star_at_a_jump_target::<FALSE>),
    ];
    for (k, make) in cases {
        let (r, fused) = run_both(make);
        let want = jump_target_want(k);
        assert_eq!(r.output, want, "kind {k}");
        assert_eq!(r.result, Ok(want[1].clone()), "kind {k}");
        // `LdaFalse; Star flag` and `<B>; Star t` (the fall-through); the jump to the Star
        // does not fuse.
        assert!(fused.iter().all(|&n| n == 2), "kind {k}: {fused:?}");
    }
}

/// The debugger off-switch. A breakpoint at the Star pc puts a break byte at that pc of the
/// function's Interp debug copy (H24-Q3). This test stands in for that copy with the same
/// code and one byte changed at the Star pc: `ToObject t` in place of `Star t` (the same
/// operand; it reads its own pc for its throw position, as a break reads the pc it stops
/// at). The producer is `GetNamedProperty` (`o.x` is undefined) or `LdaNull`, so the ToObject
/// throws a TypeError; for the other constant producers it does not throw (a wrapper), and
/// the fused count shows the off-switch.
const BREAK_SRC: &str = "t = o.x;\nBREAK;\n";

/// The producer of [`at_a_breakpoint`]: `GetNamedProperty` (`NAMED`) or a kind of [`emit`].
const NAMED: u8 = 9;

fn at_a_breakpoint(k: u8, patched: bool) -> FunctionCode {
    let src = BREAK_SRC;
    let mut s = Asm::script(src.len());
    let (o, t) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_object_literal().star(o);
    s.b.lda_undefined();
    s.set_named(o, "x");
    // LdaTrue is no producer: no fused Star before the case.
    s.b.lda_true().star(t);
    s.b.set_statement_position(pos(src, "t = o.x"));
    if k == NAMED {
        // The first load records the IC case (a miss: no lookahead); the second hits and,
        // with no breakpoint, fuses its Star.
        s.get_named(o, "x");
        s.b.star(t);
        s.b.set_statement_position(pos(src, "t = o.x"));
        s.get_named(o, "x");
    } else {
        emit(&mut s, k, 7);
    }
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
    let cases: [(u8, Make, Make); 6] = [
        (NAMED, breakpoint::<NAMED, false>, breakpoint::<NAMED, true>),
        (
            CONSTANT,
            breakpoint::<CONSTANT, false>,
            breakpoint::<CONSTANT, true>,
        ),
        (ZERO, breakpoint::<ZERO, false>, breakpoint::<ZERO, true>),
        (SMI, breakpoint::<SMI, false>, breakpoint::<SMI, true>),
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
        // No breakpoint: the producer before the Star pc fuses it (for `GetNamedProperty`
        // the IC hit of the second load).
        let (r, fused) = run_both(off);
        assert!(r.result.is_ok(), "kind {k}: {r:?}");
        assert!(fused.iter().all(|&n| n == 1), "kind {k}: {fused:?}");
        // A breakpoint at the Star pc: the producer does not fuse past it, and the instruction
        // at the Star pc runs: its throw has the position of the Star pc (line 2), not of the
        // producer (line 1).
        let (r, fused) = run_both(on);
        assert!(fused.iter().all(|&n| n == 0), "kind {k}: {fused:?}");
        if k == NAMED || k == NULL {
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
