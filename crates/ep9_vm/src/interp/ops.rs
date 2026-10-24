//! Opcode bodies: one `op!` per opcode of `ep9_bytecode::for_each_opcode!`, in list order.
//!
//! ```text
//! op!(Name(st, operand: Type, ..) { fast body }                 fast: -> Done {Next, Jump}
//! op!(Name(st, operand: Type, ..) { fast body } cold { body })  fast: -> Flow {Next, Jump, Cold}
//!                                                               cold: -> Done
//! ```
//!
//! - Operand types come from the operand kinds (`dsl::kind`): `R` register slot, `RList`,
//!   `Count`, `K` constant, `FSlot` feedback slot, `CSlot`, `Depth`, `Fwd`, `Back`, `RtId`,
//!   `TyLit`, `i32` (Imm), `u32` (UImm), `u8` (Flag8), `u16` (Flag16). A wrong type does not
//!   compile.
//! - A fast body makes no runtime call. It returns `Flow::Cold` to continue in the cold body
//!   (a separate handler in the threaded expansions, H15).
//! - `if_cold!` (end of file) lists the opcodes with a cold body. The body types make a wrong
//!   list a type error.
//! - Every shortcut of v0 is marked `// v0:`. Opcodes v0 does not run throw a catchable
//!   "not implemented in ep9 v0" Error.

#![allow(clippy::needless_return)]

use ep9_bytecode::TypeOfLiteral;
use ep9_value::Value;

use crate::call;
use crate::conv::BinOp;
use crate::dispatch::DispatchKind;
use crate::frame::{FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_bits, native_ptr, slot};
use crate::interp::dsl::{Back, CSlot, Count, Depth, FSlot, Fwd, K, R, RList, RtId, S2, S4, TyLit};
use crate::interp::slow::{self, CmpOp, CtxKind, Lit, NameMsg, PropOp, UnOp};
use crate::interp::{Done, Flow, St};
use crate::runtime::ErrorKind;
use crate::unit::FunctionInfo;
use crate::{ic, kinds, object};

