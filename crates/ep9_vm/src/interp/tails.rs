//! Out-of-line finishes of hot T0 handlers on x86-64 (M8-P1 X86-SHAPE2; plan v13 rule 81).
//!
//! The handler type pins 4 argument registers (H15). x86-64 then has 5 free caller-saved
//! registers, aarch64 18. A fast body that keeps more values live, or calls a function and
//! then dispatches, takes callee-saved registers, and LLVM saves them on entry for every run
//! of the handler. The x86-64 fast bodies leave such a case to the opcode's existing cold body
//! (`Flow::Cold`, before any side effect). The cold body calls a helper here, which finishes
//! the case first, so a hit that the fast body left stays a hit: no miss count, no IC record,
//! no feedback record. Then the helper runs the miss path that the cold body ran before.
//!
//! No new `Flow` arm, no DSL exit and no new cold block (H15 dispatch core; plan rule 72).
//! These functions are not handlers: `cargo xtask disasm-check` counts the same 284. On
//! other targets (and on x86-64 with `ic-stats`, where the fast body keeps its full hit path)
//! each name here is the function that the cold body called before.

#[cfg(all(target_arch = "x86_64", not(feature = "ic-stats")))]
use crate::{error::FrameRef, ic, interp::Cx, interp::slow};

/// The cold body of `LdaGlobal` and (since M8-P3 LEVER-6) `LdaGlobalInsideTypeof`
/// (`in_typeof`): the global IC hit (a polymorphic entry, which the x86-64 fast body leaves
/// here: [`ic::load_global_fast`]), else the generic load and its record
/// ([`slow::load_global`]).
#[cfg(all(target_arch = "x86_64", not(feature = "ic-stats")))]
#[inline(never)]
pub(crate) fn load_global(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    name: u64,
    in_typeof: bool,
    slot: u32,
) -> u64 {
    // No `ic_count!`: this form exists only without `ic-stats`.
    if let Some(v) = ic::load_global(cx, FrameRef(fp).info(), slot) {
        return v;
    }
    slow::load_global(cx, fp, pc, name, in_typeof, slot)
}

/// The cold body of `LdaGlobal` and `LdaGlobalInsideTypeof` on other targets: the generic
/// load ([`super::slow::load_global`]).
#[cfg(not(all(target_arch = "x86_64", not(feature = "ic-stats"))))]
pub(crate) use super::slow::load_global;

/// The cold body of `Wide` and `ExtraWide` for an opcode of `ops::prefix_set` (a prefixed
/// `GetNamedProperty`, `JumpLoop` or forward jump) where their fast body does not run it:
/// `ops::PREFIXED_FAST` is false, on x86-64 (M9-P1 X86-WIDE-FAST, plan v14 §4.4). It runs the
/// opcode's fast body at scale `S` (`ops::prefixed_fast`), and only if that gives
/// `Flow::Cold` the cold body (`interp::scaled::step_cold`): the same steps as the aarch64
/// `Wide` fast body and cold block, so the same IC hits, records and budget units.
///
/// The `Wide` fast body reaches it by its tail jump to the cold handler (`Flow::Cold`), so the
/// fast handler keeps its x86-64 form: 53 instructions, no entry save (plan rule 81), no new
/// `Flow` arm (rule 72). Before, the cold handler called `interp::scaled::step` for these
/// opcodes: 6 register saves and restores, a call and a return, and an indirect jump over every
/// prefixed opcode (2.3% of the x86-64 TS indirect mispredicts, S0 at m7-p5). Here the cold
/// handler runs the hit path itself (one save, as before: `rbx` on x86-64 SysV), and only the
/// forward jumps share an indirect jump. Inline in optimized builds; out of line in debug
/// builds, so a debug cold handler frame (no tail calls there) does not grow by the IC code.
/// Not called where `PREFIXED_FAST` is true (the cold block runs `step_cold` there).
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline(always))]
pub(crate) fn wide<S: crate::interp::dsl::Scale>(
    st: &mut crate::interp::St<'_>,
    op: u8,
) -> crate::interp::Done {
    use crate::interp::{Done, Flow, ops};
    #[cfg(test)]
    WIDE_RUNS.with(|n| n.set(n.get() + 1));
    match ops::prefixed_fast::<S, true>(st, op) {
        Flow::Next => Done::Next,
        Flow::Jump(t) => Done::Jump(t),
        Flow::Cold => st.scaled_cold::<S>(),
        // No opcode of the set gives a record flow (`ops::prefix_set`); as `step`, if one did.
        f => match crate::interp::dsl::settle(f, st, crate::profile::records(st.cx)) {
            Flow::Jump(t) => Done::Jump(t),
            _ => Done::Next,
        },
    }
}

