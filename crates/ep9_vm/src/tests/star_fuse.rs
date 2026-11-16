//! The Star lookahead (M9-P0 STAR-FUSE; `interp::tails::star_lookahead`): a producer that is
//! followed by a one-byte-scale `Star r` stores the accumulator to `r` and dispatches past the
//! Star. Each program runs under every dispatch expansion, with and without GC stress, with the
//! lookahead on and off (`tails::star_count::OFF`, test builds only): all runs agree (the
//! result, the output, and an exception with its message, location and stack), and the count
//! of fused Stars (`tails::star_count::FUSED`) shows that the on runs fused and the off runs
//! did not.
//!
//! The cases of the role text (plan v14 §3.4; DL M9-PRE-STARFUSE-KIND): every producer kind
//! of the change (and the constant loads, which do not fuse); a throw in the producer (no fusion; the Star register keeps its value; the
//! throw position is the producer's); a GC in the producer (a getter that allocates, under GC
//! stress); a Star whose register is the producer's input; a Star that is a jump target (a
//! jump to it runs the Star handler; only the fall-through fuses); and the debugger off-switch
//! (a breakpoint at the Star pc: the byte there is not a Star, so the producer does not fuse,
//! and the instruction at the Star pc runs and reads its own position).

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

/// f1: every producer kind of the change (`GetNamedProperty`, the four context-slot loads,
/// `LdaCurrentContext`) and five constant loads that are no producer of it, each followed by
/// `Star r`, then a print of each register. The script calls f1 three times (the
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
    // Per call of f: the four context-slot loads and the two `LdaCurrentContext` fuse (6);
    // GetNamedProperty fuses on its IC hits (calls 2 and 3); the constant loads (LdaConstant,
    // LdaZero, LdaSmi, LdaNull, LdaFalse) do not. So 3 x 6 + 2 = 20 in every on run.
    assert!(fused.iter().all(|&n| n == 20), "{fused:?}");
}

/// f1(o): `try { t = 1; t = o.x } catch (e) { print(t); print(e.message) }; return t`, with
/// `o.x` as `GetNamedProperty; Star t`; f2(o): `return o.x` with no handler, at the same
/// position. The script calls f1({x: 5}) twice (the second call hits the IC and fuses), then
/// f1(undefined) (the producer throws: no fusion, `t` stays 1), then f2(undefined): the
/// uncaught TypeError has the position of the producer.
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
    // f: `LdaCurrentContext; Star ctx` in each call (3), and `GetNamedProperty; Star t` on the
    // IC hit of the second call (1). The throwing loads of f and g fuse nothing.
    assert!(fused.iter().all(|&n| n == 4), "{fused:?}");
}

/// The getter `get x() { new Object(); new Object(); return 9 }` on `o`, then three times
/// `t = o.x` (`GetNamedProperty; Star t`) and `print(t)`: under GC stress each allocation of
/// the getter collects, inside the producer.
fn gc_in_the_producer() -> VerifiedUnit {
    let mut getter = Asm::new(meta("get x", 0, 0, 0, 0));
    let none = RegisterList::new(getter.a.get(0).local_index().unwrap(), 0);
    for _ in 0..2 {
        getter.construct_global("Object", none);
    }
    getter.b.lda_smi(9).return_();

    let mut s = Asm::script(0);
    let (o, t) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_object_literal().star(o);
    let args = s.b.new_register_list(4);
    s.b.mov(o, args.get(0));
    s.lda_str("x");
    s.b.star(args.get(1));
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0).star(args.get(2));
    s.b.lda_undefined().star(args.get(3));
    s.b.call_runtime(RuntimeId::DefineAccessorInLiteral, args, 4);
    for _ in 0..3 {
        s.get_named(o, "x");
        s.b.star(t);
        print_reg(&mut s, t);
    }
    s.b.ldar(t).return_();
    unit("", vec![s.finish(), getter.finish()])
}

#[test]
fn a_gc_in_the_producer_gives_the_same_result() {
    let (r, fused) = run_both(gc_in_the_producer);
    assert_eq!(r.result, Ok("9".to_string()));
    assert_eq!(r.output, ["9", "9", "9"]);
    // A getter load is no IC hit of the fast body, so it does not fuse (and the script has no
    // other producer of the change before a Star).
    assert!(fused.iter().all(|&n| n == 0), "{fused:?}");
}

/// f1: `GetNamedProperty r, "x"; Star r` (the Star target is the object register the producer
/// reads) and `LdaContextSlot c, 0, 0; Star c` (the context register), three calls.
fn star_into_the_input() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    let (save, r, c) = (f.b.new_register(), f.b.new_register(), f.b.new_register());
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
    f.b.ldar(r).add(c, f.bin).return_();

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
    assert_eq!(r.output, ["4", "6", "10", "4", "6", "10", "4", "6", "10"]);
    // Per call: `LdaCurrentContext; Star c` and `LdaContextSlot; Star c` (2), and
    // `GetNamedProperty; Star r` on the IC hits of calls 2 and 3.
    assert!(fused.iter().all(|&n| n == 3 * 2 + 2), "{fused:?}");
}