macro_rules! op {
    ($Name:ident ($st:ident $(, $a:ident : $t:ty)*) $fast:block) => {
        #[allow(non_snake_case)]
        pub(crate) mod $Name {
            #[allow(unused_imports)]
            use super::*;
            #[inline(always)]
            #[allow(unused_variables)]
            pub(crate) fn fast($st: &mut St<'_> $(, $a: $t)*) -> Done $fast
        }
    };
    ($Name:ident ($st:ident $(, $a:ident : $t:ty)*) $fast:block cold $cold:block) => {
        #[allow(non_snake_case)]
        pub(crate) mod $Name {
            #[allow(unused_imports)]
            use super::*;
            #[inline(always)]
            #[allow(unused_variables)]
            pub(crate) fn fast($st: &mut St<'_> $(, $a: $t)*) -> Flow $fast
            #[inline(always)]
            #[allow(unused_variables)]
            pub(crate) fn cold($st: &mut St<'_> $(, $a: $t)*) -> Done $cold
        }
    };
}

// ---- value helpers (inline, no runtime calls) ----------------------------------------------

const INT32_TAG: u64 = 0xFFFE_0000_0000_0000;

#[inline(always)]
fn is_i32(b: u64) -> bool {
    b & INT32_TAG == INT32_TAG
}

#[inline(always)]
fn i32_of(b: u64) -> i32 {
    b as u32 as i32
}

#[inline(always)]
fn int(i: i32) -> u64 {
    int32_bits(i)
}

#[inline(always)]
fn num(d: f64) -> u64 {
    Value::number(d).to_bits()
}

#[inline(always)]
fn boolean(b: bool) -> u64 {
    if b { TRUE } else { FALSE }
}

/// The number in `b`, if it is one.
#[inline(always)]
fn number(b: u64) -> Option<f64> {
    if is_i32(b) {
        Some(f64::from(i32_of(b)))
    } else if b & INT32_TAG != 0 {
        Some(f64::from_bits(
            b.wrapping_sub(ep9_value::encoding::DOUBLE_ENCODE_OFFSET),
        ))
    } else {
        None
    }
}

/// ToBoolean for the values that need no cell access.
#[inline(always)]
fn fast_truthy(v: u64) -> Option<bool> {
    // Bit `k` of FALSY is set when the immediate `k` is false, undefined or null. A bit test
    // instead of a `match`: LLVM makes no jump table, so the handler has no data in its code.
    const FALSY: u64 = (1 << FALSE) | (1 << UNDEFINED) | (1 << NULL);
    const _: () = assert!(FALSE < 64 && UNDEFINED < 64 && NULL < 64);
    if v == TRUE {
        Some(true)
    } else if v < 64 && (FALSY >> v) & 1 != 0 {
        Some(false)
    } else if is_i32(v) {
        Some(i32_of(v) != 0)
    } else {
        None
    }
}

/// `l OP r` for numbers, or `None` for the cold path (other types, `**`, `%` and bitwise
/// operators on doubles).
#[inline(always)]
fn fast_binop(op: BinOp, l: u64, r: u64) -> Option<u64> {
    if is_i32(l) && is_i32(r) {
        let (a, b) = (i32_of(l), i32_of(r));
        let exact = match op {
            BinOp::Add => a.checked_add(b).map(int),
            BinOp::Sub => a.checked_sub(b).map(int),
            // The exact product rounded once: the JS result. `num` keeps -0.
            BinOp::Mul => Some(num(f64::from(a) * f64::from(b))),
            BinOp::Div => Some(num(f64::from(a) / f64::from(b))),
            // Same sign as the dividend in Rust and JS; a >= 0 avoids -0 and overflow.
            BinOp::Mod => (a >= 0 && b != 0).then(|| int(a % b)),
            BinOp::Exp => None,
            BinOp::BitOr => Some(int(a | b)),
            BinOp::BitXor => Some(int(a ^ b)),
            BinOp::BitAnd => Some(int(a & b)),
            BinOp::Shl => Some(int(a.wrapping_shl(b as u32 & 31))),
            BinOp::Sar => Some(int(a >> (b as u32 & 31))),
            BinOp::Shr => Some(num(f64::from((a as u32) >> (b as u32 & 31)))),
        };
        if exact.is_some() {
            return exact;
        }
    }
    let (x, y) = (number(l)?, number(r)?);
    match op {
        BinOp::Add => Some(num(x + y)),
        BinOp::Sub => Some(num(x - y)),
        BinOp::Mul => Some(num(x * y)),
        BinOp::Div => Some(num(x / y)),
        _ => None,
    }
}

#[inline(always)]
fn binop_fast(st: &mut St<'_>, op: BinOp, l: u64, r: u64) -> Flow {
    match fast_binop(op, l, r) {
        Some(v) => {
            st.acc = v;
            Flow::Next
        }
        None => Flow::Cold,
    }
}

#[inline(always)]
fn binop_cold(st: &mut St<'_>, op: BinOp, l: u64, r: u64) -> Done {
    let v = slow::binary(st.cx, st.fp, st.pc, op, l, r);
    st.result(v)
}

#[inline(always)]
fn unary_cold(st: &mut St<'_>, op: UnOp) -> Done {
    let v = slow::unary(st.cx, st.fp, st.pc, op, st.acc);
    st.result(v)
}

#[inline(always)]
fn compare_cold(st: &mut St<'_>, op: CmpOp, l: u64) -> Done {
    let v = slow::compare(st.cx, st.fp, st.pc, op, l, st.acc);
    st.result(v)
}

/// `l < r` style comparisons on two numbers (`None`: cold path).
#[inline(always)]
fn fast_relational(l: u64, r: u64, f: impl Fn(f64, f64) -> bool) -> Option<u64> {
    if is_i32(l) && is_i32(r) {
        return Some(boolean(f(f64::from(i32_of(l)), f64::from(i32_of(r)))));
    }
    Some(boolean(f(number(l)?, number(r)?)))
}

#[inline(always)]
fn compare_fast(st: &mut St<'_>, r: Option<u64>) -> Flow {
    match r {
        Some(v) => {
            st.acc = v;
            Flow::Next
        }
        None => Flow::Cold,
    }
}

#[inline(always)]
fn prop_cold(st: &mut St<'_>, op: PropOp, o: u64, k: u64, v: u64) -> Done {
    let r = slow::prop(st.cx, st.fp, st.pc, op, o, k, v);
    st.result(r)
}

#[inline(always)]
fn not_impl(st: &mut St<'_>, what: &'static str) -> Done {
    slow::not_impl(st.cx, st.fp, st.pc, what);
    st.unwind()
}

#[inline(always)]
fn throw_msg(st: &mut St<'_>, kind: ErrorKind, msg: &'static str) -> Done {
    slow::throw_msg(st.cx, st.fp, st.pc, kind, msg);
    st.unwind()
}

/// The context `depth` hops out from `c`, if every hop is a context.
#[inline(always)]
fn ctx_walk(mut c: u64, depth: u32) -> Option<u64> {
    for _ in 0..depth {
        if !object::is_context(c) {
            return None;
        }
        c = object::context_previous(c);
    }
    object::is_context(c).then_some(c)
}

#[inline(always)]
fn ctx_load(st: &mut St<'_>, c: u64, i: CSlot, depth: u32) -> Flow {
    match ctx_walk(c, depth).and_then(|c| object::context_get(c, i.0)) {
        Some(v) => {
            st.acc = v;
            Flow::Next
        }
        None => Flow::Cold,
    }
}

#[inline(always)]
fn ctx_store(st: &mut St<'_>, c: u64, i: CSlot, depth: u32) -> Flow {
    let rt = st.rt();
    let v = st.acc;
    match ctx_walk(c, depth) {
        Some(c) if object::context_set(rt, c, i.0, v) => Flow::Next,
        _ => Flow::Cold,
    }
}

#[inline(always)]
fn ctx_cold(st: &mut St<'_>) -> Done {
    let r = slow::bad_context(st.cx, st.fp, st.pc);
    st.result(r)
}

#[inline(always)]
fn jump_if(st: &St<'_>, c: bool, t: Fwd) -> Done {
    if c { Done::Jump(st.fwd(t)) } else { Done::Next }
}

/// The JS→JS call fast path (H14): a bytecode callee gets its frame right above the caller's
/// registers and the dispatch continues at its first instruction. No Rust recursion. Anything
/// else (host callee, class constructor, a callee of another realm, a sloppy callee with a
/// primitive receiver, interrupt due, the call that makes the callee's feedback vector, VM
/// stack full) takes the cold path.
///
/// While the callee has no feedback vector, a call that pushes its frame here counts in
/// `invocations`; the call that reaches `feedback_threshold` goes cold, and `slow::call`
/// makes the vector before it pushes the frame (H16, `feedback.rs`).
///
/// Budget (H23): every T0 call takes 1 unit. The decrement comes after every cold exit at
/// which `slow::call` takes the unit of the call itself (a primitive receiver, the feedback
/// threshold; M3-P2 PERF3). The 2 cold exits after the decrement give the unit back
/// ([`budget_out`]), so `slow::call` takes it at its frame push, also when that push fails:
/// the call whose unit uses up the budget (X-PF1; `slow::call` polls first if the budget is
/// 0) and a full VM stack (R3-2; `slow::call` throws the RangeError after its decrement).
/// The count is taken back at both exits ([`uncount_call`]).
///
/// The callee starts with the accumulator undefined, as after `slow::call`, so the caller's
/// accumulator is dead after the last cold exit (one register less in the frame push).
#[cfg(not(feature = "spike-native"))]
#[inline(always)]
fn call_fast(
    st: &mut St<'_>,
    callee: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(&St<'_>, u32) -> u64,
) -> Flow {
    if !object::is_function(callee) {
        return Flow::Cold;
    }
    let fi = object::function_info(callee);
    if fi.slot.kind_byte() != DispatchKind::Interp as u8 || fi.is_class_constructor() {
        return Flow::Cold;
    }
    // A callee of another realm: `slow::call` runs it in its realm (H22-Q3).
    if object::function_realm(callee) != st.cx.realm {
        return Flow::Cold;
    }
    // A sloppy callee with a primitive receiver: `slow::call` wraps it (H14-Q2).
    let Some(this) = call::try_bind_this(fi, this, st.cx.global_this) else {
        return Flow::Cold;
    };
    if !count_call(st, fi) {
        return Flow::Cold;
    }
    st.cx.budget -= 1;
    if st.cx.budget <= 0 {
        budget_out(st);
        uncount_call(fi);
        return Flow::Cold;
    }
    let base = st.fp.wrapping_add(st.info().register_count as usize);
    let (limit, fp, next) = (st.cx.stack_limit, st.fp, st.next);
    let context = object::function_context(callee);
    let s: &St<'_> = st;
    // SAFETY: `base` is the first free slot above the live top frame `fp`, `limit` is the VM
    // stack cap, and `arg` reads registers of the current verified instruction.
    let nfp = unsafe {
        call::push_frame(
            base,
            limit,
            fi,
            callee,
            this,
            argc,
            |i| arg(s, i),
            fp,
            next,
            context,
            UNDEFINED,
        )
    };
    match nfp {
        Some(n) => {
            st.fp = n;
            st.acc = UNDEFINED;
            Flow::Jump(fi.code)
        }
        None => {
            budget_out(st);
            uncount_call(fi);
            Flow::Cold
        }
    }
}

/// The T0 count of a call of `fi` while it has no feedback vector (`feedback::count_call`
/// without the vector): `false` for the call that reaches the threshold, which goes cold so
/// that `slow::call` makes the vector. A function with a vector costs one load and one test.
/// The count is taken before the frame push; a cold exit after it calls [`uncount_call`], so
/// the cold path counts the call once (in `slow::call`).
#[inline(always)]
fn count_call(st: &St<'_>, fi: &FunctionInfo) -> bool {
    if fi.feedback.get().is_null() {
        // Without a vector, `invocations` is below the threshold, so `+ 1` never overflows.
        let calls = fi.invocations.get() + 1;
        if calls >= st.cx.feedback_threshold {
            return false;
        }
        fi.invocations.set(calls);
    }
    true
}

/// Gives back the budget unit of a call that `call_fast` sends cold after its decrement:
/// the unit used up the budget (X-PF1), or the frame does not fit (R3-2). The cold path takes
/// the unit of the call again at its frame push, whether the push succeeds or fails:
/// `slow::call`, or under spike-native `enter_b_cold` or, for an AotBaseline callee,
/// `enter_native` (M3-P2 fix round 5, D1). So the call takes 1 unit, not 2 and not 0.
/// `slow::call` polls before that if the budget is now 0; at a budget of 1 it does not, and
/// the call leaves the budget at 0, so the next unit taker (a call or a `JumpLoop`) polls:
/// the poll comes at most 1 unit later. The same holds when the frame does not fit: the
/// RangeError leaves the budget at 0, so caught overflows do not starve the poll (also under
/// spike-native).
#[inline(always)]
fn budget_out(st: &mut St<'_>) {
    st.cx.budget += 1;
}

/// Takes back the count of [`count_call`] before a cold exit (the budget ran out, or the VM
/// stack is full). No GC point lies between the two, so the vector state is the same.
#[inline(always)]
fn uncount_call(fi: &FunctionInfo) {
    if fi.feedback.get().is_null() {
        fi.invocations.set(fi.invocations.get() - 1);
    }
}

/// `call_fast` of a `spike-native` build (Spike B, P4-PLACEMENT-B; M2-P4-B-OPS). A T0 callee
/// takes the path of the default build above. A callee with native code that the hot path
/// enters gets the same frame push, then the dispatch continues at a VM stub byte
/// (`native::hot_stub`): `OP_ENTER_B` for a placement (b) slot (`DispatchKind::Opt`, written
/// by `native::install_native_b`), whose handler tail-enters the (b) code with the 4 handler
/// arguments; `OP_ENTER_A_HOT` for the M2-P4-B-AB report (placement (a) code in an
/// AotBaseline slot), whose handler calls the native code. Both take one budget unit as a T0
/// callee does, record no feedback and are never class constructors (the installs refuse
/// them). Any other slot (placement (a) in a Baseline slot: config A of the D21 bench) goes
/// cold, as in the default build. The match loop runs the callee's bytecode at both stub
/// bytes (D4).
#[cfg(feature = "spike-native")]
#[inline(always)]
fn call_fast(
    st: &mut St<'_>,
    callee: u64,
    this: u64,
    argc: u32,
    arg: impl Fn(&St<'_>, u32) -> u64,
) -> Flow {
    if !object::is_function(callee) {
        return Flow::Cold;
    }
    // A callee of another realm: `slow::call` runs it in its realm (H22-Q3).
    if object::function_realm(callee) != st.cx.realm {
        return Flow::Cold;
    }
    let fi = object::function_info(callee);
    let kind = fi.slot.kind_byte();
    let interp = kind == DispatchKind::Interp as u8;
    if interp {
        if fi.is_class_constructor() {
            return Flow::Cold;
        }
    } else if crate::native::hot_stub(kind).is_none() {
        return Flow::Cold;
    }
    // A sloppy callee with a primitive receiver: `slow::call` wraps it (H14-Q2).
    let Some(this) = call::try_bind_this(fi, this, st.cx.global_this) else {
        return Flow::Cold;
    };
    // A native slot records no feedback (the count is a T0 count).
    if interp && !count_call(st, fi) {
        return Flow::Cold;
    }
    st.cx.budget -= 1;
    if st.cx.budget <= 0 {
        budget_out(st);
        if interp {
            uncount_call(fi);
        }
        return Flow::Cold;
    }
    let base = st.fp.wrapping_add(st.info().register_count as usize);
    let (limit, fp, next) = (st.cx.stack_limit, st.fp, st.next);
    let context = object::function_context(callee);
    let s: &St<'_> = st;
    // SAFETY: as in the default build's `call_fast`.
    let nfp = unsafe {
        call::push_frame(
            base,
            limit,
            fi,
            callee,
            this,
            argc,
            |i| arg(s, i),
            fp,
            next,
            context,
            UNDEFINED,
        )
    };
    match nfp {
        Some(n) => {
            st.fp = n;
            // The slot kind is read again here, so no stub address stays live across the
            // frame push (a spike-native build keeps the default build's register use). The
            // stub gets the caller's accumulator as the 4th handler argument (the (b) entry
            // test `b_code_gets_the_four_handler_arguments_and_tail_jumps_back` reads it); a
            // T0 callee starts with it undefined, as in the default build.
            match crate::native::hot_stub(fi.slot.kind_byte()) {
                Some(s) => Flow::Jump(s),
                None => {
                    st.acc = UNDEFINED;
                    Flow::Jump(fi.code)
                }
            }
        }
        None => {
            if interp {
                uncount_call(fi);
            }
            // `slow::call`, `enter_b_cold` or `enter_native` takes the unit, also when the
            // push fails there (R3-2; `enter_native` since M3-P2 fix round 5, D1).
            budget_out(st);
            Flow::Cold
        }
    }
}

/// Every call and construct cold path: `slow::call` pushes the frame or runs the host
/// function, then the dispatch continues where it says.
#[inline(always)]
fn call_cold(st: &mut St<'_>) -> Done {
    let v = slow::call(st.cx, st.fp, st.pc, st.acc);
    st.switch(v)
}

/// `CallProperty`/`CallAnyReceiver`: the list holds the receiver, then the arguments.
#[inline(always)]
fn call_list_with_receiver(st: &mut St<'_>, callee: R, args: RList, argc: Count) -> Flow {
    if argc.0 == 0 {
        return Flow::Cold;
    }
    let (f, this) = (st.reg(callee), st.list(args, 0));
    call_fast(st, f, this, argc.0 - 1, move |s, i| s.list(args, i + 1))
}

// ---- special -------------------------------------------------------------------------------

op!(Illegal(st) { Flow::Cold } cold { not_impl(st, "the Illegal opcode") });
op!(Wide(st) { Flow::Cold } cold { st.scaled::<S2>() });
op!(ExtraWide(st) { Flow::Cold } cold { st.scaled::<S4>() });

// ---- accumulator and register moves --------------------------------------------------------

op!(LdaZero(st) {
    st.acc = int(0);
    Done::Next
});
op!(LdaSmi(st, value: i32) {
    st.acc = int(value);
    Done::Next
});
op!(LdaUndefined(st) {
    st.acc = UNDEFINED;
    Done::Next
});
op!(LdaNull(st) {
    st.acc = NULL;
    Done::Next
});
op!(LdaTheHole(st) {
    st.acc = HOLE;
    Done::Next
});
op!(LdaTrue(st) {
    st.acc = TRUE;
    Done::Next
});
op!(LdaFalse(st) {
    st.acc = FALSE;
    Done::Next
});
op!(LdaConstant(st, k: K) {
    let v = st.constant(k);
    if v == HOLE {
        return Flow::Cold;
    }
    st.acc = v;
    Flow::Next
} cold {
    // The hole is a BigInt (v0: no BigInt values) or the Oddball hole.
    if matches!(st.info().constant_entry(k.0), ep9_bytecode::Constant::BigInt(_)) {
        return not_impl(st, "BigInt values");
    }
    st.acc = HOLE;
    Done::Next
});
op!(Ldar(st, src: R) {
    st.acc = st.reg(src);
    Done::Next
});
op!(Star(st, dst: R) {
    let v = st.acc;
    st.set_reg(dst, v);
    Done::Next
});
op!(Mov(st, src: R, dst: R) {
    let v = st.reg(src);
    st.set_reg(dst, v);
    Done::Next
});
op!(LdaCurrentFunction(st) {
    st.acc = st.hdr(slot::CALLEE);
    Done::Next
});
// FRAME 2 (H14-Q1): fp-3 holds new.target, undefined for a call (call::push_frame), or the
// generator object of a resumed frame (call::push_resume_frame). No construct-bit check.
op!(LdaNewTarget(st) {
    st.acc = st.hdr(slot::NEW_TARGET);
    Done::Next
});

// ---- globals -------------------------------------------------------------------------------

// Global ICs (H16, `ic.rs`): a lexical binding or an own data property of the global object.
op!(LdaGlobal(st, name: K, s: FSlot) {
    match ic::load_global(st.rt(), st.info(), st.cx.global_this, s.0) {
        Some(v) => {
            ic::ic_count!(hit, LoadGlobal);
            st.acc = v;
            Flow::Next
        }
        None => {
            ic::ic_count!(miss, LoadGlobal, st.info(), s.0);
            Flow::Cold
        }
    }
} cold {
    let n = st.constant(name);
    let r = slow::load_global(st.cx, st.fp, st.pc, n, false, s.0);
    st.result(r)
});
op!(LdaGlobalInsideTypeof(st, name: K, s: FSlot) {
    match ic::load_global(st.rt(), st.info(), st.cx.global_this, s.0) {
        Some(v) => {
            ic::ic_count!(hit, LoadGlobal);
            st.acc = v;
            Flow::Next
        }
        None => {
            ic::ic_count!(miss, LoadGlobal, st.info(), s.0);
            Flow::Cold
        }
    }
} cold {
    let n = st.constant(name);
    let r = slow::load_global(st.cx, st.fp, st.pc, n, true, s.0);
    st.result(r)
});
op!(StaGlobal(st, name: K, s: FSlot) {
    let v = st.acc;
    if ic::store_global(st.rt(), st.info(), st.cx.global_this, s.0, v) {
        ic::ic_count!(hit, StoreGlobal);
        Flow::Next
    } else {
        ic::ic_count!(miss, StoreGlobal, st.info(), s.0);
        Flow::Cold
    }
} cold {
    let n = st.constant(name);
    let r = slow::store_global(st.cx, st.fp, st.pc, n, st.acc, s.0);
    st.result(r)
});

// ---- contexts ------------------------------------------------------------------------------

op!(PushContext(st, save: R) {
    let cur = st.hdr(slot::CONTEXT);
    st.set_reg(save, cur);
    let v = st.acc;
    st.set_hdr(slot::CONTEXT, v);
    Done::Next
});
op!(PopContext(st, context: R) {
    let c = st.reg(context);
    st.set_hdr(slot::CONTEXT, c);
    Done::Next
});
op!(LdaCurrentContext(st) {
    st.acc = st.hdr(slot::CONTEXT);
    Done::Next
});
op!(LdaContextSlot(st, context: R, i: CSlot, depth: Depth) {
    let c = st.reg(context);
    ctx_load(st, c, i, depth.0)
} cold { ctx_cold(st) });
op!(LdaImmutableContextSlot(st, context: R, i: CSlot, depth: Depth) {
    let c = st.reg(context);
    ctx_load(st, c, i, depth.0)
} cold { ctx_cold(st) });
op!(LdaCurrentContextSlot(st, i: CSlot) {
    let c = st.hdr(slot::CONTEXT);
    ctx_load(st, c, i, 0)
} cold { ctx_cold(st) });
op!(LdaImmutableCurrentContextSlot(st, i: CSlot) {
    let c = st.hdr(slot::CONTEXT);
    ctx_load(st, c, i, 0)
} cold { ctx_cold(st) });
op!(StaContextSlot(st, context: R, i: CSlot, depth: Depth) {
    let c = st.reg(context);
    ctx_store(st, c, i, depth.0)
} cold { ctx_cold(st) });
op!(StaCurrentContextSlot(st, i: CSlot) {
    let c = st.hdr(slot::CONTEXT);
    ctx_store(st, c, i, 0)
} cold { ctx_cold(st) });
// Dynamic scope lookup (sloppy direct eval, `with`): slow/lookup.rs (P2-LOOKUP).
op!(LdaLookupSlot(st, name: K) { Flow::Cold } cold {
    let r = slow::lda_lookup_slot(st.cx, st.fp, st.pc, name.0, false);
    st.result(r)
});
op!(LdaLookupSlotInsideTypeof(st, name: K) { Flow::Cold } cold {
    let r = slow::lda_lookup_slot(st.cx, st.fp, st.pc, name.0, true);
    st.result(r)
});
op!(StaLookupSlot(st, name: K, flags: u8) { Flow::Cold } cold {
    let v = st.acc;
    let r = slow::sta_lookup_slot(st.cx, st.fp, st.pc, name.0, flags, v);
    st.result(r)
});
// Module variables: slow/module.rs (M3-P2 MODULE; M3-P0 stubs throw "module variables").
op!(LdaModuleVariable(st, cell: i32, depth: Depth) { Flow::Cold } cold {
    let r = slow::lda_module_variable(st.cx, st.fp, st.pc, cell, depth.0);
    st.result(r)
});
op!(StaModuleVariable(st, cell: i32, depth: Depth) { Flow::Cold } cold {
    let v = st.acc;
    let r = slow::sta_module_variable(st.cx, st.fp, st.pc, cell, depth.0, v);
    st.result(r)
});

// ---- property access -----------------------------------------------------------------------

// Named loads and stores run the IC hit path inline (H16, `ic.rs`); a miss runs the generic
// operation and records a case.
op!(GetNamedProperty(st, object: R, name: K, s: FSlot) {
    let o = st.reg(object);
    match ic::load(st.info(), st.cx.proto_epoch, o, s.0) {
        Some(v) => {
            ic::ic_count!(hit, Load);
            st.acc = v;
            Flow::Next
        }
        None => {
            ic::ic_count!(miss, Load, st.info(), s.0);
            Flow::Cold
        }
    }
} cold {
    let (o, k) = (st.reg(object), st.constant(name));
    let r = slow::get_named(st.cx, st.fp, st.pc, o, k, s.0);
    st.result(r)
});
// The accumulator holds the home object; the IC is keyed on its prototype (the lookup start)
// and `receiver` is the `this` a getter sees.
op!(GetNamedPropertyFromSuper(st, receiver: R, name: K, s: FSlot) {
    let home = st.acc;
    if !object::is_object(home) {
        return Flow::Cold;
    }
    match ic::load(st.info(), st.cx.proto_epoch, object::proto_of(home), s.0) {
        Some(v) => {
            ic::ic_count!(hit, Load);
            st.acc = v;
            Flow::Next
        }
        None => {
            ic::ic_count!(miss, Load, st.info(), s.0);
            Flow::Cold
        }
    }
} cold {
    let (home, k, this) = (st.acc, st.constant(name), st.reg(receiver));
    let r = slow::get_super(st.cx, st.fp, st.pc, home, k, this, s.0);
    st.result(r)
});
op!(GetKeyedProperty(st, object: R, _s: FSlot) {
    let (o, k) = (st.reg(object), st.acc);
    if is_i32(k)
        && i32_of(k) >= 0
        && object::is_array(o)
        && let Some(v) = object::array_get(o, i32_of(k) as u32)
    {
        st.acc = v;
        return Flow::Next;
    }
    Flow::Cold
} cold {
    let (o, k) = (st.reg(object), st.acc);
    prop_cold(st, PropOp::GetKeyed, o, k, UNDEFINED)
});
op!(SetNamedProperty(st, object: R, name: K, s: FSlot) {
    let (o, v) = (st.reg(object), st.acc);
    if ic::store::<true>(st.rt(), st.info(), st.cx.proto_epoch, o, s.0, v) {
        ic::ic_count!(hit, Store);
        Flow::Next
    } else {
        ic::ic_count!(miss, Store, st.info(), s.0);
        Flow::Cold
    }
} cold {
    let (o, k, v) = (st.reg(object), st.constant(name), st.acc);
    let r = slow::set_named(st.cx, st.fp, st.pc, o, k, v, s.0, false);
    st.result(r)
});
// CreateDataPropertyOrThrow: an add needs no epoch (the prototype chain does not matter).
op!(DefineNamedOwnProperty(st, object: R, name: K, s: FSlot) {
    let (o, v) = (st.reg(object), st.acc);
    if ic::store::<false>(st.rt(), st.info(), st.cx.proto_epoch, o, s.0, v) {
        ic::ic_count!(hit, Store);
        Flow::Next
    } else {
        ic::ic_count!(miss, Store, st.info(), s.0);
        Flow::Cold
    }
} cold {
    let (o, k, v) = (st.reg(object), st.constant(name), st.acc);
    let r = slow::set_named(st.cx, st.fp, st.pc, o, k, v, s.0, true);
    st.result(r)
});
op!(SetKeyedProperty(st, object: R, key: R, _s: FSlot) { Flow::Cold } cold {
    let (o, k, v) = (st.reg(object), st.reg(key), st.acc);
    prop_cold(st, PropOp::SetKeyed, o, k, v)
});
op!(DefineKeyedOwnProperty(st, object: R, key: R, flags: u8, _s: FSlot) { Flow::Cold } cold {
    let (o, k, v) = (st.reg(object), st.reg(key), st.acc);
    prop_cold(st, PropOp::DefineKeyedOwn(flags), o, k, v)
});
op!(StaInArrayLiteral(st, array: R, index: R, _s: FSlot) { Flow::Cold } cold {
    let (o, k, v) = (st.reg(array), st.reg(index), st.acc);
    prop_cold(st, PropOp::DefineKeyed, o, k, v)
});
// Flag bit 0 (set the function name from the key) is applied in slow/class_fields.rs
// `define_keyed_own`, from the DefineKeyedOwn arm of slow/property.rs (D15).
op!(DefineKeyedOwnPropertyInLiteral(st, object: R, key: R, flags: u8, _s: FSlot) { Flow::Cold } cold {
    let (o, k, v) = (st.reg(object), st.reg(key), st.acc);
    prop_cold(st, PropOp::DefineKeyedOwn(flags), o, k, v)
});

// ---- binary operators: acc = lhs OP acc ----------------------------------------------------

macro_rules! binary_ops {
    ($($Name:ident => $op:ident,)*) => {$(
        op!($Name(st, lhs: R, _s: FSlot) {
            let (l, r) = (st.reg(lhs), st.acc);
            binop_fast(st, BinOp::$op, l, r)
        } cold {
            let (l, r) = (st.reg(lhs), st.acc);
            binop_cold(st, BinOp::$op, l, r)
        });
    )*};
}

binary_ops! {
    Add => Add,
    Sub => Sub,
    Mul => Mul,
    Div => Div,
    Mod => Mod,
    Exp => Exp,
    BitwiseOr => BitOr,
    BitwiseXor => BitXor,
    BitwiseAnd => BitAnd,
    ShiftLeft => Shl,
    ShiftRight => Sar,
    ShiftRightLogical => Shr,
}

// ---- binary operators with an immediate: acc = acc OP value --------------------------------

macro_rules! smi_ops {
    ($($Name:ident => $op:ident,)*) => {$(
        op!($Name(st, value: i32, _s: FSlot) {
            let l = st.acc;
            binop_fast(st, BinOp::$op, l, int(value))
        } cold {
            let l = st.acc;
            binop_cold(st, BinOp::$op, l, int(value))
        });
    )*};
}

smi_ops! {
    AddSmi => Add,
    SubSmi => Sub,
    MulSmi => Mul,
    DivSmi => Div,
    ModSmi => Mod,
    ExpSmi => Exp,
    BitwiseOrSmi => BitOr,
    BitwiseXorSmi => BitXor,
    BitwiseAndSmi => BitAnd,
    ShiftLeftSmi => Shl,
    ShiftRightSmi => Sar,
    ShiftRightLogicalSmi => Shr,
}

// ---- unary operators -----------------------------------------------------------------------

op!(Inc(st, _s: FSlot) {
    let v = st.acc;
    if is_i32(v) && i32_of(v) != i32::MAX {
        st.acc = int(i32_of(v) + 1);
        return Flow::Next;
    }
    match number(v) {
        Some(x) => {
            st.acc = num(x + 1.0);
            Flow::Next
        }
        None => Flow::Cold,
    }
} cold { unary_cold(st, UnOp::Inc) });
op!(Dec(st, _s: FSlot) {
    let v = st.acc;
    if is_i32(v) && i32_of(v) != i32::MIN {
        st.acc = int(i32_of(v) - 1);
        return Flow::Next;
    }
    match number(v) {
        Some(x) => {
            st.acc = num(x - 1.0);
            Flow::Next
        }
        None => Flow::Cold,
    }
} cold { unary_cold(st, UnOp::Dec) });
op!(Negate(st, _s: FSlot) {
    match number(st.acc) {
        Some(x) => {
            st.acc = num(-x);
            Flow::Next
        }
        None => Flow::Cold,
    }
} cold { unary_cold(st, UnOp::Negate) });
op!(BitwiseNot(st, _s: FSlot) {
    let v = st.acc;
    if is_i32(v) {
        st.acc = int(!i32_of(v));
        return Flow::Next;
    }
    Flow::Cold
} cold { unary_cold(st, UnOp::BitNot) });
op!(ToBooleanLogicalNot(st) {
    match fast_truthy(st.acc) {
        Some(b) => {
            st.acc = boolean(!b);
            Flow::Next
        }
        None => Flow::Cold,
    }
} cold {
    st.acc = boolean(!slow::to_boolean(st.acc));
    Done::Next
});
// The accumulator is a boolean (BytecodeGen contract, as in V8).
op!(LogicalNot(st) {
    st.acc = boolean(st.acc == FALSE);
    Done::Next
});
op!(TypeOf(st, _s: FSlot) { Flow::Cold } cold {
    st.acc = slow::type_of(st.cx, st.acc);
    Done::Next
});
op!(DeletePropertyStrict(st, object: R) { Flow::Cold } cold {
    let (o, k) = (st.reg(object), st.acc);
    prop_cold(st, PropOp::DeleteStrict, o, k, UNDEFINED)
});
op!(DeletePropertySloppy(st, object: R) { Flow::Cold } cold {
    let (o, k) = (st.reg(object), st.acc);
    prop_cold(st, PropOp::DeleteSloppy, o, k, UNDEFINED)
});

// ---- comparisons: acc = lhs OP acc ---------------------------------------------------------

op!(TestEqual(st, lhs: R, _s: FSlot) {
    let (l, r) = (st.reg(lhs), st.acc);
    if is_i32(l) && is_i32(r) {
        st.acc = boolean(l == r);
        return Flow::Next;
    }
    Flow::Cold
} cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::Eq, l)
});
op!(TestEqualStrict(st, lhs: R, _s: FSlot) {
    let (l, r) = (st.reg(lhs), st.acc);
    if let (Some(x), Some(y)) = (number(l), number(r)) {
        st.acc = boolean(x == y);
        return Flow::Next;
    }
    if l == r {
        st.acc = TRUE;
        return Flow::Next;
    }
    if object::is_string(l) && object::is_string(r) {
        return Flow::Cold;
    }
    st.acc = FALSE;
    Flow::Next
} cold {
    let l = st.reg(lhs);
    st.acc = boolean(slow::strict_equals(l, st.acc));
    Done::Next
});
op!(TestLessThan(st, lhs: R, _s: FSlot) {
    let r = fast_relational(st.reg(lhs), st.acc, |x, y| x < y);
    compare_fast(st, r)
} cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::Lt, l)
});
op!(TestGreaterThan(st, lhs: R, _s: FSlot) {
    let r = fast_relational(st.reg(lhs), st.acc, |x, y| x > y);
    compare_fast(st, r)
} cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::Gt, l)
});
op!(TestLessThanOrEqual(st, lhs: R, _s: FSlot) {
    let r = fast_relational(st.reg(lhs), st.acc, |x, y| x <= y);
    compare_fast(st, r)
} cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::Le, l)
});
op!(TestGreaterThanOrEqual(st, lhs: R, _s: FSlot) {
    let r = fast_relational(st.reg(lhs), st.acc, |x, y| x >= y);
    compare_fast(st, r)
} cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::Ge, l)
});
op!(TestReferenceEqual(st, lhs: R) {
    st.acc = boolean(st.reg(lhs) == st.acc);
    Done::Next
});
op!(TestInstanceOf(st, lhs: R, _s: FSlot) { Flow::Cold } cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::InstanceOf, l)
});
op!(TestIn(st, lhs: R, _s: FSlot) { Flow::Cold } cold {
    let l = st.reg(lhs);
    compare_cold(st, CmpOp::In, l)
});
// v0: no `document.all`, so only null and undefined are undetectable (as in V8 otherwise).
op!(TestUndetectable(st) {
    st.acc = boolean(st.acc == NULL || st.acc == UNDEFINED);
    Done::Next
});
op!(TestNull(st) {
    st.acc = boolean(st.acc == NULL);
    Done::Next
});
op!(TestUndefined(st) {
    st.acc = boolean(st.acc == UNDEFINED);
    Done::Next
});
op!(TestTypeOf(st, literal: TyLit) {
    let v = st.acc;
    let r = match TypeOfLiteral::from_u8(literal.0) {
        Some(TypeOfLiteral::Number) => number(v).is_some(),
        Some(TypeOfLiteral::String) => object::is_string(v),
        Some(TypeOfLiteral::Boolean) => v == TRUE || v == FALSE,
        Some(TypeOfLiteral::Undefined) => v == UNDEFINED,
        Some(TypeOfLiteral::Function) => kinds::is_callable(v),
        Some(TypeOfLiteral::Object) => {
            v == NULL || (object::is_object(v) && !kinds::is_callable(v))
        }
        Some(TypeOfLiteral::Symbol) => kinds::is_symbol(v),
        // v0: no BigInts (and no symbol values exist yet).
        _ => false,
    };
    st.acc = boolean(r);
    Done::Next
});

