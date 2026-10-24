//! Generator opcodes and runtime functions (owner P1-GEN-VM), the V8 model. The three opcodes
//! switch frames or jump, so they use the frame-switch protocol of `slow::call`: the entry sets
//! `cx.out_fp` and `cx.out_pc` (also when it unwinds) and returns the accumulator, and the
//! handler adopts them (`St::switch`).
//!
//! ```text
//! SwitchOnGeneratorState g, table   g undefined: out_pc = next (first call); else jump to
//!                                   table[state] and state = executing; acc unchanged
//! SuspendGenerator g, regs, n, id   copy the params and regs into the generator's REGS cell,
//!                                   save the context, state = id, then return like `Return`
//!                                   (an async function returns its promise, `slow/async_fn.rs`)
//! ResumeGenerator g, regs, n        copy the regs back; acc = the sent value
//! ```
//!
//! The REGS cell (an ELEMENTS cell, plan rule 8) of a generator whose function has `P`
//! parameters and `R` registers holds `1 + P + R` Values: the sent value
//! (`genf::REGS_INPUT`), the parameters, then the registers of the suspend's register list.
//! The natives push the resumed frame with the saved parameters, `this` and context
//! (`generator::resume`); `ResumeGenerator` restores the registers.
//!
//! Operands: `generator` and `regs` are register slot offsets from `fp` (`dsl::R`,
//! `dsl::RList`), `table` a constant index (a JumpTable). The verifier does not know which
//! value a register holds, so every entry checks that the generator is a GENERATOR of this
//! frame's function, in the right state, with enough saved slots; a check that fails is a
//! BytecodeGen or VM bug and throws a catchable "ep9 internal error" (never UB).
//!
//! Runtime ids 19-22 (ABI 3, `slow/runtime.rs` routes them here) use the result protocol of
//! `CallRuntime` (`Res<u64>`, the new accumulator).
//!
//! Numbers (M2-P1-GEN-NUMBERS, `object::genf`): resume mode next 0, return 1, throw 2; state
//! >= 0 is a suspend id, -1 closed, -2 executing.

use ep9_bytecode::Constant;

use crate::conv::Res;
use crate::error::{self, throw_error};
use crate::frame::{UNDEFINED, int32_bits, native_ptr, slot};
use crate::generator::is_generator;
use crate::interp::Cx;
use crate::kinds;
use crate::object::{self, genf};
use crate::property;
use crate::runtime::{Atom, ErrorKind};

use super::{context, enter, info};

/// Throws the internal error `ep9 internal error: <msg>` (a BytecodeGen or VM bug).
fn internal(cx: &mut Cx, msg: &str) -> ep9_value::Throw {
    throw_error(cx, ErrorKind::Error, &format!("ep9 internal error: {msg}"))
}

/// [`internal`], then unwind from `(fp, pc)` (frame-switch protocol: sets `out_fp`/`out_pc`).
fn internal_unwind(cx: &mut Cx, fp: *mut u64, pc: *const u8, msg: &str) -> u64 {
    internal(cx, msg);
    error::unwind(cx, fp, pc)
}

/// The register at slot offset `r` of the frame `fp`.
#[inline]
fn reg(fp: *mut u64, r: isize) -> u64 {
    // SAFETY: `r` comes from a register operand of verified code in the live frame `fp`.
    unsafe { *fp.offset(r) }
}

/// The checks that `SuspendGenerator` and `ResumeGenerator` share: `g` is a GENERATOR of the
/// function of frame `fp` whose REGS cell holds the input, the parameters and `count`
/// registers. Returns the REGS view and the parameter count, or the internal error text.
fn frame_generator(
    fp: *mut u64,
    g: u64,
    count: u32,
) -> Result<(object::GenRegs, usize), &'static str> {
    if !is_generator(g) {
        return Err("the generator register holds no generator");
    }
    let fi = info(fp);
    let f = object::gen_get(g, genf::FUNCTION);
    if !kinds::is_function(f) || !core::ptr::eq(object::function_info(f), fi) {
        return Err("the generator belongs to another function");
    }
    let Some(regs) = object::gen_regs(g) else {
        return Err("the generator has no saved registers");
    };
    let params = fi.parameter_count as usize;
    if 1 + params + count as usize > regs.len() {
        return Err("the register list does not fit the generator");
    }
    Ok((regs, params))
}

