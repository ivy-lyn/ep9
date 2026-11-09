//! Tests of the A2 unit loader (`unit_loader.rs`, M7-P4; plan v12 §7.3). They build and run in
//! every build: no `jit`, no `spike-native`. Every test name starts with `unit_loader_`.
//!
//! | Tests | What they show |
//! |---|---|
//! | `unit_loader_*_preset*` | MC-p0-cx-1: a unit variant with lowering `jit` is skipped under every preset (JIT, jitless, hybrid), before `select()`; `select()` alone would pick it under hybrid |
//! | `unit_loader_counts_*` | each skip reason, the digest binding (MC-w31-8), one load per function |
//! | `unit_loader_*_fuse*` | steps 6-7: a popped or unknown fuse makes the object a dependency that does not hold |
//! | `unit_loader_api_*` | the Cx API: the isolate's policy (MC-w24-3 form), no install in M7-P4 (MC-w30-2), the bytecode runs |
//! | `unit_loader_registered_*` | MC-w33-2.2: unit text copied at run time (`ep9_platform::unit_text`, no `jit`), its unwind records registered at run time, installed by step 8 from Rust, entered by `call_native` and walked by `ep9_platform::unwind` |
//! | `unit_loader_build_*` | SHSTK-ENGINE: the ep9_vm build script runs the shadow-stack refusal |
//!
//! The JS-level checks (a unit from `ep9_unit::check`, M-R9 with and without a loaded unit,
//! one intern table) are in `crates/ep9_unit/tests/loader.rs`: ep9_vm tests cannot compile JS.

use ep9_bytecode::{FunctionFlags, FunctionId, FunctionKind, VerifiedUnit};
use ep9_platform::CodeRange;

use crate::dispatch::DispatchKind;
use crate::frame::UNDEFINED;
use crate::fuse::FuseRef;
use crate::layout;
use crate::native::call_native;
use crate::select::{self, CodeObject, CodeSet, Pick, Policy, Session, lowering};
use crate::tests::{Asm, meta, pos, unit};
use crate::unit::FunctionInfo;
use crate::unit_loader::{
    self, FunctionCode, Reason, Report, VariantCode, code_of, install_picked, unit_function_digest,
};
use crate::{Args, Cx, Exception, ExecOptions, Isolate, IsolateConfig, JsResult, Local, Value};

/// The source of the unit (positions for stack traces).
const SRC: &str = "function g(f) { return f(); }\n\
function id(x) { return x; }\n\
class K {}\n";

/// The unit: function 1 `g(f) { return f(); }` (the call at bytecode offset 0, the offset the
/// frame stub passes), function 2 `id(x)`, function 3 `K` (a class constructor). Each is a
/// global.
fn defs() -> VerifiedUnit {
    let p = |n: &str| pos(SRC, n);
    let mut g = Asm::new(meta("g", 1, 0, p("function g"), p("function id") - 1));
    let a0 = g.b.parameter(0);
    g.b.set_expression_position(p("f()"));
    g.b.call_undefined_receiver0(a0, g.call).return_();
    let mut id = Asm::new(meta("id", 1, 0, p("function id"), p("class K") - 1));
    let a0 = id.b.parameter(0);
    id.b.ldar(a0).return_();
    let mut m = meta("K", 0, 0, p("class K"), SRC.len() as u32);
    m.kind = FunctionKind::ClassConstructor;
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
    let mut k = Asm::new(m);
    k.b.lda_undefined().return_();
    let fns = vec![("g", g), ("id", id), ("K", k)];
    let mut s = Asm::script(SRC.len());
    for (i, (name, _)) in fns.iter().enumerate() {
        let c = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(c, s.closure, 0);
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns.into_iter().map(|(_, a)| a.finish()));
    unit(SRC, all)
}

const G: u32 = 1;
const ID: u32 = 2;
const K: u32 = 3;

/// An isolate of `exec` that ran `defs()`, with the host function `walk`.
fn defs_isolate(exec: ExecOptions) -> Isolate {
    let mut iso = Isolate::new_with(IsolateConfig::default(), exec).unwrap();
    iso.register_host_function("walk", 0, walk);
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
        .unwrap();
    iso
}