// ---- conversions ---------------------------------------------------------------------------

op!(ToName(st) {
    if object::is_string(st.acc) { Flow::Next } else { Flow::Cold }
} cold { unary_cold(st, UnOp::ToName) });
op!(ToNumber(st, _s: FSlot) {
    if number(st.acc).is_some() { Flow::Next } else { Flow::Cold }
} cold { unary_cold(st, UnOp::ToNumber) });
op!(ToNumeric(st, _s: FSlot) {
    if number(st.acc).is_some() { Flow::Next } else { Flow::Cold }
} cold { unary_cold(st, UnOp::ToNumeric) });
op!(ToObject(st, dst: R) {
    let v = st.acc;
    if object::is_object(v) {
        st.set_reg(dst, v);
        return Flow::Next;
    }
    Flow::Cold
} cold {
    let r = slow::unary(st.cx, st.fp, st.pc, UnOp::ToObject, st.acc);
    if r == 0 {
        return st.unwind();
    }
    st.set_reg(dst, r);
    Done::Next
});
op!(ToString(st) {
    if object::is_string(st.acc) { Flow::Next } else { Flow::Cold }
} cold { unary_cold(st, UnOp::ToString) });
op!(ToBoolean(st) {
    match fast_truthy(st.acc) {
        Some(b) => {
            st.acc = boolean(b);
            Flow::Next
        }
        None => Flow::Cold,
    }
} cold {
    st.acc = boolean(slow::to_boolean(st.acc));
    Done::Next
});