// ---- The Star lookahead (M9-P0 STAR-FUSE; M10-P2 STAR-LA2) ---------------------------------
//
// V8 `Bytecodes::IsStarLookahead`, `InterpreterAssembler::StarDispatchLookahead`: `star_next`
// and `star_done` for the `Flow::Next` and `Done::Next` exits of a producer's fast body. It is
// a body form on every target: no new opcode, handler, `Flow` arm or cold block, and no
// change to `interp/dsl.rs` (H15 dispatch core; DL M9-PLAN-D6, M9-PRE-STARFUSE-KIND).

use crate::interp::{St, dsl};
use ep9_bytecode::Opcode;

/// The Star lookahead: if the next instruction is a one-byte-scale `Star r`, the producer runs
/// it here: it stores the accumulator to `r` and sets `st.next` past the Star, so the dispatch
/// goes to the instruction after the Star and the Star handler does not run. A producer calls
/// it only on its success exit ([`star_next`], [`star_done`]), after it wrote the accumulator;
/// an exception, a cold exit or a record exit runs no lookahead, so the Star runs in its own
/// handler there, as before.
///
/// The producers (M10-P2 STAR-LA2: the 6 of M9-P0 and 5 more; DL M6-ORCH-776 "11 producers"):
/// the `GetNamedProperty` IC hit, the four context-slot loads (`ops::ctx_load`),
/// `LdaCurrentContext`, `LdaConstant`, `LdaZero`, `LdaSmi`, `LdaNull` and `LdaFalse`.
///
/// - **Same result.** The Star handler does the same: the same operand decode
///   (`kind::RegOut`), the same store, the next instruction 2 bytes after the Star. A Star
///   whose register is the producer's input (`GetNamedProperty r1; Star r1`) stores after the
///   producer read it, as the two handlers do. A Star that is also a jump target runs in the
///   Star handler when a jump reaches it; only the fall-through from the producer fuses.
/// - **The debugger off-switch.** The test reads the opcode byte at `st.next`, not a table of
///   static pairs. A breakpoint patches the byte at its pc in the function's Interp debug copy
///   (H24-Q3), so a producer before a breakpoint at a Star sees no Star byte and does not
///   fuse; the break then runs at the Star pc. A throw never has the Star pc as its position:
///   a Star does not throw, and a producer that throws takes no lookahead.
/// - **No GC point.** It allocates nothing and calls nothing; the stored value is the
///   accumulator, which the producer already holds.
/// - A `Wide` or `ExtraWide` Star (a prefix byte at `st.next`) does not fuse. A prefixed
///   producer (`Wide GetNamedProperty`, aarch64 `ops::prefixed_fast`) runs the lookahead too:
///   its `st.next` is the end of the prefixed instruction.
///
/// The pc forms ([`lookahead_pc`]). aarch64: `st.next` goes through an empty `asm!` block
/// before the test, so LLVM keeps it in `x1` (M9-P0: else 1 or 2 more instructions on each
/// path). x86-64 with `ONE_PC`: through [`dsl::dispatch_pc`] before the test and after it, so
/// LLVM keeps the next pc in one register and reads the opcode again at the dispatch: one
/// dispatch jump. x86-64 without `ONE_PC`: the fused and the other path each get a dispatch
/// jump (the V8 duplicated dispatch site). The form of each producer (asm read, M10-P2; the
/// const argument at its exit in `interp/ops.rs`):
///
/// - `ONE_PC`: the `GetNamedProperty` IC hit and the four context-slot loads. Their success
///   exit is a join of several paths (the IC hit paths; the context walk). x86-64: one
///   dispatch jump. The plain form also gave one jump, and more instructions on each path
///   (`GetNamedProperty` +2; the context-slot loads +1 not fused, +2 fused). The 4 dispatch
///   jumps of the `GetNamedProperty` hit paths become 1 (x86-64 and aarch64).
/// - Plain: `LdaCurrentContext`, `LdaConstant`, `LdaZero`, `LdaSmi`, `LdaNull` and `LdaFalse`
///   (one path to the exit). x86-64: 2 dispatch jumps, one for the fused path and one for the
///   other path. The not-fused path is the same as with `ONE_PC`, the fused path 1
///   instruction shorter.
/// - aarch64 (`ONE_PC` changes nothing there): LLVM gives each producer one dispatch jump;
///   the not-fused path dispatches with the opcode byte that the test loaded.
///
/// No producer saves a register on entry (the frame goldens; plan rule 81).
///
/// Debug builds (M10-P2 STAR-LA2 step 0): [`star_next`] and [`star_done`] are out of line, so
/// a debug producer frame (no tail calls there: each instruction nests one) does not grow by
/// the lookahead code. At M9-P0 the inline form made the aarch64 debug `Wide` frame (its fast
/// body runs `GetNamedProperty`) 12,064 -> 12,400 bytes, and `tests::profile` (2 `Wide` a turn)
/// reached the debug stack floor at turn 10,469 of 10,600. Release builds inline them (the
/// same code as the inline form).
///
/// Optimized builds with debug assertions (M10-P3 STAR-LA2 step 3; the Spike C test build:
/// `opt-level = 2` for the engine crates, spikes/spike-c/Cargo.toml): there the handlers have
/// tail calls. An out-of-line function that takes `&mut St` makes the `St` of the producer
/// handler a stack value whose address escapes, and LLVM then gives no tail call: the handler
/// calls the next handler and keeps its frame (aarch64: `LdaZero` 32 -> 64 bytes and `blr` +
/// `ret` in place of `br`; `LdaConstant` 80 -> 160). A loop of 100,000 turns then overflows a
/// 512 KiB thread (spike-c `harness::tests::cold_string_arms_keep_the_native_stack_flat`). So
/// the out-of-line part is [`star_at`], which takes the values (`cx`, `fp`, `acc`, `next`) and
/// gives the new `next`, and [`star_next`] and [`star_done`] are `#[inline]` in debug builds:
/// - `opt-level = 0`: no inlining but `#[inline(always)]`, so the producer calls `star_next`
///   or `star_done` out of line with `&mut St`, as in the step 0 form (the frame of the base;
///   no tail calls at `opt-level = 0` in any case).
/// - an optimized build: LLVM inlines `star_next` and `star_done`, the `St` stays in
///   registers, the producer calls `star_at` with values and keeps its tail call.
///
/// The debug native-stack check (M10-P3 STAR-LA2, Codex sl2-r1 F1): a debug threaded handler
/// runs `dsl::debug_native_stack_check` before it decodes or writes anything (the floor is
/// `DEBUG_SLACK` below `native_limit`). A fused Star writes its register in the producer, so
/// it would skip the check of the Star handler. So in debug builds the lookahead fuses only
/// while the native stack is above `native_limit` ([`star_room`]). Below it (the last
/// `DEBUG_SLACK` before the floor) the Star runs in its own handler, and that handler checks
/// the floor before its write, as without the lookahead. Above `native_limit` the check of
/// the Star handler would pass: its frame is a few hundred bytes below the frame that tests
/// here, far less than `DEBUG_SLACK`.
#[inline(always)]
fn star_lookahead<const ONE_PC: bool>(st: &mut St<'_>) {
    st.next = star_at::<ONE_PC>(st.cx, st.fp, st.acc, st.next);
}

