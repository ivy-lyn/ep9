//! The OBJECT_LAYOUT 6, IC_LAYOUT 4 and VM_LAYOUT 1 tests (M6-P0 VM6, plan v11 §2.4 step 8):
//! the version pins, kind 84 registered, the FUSE cell size and the `Root::RealmFuses` index
//! against live objects, the (IC_LAYOUT_VERSION, keyed kinds-set mask) pair, the shape flags
//! probe at bit 30, and each hand-fact export against the live value. The module is declared in
//! VM6 step 0 with the other month-6 test modules; VM6 step 8 fills it.

use ep9_bytecode::{FunctionId, VerifiedUnit};

use super::{Asm, config, isolate, meta, unit};
use crate::frame::{UNDEFINED, int32_of};
use crate::{Dispatch, IsolateConfig, kinds, layout, object, shape};

/// The word at byte `off` of the cell value `c`.
///
/// # Safety
/// `c` is a live cell of an isolate that runs nothing while this reads, at least `off + 8`
/// bytes long.
unsafe fn word_at(c: u64, off: usize) -> u64 {
    // SAFETY: caller contract.
    unsafe { ((c as usize + off) as *const u64).read() }
}

/// The kind byte of the cell value `c` (as [`word_at`]).
unsafe fn kind_byte(c: u64) -> u8 {
    // SAFETY: caller contract.
    unsafe { ((c as usize + layout::KIND_BYTE as usize) as *const u8).read() }
}

/// The 4 OBJECT_LAYOUT hand-fact exports of M6-P0 step 3 (were Spike C `HAND_FACTS`) on live
/// cells: the shape rule with [`layout::SHAPE_CAGE_ALIGN`], the shape's `counts` word at
/// [`layout::SHAPE_COUNTS`] (its fields equal the shape accessors), and the SHAPE and CONTEXT
/// kind ids.
#[test]
fn the_object_layout_hand_fact_exports_hold_on_live_cells() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    assert!(
        rt.heap
            .cage_base()
            .is_multiple_of(layout::SHAPE_CAGE_ALIGN as usize)
    );
    let o = object::new_object(rt, rt.root(crate::runtime::Root::ObjectPrototype));
    let a = object::new_array(rt, 2);
    object::array_set(rt, a, 0, crate::frame::int32_bits(5));
    let c = object::new_context(rt, UNDEFINED, core::ptr::null(), 2);
    for v in [o, a] {
        let s = shape::shape_of(v);
        // SAFETY: `v`, its shape and `c` are live cells of `iso`, which runs nothing here.
        unsafe {
            let id = u64::from((word_at(v, 0) as u32) >> (8 * layout::SHAPE_ID_BYTE));
            assert_eq!((v & !(layout::SHAPE_CAGE_ALIGN - 1)) | id, s);
            assert_eq!(kind_byte(s), layout::KIND_SHAPE);
            let counts = int32_of(word_at(s, layout::SHAPE_COUNTS as usize));
            let bits = counts as u32;
            assert_eq!(
                (bits >> layout::SHAPE_ELEMENTS_KIND_SHIFT) as u8,
                shape::elements_kind(s)
            );
            assert_eq!(
                (bits >> 20) as u8,
                shape::object_kind(s),
                "counts bits 20-27"
            );
            assert_eq!((bits >> 12) as u8, shape::capacity(s), "counts bits 12-19");
            assert_eq!((bits & 0xFFF) as usize, shape::count(s), "counts bits 0-11");
        }
    }
    // SAFETY: as above.
    unsafe {
        assert_eq!(kind_byte(c), layout::KIND_CONTEXT);
        assert_eq!(kind_byte(c), kinds::CONTEXT.get());
    }
    assert_eq!(
        (
            layout::SHAPE_CAGE_ALIGN,
            layout::SHAPE_COUNTS,
            layout::KIND_SHAPE,
            layout::KIND_CONTEXT
        ),
        (64 << 30, 48, 128, 66),
        "the values Spike C baked as hand facts (OBJECT_LAYOUT 5)"
    );
}