/// `SwitchOnGeneratorState(generator, table)`. Frame-switch protocol.
///
/// The first call of a generator function has undefined in fp-3 (no generator yet): the
/// dispatch falls through to the prologue. A resumed frame has the generator there; its state
/// is the suspend id, which indexes `table`: jump to that entry and mark the generator
/// executing (GeneratorResume §27.5.3.3 step 7). The opcode does not change the accumulator
/// (`acc: None`, opcodes.rs): both paths give `acc` back (GV-7, M2-P2-GEN-ACC; closed in
/// M3-P1 ASYNC).
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn switch_on_generator_state(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    next: *const u8,
    acc: u64,
    generator: isize,
    table: u32,
) -> u64 {
    enter(cx, fp, pc);
    let g = reg(fp, generator);
    if g == UNDEFINED {
        cx.out_fp = fp;
        cx.out_pc = next;
        return acc;
    }
    if !is_generator(g) {
        return internal_unwind(cx, fp, pc, "SwitchOnGeneratorState of a non-generator");
    }
    let state = object::gen_get_i32(g, genf::STATE);
    let Constant::JumpTable(t) = info(fp).constant_entry(table) else {
        return internal_unwind(cx, fp, pc, "SwitchOnGeneratorState without a jump table");
    };
    let Some(&off) = usize::try_from(state).ok().and_then(|k| t.get(k)) else {
        return internal_unwind(cx, fp, pc, "a generator resumed in a state with no entry");
    };
    object::gen_set(cx.rt(), g, genf::STATE, int32_bits(genf::STATE_EXECUTING));
    cx.out_fp = fp;
    // The verifier checked that every entry is an instruction start after `pc`.
    cx.out_pc = pc.wrapping_add(off as usize);
    acc
}

/// `SuspendGenerator(generator, registers, count, suspend_id)`; `acc` is the value to give to
/// the caller (the generator object at the first suspend, else an iterator result; an async
/// function gives its promise instead). Frame-switch protocol.
///
/// Copies the frame's parameters and the `count` registers from `registers` into the REGS
/// cell, saves the current context, sets the state to `suspend_id`, then returns like
/// `Return`: the caller's fp and pc come from the header (H14). Not a GC point (the REGS cell
/// was allocated by `CreateGeneratorObject`).
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn suspend_generator(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    _next: *const u8,
    acc: u64,
    generator: isize,
    registers: isize,
    count: u32,
    suspend_id: u32,
) -> u64 {
    enter(cx, fp, pc);
    let g = reg(fp, generator);
    let (regs, params) = match frame_generator(fp, g, count) {
        Ok(v) => v,
        Err(msg) => return internal_unwind(cx, fp, pc, msg),
    };
    if object::gen_get_i32(g, genf::STATE) != genf::STATE_EXECUTING {
        return internal_unwind(
            cx,
            fp,
            pc,
            "SuspendGenerator of a generator that is not running",
        );
    }
    let Ok(id) = i32::try_from(suspend_id) else {
        return internal_unwind(cx, fp, pc, "suspend id out of range");
    };
    let rt = cx.rt();
    for i in 0..params {
        // SAFETY: a frame has max(argc, parameter_count) argument slots (`call::push_frame`),
        // so parameter `i < parameter_count` is a slot of the live frame `fp`.
        let v = unsafe { *fp.offset(slot::PARAM0 - i as isize) };
        regs.set(rt, 1 + i, v);
    }
    for i in 0..count as usize {
        // The verifier checked that the list `registers .. registers + count` lies in the
        // frame (`RegList`).
        regs.set(rt, 1 + params + i, reg(fp, registers + i as isize));
    }
    object::gen_set(rt, g, genf::CONTEXT, context(fp));
    object::gen_set(rt, g, genf::STATE, int32_bits(id));
    // `Return` (ops.rs): pop the frame.
    // SAFETY: header slots of the live frame `fp`.
    let (ret, caller) = unsafe { (*fp.offset(slot::RETURN_PC), *fp.offset(slot::CALLER_FP)) };
    cx.out_fp = native_ptr(caller);
    cx.out_pc = native_ptr(ret);
    // An async function's `await` gives its caller the promise (§27.7.5.3 Await steps 8-9
    // return to AsyncFunctionStart, and EvaluateAsyncFunctionBody returns the promise); the
    // caller of a resumed frame (a reaction job) ignores it.
    if object::gen_get_i32(g, genf::MODE) == genf::MODE_ASYNC_FUNCTION {
        return object::gen_get(g, genf::ASYNC);
    }
    acc
}