/// f1: `t = 5` reaches the Star by a jump (the Star handler runs), then `t = 9` reaches it by
/// the fall-through from a context-slot load (fused):
///
/// ```text
///     CreateFunctionContext; PushContext save; slot 0 = 5; slot 1 = 9
///     LdaFalse; Star flag            ; LdaFalse is no producer of the change
///     LdaCurrentContextSlot 0; Jump L ; no Star after this load
///  M: LdaCurrentContextSlot 1        ; falls through to the Star: fused
///  L: Star t                         ; a jump target
///     print(t)
///     Ldar flag; JumpIfTrue END
///     LdaTrue; Star flag
///     JumpLoop M
///  END: PopContext save; Ldar t; Return
/// ```
fn star_at_a_jump_target() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 0, 0, 0, 0));
    let (save, flag, t) = (f.b.new_register(), f.b.new_register(), f.b.new_register());
    let (m, l, end) = (f.b.new_label(), f.b.new_label(), f.b.new_label());
    let ks = f.b.scope_constant(ScopeId(0));
    f.b.create_function_context(ks, 2).push_context(save);
    f.b.lda_smi(5).sta_current_context_slot(0);
    f.b.lda_smi(9).sta_current_context_slot(1);
    f.b.lda_false().star(flag);
    f.b.lda_current_context_slot(0).jump(l);
    f.b.bind(m);
    f.b.lda_current_context_slot(1);
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

#[test]
fn a_star_at_a_jump_target_fuses_only_on_the_fall_through() {
    let (r, fused) = run_both(star_at_a_jump_target);
    assert_eq!(r.result, Ok("9".to_string()));
    assert_eq!(r.output, ["5", "9"]);
    // Only `LdaCurrentContextSlot 1; Star t` (the fall-through); the jump to the Star does
    // not fuse.
    assert!(fused.iter().all(|&n| n == 1), "{fused:?}");
}

/// The debugger off-switch. A breakpoint at the Star pc puts a break byte at that pc of the
/// function's Interp debug copy (H24-Q3). This test stands in for that copy with the same
/// code and one byte changed at the Star pc: `ToObject t` in place of `Star t` (the same
/// operand; it reads its own pc for its throw position, as a break reads the pc it stops
/// at). `o.x` is undefined, so the ToObject throws a TypeError.
const BREAK_SRC: &str = "t = o.x;\nBREAK;\n";

fn at_a_breakpoint(patched: bool) -> FunctionCode {
    let src = BREAK_SRC;
    let mut s = Asm::script(src.len());
    let (o, t) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_object_literal().star(o);
    s.b.lda_undefined();
    s.set_named(o, "x");
    s.b.lda_smi(1).star(t);
    // The first load records the IC case (a miss: no lookahead); the second hits and, with no
    // breakpoint, fuses its Star. The breakpoint is at the Star pc of the second load.
    s.b.set_statement_position(pos(src, "t = o.x"));
    s.get_named(o, "x");
    s.b.star(t);
    s.b.set_statement_position(pos(src, "t = o.x"));
    s.get_named(o, "x");
    s.b.set_statement_position(pos(src, "BREAK"));
    if patched {
        s.b.to_object(t);
    } else {
        s.b.star(t);
    }
    s.b.ldar(t).return_();
    s.finish()
}

fn breakpoint_off() -> VerifiedUnit {
    unit(BREAK_SRC, vec![at_a_breakpoint(false)])
}

fn breakpoint_on() -> VerifiedUnit {
    unit(BREAK_SRC, vec![at_a_breakpoint(true)])
}

#[test]
fn a_breakpoint_at_the_star_pc_turns_the_lookahead_off() {
    // The two forms differ in the opcode byte at the Star pc only.
    let (a, b) = (at_a_breakpoint(false), at_a_breakpoint(true));
    let diff: Vec<usize> = (0..a.bytecode().len())
        .filter(|&i| a.bytecode()[i] != b.bytecode()[i])
        .collect();
    assert_eq!(a.bytecode().len(), b.bytecode().len());
    assert_eq!(diff.len(), 1, "{diff:?}");
    for &i in &diff {
        assert_eq!(a.bytecode()[i], Opcode::Star as u8);
        assert_eq!(b.bytecode()[i], Opcode::ToObject as u8);
    }
    // No breakpoint: the IC hit of the second load fuses its Star.
    let (r, fused) = run_both(breakpoint_off);
    assert_eq!(r.result, Ok("undefined".to_string()));
    assert!(fused.iter().all(|&n| n == 1), "{fused:?}");
    // A breakpoint at the Star pc: the producer does not fuse past it, and the instruction at
    // the Star pc runs: its throw has the position of the Star pc (line 2), not of the
    // producer (line 1).
    let (r, fused) = run_both(breakpoint_on);
    assert!(fused.iter().all(|&n| n == 0), "{fused:?}");
    let e = r.result.unwrap_err();
    assert_eq!(e.constructor.as_deref(), Some("TypeError"), "{e:?}");
    let loc = e.location.unwrap();
    assert_eq!((loc.line, loc.column), (2, 1));
}