/// The FunctionInfo of the global function `name`.
fn info<'i>(iso: &'i Isolate, name: &str) -> &'i FunctionInfo {
    let f = iso.global_raw(name).unwrap().to_bits();
    assert!(crate::object::is_function(f), "{name}");
    crate::object::function_info(f)
}

/// The digest of function `i` of `defs()`.
fn digest(i: u32) -> [u8; 16] {
    unit_function_digest(&defs(), i).unwrap()
}

/// A variant of `kind` and `lowering` over `entry..entry + len`, no CPU need, no fuse.
fn variant(kind: DispatchKind, lowering: u8, entry: u32, len: u32) -> VariantCode {
    VariantCode {
        kind: kind as u8,
        lowering,
        entry,
        len,
        cpu: 0,
        fuses: Vec::new(),
    }
}

/// Text of `n` bytes that is never run (the tests that run code use [`call0_bytes`]).
fn filler(n: usize) -> Vec<u8> {
    vec![0xCC; n]
}

/// Loads `functions` for the unit of `defs()` in `iso` with `policy`.
fn load(iso: &Isolate, policy: &Policy, functions: &[FunctionCode]) -> Report {
    let lu = info(iso, "g").loaded_unit().unwrap();
    unit_loader::load(&iso.rt, lu, &filler(256), functions, Some(policy), 0)
}

// ---- MC-p0-cx-1: the jit lowering in a unit, under every preset ------------------------------

/// The three variants of `g`: AotBaseline with lowering jit (never stored in a unit), hybrid
/// and jitless.
fn three_lowerings() -> Vec<VariantCode> {
    vec![
        variant(DispatchKind::AotBaseline, lowering::JIT, 0, 4),
        variant(DispatchKind::AotBaseline, lowering::HYBRID, 4, 4),
        variant(DispatchKind::AotBaseline, lowering::JITLESS, 8, 4),
    ]
}

/// The bug the loader closes: `select()` alone (one accept filter, MODES-D13) picks a unit
/// object whose lowering is `jit` under hybrid, because hybrid accepts `jit` first.
#[test]
fn unit_loader_select_alone_picks_a_jit_lowering_under_the_hybrid_preset() {
    let mut set = CodeSet::default();
    for v in three_lowerings() {
        set.objects
            .push(CodeObject::new(DispatchKind::AotBaseline, v.lowering));
    }
    let s = Session::default();
    assert_eq!(select::select(&set, &Policy::HYBRID, &s), Pick::Code(0));
}

/// MC-p0-cx-1 under every preset: the loader skips the `jit` variant before `select()`; the
/// pick is the hybrid variant under hybrid, the jitless one under jitless, the bytecode under
/// JIT (no AOT kind in its install rank). The skip is counted.
#[test]
fn unit_loader_skips_the_jit_lowering_under_every_preset() {
    // (policy, the pick as an index of the code set the loader built: hybrid 0, jitless 1)
    let cases = [
        ("jit", Policy::JIT, Pick::Interp),
        ("jitless", Policy::JITLESS, Pick::Code(1)),
        ("hybrid", Policy::HYBRID, Pick::Code(0)),
    ];
    for (name, policy, want) in cases {
        let iso = defs_isolate(ExecOptions::jitless());
        let f = FunctionCode {
            function: G,
            digest: digest(G),
            variants: three_lowerings(),
        };
        let r = load(&iso, &policy, &[f]);
        assert_eq!(r.count(Reason::JitLowering), 1, "{name}");
        let uc = code_of(info(&iso, "g")).expect("a code set");
        assert_eq!(
            uc.set.objects.len(),
            2,
            "{name}: the jit variant is not in the set"
        );
        assert!(
            uc.set.objects.iter().all(|o| o.lowering != lowering::JIT),
            "{name}"
        );
        assert_eq!(uc.pick, want, "{name}");
        if let Pick::Code(i) = want {
            assert_eq!(
                uc.set.objects[i].lowering,
                if name == "hybrid" {
                    lowering::HYBRID
                } else {
                    lowering::JITLESS
                },
                "{name}"
            );
        }
        // No install in M7-P4: the slot still runs bytecode.
        assert_eq!(r.count(Reason::Deferred), r.count(Reason::Picked), "{name}");
        assert_eq!(
            info(&iso, "g").slot.kind_byte(),
            DispatchKind::Interp as u8,
            "{name}"
        );
    }
}

