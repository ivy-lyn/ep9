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
//!
//! The Star lookahead (M9-P0 STAR-FUSE; V8 `Bytecodes::IsStarLookahead`,
//! `InterpreterAssembler::StarDispatchLookahead`): `star_lookahead`, with `star_next` and
//! `star_done` for the `Flow::Next` and `Done::Next` exits of a producer's fast body. It is a
//! body form on every target: no new opcode, handler, `Flow` arm or cold block, and no change
//! to `interp/dsl.rs` (H15 dispatch core; DL M9-PLAN-D6, M9-PRE-STARFUSE-KIND).

use crate::interp::{Done, Flow, St, dsl};
#[cfg(all(target_arch = "x86_64", not(feature = "ic-stats")))]
use crate::{error::FrameRef, ic, interp::Cx, interp::slow};
use ep9_bytecode::Opcode;

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

/// The Star lookahead (M9-P0 STAR-FUSE): if the next instruction is a one-byte-scale `Star r`,
/// the producer runs it here: it stores the accumulator to `r` and sets `st.next` past the
/// Star, so the dispatch goes to the instruction after the Star and the Star handler does not
/// run. A producer calls it only on its success exit ([`star_next`], [`star_done`]), after it
/// wrote the accumulator; an exception, a cold exit or a record exit runs no lookahead, so the
/// Star runs in its own handler there, as before.
///
/// The producers (M9-P0, from the TS bytecode count: the producer kinds with the most Stars
/// after them and a share of at least 81%): `GetNamedProperty` (its IC hit), the four
/// context-slot loads and `LdaCurrentContext`. A producer that a Star follows less often pays
/// the test on each run and gains little: with `LdaConstant`, `LdaZero`, `LdaSmi`, `LdaNull`
/// and `LdaFalse` too (36% to 60% of them before a Star) the Mac a64 count of
/// octane-typescript was 0.16% higher.
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
/// - A `Wide` or `ExtraWide` Star (a prefix byte at `st.next`) does not fuse.
///
/// The pc forms (asm read, M9-P0; [`lookahead_pc`]). aarch64: `st.next` goes through an empty
/// `asm!` block before the test, so LLVM keeps it in `x1` (else it copied it to another
/// register and back: 1 or 2 more instructions on each path; Mac a64 octane-typescript count
/// +0.93% with the copies, +0.46% without them). x86-64 with `ONE_PC`: through
/// [`dsl::dispatch_pc`] before the test and after it, so LLVM keeps the next pc in one
/// register and reads the opcode again at the dispatch. The producers whose success exit is a
/// join of several paths take it (`GetNamedProperty`: the IC hit paths; the context-slot
/// loads: the context walk); without it LLVM kept the old pc, the new pc and the loaded opcode
/// byte live into the join (`GetNamedProperty` 2 more instructions on each path, the
/// context-slot loads 1). x86-64 without `ONE_PC` (`LdaCurrentContext`): LLVM gives the fused
/// and the other path a dispatch jump each, as V8 does.
#[inline(always)]
pub(crate) fn star_lookahead<const ONE_PC: bool>(st: &mut St<'_>) {
    #[cfg(test)]
    if star_count::OFF.with(|c| c.get()) {
        return;
    }
    st.next = lookahead_pc::<ONE_PC>(st.next);
    // SAFETY: `st.next` is the start of the next instruction of verified code (`St` contract):
    // its opcode byte is readable.
    if unsafe { *st.next } == Opcode::Star as u8 {
        // SAFETY: verified code: a one-byte-scale `Star` is followed by its `RegOut` operand,
        // a register of the live frame `fp`, and the instruction after it (a verified function
        // never runs off its end).
        let (len, (dst,)) = unsafe { dsl::decode!(dsl::S1, st.next; dst: RegOut) };
        let v = st.acc;
        st.set_reg(dst, v);
        // SAFETY: as above: the next instruction starts `len` bytes after the Star.
        st.next = unsafe { st.next.add(len) };
        #[cfg(test)]
        star_count::FUSED.with(|c| c.set(c.get() + 1));
    }
    if ONE_PC {
        st.next = dsl::dispatch_pc(st.next);
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

/// `Flow::Next` after the Star lookahead ([`star_lookahead`], `ONE_PC`): the success exit of
/// `GetNamedProperty` and the context-slot loads.
#[inline(always)]
pub(crate) fn star_next(st: &mut St<'_>) -> Flow {
    star_lookahead::<true>(st);
    Flow::Next
}

/// `Done::Next` after the Star lookahead ([`star_lookahead`], plain form): the exit of
/// `LdaCurrentContext`.
#[inline(always)]
pub(crate) fn star_done(st: &mut St<'_>) -> Done {
    star_lookahead::<false>(st);
    Done::Next
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