/// `f(a)`: `if (a) return undefined; return g` (its global-load slot stays unfilled when
/// `a` is true).
fn unfilled_unit() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 1, 0, 0, 0));
    let a0 = f.b.parameter(0);
    let skip = f.b.new_label();
    f.b.ldar(a0).jump_if_true(skip);
    f.lda_global("g");
    f.b.return_();
    f.b.bind(skip);
    f.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("f");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), f.finish()])
}

/// A script: `g = 1` if `define_g`, then `f(a)`.
fn call_unit(a: bool, define_g: bool) -> VerifiedUnit {
    let mut s = Asm::script(0);
    if define_g {
        s.b.lda_smi(1);
        s.sta_global("g");
    }
    let (f, a0) = (s.f, s.a.get(0));
    if a {
        s.b.lda_true();
    } else {
        s.b.lda_false();
    }
    s.b.star(a0);
    s.lda_global("f");
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// The IC_LAYOUT and DEOPT_META hand-fact exports of M6-P0 step 3: a feedback slot that no
/// site filled has the key [`layout::VALUE_UNDEFINED`] and reads Uninitialized, and a filled
/// one does not; [`layout::DUMP_BYTES`] holds the `deopt::DUMP_REGS` words, 16-aligned.
#[test]
fn the_ic_and_deopt_hand_fact_exports_hold() {
    let c = IsolateConfig {
        feedback_threshold: 0,
        ..config(Dispatch::DEFAULT, false)
    };
    let mut iso = isolate(c);
    let mut run =
        |u: VerifiedUnit| iso.with_cx_checkpoint(|cx| cx.run_script(u, "t.js").map(|_| ()));
    run(unfilled_unit()).unwrap();
    run(call_unit(true, false)).unwrap();
    let f = iso.global_raw("f").unwrap();
    let fi = object::function_info(f.to_bits());
    let v = crate::feedback::vector(fi).expect("a vector (threshold 0)");
    // `Asm::new`'s `load` slot (the 4th, index 3) is the global load's slot.
    assert_eq!(crate::feedback::key(v, 3), layout::VALUE_UNDEFINED);
    assert_eq!(crate::ic::state(v, 3), crate::ic::State::Uninitialized);
    // Control: the load of a missing `g` throws and records no case; once `g` exists, the
    // load fills the key, which is then not the unfilled value.
    let mut run =
        |u: VerifiedUnit| iso.with_cx_checkpoint(|cx| cx.run_script(u, "t.js").map(|_| ()));
    assert!(run(call_unit(false, false)).is_err());
    assert_eq!(crate::feedback::key(v, 3), layout::VALUE_UNDEFINED);
    run(call_unit(false, true)).unwrap();
    assert_ne!(crate::feedback::key(v, 3), layout::VALUE_UNDEFINED);
    assert_eq!(crate::ic::state(v, 3), crate::ic::State::Monomorphic);
    assert_eq!(UNDEFINED, ep9_value::encoding::VALUE_UNDEFINED);

    assert_eq!(
        layout::DUMP_BYTES,
        256,
        "the value Spike C baked (DEOPT_META 0)"
    );
    assert!(layout::DUMP_BYTES as usize >= 8 * crate::deopt::DUMP_REGS);
    assert!(layout::DUMP_BYTES.is_multiple_of(16));
    assert_eq!(crate::deopt::DUMP_REGS, 31, "x0-x30");
}

/// RuntimeTable entry 11 `load_global` (M6-P0 VM6 step 2, V11-D43): a Rust function stands in
/// for native code. It is installed as the native code of a bytecode function and calls the
/// entry for that function's global load, so the entry runs with a real native frame.
#[cfg(feature = "spike-native")]
mod load_global {
    use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

    use ep9_bytecode::{
        FeedbackSlotKind, FunctionId, Opcode, RegisterList, RuntimeId, ScopeKind, VariableMode,
        VerifiedUnit,
    };
    use ep9_value::Value;

    use crate::dispatch::DispatchKind;
    use crate::native::{NativeCx, RuntimeTable};
    use crate::tests::{Asm, config, isolate, meta, on_big_stack, unit_with_scopes};
    use crate::{Dispatch, Exception, Isolate, IsolateConfig};

    /// The bytecode offset the stand-in passes to the entry.
    static OFF: AtomicU32 = AtomicU32::new(0);

    /// The stand-in for native code: the entry's result for the load at [`OFF`].
    #[cfg(target_arch = "x86_64")]
    unsafe extern "sysv64" fn entry(cx: *mut NativeCx, fp: *mut u64) -> u64 {
        // SAFETY: called as native code, with the Cx and the live frame of its function.
        unsafe { (RuntimeTable::process().load_global)(cx, fp, OFF.load(Relaxed)) }
    }
    /// The stand-in for native code: the entry's result for the load at [`OFF`].
    #[cfg(not(target_arch = "x86_64"))]
    unsafe extern "C" fn entry(cx: *mut NativeCx, fp: *mut u64) -> u64 {
        // SAFETY: called as native code, with the Cx and the live frame of its function.
        unsafe { (RuntimeTable::process().load_global)(cx, fp, OFF.load(Relaxed)) }
    }

    /// `x = 7`; the functions below; `let tdz` declared, then a throw before it is set, so
    /// `tdz` stays in its TDZ.
    /// lg(a): `if (a) return undefined; return x` (the load after a jump, at offset > 0);
    /// lt(): `return typeof nope` (the load part); lm(): `return nope`; lz(): `return tdz`;
    /// nl(): `return undefined` (no global load at offset 0).
    fn defs() -> VerifiedUnit {
        let mut lg = Asm::new(meta("lg", 1, 0, 0, 0));
        let a0 = lg.b.parameter(0);
        let skip = lg.b.new_label();
        lg.b.ldar(a0).jump_if_true(skip);
        lg.lda_global("x");
        lg.b.return_();
        lg.b.bind(skip);
        lg.b.lda_undefined().return_();
        let mut lt = Asm::new(meta("lt", 0, 0, 0, 0));
        let k = lt.b.string_constant("nope");
        let typeof_slot = lt.b.feedback_slot(FeedbackSlotKind::LoadGlobalInsideTypeof);
        lt.b.lda_global_inside_typeof(k, typeof_slot).return_();
        let mut lm = Asm::new(meta("lm", 0, 0, 0, 0));
        lm.lda_global("nope");
        lm.b.return_();
        let mut lz = Asm::new(meta("lz", 0, 0, 0, 0));
        lz.lda_global("tdz");
        lz.b.return_();
        let mut nl = Asm::new(meta("nl", 0, 0, 0, 0));
        nl.b.lda_undefined().return_();
        let fns = vec![("lg", lg), ("lt", lt), ("lm", lm), ("lz", lz), ("nl", nl)];
        let mut s = Asm::script(0);
        s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
        s.b.lda_smi(7);
        s.sta_global("x");
        for (i, (name, _)) in fns.iter().enumerate() {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        s.b.lda_smi(1).throw();
        let mut all = vec![s.finish()];
        all.extend(fns.into_iter().map(|(_, a)| a.finish()));
        unit_with_scopes(
            "",
            vec![(ScopeKind::Script, None, vec![("tdz", VariableMode::Let)])],
            all,
        )
    }

    /// An isolate that ran `defs()` (it throws 1 at its end, after the definitions).
    fn defs_isolate(config: IsolateConfig) -> Isolate {
        let mut iso = isolate(config);
        let e = iso
            .with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
            .unwrap_err();
        assert_eq!(e.message, "1");
        iso
    }

    /// The offset of the first `op` in the global function `name`.
    fn offset_of(iso: &Isolate, name: &str, op: Opcode) -> u32 {
        let t = crate::native_target(iso, iso.global_raw(name).unwrap()).unwrap();
        ep9_bytecode::Instructions::new(t.bytecode)
            .map(|i| i.unwrap())
            .find(|i| i.opcode == op)
            .unwrap_or_else(|| panic!("no {op:?} in {name}"))
            .offset
    }

    /// Installs [`entry`] as the native code of `name`, with the entry offset `off`.
    fn install(iso: &Isolate, name: &str, off: u32) -> Value {
        OFF.store(off, Relaxed);
        let f = iso.global_raw(name).unwrap();
        let start = entry as *const () as usize;
        // SAFETY: the range starts at `entry`, a function with the native entry ABI that lives
        // as long as the process.
        unsafe { crate::install_native(iso, f, DispatchKind::Baseline, start..start + 4) }.unwrap();
        f
    }

    /// The result bits of a call of `f` from Rust (the native entry of `f` if it has one).
    fn call(iso: &mut Isolate, f: Value, args: &[Value]) -> Result<u64, Exception> {
        crate::call_from_rust(iso, f, Value::UNDEFINED, args).map(Value::to_bits)
    }

    /// The value of a global data property, `undefined` for a missing name in `typeof`, the
    /// T0 ReferenceErrors for a missing name and a TDZ binding, and an internal error for an
    /// offset that holds no global load. Each run in a new isolate (one install each).
    #[test]
    fn load_global_runs_the_t0_lookup() {
        for &d in Dispatch::ALL {
            on_big_stack(move || {
                let mut iso = defs_isolate(config(d, false));
                let off = offset_of(&iso, "lg", Opcode::LdaGlobal);
                assert!(off > 0, "the load follows a jump");
                let f = install(&iso, "lg", off);
                assert_eq!(
                    call(&mut iso, f, &[Value::FALSE]).unwrap(),
                    Value::int32(7).to_bits()
                );

                let mut iso = defs_isolate(config(d, true));
                let f = install(&iso, "lt", 0);
                assert_eq!(call(&mut iso, f, &[]).unwrap(), Value::UNDEFINED.to_bits());

                let mut iso = defs_isolate(config(d, true));
                let f = install(&iso, "lm", 0);
                let e = call(&mut iso, f, &[]).unwrap_err();
                assert_eq!(e.constructor.as_deref(), Some("ReferenceError"));
                assert_eq!(e.message, "ReferenceError: nope is not defined");

                let mut iso = defs_isolate(config(d, false));
                let f = install(&iso, "lz", 0);
                let e = call(&mut iso, f, &[]).unwrap_err();
                assert_eq!(
                    e.message,
                    "ReferenceError: Cannot access 'tdz' before initialization"
                );

                let mut iso = defs_isolate(config(d, false));
                let f = install(&iso, "nl", 0);
                let e = call(&mut iso, f, &[]).unwrap_err();
                assert!(
                    e.message.contains("no global load at offset 0"),
                    "{}",
                    e.message
                );
            });
        }
    }

    /// MC-w15-6: the entry records no feedback. The vector of `lg` exists (threshold 0, a T0
    /// call that skips the load); the native load leaves the slot of the load Uninitialized.
    /// Control: the T0 load of the same function records a case there.
    #[test]
    fn load_global_records_no_feedback() {
        on_big_stack(|| {
            let c = IsolateConfig {
                feedback_threshold: 0,
                ..config(Dispatch::Match, false)
            };
            let mut iso = defs_isolate(c);
            let f = iso.global_raw("lg").unwrap();
            assert_eq!(
                call(&mut iso, f, &[Value::TRUE]).unwrap(),
                Value::UNDEFINED.to_bits()
            );
            let state = || {
                let fi = crate::object::function_info(f.to_bits());
                let v = crate::feedback::vector(fi).expect("a vector (threshold 0)");
                // `lg` has one global-load slot, `Asm::new`'s `load` slot (the 4th, index 3).
                crate::ic::state(v, 3)
            };
            assert_eq!(state(), crate::ic::State::Uninitialized);
            let off = offset_of(&iso, "lg", Opcode::LdaGlobal);
            install(&iso, "lg", off);
            for _ in 0..3 {
                assert_eq!(
                    call(&mut iso, f, &[Value::FALSE]).unwrap(),
                    Value::int32(7).to_bits()
                );
            }
            assert_eq!(state(), crate::ic::State::Uninitialized);
            crate::uninstall_native(&iso, f).unwrap();
            assert_eq!(
                call(&mut iso, f, &[Value::FALSE]).unwrap(),
                Value::int32(7).to_bits()
            );
            assert_eq!(state(), crate::ic::State::Monomorphic);
        });
    }
}