// ---- the counts, the digest, one load per function --------------------------------------------

/// Every skip reason once, and the digest binding (MC-w31-8): a variant made from other
/// bytecode is never in a code set.
#[test]
fn unit_loader_counts_each_reason() {
    let iso = defs_isolate(ExecOptions::jitless());
    let jl = lowering::JITLESS;
    let fns = [
        // no function 9 in the unit
        FunctionCode {
            function: 9,
            digest: [0; 16],
            variants: vec![variant(DispatchKind::AotBaseline, jl, 0, 4)],
        },
        // `id` with the digest of `g`: made from other bytecode
        FunctionCode {
            function: ID,
            digest: digest(G),
            variants: vec![variant(DispatchKind::AotBaseline, jl, 0, 4)],
        },
        // `g`: a kind that is not AOT, a range outside the text, an empty range, one good
        FunctionCode {
            function: G,
            digest: digest(G),
            variants: vec![
                variant(DispatchKind::Baseline, jl, 0, 4),
                variant(DispatchKind::AotBaseline, jl, 250, 7),
                variant(DispatchKind::AotBaseline, jl, 8, 0),
                variant(DispatchKind::AotBaseline, jl, 16, 8),
            ],
        },
        // `g` again in the same table
        FunctionCode {
            function: G,
            digest: digest(G),
            variants: vec![variant(DispatchKind::AotBaseline, jl, 0, 4)],
        },
        // `K`: only an AotOpt variant of another lowering class (not accepted by jitless)
        FunctionCode {
            function: K,
            digest: digest(K),
            variants: vec![variant(DispatchKind::AotOpt, lowering::HYBRID, 0, 4)],
        },
    ];
    let r = load(&iso, &Policy::JITLESS, &fns);
    let want = [
        (Reason::NoText, 0),
        (Reason::Function, 1),
        (Reason::Twice, 1),
        (Reason::Digest, 1),
        (Reason::Kind, 1),
        (Reason::Range, 2),
        (Reason::JitLowering, 0),
        (Reason::Bytecode, 1),
        (Reason::Picked, 1),
        (Reason::Deferred, 1),
        (Reason::Installed, 0),
        (Reason::NotInstallable, 0),
    ];
    for (reason, n) in want {
        assert_eq!(r.count(reason), n, "{}", reason.name());
    }
    assert_eq!(r.rows().len(), Reason::ALL.len());
    assert!(code_of(info(&iso, "id")).is_none(), "the digest skip");
    let g = code_of(info(&iso, "g")).unwrap();
    assert_eq!((g.set.objects.len(), g.pick), (1, Pick::Code(0)));
    assert_eq!(g.code[0].len(), 8);
    assert_eq!(code_of(info(&iso, "K")).unwrap().pick, Pick::Interp);
    // A second load of the same unit: every listed function has its code already.
    let r2 = load(&iso, &Policy::JITLESS, &fns[2..3]);
    assert_eq!(r2.count(Reason::Twice), 1);
}

/// The digest is a function of the bytecode: two compiles of one unit agree, two functions
/// differ, a changed bytecode byte differs, and a function index outside the unit has none.
#[test]
fn unit_loader_digest_binds_the_bytecode() {
    assert_eq!(digest(G), digest(G));
    assert_ne!(digest(G), digest(ID));
    assert_eq!(unit_function_digest(&defs(), 99), None);
    // One more register in `id` (the bytecode does not change; the frame does): another digest.
    let mut cu = defs().into_inner();
    cu.functions[ID as usize].register_count += 1;
    let changed = ep9_bytecode::verify(cu).expect("still a valid unit");
    assert_ne!(unit_function_digest(&changed, ID), Some(digest(ID)));
    assert_eq!(unit_function_digest(&changed, G), Some(digest(G)));
}