/// The lookahead of [`star_lookahead`] on the values that it needs: `fp` the live frame, `acc`
/// the accumulator that the producer wrote, `next` the start of the next instruction (`St`
/// contract). Gives the pc that the producer dispatches to. Out of line in debug builds, with
/// no `&mut St` (see [`star_lookahead`]); inline in release builds.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline(always))]
fn star_at<const ONE_PC: bool>(
    cx: &crate::interp::Cx,
    fp: *mut u64,
    acc: u64,
    next: *const u8,
) -> *const u8 {
    #[cfg(test)]
    if star_count::OFF.with(|c| c.get()) {
        return next;
    }
    let mut next = lookahead_pc::<ONE_PC>(next);
    // SAFETY: `next` is the start of the next instruction of verified code (`St` contract):
    // its opcode byte is readable.
    if unsafe { *next } == Opcode::Star as u8 && star_room(cx) {
        // SAFETY: verified code: a one-byte-scale `Star` is followed by its `RegOut` operand,
        // a register of the live frame `fp`, and the instruction after it (a verified function
        // never runs off its end).
        let (len, (dst,)) = unsafe { dsl::decode!(dsl::S1, next; dst: RegOut) };
        // SAFETY: `dst` is a register of the live frame `fp` (the verifier): the store of
        // `St::set_reg`, as in the Star handler.
        unsafe { *fp.offset(dst.0) = acc };
        // SAFETY: as above: the next instruction starts `len` bytes after the Star.
        next = unsafe { next.add(len) };
        #[cfg(test)]
        star_count::FUSED.with(|c| c.set(c.get() + 1));
    }
    if ONE_PC {
        next = dsl::dispatch_pc(next);
    }
    next
}