// ---- calls ---------------------------------------------------------------------------------

op!(CallAnyReceiver(st, callee: R, args: RList, argc: Count, _s: FSlot) {
    call_list_with_receiver(st, callee, args, argc)
} cold { call_cold(st) });
op!(CallProperty(st, callee: R, args: RList, argc: Count, _s: FSlot) {
    call_list_with_receiver(st, callee, args, argc)
} cold { call_cold(st) });
op!(CallProperty0(st, callee: R, receiver: R, _s: FSlot) {
    let (f, this) = (st.reg(callee), st.reg(receiver));
    call_fast(st, f, this, 0, |_, _| UNDEFINED)
} cold { call_cold(st) });
op!(CallProperty1(st, callee: R, receiver: R, a0: R, _s: FSlot) {
    let (f, this, v0) = (st.reg(callee), st.reg(receiver), st.reg(a0));
    call_fast(st, f, this, 1, move |_, _| v0)
} cold { call_cold(st) });
op!(CallProperty2(st, callee: R, receiver: R, a0: R, a1: R, _s: FSlot) {
    let (f, this, v0, v1) = (st.reg(callee), st.reg(receiver), st.reg(a0), st.reg(a1));
    call_fast(st, f, this, 2, move |_, i| if i == 0 { v0 } else { v1 })
} cold { call_cold(st) });
op!(CallUndefinedReceiver(st, callee: R, args: RList, argc: Count, _s: FSlot) {
    let f = st.reg(callee);
    call_fast(st, f, UNDEFINED, argc.0, move |s, i| s.list(args, i))
} cold { call_cold(st) });
op!(CallUndefinedReceiver0(st, callee: R, _s: FSlot) {
    let f = st.reg(callee);
    call_fast(st, f, UNDEFINED, 0, |_, _| UNDEFINED)
} cold { call_cold(st) });
op!(CallUndefinedReceiver1(st, callee: R, a0: R, _s: FSlot) {
    let (f, v0) = (st.reg(callee), st.reg(a0));
    call_fast(st, f, UNDEFINED, 1, move |_, _| v0)
} cold { call_cold(st) });
op!(CallUndefinedReceiver2(st, callee: R, a0: R, a1: R, _s: FSlot) {
    let (f, v0, v1) = (st.reg(callee), st.reg(a0), st.reg(a1));
    call_fast(st, f, UNDEFINED, 2, move |_, i| if i == 0 { v0 } else { v1 })
} cold { call_cold(st) });
op!(CallWithSpread(st, _callee: R, _args: RList, _argc: Count, _s: FSlot) { Flow::Cold } cold {
    call_cold(st)
});
op!(CallRuntime(st, id: RtId, args: RList, argc: Count) { Flow::Cold } cold {
    let r = slow::call_runtime(st.cx, st.fp, st.pc, id.0, args.0, argc.0);
    st.result(r)
});
// The accumulator holds `new.target`.
op!(Construct(st, _ctor: R, _args: RList, _argc: Count, _s: FSlot) { Flow::Cold } cold {
    call_cold(st)
});
op!(ConstructWithSpread(st, _ctor: R, _args: RList, _argc: Count, _s: FSlot) { Flow::Cold } cold {
    call_cold(st)
});
// Reserved (D7): the verifier rejects it.
op!(TailCall(st, _callee: R, _args: RList, _argc: Count, _s: FSlot) { Flow::Cold } cold {
    not_impl(st, "TailCall (reserved)")
});