// ---- steps 6-7: fuse refs --------------------------------------------------------------------

/// A variant that names a fuse: picked while the fuse is wired and intact; not picked after
/// the fuse pops, and never for an index this build does not know.
#[test]
fn unit_loader_a_popped_or_unknown_fuse_is_never_picked() {
    let with_fuse = |index: u32| FunctionCode {
        function: G,
        digest: digest(G),
        variants: vec![VariantCode {
            fuses: vec![index],
            ..variant(DispatchKind::AotBaseline, lowering::JITLESS, 0, 4)
        }],
    };
    // Wired and intact.
    let iso = defs_isolate(ExecOptions::jitless());
    iso.rt.fuses.wire(&iso.rt, 0).unwrap();
    let r = load(&iso, &Policy::JITLESS, &[with_fuse(0)]);
    assert_eq!(r.count(Reason::Picked), 1);
    // Popped before the load.
    let iso = defs_isolate(ExecOptions::jitless());
    iso.rt.fuses.wire(&iso.rt, 0).unwrap();
    iso.rt.fuses.pop(&iso.rt, FuseRef::Static(0));
    let r = load(&iso, &Policy::JITLESS, &[with_fuse(0)]);
    assert_eq!((r.count(Reason::Picked), r.count(Reason::Bytecode)), (0, 1));
    // An index past `fuse::STATIC`.
    let iso = defs_isolate(ExecOptions::jitless());
    let r = load(&iso, &Policy::JITLESS, &[with_fuse(1000)]);
    assert_eq!(r.count(Reason::Bytecode), 1);
    // A wired fuse is a dependency: step 8 does not install it (no pop path re-selects yet).
    let iso = defs_isolate(ExecOptions::jitless());
    iso.rt.fuses.wire(&iso.rt, 0).unwrap();
    load(&iso, &Policy::JITLESS, &[with_fuse(0)]);
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: the dependency refusal comes before `install_code`; no code is installed.
    let r = unsafe { install_picked(&iso.rt, lu) };
    assert_eq!(
        (r.count(Reason::Installed), r.count(Reason::NotInstallable)),
        (0, 1)
    );
    assert_eq!(info(&iso, "g").slot.kind_byte(), DispatchKind::Interp as u8);
}

/// One variant of the Cx API: (kind, lowering, entry, len, cpu, fuse ids).
type Var<'a> = (u8, u8, u32, u32, u64, &'a [u32]);

// ---- the Cx API ------------------------------------------------------------------------------

/// The Cx API with the isolate's policy: a jitless isolate picks the jitless variant of `id`,
/// the slot stays Interp (M7-P4: no install), and the script and a later call give the
/// bytecode results. A JIT isolate (with the `jit` feature) picks nothing from a unit.
#[test]
fn unit_loader_api_picks_by_the_isolate_policy_and_installs_nothing() {
    let mut presets = vec![(ExecOptions::jitless(), 1)];
    if cfg!(feature = "jit") {
        presets.push((ExecOptions::jit(), 0));
    }
    for (exec, picked) in presets {
        let mut iso = Isolate::new_with(IsolateConfig::default(), exec).unwrap();
        let d = digest(ID);
        let vars: &[Var<'_>] = &[
            (
                DispatchKind::AotBaseline as u8,
                lowering::JITLESS,
                0,
                8,
                0,
                &[],
            ),
            (DispatchKind::AotBaseline as u8, lowering::JIT, 8, 8, 0, &[]),
        ];
        let fns = [(ID, d, vars)];
        let text = filler(64);
        let rows = iso.with_cx(|cx| {
            let (rows, r) = cx.run_script_with_unit_code(defs(), "defs.js", &text, 0, &fns);
            r.map(|_| rows)
        });
        let rows = rows.unwrap();
        let get = |n: &str| rows.iter().find(|(k, _)| *k == n).unwrap().1;
        assert_eq!(get("picked"), picked, "{}", exec.preset().name());
        assert_eq!(get("deferred"), picked);
        assert_eq!(get("jit-lowering"), 1);
        assert_eq!(get("installed"), 0);
        let fi = info(&iso, "id");
        assert_eq!(fi.slot.kind_byte(), DispatchKind::Interp as u8);
        assert!(code_of(fi).is_some());
        // The bytecode runs: id(42) through Rust (T0).
        let id = iso.global_raw("id").unwrap();
        let v = iso
            .with_cx(|cx| {
                let f: Local = Local::from_bits(id.to_bits());
                let a: Local = Local::from_bits(Value::int32(42).to_bits());
                let u: Local = Local::from_bits(UNDEFINED);
                cx.call(f, u, &[a]).map(|v| v.bits())
            })
            .unwrap();
        assert_eq!(v, Value::int32(42).to_bits());
    }
}