/// `ResumeGenerator(generator, registers, count)`. Frame-switch protocol.
///
/// Copies the saved registers back into `registers .. registers + count` and gives the sent
/// value (the REGS input slot) as the accumulator. The saved slots are then cleared, so the
/// REGS cell holds nothing the running frame does not (V8's stale registers). The parameters,
/// `this` and the context were restored when the frame was pushed (`generator::resume`).
#[cold]
#[inline(never)]
pub(crate) fn resume_generator(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    next: *const u8,
    generator: isize,
    registers: isize,
    count: u32,
) -> u64 {
    enter(cx, fp, pc);
    let g = reg(fp, generator);
    let (regs, params) = match frame_generator(fp, g, count) {
        Ok(v) => v,
        Err(msg) => return internal_unwind(cx, fp, pc, msg),
    };
    let rt = cx.rt();
    for i in 0..count as usize {
        let at = 1 + params + i;
        // SAFETY: the verifier checked that the list lies in the frame (`RegOutList`).
        unsafe { *fp.offset(registers + i as isize) = regs.get(at) };
        regs.set(rt, at, UNDEFINED);
    }
    for i in 0..params {
        regs.set(rt, 1 + i, UNDEFINED);
    }
    let sent = regs.get(genf::REGS_INPUT);
    regs.set(rt, genf::REGS_INPUT, UNDEFINED);
    cx.out_fp = fp;
    cx.out_pc = next;
    sent
}

// ---- runtime ids 19-22 (ABI 3) -------------------------------------------------------------

/// `CreateGeneratorObject(function, receiver)` (runtime id 19): the generator object of the
/// running generator function `function` in frame `fp`, with `receiver` as its `this`
/// (§15.5.2 EvaluateGeneratorBody steps 2-4: OrdinaryCreateFromConstructor(functionObject,
/// "%GeneratorPrototype%")). Its REGS cell has room for the input, the parameters and every
/// register of the frame; the context is the frame's current one. The new generator is
/// executing until the first `SuspendGenerator`. GC point.
pub(crate) fn create_generator_object(
    cx: &mut Cx,
    fp: *mut u64,
    function: u64,
    receiver: u64,
) -> Res<u64> {
    let fi = info(fp);
    if !fi.is_generator()
        || !kinds::is_function(function)
        || !core::ptr::eq(object::function_info(function), fi)
    {
        return Err(internal(
            cx,
            "CreateGeneratorObject outside its generator function",
        ));
    }
    let rt = cx.rt();
    // §10.1.14 GetPrototypeFromConstructor: `prototype` is a data property of a generator
    // function ({w}, made lazily: GC point), so this runs no JS.
    let p = property::get(cx, function, rt.atom(Atom::Prototype))?;
    let proto = if kinds::is_object(p) {
        p
    } else {
        crate::builtins::intrinsic(rt, crate::builtins::IntrinsicId::GeneratorPrototype)
    };
    let n = 1 + fi.parameter_count as usize + fi.register_count as usize;
    Ok(object::new_generator(
        rt,
        proto,
        function,
        receiver,
        context(fp),
        genf::MODE_SYNC,
        n,
    ))
}

/// `GeneratorGetResumeMode(generator)` (runtime id 20): next 0, return 1 or throw 2, as a Smi
/// (the mode of the `next`/`return`/`throw` call that resumed the frame).
pub(crate) fn generator_get_resume_mode(cx: &mut Cx, generator: u64) -> Res<u64> {
    if !is_generator(generator) {
        return Err(internal(cx, "GeneratorGetResumeMode of a non-generator"));
    }
    Ok(object::gen_get(generator, genf::RESUME_MODE))
}

/// `GeneratorClose(generator)` (runtime id 21): the generator is completed (state closed) and
/// drops its saved values. A frame that returns while executing is closed by the native
/// that resumed it anyway (`generator::resume`).
pub(crate) fn generator_close(cx: &mut Cx, generator: u64) -> Res<u64> {
    if !is_generator(generator) {
        return Err(internal(cx, "GeneratorClose of a non-generator"));
    }
    crate::generator::close(cx.rt(), generator);
    Ok(UNDEFINED)
}

/// `ThrowThrowMethodMissing()` (runtime id 22): the TypeError of `yield*` when the inner
/// iterator has no `throw` method (§15.5.5 step 7.b.iii, after its IteratorClose).
pub(crate) fn throw_throw_method_missing(cx: &mut Cx) -> Res<u64> {
    Err(throw_error(
        cx,
        ErrorKind::TypeError,
        "The iterator does not provide a 'throw' method.",
    ))
}