// ---- literals ------------------------------------------------------------------------------

op!(CreateRegExpLiteral(st, _pattern: K, _s: FSlot, _flags: u16) { Flow::Cold } cold {
    not_impl(st, "RegExp literals")
});
op!(CreateArrayLiteral(st, boilerplate: K, _s: FSlot, _flags: u8) { Flow::Cold } cold {
    let r = slow::literal(st.cx, st.fp, st.pc, Lit::Array, boilerplate.0);
    st.result(r)
});
op!(CreateEmptyArrayLiteral(st, _s: FSlot) { Flow::Cold } cold {
    let r = slow::literal(st.cx, st.fp, st.pc, Lit::EmptyArray, 0);
    st.result(r)
});
op!(CreateArrayFromIterable(st) { Flow::Cold } cold {
    let r = slow::array_from_iterable(st.cx, st.fp, st.pc, st.acc);
    st.result(r)
});
op!(CreateObjectLiteral(st, boilerplate: K, _s: FSlot, _flags: u8) { Flow::Cold } cold {
    let r = slow::literal(st.cx, st.fp, st.pc, Lit::Object, boilerplate.0);
    st.result(r)
});
op!(CreateEmptyObjectLiteral(st) { Flow::Cold } cold {
    let r = slow::literal(st.cx, st.fp, st.pc, Lit::EmptyObject, 0);
    st.result(r)
});
op!(CloneObject(st, source: R, _flags: u8, _s: FSlot) { Flow::Cold } cold {
    let s = st.reg(source);
    let r = slow::clone_object(st.cx, st.fp, st.pc, s);
    st.result(r)
});
op!(GetTemplateObject(st, description: K, _s: FSlot) { Flow::Cold } cold {
    let r = slow::literal(st.cx, st.fp, st.pc, Lit::Template, description.0);
    st.result(r)
});

// ---- closures, contexts, arguments ---------------------------------------------------------