// ---- MC-w33-2.2: unit text and unwind records at run time, no jit ------------------------------

thread_local! {
    /// The return addresses that `walk` saw (`ep9_platform::unwind`).
    static WALKED: core::cell::RefCell<Option<Vec<usize>>> = const { core::cell::RefCell::new(None) };
}

/// `walk()`: records the native stack (under the stub frame) and returns 42.
fn walk<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let ips = ep9_platform::unwind::native_return_addresses(512);
    WALKED.with(|w| *w.borrow_mut() = Some(ips));
    Ok(Local::from_bits(Value::int32(42).to_bits()))
}

/// The frame stub `return call_any(cx, fp, 0, 0)` (the `NativeEntry` ABI; the hand encoding
/// of `ep9_a2entry_stub_call0` in `tests/native_entry.rs`) and the offset of the return
/// address of its call.
fn call0_bytes() -> (Vec<u8>, usize) {
    #[cfg(target_arch = "aarch64")]
    {
        let ldr = |t: u32, n: u32, off: usize| {
            assert!(off.is_multiple_of(8) && off / 8 < 4096);
            0xF940_0000 | (((off / 8) as u32) << 10) | (n << 5) | t
        };
        let w = [
            0xA9BF_7BFD, // stp x29, x30, [sp, #-16]!
            0x9100_03FD, // mov x29, sp
            ldr(16, 0, layout::CX_RUNTIME_TABLE),
            ldr(16, 16, layout::RT_CALL_ANY),
            0x5280_0002, // mov w2, #0
            0xAA1F_03E3, // mov x3, xzr
            0xD63F_0200, // blr x16
            0xA8C1_7BFD, // ldp x29, x30, [sp], #16
            0xD65F_03C0, // ret
        ];
        (w.iter().flat_map(|w| w.to_le_bytes()).collect(), 28)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut v = vec![0x55, 0x48, 0x89, 0xE5]; // push rbp; mov rbp, rsp
        let table = i32::try_from(layout::CX_RUNTIME_TABLE).unwrap();
        assert!(table > 127);
        v.extend([0x48, 0x8B, 0x87]); // mov rax, [rdi + disp32]
        v.extend(table.to_le_bytes());
        v.extend([0x31, 0xD2, 0x31, 0xC9]); // xor edx, edx; xor ecx, ecx
        let e = u8::try_from(layout::RT_CALL_ANY).unwrap();
        assert!(e < 128);
        v.extend([0xFF, 0x50, e]); // call [rax + disp8]
        let ret = v.len();
        v.extend([0x5D, 0xC3]); // pop rbp; ret
        (v, ret)
    }
}

/// The offset of the Win64 UNWIND_INFO in the unit text (inside the registered range).
#[cfg(windows)]
const UNWIND_INFO_OFF: usize = 256;

/// The unit text: the frame stub at offset 0 (and on Windows its UNWIND_INFO).
fn stub_text() -> Vec<u8> {
    #[allow(unused_mut)]
    let mut text = call0_bytes().0;
    #[cfg(windows)]
    {
        use ep9_platform::unwind::win64::{UnwindInfoBuilder, reg};
        let info = UnwindInfoBuilder::new().push_nonvol(1, reg::RBP).encode();
        text.resize(UNWIND_INFO_OFF, 0xCC);
        text.extend(info);
    }
    text
}