/// True if the lookahead may fuse the Star here ([`star_lookahead`], sl2-r1 F1). Debug builds:
/// the native stack is above `native_limit` (`Runtime::native_stack_ok`), so the debug
/// native-stack check of the Star handler would pass. Release builds have no such check:
/// always true (no code).
#[inline(always)]
fn star_room(cx: &crate::interp::Cx) -> bool {
    #[cfg(debug_assertions)]
    {
        cx.rt().native_stack_ok()
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = cx;
        true
    }
}

/// The pc that [`star_lookahead`] tests, as the target wants it (see there). aarch64: through
/// an empty `asm!` block. x86-64: [`dsl::dispatch_pc`] with `ONE_PC`, else the same value.
/// Other targets: the same value.
#[inline(always)]
fn lookahead_pc<const ONE_PC: bool>(next: *const u8) -> *const u8 {
    #[cfg(target_arch = "aarch64")]
    {
        let mut a = next.expose_provenance();
        // SAFETY: an empty block (a comment): no instruction, no memory access, no flag
        // change. `a` comes back as it went in.
        unsafe {
            core::arch::asm!(
                "/* {a} */",
                a = inout(reg) a,
                options(pure, nomem, nostack, preserves_flags)
            );
        }
        core::ptr::with_exposed_provenance(a)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        if ONE_PC { dsl::dispatch_pc(next) } else { next }
    }
}

/// `Flow::Next` after the Star lookahead ([`star_lookahead`]): the success exit of a producer
/// whose fast body gives a `Flow`. `#[inline]` in debug builds (see [`star_lookahead`]).
#[cfg_attr(debug_assertions, inline)]
#[cfg_attr(not(debug_assertions), inline(always))]
pub(crate) fn star_next<const ONE_PC: bool>(st: &mut St<'_>) -> crate::interp::Flow {
    star_lookahead::<ONE_PC>(st);
    crate::interp::Flow::Next
}

/// `Done::Next` after the Star lookahead ([`star_lookahead`]): the exit of a producer whose
/// fast body gives a `Done`. `#[inline]` in debug builds (see [`star_lookahead`]).
#[cfg_attr(debug_assertions, inline)]
#[cfg_attr(not(debug_assertions), inline(always))]
pub(crate) fn star_done<const ONE_PC: bool>(st: &mut St<'_>) -> crate::interp::Done {
    star_lookahead::<ONE_PC>(st);
    crate::interp::Done::Next
}