op!(CreateClosure(st, function: K, _s: FSlot, _flags: u8) { Flow::Cold } cold {
    let r = slow::create_closure(st.cx, st.fp, st.pc, function.0);
    st.result(r)
});
op!(CreateFunctionContext(st, scope: K, slots: u32) { Flow::Cold } cold {
    let r = slow::create_context(st.cx, st.fp, st.pc, CtxKind::Function, scope.0, slots, UNDEFINED);
    st.result(r)
});
op!(CreateBlockContext(st, scope: K) { Flow::Cold } cold {
    let r = slow::create_context(st.cx, st.fp, st.pc, CtxKind::Block, scope.0, 0, UNDEFINED);
    st.result(r)
});
op!(CreateCatchContext(st, exception: R, scope: K) { Flow::Cold } cold {
    let e = st.reg(exception);
    let r = slow::create_context(st.cx, st.fp, st.pc, CtxKind::Catch, scope.0, 0, e);
    st.result(r)
});
op!(CreateEvalContext(st, scope: K, slots: u32) { Flow::Cold } cold {
    let r = slow::create_context(st.cx, st.fp, st.pc, CtxKind::Eval, scope.0, slots, UNDEFINED);
    st.result(r)
});
op!(CreateWithContext(st, object: R, scope: K) { Flow::Cold } cold {
    let o = st.reg(object);
    let r = slow::create_with_context(st.cx, st.fp, st.pc, o, scope.0);
    st.result(r)
});
// Arguments objects: arguments.rs (M3-P2 VM-ARGS; V8-A15).
op!(CreateMappedArguments(st) { Flow::Cold } cold {
    let r = crate::arguments::create(st.cx, st.fp, st.pc, true);
    st.result(r)
});
op!(CreateUnmappedArguments(st) { Flow::Cold } cold {
    let r = crate::arguments::create(st.cx, st.fp, st.pc, false);
    st.result(r)
});
op!(CreateRestParameter(st) { Flow::Cold } cold {
    let r = slow::arguments(st.cx, st.fp, st.pc, true);
    st.result(r)
});

// ---- control flow --------------------------------------------------------------------------

// The interrupt poll of loops (04 §2): one budget unit per back edge.
op!(JumpLoop(st, target: Back, _depth: i32, _s: FSlot) {
    st.cx.budget -= 1;
    if st.cx.budget > 0 { Flow::Jump(st.back(target)) } else { Flow::Cold }
} cold {
    if slow::interrupt_at(st.cx, st.fp, st.pc) == 0 {
        return st.unwind();
    }
    Done::Jump(st.back(target))
});
op!(Jump(st, target: Fwd) { Done::Jump(st.fwd(target)) });
op!(JumpIfTrue(st, target: Fwd) { jump_if(st, st.acc == TRUE, target) });
op!(JumpIfFalse(st, target: Fwd) { jump_if(st, st.acc == FALSE, target) });
op!(JumpIfToBooleanTrue(st, target: Fwd) {
    match fast_truthy(st.acc) {
        Some(true) => Flow::Jump(st.fwd(target)),
        Some(false) => Flow::Next,
        None => Flow::Cold,
    }
} cold { jump_if(st, slow::to_boolean(st.acc), target) });
op!(JumpIfToBooleanFalse(st, target: Fwd) {
    match fast_truthy(st.acc) {
        Some(false) => Flow::Jump(st.fwd(target)),
        Some(true) => Flow::Next,
        None => Flow::Cold,
    }
} cold { jump_if(st, !slow::to_boolean(st.acc), target) });
op!(JumpIfNull(st, target: Fwd) { jump_if(st, st.acc == NULL, target) });
op!(JumpIfNotNull(st, target: Fwd) { jump_if(st, st.acc != NULL, target) });
op!(JumpIfUndefined(st, target: Fwd) { jump_if(st, st.acc == UNDEFINED, target) });
op!(JumpIfNotUndefined(st, target: Fwd) { jump_if(st, st.acc != UNDEFINED, target) });
op!(JumpIfUndefinedOrNull(st, target: Fwd) {
    jump_if(st, st.acc == UNDEFINED || st.acc == NULL, target)
});
op!(JumpIfJSReceiver(st, target: Fwd) { jump_if(st, object::is_object(st.acc), target) });
op!(JumpIfForInDone(st, target: Fwd, index: R, cache_length: R) {
    jump_if(st, st.reg(index) == st.reg(cache_length), target)
});
// A non-int32 accumulator falls through (V8).
op!(SwitchOnSmi(st, table: K, base: i32) {
    if is_i32(st.acc) { Flow::Cold } else { Flow::Next }
} cold {
    Done::Jump(slow::switch_target(st.fp, st.pc, st.next, table.0, base, i32_of(st.acc)))
});

// ---- iteration -----------------------------------------------------------------------------

op!(ForInEnumerate(st, receiver: R) { Flow::Cold } cold {
    let o = st.reg(receiver);
    let r = slow::for_in_enumerate(st.cx, st.fp, st.pc, o);
    st.result(r)
});
// v0: the key array is the cache type and the cache array (no enum caches yet).
op!(ForInPrepare(st, cache: R, _s: FSlot) {
    let keys = st.acc;
    if !object::is_array(keys) {
        return Flow::Cold;
    }
    let n = int(object::array_len(keys) as i32);
    st.set_reg(cache, keys);
    st.set_reg(cache.plus(1), keys);
    st.set_reg(cache.plus(2), n);
    Flow::Next
} cold {
    throw_msg(st, ErrorKind::Error, "ep9 internal error: ForInPrepare without a key array")
});
op!(ForInNext(st, receiver: R, index: R, cache: R, _s: FSlot) { Flow::Cold } cold {
    let (o, keys, i) = (st.reg(receiver), st.reg(cache.plus(1)), st.reg(index));
    let r = slow::for_in_next(st.cx, st.fp, st.pc, o, keys, i);
    st.result(r)
});
op!(ForInStep(st, index: R) {
    let i = i32_of(st.reg(index));
    st.set_reg(index, int(i.wrapping_add(1)));
    Done::Next
});
// GetIterator (§7.4.4): looks up `@@iterator` and calls it (`slow::get_iterator`). While the
// method is %Array.prototype.values% (on an object) or %String.prototype[@@iterator]% (on a
// string), the iterator cell is made without the call. A value with no callable `@@iterator`
// is not iterable (TypeError).
op!(GetIterator(st, object: R, _load: FSlot, _call: FSlot) { Flow::Cold } cold {
    let o = st.reg(object);
    let r = slow::get_iterator(st.cx, st.fp, st.pc, o);
    st.result(r)
});

// ---- exceptions and returns ----------------------------------------------------------------

op!(Throw(st) { Flow::Cold } cold {
    slow::throw(st.cx, st.fp, st.pc, st.acc, false);
    st.unwind()
});
op!(ReThrow(st) { Flow::Cold } cold {
    slow::throw(st.cx, st.fp, st.pc, st.acc, true);
    st.unwind()
});
// Pops the frame: the caller's pc and fp come from the header (H14). An entry frame returns
// to the EXIT stub, so the dispatch leaves to Rust.
op!(Return(st) {
    let ret = native_ptr::<u8>(st.hdr(slot::RETURN_PC));
    st.fp = native_ptr::<u64>(st.hdr(slot::CALLER_FP));
    Done::Jump(ret)
});
op!(SetPendingMessage(st) {
    let old = st.rt().shared.pending_message.replace(st.acc);
    st.acc = old;
    Done::Next
});
op!(ThrowReferenceErrorIfHole(st, name: K) {
    if st.acc == HOLE { Flow::Cold } else { Flow::Next }
} cold {
    let n = st.constant(name);
    slow::throw_name(st.cx, st.fp, st.pc, NameMsg::Tdz, n);
    st.unwind()
});
op!(ThrowSuperNotCalledIfHole(st) {
    if st.acc == HOLE { Flow::Cold } else { Flow::Next }
} cold {
    throw_msg(
        st,
        ErrorKind::ReferenceError,
        "Must call super constructor in derived class before accessing 'this' or returning from derived constructor",
    )
});
op!(ThrowSuperAlreadyCalledIfNotHole(st) {
    if st.acc == HOLE { Flow::Next } else { Flow::Cold }
} cold {
    throw_msg(st, ErrorKind::ReferenceError, "Super constructor may only be called once")
});
op!(ThrowIfNotSuperConstructor(st, constructor: R) {
    if object::is_constructor(st.reg(constructor)) { Flow::Next } else { Flow::Cold }
} cold {
    let c = st.reg(constructor);
    slow::not_super_constructor(st.cx, st.fp, st.pc, c);
    st.unwind()
});
// The accumulator is the active function; its [[Prototype]] is the super constructor.
op!(GetSuperConstructor(st, dst: R) {
    let f = st.acc;
    if !object::is_object(f) {
        return Flow::Cold;
    }
    st.set_reg(dst, object::proto_of(f));
    Flow::Next
} cold {
    throw_msg(st, ErrorKind::Error, "ep9 internal error: GetSuperConstructor of a non-object")
});

// ---- generators: slow/generator.rs (P1-GEN-VM), frame-switch protocol ----------------------