#[cfg(unix)]
mod probe {
    //! A jitless build has no code of its own for the unwind self-test: the loader's text gives
    //! the probe (`ep9_platform::unwind` mode doc). The probe bytes and CFI are the built-in
    //! probe's (`ep9_platform/src/unwind/mode.rs`, `builtin`).
    use ep9_platform::unwind::cfi::{CfiProgram, Cie, EhFrameBuilder, reg};
    use ep9_platform::unwind::{ProbeFn, SelfTestProbe, self_test};
    use ep9_platform::{CodeRange, unit_text::UnitText};

    const OTHER_OFF: usize = 64;

    fn probe_code() -> Vec<u8> {
        #[cfg(target_arch = "aarch64")]
        {
            [
                0xa9be_7bfdu32, //  0 stp x29, x30, [sp, #-32]!
                0x9100_03fd,    //  4 mov x29, sp
                0xa901_53f3,    //  8 stp x19, x20, [sp, #16]
                0xaa00_03f3,    // 12 mov x19, x0
                0x9100_0400,    // 16 add x0, x0, #1
                0xd63f_0020,    // 20 blr x1
                0x8b13_0000,    // 24 add x0, x0, x19
                0xa941_53f3,    // 28 ldp x19, x20, [sp, #16]
                0xa8c2_7bfd,    // 32 ldp x29, x30, [sp], #32
                0xd65f_03c0,    // 36 ret
            ]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
        }
        #[cfg(target_arch = "x86_64")]
        {
            #[rustfmt::skip]
            let v = vec![
                0x55, 0x48, 0x89, 0xe5, 0x53, 0x48, 0x83, 0xec, 0x08, 0x48, 0x89, 0xfb,
                0x48, 0x8d, 0x7f, 0x01, 0xff, 0xd6, 0x48, 0x01, 0xd8, 0x48, 0x83, 0xc4,
                0x08, 0x5b, 0x5d, 0xc3,
            ];
            v
        }
    }

    fn other_code() -> Vec<u8> {
        #[cfg(target_arch = "aarch64")]
        {
            0xd65f_03c0u32.to_le_bytes().to_vec()
        }
        #[cfg(target_arch = "x86_64")]
        {
            vec![0xc3]
        }
    }

    fn probe_cfi() -> CfiProgram {
        let mut p = CfiProgram::new();
        #[cfg(target_arch = "aarch64")]
        {
            use reg::aarch64::*;
            p.advance_loc(1)
                .def_cfa_offset(32)
                .offset(FP, 4)
                .offset(LR, 3)
                .advance_loc(1)
                .def_cfa_register(FP)
                .advance_loc(1)
                .offset(X19, 2)
                .offset(X20, 1)
                .advance_loc(5)
                .restore(X19)
                .restore(X20)
                .advance_loc(1)
                .def_cfa(SP, 0)
                .restore(FP)
                .restore(LR);
        }
        #[cfg(target_arch = "x86_64")]
        {
            use reg::x86_64::*;
            p.advance_loc(1)
                .def_cfa_offset(16)
                .offset(RBP, 2)
                .advance_loc(3)
                .def_cfa_register(RBP)
                .advance_loc(1)
                .offset(RBX, 3)
                .advance_loc(21)
                .restore(RBX)
                .advance_loc(1)
                .def_cfa(RSP, 8)
                .restore(RBP);
        }
        p
    }

    /// Runs the unwind self-test on a probe in unit text (a no-op check once a mode is set).
    pub(super) fn choose_mode() {
        let (code, other) = (probe_code(), other_code());
        let mut text = code.clone();
        text.resize(OTHER_OFF, 0);
        text.extend(&other);
        let t = UnitText::map(CodeRange::process().unwrap(), &text).unwrap();
        let base = t.base();
        let mut b = EhFrameBuilder::new(&Cie::host());
        b.add_fde(base + OTHER_OFF, other.len(), &CfiProgram::new());
        b.add_fde(base, code.len(), &probe_cfi());
        // SAFETY: offset 0 of the text is the probe (the `ProbeFn` contract), mapped until `t`
        // drops after the test; the FDEs describe the text exactly; the page is new.
        let entry: ProbeFn = unsafe { core::mem::transmute::<usize, ProbeFn>(base) };
        let probe = SelfTestProbe {
            entry,
            code: base..base + code.len(),
            section: b.finish(),
        };
        // SAFETY: as above.
        unsafe { self_test(probe) }.expect("the unwind self-test on unit text");
        drop(t);
    }
}

