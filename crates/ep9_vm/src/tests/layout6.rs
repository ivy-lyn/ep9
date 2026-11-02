//! The OBJECT_LAYOUT 6, IC_LAYOUT 4 and VM_LAYOUT 1 tests (M6-P0 VM6, plan v11 §2.4 step 8):
//! the version pins, kind 84 registered, the FUSE cell size and the `Root::RealmFuses` index
//! against live objects, the (IC_LAYOUT_VERSION, keyed kinds-set mask) pair, the shape flags
//! probe at bit 30, and each hand-fact export against the live value. The module is declared in
//! VM6 step 0 with the other month-6 test modules; VM6 step 8 fills it.

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