op!(SwitchOnGeneratorState(st, generator: R, table: K) { Flow::Cold } cold {
    let (acc, next) = (st.acc, st.next);
    let r = slow::switch_on_generator_state(st.cx, st.fp, st.pc, next, acc, generator.0, table.0);
    st.switch(r)
});
op!(SuspendGenerator(st, generator: R, registers: RList, count: Count, id: u32) { Flow::Cold } cold {
    let (acc, next) = (st.acc, st.next);
    let r = slow::suspend_generator(
        st.cx, st.fp, st.pc, next, acc, generator.0, registers.0, count.0, id,
    );
    st.switch(r)
});
op!(ResumeGenerator(st, generator: R, registers: RList, count: Count) { Flow::Cold } cold {
    let r = slow::resume_generator(st.cx, st.fp, st.pc, st.next, generator.0, registers.0, count.0);
    st.switch(r)
});

// ---- debugger ------------------------------------------------------------------------------

// v0: no debugger; the statement does nothing.
op!(Debugger(st) { Done::Next });

// ---- runtime calls with two results (ABI 3) ------------------------------------------------

// Like `CallRuntime`, for a pair function (only those; the verifier checks it): `out` and
// `out + 1` get the two results, and the accumulator stays (acc None).
op!(CallRuntimeForPair(st, id: RtId, args: RList, argc: Count, out: R) { Flow::Cold } cold {
    let acc = st.acc;
    let r = slow::call_runtime_pair(st.cx, st.fp, st.pc, id.0, args.0, argc.0, out.0, acc);
    st.result(r)
});

// ---- the cold list -------------------------------------------------------------------------