/// The unwind records of the frame stub at `code`, registered with the system unwinder.
fn register(code: core::ops::Range<usize>) -> impl Drop {
    let ret = call0_bytes().1;
    #[cfg(unix)]
    {
        use ep9_platform::unwind::cfi::{CfiProgram, Cie, EhFrameBuilder};
        use ep9_platform::unwind::{self, cfi::reg};
        probe::choose_mode();
        let len = call0_bytes().0.len();
        let mut p = CfiProgram::new();
        #[cfg(target_arch = "aarch64")]
        {
            use reg::aarch64::*;
            let _ = ret;
            p.advance_loc(1)
                .def_cfa_offset(16)
                .offset(FP, 2)
                .offset(LR, 1)
                .advance_loc(1)
                .def_cfa_register(FP)
                .advance_loc(6)
                .def_cfa(SP, 0)
                .restore(FP)
                .restore(LR);
        }
        #[cfg(target_arch = "x86_64")]
        {
            use reg::x86_64::*;
            p.advance_loc(1)
                .def_cfa_offset(16)
                .offset(RBP, 2)
                .advance_loc(3)
                .def_cfa_register(RBP)
                .advance_loc((ret + 1 - 4) as u32)
                .def_cfa(RSP, 8)
                .restore(RBP);
        }
        let mut b = EhFrameBuilder::new(&Cie::host());
        b.add_fde(code.start, len, &p);
        // SAFETY: the FDE describes the stub at `code.start` exactly; the caller drops the
        // registration before the text.
        unsafe { unwind::EhFrameRegistration::register(b.finish()) }.expect("register the FDE")
    }
    #[cfg(windows)]
    {
        use ep9_platform::unwind::{self, win64::RuntimeFunction};
        let _ = ret;
        let len = call0_bytes().0.len();
        let all = code.start..code.start + stub_text().len();
        let entry = RuntimeFunction {
            begin: 0,
            end: len as u32,
            unwind_info: UNWIND_INFO_OFF as u32,
        };
        // SAFETY: the UNWIND_INFO at offset 256 describes the prologue of the stub at offset 0;
        // the caller drops the registration before the text.
        unsafe { unwind::FunctionTableRegistration::register(all, Box::new([entry])) }
            .expect("register the function table")
    }
}

/// Call(`f`, undefined, `args`) through the Rust entry `call_native`; `None` if `f` has no
/// native code.
#[inline(never)]
fn call_through_entry(
    iso: &mut Isolate,
    f: Value,
    args: &[Value],
) -> Option<Result<Value, Exception>> {
    let f = f.to_bits();
    let mut entered = false;
    let r = iso.with_cx(|acx| {
        let cx = acx.inner();
        let arg = |i: u32| args[i as usize].to_bits();
        let Some(r) = call_native(cx, f, UNDEFINED, args.len() as u32, arg) else {
            return Ok(UNDEFINED);
        };
        entered = true;
        r
    });
    entered.then(|| r.map(crate::runtime::val))
}

