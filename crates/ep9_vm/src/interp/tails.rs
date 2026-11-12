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

#[cfg(test)]
mod tests {
    //! A polymorphic global site through `LdaGlobal` and `LdaGlobalInsideTypeof` (the x86-64
    //! fast body leaves it to the cold body, [`super::load_global`]): it hits there, with no
    //! scan of the global object (the miss path would scan it). They run on every target; on
    //! x86-64 they read the forms of M8-P1 X86-SHAPE2 and M8-P3 LEVER-6.

    use ep9_bytecode::{FeedbackSlotKind, FunctionBuilder, FunctionId, VerifiedUnit};

    use crate::frame::int32_bits;
    use crate::object::attr;
    use crate::tests::{Asm, config, isolate, meta, on_big_stack, unit};
    use crate::{Dispatch, Isolate, Value, atoms, feedback, ic, props, shape};

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
}