/// `if_cold!(Name { tokens if Name has a cold body } else { tokens if not })`.
macro_rules! if_cold {
    (Illegal { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Wide { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ExtraWide { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaConstant { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaGlobal { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaGlobalInsideTypeof { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (StaGlobal { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaContextSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaImmutableContextSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaCurrentContextSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaImmutableCurrentContextSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (StaContextSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (StaCurrentContextSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaLookupSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaLookupSlotInsideTypeof { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (StaLookupSlot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (LdaModuleVariable { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (StaModuleVariable { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (GetNamedProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (GetNamedPropertyFromSuper { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (GetKeyedProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (SetNamedProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (DefineNamedOwnProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (SetKeyedProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (DefineKeyedOwnProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (StaInArrayLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (DefineKeyedOwnPropertyInLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Add { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Sub { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Mul { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Div { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Mod { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Exp { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseOr { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseXor { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseAnd { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ShiftLeft { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ShiftRight { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ShiftRightLogical { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (AddSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (SubSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (MulSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (DivSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ModSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ExpSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseOrSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseXorSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseAndSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ShiftLeftSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ShiftRightSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ShiftRightLogicalSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Inc { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Dec { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Negate { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (BitwiseNot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToBooleanLogicalNot { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TypeOf { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (DeletePropertyStrict { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (DeletePropertySloppy { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestEqual { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestEqualStrict { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestLessThan { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestGreaterThan { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestLessThanOrEqual { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestGreaterThanOrEqual { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestInstanceOf { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TestIn { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToName { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToNumber { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToNumeric { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToObject { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToString { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ToBoolean { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallAnyReceiver { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallProperty { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallProperty0 { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallProperty1 { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallProperty2 { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallUndefinedReceiver { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallUndefinedReceiver0 { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallUndefinedReceiver1 { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallUndefinedReceiver2 { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallWithSpread { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallRuntime { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Construct { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ConstructWithSpread { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (TailCall { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateRegExpLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateArrayLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateEmptyArrayLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateArrayFromIterable { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateObjectLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateEmptyObjectLiteral { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CloneObject { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (GetTemplateObject { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateClosure { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateFunctionContext { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateBlockContext { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateCatchContext { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateEvalContext { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateWithContext { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateMappedArguments { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateUnmappedArguments { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CreateRestParameter { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (JumpLoop { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (JumpIfToBooleanTrue { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (JumpIfToBooleanFalse { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (SwitchOnSmi { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ForInEnumerate { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ForInPrepare { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ForInNext { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (GetIterator { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (Throw { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ReThrow { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ThrowReferenceErrorIfHole { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ThrowSuperNotCalledIfHole { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ThrowSuperAlreadyCalledIfNotHole { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ThrowIfNotSuperConstructor { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (GetSuperConstructor { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (SwitchOnGeneratorState { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (SuspendGenerator { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (ResumeGenerator { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    (CallRuntimeForPair { $($y:tt)* } else { $($n:tt)* }) => { $($y)* };
    ($other:ident { $($y:tt)* } else { $($n:tt)* }) => { $($n)* };
}
pub(crate) use if_cold;

/// Opcodes with a cold body (the `if_cold!` list), for tests and the disasm-check count.
pub(crate) const COLD_COUNT: usize = {
    macro_rules! count_cold {
        ([] $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
           flags: [$($flag:ident),*],
           operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*) => {
            0 $( + if_cold!($name { 1 } else { 0 }) )*
        };
    }
    ep9_bytecode::for_each_opcode!(count_cold [])
};

/// The budget units and the T0 call count of `call_fast` (H23 "Interrupt poll", H16): one unit
/// per call, also for a call that `call_fast` sends cold (M3-P2 PERF3; before, such a call
/// took 2 units: RV2-H23-DOUBLE-UNIT), for the call whose unit uses up the budget (M3-P2
/// fix round 2, X-PF1) and for a call or `new` whose frame does not fit (M3-P2 fix round 3,
/// R3-2), and one count per call while the callee has no vector.
#[cfg(test)]
mod call_budget {
    use ep9_bytecode::{FunctionId, HandlerKind, RegisterList, VerifiedUnit};

    use crate::tests::{Asm, config, isolate, meta, on_big_stack, unit};
    use crate::{Dispatch, IsolateConfig};

    /// The receiver of the calls.
    #[derive(Clone, Copy, Debug)]
    enum Recv {
        /// `CallUndefinedReceiver0`.
        Undefined,
        /// `CallProperty0` with the receiver 1: a sloppy callee needs a wrapper, so
        /// `call_fast` goes cold (H14-Q2).
        Smi,
        /// `CallProperty0` with the callee itself (an object) as the receiver.
        Object,
    }

    /// The script: `f = function f() { return 5 }` (sloppy, function 1), then `calls` calls of
    /// `f` with the receiver `recv`; it returns the last result.
    fn program(calls: u32, recv: Recv) -> VerifiedUnit {
        let mut f = Asm::new(meta("f", 0, 0, 0, 0));
        f.b.lda_smi(5).return_();
        let mut s = Asm::script(0);
        let r = s.b.new_register();
        let o = s.b.new_register();
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(r);
        match recv {
            Recv::Undefined => {}
            Recv::Smi => {
                s.b.lda_smi(1).star(o);
            }
            Recv::Object => {
                s.b.ldar(r).star(o);
            }
        }
        for _ in 0..calls {
            match recv {
                Recv::Undefined => s.b.call_undefined_receiver0(r, s.call),
                Recv::Smi | Recv::Object => s.b.call_property0(r, o, s.call),
            };
        }
        s.b.return_();
        unit("", vec![s.finish(), f.finish()])
    }

    /// The script: `f = function f() { return 5 }` (sloppy, function 1), then a loop of
    /// `iters` iterations with 2 calls `f()` per back edge; it returns the iteration count.
    fn loop_program(iters: i32) -> VerifiedUnit {
        let mut f = Asm::new(meta("f", 0, 0, 0, 0));
        f.b.lda_smi(5).return_();
        let mut s = Asm::script(0);
        let (r, i) = (s.b.new_register(), s.b.new_register());
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(r);
        s.b.lda_zero().star(i);
        let header = s.b.bound_label();
        let exit = s.b.new_label();
        s.b.lda_smi(iters)
            .test_less_than(i, s.cmp)
            .jump_if_false(exit);
        s.b.call_undefined_receiver0(r, s.call);
        s.b.call_undefined_receiver0(r, s.call);
        s.b.ldar(i).inc(s.bin).star(i);
        s.b.jump_loop(header, 0, s.loop_);
        s.b.bind(exit);
        s.b.ldar(i).return_();
        unit("", vec![s.finish(), f.finish()])
    }

    /// What a run left: the budget, `f`'s call count and whether the script function has a
    /// feedback vector.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct After {
        budget: i64,
        count: u32,
        script_vector: bool,
    }

    /// Runs the unit of `make` with the budget `budget` and the feedback threshold
    /// `threshold` under every dispatch expansion, and checks its result `expect`. The state
    /// after the run must agree across the expansions.
    fn run_unit(
        make: impl Fn() -> VerifiedUnit + Copy + Send + 'static,
        expect: &'static str,
        threshold: u32,
        budget: i64,
    ) -> After {
        let mut first: Option<(&str, After)> = None;
        for &d in Dispatch::ALL {
            let got = on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: threshold,
                    ..config(d, false)
                });
                iso.cx.budget = budget;
                let r = iso.run_script(make(), "budget.js");
                assert_eq!(r.map(|v| iso.display(v)).as_deref(), Ok(expect));
                let units_ref = iso.rt.units.borrow();
                let infos = &units_ref.last().expect("a loaded unit").infos;
                After {
                    budget: iso.cx.budget,
                    count: infos[1].invocations.get(),
                    script_vector: !infos[0].feedback.get().is_null(),
                }
            });
            match first {
                None => first = Some((d.name(), got)),
                Some((n0, g0)) => assert_eq!(got, g0, "{} differs from {n0}", d.name()),
            }
        }
        first.unwrap().1
    }

    /// Runs `program(calls, recv)` (see [`run_unit`]). Returns the units the run took (the
    /// script entry takes one) and `f`'s call count.
    fn run(calls: u32, recv: Recv, threshold: u32, budget: i64) -> (i64, u32) {
        let a = run_unit(move || program(calls, recv), "5", threshold, budget);
        (budget - a.budget, a.count)
    }

    #[test]
    fn every_t0_call_takes_one_unit() {
        for recv in [Recv::Undefined, Recv::Smi, Recv::Object] {
            for calls in 1..=3 {
                // No vector yet (threshold 8): each call counts once.
                assert_eq!(
                    run(calls, recv, 8, 1000),
                    (1 + i64::from(calls), calls),
                    "{recv:?} x{calls}"
                );
            }
        }
    }

    #[test]
    fn the_call_that_makes_the_vector_takes_one_unit() {
        // Threshold 2: the 2nd call goes cold in `call_fast` and `slow::call` makes the
        // vector; the count stays 2 after it.
        for recv in [Recv::Undefined, Recv::Smi, Recv::Object] {
            assert_eq!(run(3, recv, 2, 1000), (4, 2), "{recv:?}");
        }
    }

    #[test]
    fn a_call_that_uses_up_the_budget_takes_one_unit_and_counts_once() {
        // The script entry and the 3 calls take 4 units in all, also across the poll that
        // resets the budget (X-PF1): the units of the first budget (all of `budget`) plus the
        // units of the new one (BUDGET - the budget at the end). The call whose `call_fast`
        // unit uses up the budget gives the unit back and goes cold; `slow::call` takes the
        // unit (it polls first if the budget is 0). Each call counts once.
        use crate::interp::BUDGET;
        for recv in [Recv::Undefined, Recv::Smi, Recv::Object] {
            for budget in 1..=3 {
                let (units, count) = run(3, recv, 8, budget);
                assert_eq!(count, 3, "{recv:?} budget {budget}");
                let left = budget - units;
                assert_eq!(budget + (BUDGET - left), 4, "{recv:?} budget {budget}");
            }
        }
    }

    #[test]
    fn a_loop_with_two_calls_per_back_edge_gets_its_vector() {
        // X-PF1: with 2 calls and 1 back edge per iteration (3 units), a call that took 2
        // units at the budget's end put every poll on the same call, so the `JumpLoop` poll
        // (where `feedback::ensure` gives the running script its vector) never came. With 1
        // unit per call, the poll moves and a `JumpLoop` poll comes (near iteration 3,334).
        // Other loop shapes can still put every poll on a call (3 or 4 calls per back edge
        // at budget 2): `feedback::ensure` runs only at the `JumpLoop` poll (PERF3-PHASE,
        // PERF3 step-2 item 4, M3-P4).
        let a = run_unit(|| loop_program(10_000), "10000", 8, 2);
        assert!(a.script_vector, "{a:?}");
        assert_eq!(a.count, 8, "{a:?}");
    }

    /// How [`overflow_program`] calls `f`.
    #[derive(Clone, Copy, Debug)]
    enum Site {
        /// A call with the receiver `recv` (see [`Recv`]).
        Call(Recv),
        /// `new f()`.
        Construct,
        /// `new f()` in a script frame so big that the construct stub frame does not fit
        /// (fewer than [`STUB_FRAME`] slots are free above it), so the stub push fails, not
        /// the push of `f`'s frame (RV4-2).
        ConstructStub,
    }

    /// The registers of `f` in [`overflow_program`]: more than [`OVERFLOW_SLOTS`], so its
    /// frame never fits.
    const BIG_FRAME: u32 = 2048;
    /// The VM stack of the overflow runs (the smallest that `Isolate::new` accepts).
    const OVERFLOW_SLOTS: usize = 1024;
    /// The slots of a construct stub frame: the receiver and the header (no arguments, no
    /// registers).
    const STUB_FRAME: usize = 1 + crate::frame::HEADER;
    /// The slots that [`Site::ConstructStub`] leaves free above the script frame: fewer than
    /// [`STUB_FRAME`].
    const STUB_FREE: usize = 4;
    const _: () = assert!(STUB_FREE < STUB_FRAME);

    /// The script: `f = function f() { return 5 }` (sloppy, function 1, with [`BIG_FRAME`]
    /// registers), then `calls` times `try { f() } catch {}` (the call of `site`), with no
    /// loop. It returns 7. Each call throws the VM stack's RangeError and the script catches
    /// it, so the calls are the only budget units after the script entry.
    fn overflow_program(calls: u32, site: Site) -> VerifiedUnit {
        let mut f = Asm::new(meta("f", 0, 0, 0, 0));
        f.b.new_register_list(BIG_FRAME);
        f.b.lda_smi(5).return_();
        let mut s = Asm::script(0);
        let (r, o, ctx) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(r);
        match site {
            Site::Call(Recv::Smi) => {
                s.b.lda_smi(1).star(o);
            }
            Site::Call(Recv::Object) => {
                s.b.ldar(r).star(o);
            }
            Site::Call(Recv::Undefined) | Site::Construct | Site::ConstructStub => {}
        }
        s.b.lda_current_context().star(ctx);
        for _ in 0..calls {
            let (start, end, handler, done) = (
                s.b.new_label(),
                s.b.new_label(),
                s.b.new_label(),
                s.b.new_label(),
            );
            s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
            s.b.bind(start);
            match site {
                Site::Call(Recv::Undefined) => {
                    s.b.call_undefined_receiver0(r, s.call);
                }
                Site::Call(Recv::Smi | Recv::Object) => {
                    s.b.call_property0(r, o, s.call);
                }
                Site::Construct | Site::ConstructStub => {
                    s.b.ldar(r).construct(r, RegisterList::EMPTY, 0, s.call);
                }
            }
            s.b.bind(end);
            s.b.jump(done);
            s.b.bind(handler);
            s.b.bind(done);
        }
        if let Site::ConstructStub = site {
            // The script frame (receiver, header and registers, from the stack base) ends
            // [`STUB_FREE`] slots below the stack end.
            let used = 1 + crate::frame::HEADER + s.b.register_count() as usize;
            let pad = OVERFLOW_SLOTS - STUB_FREE - used;
            s.b.new_register_list(u32::try_from(pad).expect("a small pad"));
        }
        s.b.lda_smi(7).return_();
        unit("", vec![s.finish(), f.finish()])
    }

    const SITES: [Site; 5] = [
        Site::Call(Recv::Undefined),
        Site::Call(Recv::Smi),
        Site::Call(Recv::Object),
        Site::Construct,
        Site::ConstructStub,
    ];

    /// Runs `overflow_program(calls, site)` with the budget `budget`, a VM stack of
    /// [`OVERFLOW_SLOTS`] slots and an interrupt callback queued before the run, under every
    /// dispatch expansion. Returns the budget at the end and whether the callback ran.
    fn run_overflow(calls: u32, site: Site, budget: i64) -> (i64, bool) {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut first: Option<(&str, (i64, bool))> = None;
        for &d in Dispatch::ALL {
            let got = on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    vm_stack_slots: OVERFLOW_SLOTS,
                    ..config(d, false)
                });
                let ran = Arc::new(AtomicBool::new(false));
                let r2 = Arc::clone(&ran);
                iso.interrupt_handle()
                    .request(Box::new(move || r2.store(true, Ordering::SeqCst)));
                iso.cx.budget = budget;
                let r = iso.run_script(overflow_program(calls, site), "overflow.js");
                assert_eq!(r.map(|v| iso.display(v)).as_deref(), Ok("7"), "{site:?}");
                (iso.cx.budget, ran.load(Ordering::SeqCst))
            });
            match first {
                None => first = Some((d.name(), got)),
                Some((n0, g0)) => assert_eq!(got, g0, "{} differs from {n0}", d.name()),
            }
        }
        first.unwrap().1
    }

    #[test]
    fn a_call_whose_frame_does_not_fit_takes_one_unit() {
        // R3-2: the script entry and the 3 calls take 4 units in all, also across the poll
        // that resets the budget (as in the X-PF1 test above). A call whose frame push fails
        // takes its unit like a call whose push succeeds: `call_fast` gives its unit back at
        // each cold exit after its decrement, and `slow::call` takes the unit at its frame
        // push, also when the push fails. Before, at 1 unit left, the call took 0 units (the
        // X-PF1 refund, then the RangeError before the decrement of `slow::call`), and a call
        // that `call_fast` sends cold before its decrement (a primitive receiver, `new`) took
        // 0 units at any budget.
        use crate::interp::BUDGET;
        for site in SITES {
            for budget in [1, 2, 3, 4, 1000] {
                let (left, _) = run_overflow(3, site, budget);
                let units = if left <= budget {
                    budget - left
                } else {
                    budget + (BUDGET - left)
                };
                assert_eq!(units, 4, "{site:?} budget {budget}");
            }
        }
    }

    #[test]
    fn caught_overflows_at_one_unit_left_still_poll() {
        // R3-2 (Codex round 2): budget 2, so 1 unit is left after the script entry. 50 caught
        // overflows in straight-line code (no `JumpLoop`): the first one uses up the budget,
        // and the next call polls and runs the queued interrupt callback. Before, each failed
        // call left the budget at 1, so no poll came and the callback never ran.
        for site in SITES {
            let (_, ran) = run_overflow(50, site, 2);
            assert!(ran, "{site:?}: the queued interrupt did not run");
        }
    }
}