/// Test builds only: the fused Stars on this thread ([`star_lookahead`]) and a switch that
/// turns the lookahead off on this thread, so a test runs the same code with and without it.
#[cfg(test)]
pub(crate) mod star_count {
    use core::cell::Cell;
    thread_local! {
        /// The Stars that a producer ran ([`super::star_lookahead`]).
        pub(crate) static FUSED: Cell<u64> = const { Cell::new(0) };
        /// True: no lookahead (every Star runs in its own handler).
        pub(crate) static OFF: Cell<bool> = const { Cell::new(false) };
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: the runs of [`wide`] on this thread (0 where `ops::PREFIXED_FAST` is true).
    pub(crate) static WIDE_RUNS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    //! A polymorphic global site through `LdaGlobal` and `LdaGlobalInsideTypeof` (the x86-64
    //! fast body leaves it to the cold body, [`super::load_global`]): it hits there, with no
    //! scan of the global object (the miss path would scan it). A `ForInPrepare` of a key
    //! array with a long length (the x86-64 fast body leaves it to the cold body,
    //! `ops::for_in_prepare_cold`). They run on every target; on x86-64 they read the forms of
    //! M8-P1 X86-SHAPE2 and M8-P3 LEVER-6.

    use ep9_bytecode::{FeedbackSlotKind, FunctionBuilder, FunctionId, VerifiedUnit};

    use crate::frame::int32_bits;
    use crate::object::attr;
    use crate::tests::{Asm, config, isolate, meta, on_big_stack, unit};
    use crate::{Dispatch, Isolate, Value, atoms, feedback, ic, object, props, shape};

    /// The script defines the global `f0`: `return icP` (one LdaGlobal site, slot 0).
    fn program() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("f0");
        s.b.lda_undefined().return_();
        let mut b = FunctionBuilder::new(meta("f0", 0, 0, 0, 0));
        let slot = b.feedback_slot(FeedbackSlotKind::LoadGlobal);
        let name = b.string_constant("icP");
        b.lda_global(name, slot).return_();
        unit("", vec![s.finish(), b.finish().unwrap()])
    }

    fn call(iso: &mut Isolate, f: Value) -> u64 {
        iso.call_raw(f, Value::UNDEFINED, &[]).unwrap().to_bits()
    }

    fn define(iso: &Isolate, name: &str, v: i32) {
        let rt = &iso.rt;
        let k = atoms::intern_str(rt, name);
        props::own_define(rt, rt.global(), k, int32_bits(v), attr::DEFAULT);
    }