/// MC-w33-2.2 in a build without `jit`: the loader maps the unit text (the protect form; with
/// `jit` the window form), its unwind records are registered at run time, step 8 installs the
/// AotBaseline pick of `g` (from Rust: [`install_picked`]), `call_native` enters it, the stub
/// calls `walk` back through T0, and the system unwinder walks from `walk` through the stub
/// frame into the Rust caller. Without the registration the walk stops at the stub frame (the
/// `ep9_platform` self-test check); the `jit`-only form is `native_entry_registered_*`.
#[test]
fn unit_loader_registered_unit_text_runs_and_unwinds_without_jit() {
    let mut iso = defs_isolate(ExecOptions::jitless());
    let text = stub_text();
    let len = call0_bytes().0.len() as u32;
    let f = FunctionCode {
        function: G,
        digest: digest(G),
        variants: vec![variant(
            DispatchKind::AotBaseline,
            lowering::JITLESS,
            0,
            len,
        )],
    };
    let lu = info(&iso, "g").loaded_unit().unwrap();
    let r = unit_loader::load(&iso.rt, lu, &text, &[f], Some(&Policy::JITLESS), 0);
    assert_eq!(r.count(Reason::Picked), 1);
    let code = code_of(info(&iso, "g")).unwrap().code[0].clone();
    let range = CodeRange::process().unwrap();
    assert!(range.contains(code.start));
    if !cfg!(feature = "jit") {
        assert!(!range.is_map_jit());
    }
    // SAFETY: the text at `code` is the frame stub (it runs the bytecode of `g`: its call at
    // offset 0), mapped while the isolate lives.
    let installed = unsafe { install_picked(&iso.rt, lu) };
    assert_eq!(installed.count(Reason::Installed), 1);
    assert_eq!(
        info(&iso, "g").slot.kind_byte(),
        DispatchKind::AotBaseline as u8
    );
    let reg = register(code.clone());
    let g = iso.global_raw("g").unwrap();
    let w = iso.global_raw("walk").unwrap();
    WALKED.with(|x| x.borrow_mut().take());
    let v = call_through_entry(&mut iso, g, &[w]).unwrap().unwrap();
    assert_eq!(v.to_bits(), Value::int32(42).to_bits());
    let ips = WALKED.with(|x| x.borrow_mut().take()).expect("walk ran");
    let ret = code.start + call0_bytes().1;
    let at = ips.iter().position(|&ip| ip == ret).unwrap_or_else(|| {
        panic!("the walk misses the unit frame (return address {ret:#x}): {ips:x?}")
    });
    assert!(at >= 1, "the callee frames come first: {ips:x?}");
    assert!(
        ips.len() - at >= 4,
        "the walk stops at the unit frame: {ips:x?}"
    );
    assert!(!range.contains(ips[at + 1]), "{:#x}: {ips:x?}", ips[at + 1]);
    drop(reg);
    drop(iso);
}

/// Step 8 refuses what `install_code` refuses and what has no entry yet: an AotOpt pick and a
/// class constructor stay on T0.
#[test]
fn unit_loader_install_refuses_aot_opt_and_a_class_constructor() {
    let iso = defs_isolate(ExecOptions::jitless());
    let jl = lowering::JITLESS;
    let fns = [
        FunctionCode {
            function: ID,
            digest: digest(ID),
            variants: vec![variant(DispatchKind::AotOpt, jl, 0, 4)],
        },
        FunctionCode {
            function: K,
            digest: digest(K),
            variants: vec![variant(DispatchKind::AotBaseline, jl, 0, 4)],
        },
    ];
    let r = load(&iso, &Policy::JITLESS, &fns);
    assert_eq!(r.count(Reason::Picked), 2);
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: both picks are refused before any slot write (AotOpt; a class constructor).
    let r = unsafe { install_picked(&iso.rt, lu) };
    assert_eq!(
        (r.count(Reason::Installed), r.count(Reason::NotInstallable)),
        (0, 2)
    );
    for name in ["id", "K"] {
        assert_eq!(
            info(&iso, name).slot.kind_byte(),
            DispatchKind::Interp as u8
        );
    }
}

// ---- SHSTK-ENGINE ------------------------------------------------------------------------------

/// The ep9_vm build script includes the MC-w28-9.1 rule file of ep9_unit and refuses a flag
/// it names (the refusal itself is `crates/ep9_unit/tests/build_flags.rs`).
#[test]
fn unit_loader_build_script_runs_the_shadow_stack_refusal() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(dir.join("build.rs")).unwrap();
    assert!(text.contains("#[path = \"../ep9_unit/build_flags.rs\"]"));
    assert!(text.contains("build_flags::shadow_stack_opt_in(&flags)"));
    assert!(text.contains("panic!("));
    assert!(dir.join("../ep9_unit/build_flags.rs").exists());
}