    #[test]
    fn a_polymorphic_global_site_hits_in_the_cold_body_with_no_scan() {
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                on_big_stack(move || {
                    let mut c = config(d, stress);
                    c.feedback_threshold = 0;
                    let mut iso = isolate(c);
                    iso.with_cx_checkpoint(|cx| cx.run_script(program(), "t.js").map(|_| ()))
                        .unwrap();
                    define(&iso, "icP", 5);
                    // (Read before the scan count: this lookup scans the global object.)
                    let f = iso.global_raw("f0").unwrap();
                    // A miss records the global object case.
                    assert_eq!(call(&mut iso, f), int32_bits(5));
                    // A new global property gives the global object a new shape (the old one
                    // stays alive: a shape keeps its parent). The site misses again and keeps
                    // a second case: a polymorphic entry.
                    define(&iso, "icQ", 6);
                    assert_eq!(call(&mut iso, f), int32_bits(5));
                    let state = {
                        let units = iso.rt.units.borrow();
                        let v = feedback::vector(&units.last().unwrap().infos[1]).unwrap();
                        ic::state(v, 0)
                    };
                    assert_eq!(state, ic::State::Polymorphic(2), "{d:?} stress {stress}");
                    // Hits: no scan of the global object's shape.
                    let gs = shape::shape_of(iso.rt.global());
                    shape::SCANS.with(|c| c.set((gs, 0)));
                    for _ in 0..3 {
                        assert_eq!(call(&mut iso, f), int32_bits(5));
                    }
                    assert_eq!(shape::SCANS.with(|c| c.get().1), 0, "{d:?} stress {stress}");
                });
            }
        }
    }

    /// The script defines the global `f0`: `return typeof icP` as one LdaGlobalInsideTypeof
    /// site (slot 0).
    fn typeof_program() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("f0");
        s.b.lda_undefined().return_();
        let mut b = FunctionBuilder::new(meta("f0", 0, 0, 0, 0));
        let slot = b.feedback_slot(FeedbackSlotKind::LoadGlobalInsideTypeof);
        let name = b.string_constant("icP");
        b.lda_global_inside_typeof(name, slot).return_();
        unit("", vec![s.finish(), b.finish().unwrap()])
    }

    /// M8-P3 LEVER-6: `LdaGlobalInsideTypeof` has the `LdaGlobal` form (the x86-64 fast body
    /// leaves a polymorphic entry to its cold body, which hits there). An undefined name reads
    /// undefined (no ReferenceError: the `typeof` form) on the first and on later calls.
    #[test]
    fn a_polymorphic_typeof_global_site_hits_in_the_cold_body_with_no_scan() {
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                on_big_stack(move || {
                    let mut c = config(d, stress);
                    c.feedback_threshold = 0;
                    let mut iso = isolate(c);
                    iso.with_cx_checkpoint(|cx| {
                        cx.run_script(typeof_program(), "t.js").map(|_| ())
                    })
                    .unwrap();
                    let f = iso.global_raw("f0").unwrap();
                    assert_eq!(call(&mut iso, f), Value::UNDEFINED.to_bits());
                    define(&iso, "icP", 5);
                    assert_eq!(call(&mut iso, f), int32_bits(5));
                    define(&iso, "icQ", 6);
                    assert_eq!(call(&mut iso, f), int32_bits(5));
                    let state = {
                        let units = iso.rt.units.borrow();
                        let v = feedback::vector(&units.last().unwrap().infos[1]).unwrap();
                        ic::state(v, 0)
                    };
                    assert_eq!(state, ic::State::Polymorphic(2), "{d:?} stress {stress}");
                    let gs = shape::shape_of(iso.rt.global());
                    shape::SCANS.with(|c| c.set((gs, 0)));
                    for _ in 0..3 {
                        assert_eq!(call(&mut iso, f), int32_bits(5));
                    }
                    assert_eq!(shape::SCANS.with(|c| c.get().1), 0, "{d:?} stress {stress}");
                });
            }
        }
    }

    /// The script defines the global `f0`: `ForInPrepare` of the global `keys` as the key
    /// array, then the key count (the third cache register). Feedback slot 1 is the ForIn slot.
    fn for_in_program() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("f0");
        s.b.lda_undefined().return_();
        let mut b = FunctionBuilder::new(meta("f0", 0, 0, 0, 0));
        let load = b.feedback_slot(FeedbackSlotKind::LoadGlobal);
        let fs = b.feedback_slot(FeedbackSlotKind::ForIn);
        let cache = b.new_register_list(3);
        let name = b.string_constant("keys");
        b.lda_global(name, load)
            .for_in_prepare(cache, fs)
            .ldar(cache.get(2))
            .return_();
        unit("", vec![s.finish(), b.finish().unwrap()])
    }

    /// M8-P3 LEVER-6: the x86-64 `ForInPrepare` fast body leaves a key array whose length is
    /// not an int32 (2^31 or more) to its cold body, which finishes it as the fast body does:
    /// the same key count (the length as a wrapped int32, as before) and the same ForIn record.
    /// A short array runs the fast body on every target.
    #[test]
    fn a_long_for_in_key_array_finishes_in_the_cold_body() {
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                on_big_stack(move || {
                    let mut got = Vec::new();
                    for len in [3u32, 1 << 31, u32::MAX] {
                        let mut c = config(d, stress);
                        c.feedback_threshold = 0;
                        let mut iso = isolate(c);
                        iso.with_cx_checkpoint(|cx| {
                            cx.run_script(for_in_program(), "t.js").map(|_| ())
                        })
                        .unwrap();
                        let rt = &iso.rt;
                        let a = object::new_array(rt, 0);
                        assert!(object::array_set_length(rt, a, len));
                        let k = atoms::intern_str(rt, "keys");
                        props::own_define(rt, rt.global(), k, a, attr::DEFAULT);
                        let f = iso.global_raw("f0").unwrap();
                        assert_eq!(call(&mut iso, f), int32_bits(len as i32), "{d:?} {len}");
                        let units = iso.rt.units.borrow();
                        let v = feedback::vector(&units.last().unwrap().infos[1]).unwrap();
                        got.push(feedback::value(v, 1));
                    }
                    // The cold finish records the bits of the fast record.
                    assert!(
                        got.iter().all(|&b| b == got[0]),
                        "{d:?} stress {stress} {got:x?}"
                    );
                });
            }
        }
    }
}
