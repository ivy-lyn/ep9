//! Tests of the A2 unit loader (`unit_loader.rs`, M7-P4; plan v12 §7.3). They build and run in
//! every build: no `jit`, no `spike-native`. Every test name starts with `unit_loader_`.
//!
//! | Tests | What they show |
//! |---|---|
//! | `unit_loader_*_preset*` | MC-p0-cx-1: a unit variant with lowering `jit` is skipped under every preset (JIT, jitless, hybrid), before `select()`; `select()` alone would pick it under hybrid |
//! | `unit_loader_counts_*` | each skip reason, the digest binding (MC-w31-8), one load per function |
//! | `unit_loader_*_fuse*` | steps 6-7: a popped or unknown fuse makes the object a dependency that does not hold; MC-w35-5 (M8-P4): step 8 installs a pick whose fuse refs are GuardFuse reads |
//! | `unit_loader_api_*` | the Cx API: the isolate's policy (MC-w24-3 form), no install in M7-P4 (MC-w30-2), the bytecode runs; one intern table (MC-w25-9) |
//! | `unit_loader_registered_*` | MC-w33-2.2: unit text copied at run time (`ep9_platform::unit_text`, no `jit`), its unwind records registered by the loader (JIT review O3), installed by step 8 from Rust, entered by `call_native` and walked by `ep9_platform::unwind` |
//! | `unit_loader_installs_nothing_*`, `unit_loader_maps_*`, `unit_loader_a_text_larger_*` | O3: no unwind data, no install; A2-UNIT-SHARE and MC-w34-4: one text per process keyed by its bytes, mapped only on a pick, and a text that cannot fit gives "no-text" |
//! | `unit_loader_build_*` | SHSTK-ENGINE: the ep9_vm build script runs the shadow-stack refusal |
//! | `unit_loader_a_text_from_*`, `unit_loader_a_whole_*`, `unit_loader_a_full_*` | M7-P5 Codex A1: a shared text is synced on the thread that runs it; Codex A2 (child processes, `unit_loader_child_case`): the unwind mode is chosen before a text maps, and a probe that could not run is not cached |
//! | `unit_loader_a_constructor_*`, `unit_loader_construct_*`, `class_ctors::*` | M7-P5 JIT review F1 (M6-ORCH-386) form 2, then M8-P1 A2-CONSTRUCT (M8-PLAN-D25, D26): step 8 installs an ordinary function, and `new`, the Rust construct path (`Reflect.construct`), `super()`, a bound target and a construct of another realm enter its unit code with the results, receiver, errors and stacks of its bytecode; the caller counts a call or a construct of unit code (the vector at call 8). M8-P4 A2-GATE (MC-w29-8 step 2, MC-w33-6): a class constructor too (not with `spike-native`), and a call of one throws its TypeError |
//!
//! The JS-level checks (a unit from `ep9_unit::check`, M-R9 with and without a loaded unit)
//! are in `crates/ep9_unit/tests/loader.rs`: ep9_vm tests cannot compile JS.

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
    self, FunctionCode, Reason, Report, VariantCode, code_of, unit_function_digest,
};
use crate::{Args, Cx, Exception, ExecOptions, Isolate, IsolateConfig, JsResult, Local, Value};
use std::sync::Arc;

/// The source of the unit (positions for stack traces).
const SRC: &str = "var o = { g(f) { return f(); },\n\
id(x) { return x; } };\n\
class K {}\n\
function c(f) { return f(); }\n";

/// The unit: function 1 `g(f) { return f(); }` (the call at bytecode offset 0, the offset the
/// frame stub passes), function 2 `id(x)`, function 3 `K` (a class constructor), function 4
/// `c(f)` (an ordinary function: a constructor, with the bytecode of `g`). `g` and `id` are
/// methods: they have no `[[Construct]]`, so step 8 installs them (M6-ORCH-386: it installs no
/// constructor). Each is a global.
fn defs() -> VerifiedUnit {
    let p = |n: &str| pos(SRC, n);
    let method = |name: &str, params: u32, start: u32, end: u32| {
        let mut m = meta(name, params, 0, start, end);
        m.kind = FunctionKind::Method;
        Asm::new(m)
    };
    let mut g = method("g", 1, p("g(f)"), p("id(x)") - 1);
    let a0 = g.b.parameter(0);
    g.b.set_expression_position(p("f()"));
    g.b.call_undefined_receiver0(a0, g.call).return_();
    let mut id = method("id", 1, p("id(x)"), p("class K") - 1);
    let a0 = id.b.parameter(0);
    id.b.ldar(a0).return_();
    let mut m = meta("K", 0, 0, p("class K"), p("function c") - 1);
    m.kind = FunctionKind::ClassConstructor;
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
    let mut k = Asm::new(m);
    k.b.lda_undefined().return_();
    let mut c = Asm::new(meta("c", 1, 0, p("function c"), SRC.len() as u32));
    let a0 = c.b.parameter(0);
    c.b.set_expression_position(SRC.rfind("f()").unwrap() as u32);
    c.b.call_undefined_receiver0(a0, c.call).return_();
    let fns = vec![("g", g), ("id", id), ("K", k), ("c", c)];
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
const C: u32 = 4;

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
        (Reason::NoUnwind, 0),
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

/// One load hashes the unit's string table once, not once per listed function (M7-P4 fixer
/// r2, Codex r1 a2-loader:digest-per-function: 1,764 functions of Octane typescript hashed
/// 47,652 string bytes each, 500.9 ms per load). 3 listed functions, 1 pass; the 3 digests
/// still match `unit_function_digest`.
#[test]
fn unit_loader_counts_one_string_table_pass_per_load() {
    let iso = defs_isolate(ExecOptions::jitless());
    let fns: Vec<FunctionCode> = [G, ID, K]
        .into_iter()
        .map(|function| FunctionCode {
            function,
            digest: digest(function),
            variants: vec![variant(DispatchKind::AotBaseline, lowering::JITLESS, 0, 4)],
        })
        .collect();
    let before = unit_loader::string_table_passes();
    let r = load(&iso, &Policy::JITLESS, &fns);
    assert_eq!(unit_loader::string_table_passes() - before, 1);
    assert_eq!(r.count(Reason::Digest), 0, "{:?}", r.rows());
    assert_eq!(r.count(Reason::Picked), 3, "{:?}", r.rows());
}

// ---- steps 6-7: fuse refs --------------------------------------------------------------------

/// A variant that names a fuse: picked while the fuse is wired and intact; not picked after the
/// fuse pops, never for an unknown index. Since MC-w35-5 (M8-P4) step 8 installs the pick.
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
    // MC-w35-5 (M8-P4): a wired static fuse is a GuardFuse read (H31:138): step 8 installs it.
    let iso = defs_isolate(ExecOptions::jitless());
    iso.rt.fuses.wire(&iso.rt, 0).unwrap();
    let lu = info(&iso, "g").loaded_unit().unwrap();
    let mut f = with_fuse(0);
    f.variants[0].len = call0_bytes().0.len() as u32;
    // SAFETY: the text at offset 0 is the frame stub (it runs the bytecode of `g` and reads no
    // fuse), and `stub_unwind` describes it.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &stub_text(),
            &stub_unwind(),
            &[],
            0,
            &[f],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(
        (r.count(Reason::Installed), r.count(Reason::NotInstallable)),
        (1, 0)
    );
    assert_eq!(info(&iso, "g").slot.kind(), Some(DispatchKind::AotBaseline));
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

/// MC-w25-9 (H19 "one atom per text"): a unit loaded with its native part keeps no string
/// table of its own. Each unit string is the isolate's atom for its text, and a second load of
/// the same unit without unit code gets the same atoms.
#[test]
fn unit_loader_api_keeps_one_intern_table() {
    let mut iso = Isolate::new_with(IsolateConfig::default(), ExecOptions::jitless()).unwrap();
    let vars: &[Var<'_>] = &[(
        DispatchKind::AotBaseline as u8,
        lowering::JITLESS,
        0,
        8,
        0,
        &[],
    )];
    let fns = [(ID, digest(ID), vars)];
    let text = filler(64);
    iso.with_cx(|cx| {
        let (rows, r) = cx.run_script_with_unit_code(defs(), "defs.js", &text, 0, &fns);
        assert_eq!(rows.iter().find(|(k, _)| *k == "picked").unwrap().1, 1);
        r.map(|_| ())
    })
    .unwrap();
    iso.with_cx(|cx| cx.run_script(defs(), "again.js").map(|_| ()))
        .unwrap();
    let units = iso.rt.units.borrow();
    let (with, plain) = (&units[units.len() - 2], &units[units.len() - 1]);
    assert!(!with.unit.strings.is_empty());
    for (i, s) in with.unit.strings.iter().enumerate() {
        let atom = match s {
            ep9_bytecode::UnitString::Latin1(b) => crate::atoms::intern_latin1(&iso.rt, b),
            ep9_bytecode::UnitString::Utf16(u) => crate::atoms::intern_utf16(&iso.rt, u),
        };
        assert_eq!(with.string(i), atom, "string {i}");
        assert_eq!(plain.string(i), atom, "string {i}");
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
    stub_text_at(&[0])
}

/// The unit text with one frame stub at each offset of `starts` (in order, each below
/// 256 - the stub length; and on Windows their UNWIND_INFO).
fn stub_text_at(starts: &[usize]) -> Vec<u8> {
    let mut text = Vec::new();
    for &start in starts {
        text.resize(start, 0xCC);
        text.extend(call0_bytes().0);
    }
    #[cfg(windows)]
    {
        use ep9_platform::unwind::win64::{UnwindInfoBuilder, reg};
        let info = UnwindInfoBuilder::new().push_nonvol(1, reg::RBP).encode();
        text.resize(UNWIND_INFO_OFF, 0xCC);
        text.extend(info);
    }
    text
}

/// The unwind data of [`stub_text`] in the loader's form 1 (`unit_loader.rs` module doc): on
/// unix an `.eh_frame` image whose FDE `pc_begin` is the text offset 0, on Windows one
/// RUNTIME_FUNCTION (offsets into the text). The loader registers it (JIT review O3).
fn stub_unwind() -> Vec<u8> {
    stub_unwind_at(&[0])
}

/// The unwind data of [`stub_text_at`]: one FDE (on Windows one RUNTIME_FUNCTION) for the
/// frame stub at each offset of `starts`.
fn stub_unwind_at(starts: &[usize]) -> Vec<u8> {
    let (code, ret) = call0_bytes();
    let len = code.len();
    #[cfg(unix)]
    {
        use ep9_platform::unwind::cfi::{CfiProgram, Cie, EhFrameBuilder, reg};
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
        for &start in starts {
            b.add_fde(start, len, &p);
        }
        b.finish().bytes().to_vec()
    }
    #[cfg(windows)]
    {
        let _ = ret;
        let mut v = Vec::new();
        for &start in starts {
            let start = start as u32;
            for w in [start, start + len as u32, UNWIND_INFO_OFF as u32] {
                v.extend(w.to_le_bytes());
            }
        }
        v
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

/// The AotBaseline jitless variant of `g` over the frame stub at text offset 0.
fn g_stub() -> FunctionCode {
    let len = call0_bytes().0.len() as u32;
    FunctionCode {
        function: G,
        digest: digest(G),
        variants: vec![variant(
            DispatchKind::AotBaseline,
            lowering::JITLESS,
            0,
            len,
        )],
    }
}

/// MC-w33-2.2 in a build without `jit`, and JIT review O3 (M7-P5 A2-RW): an installing load
/// maps the unit text (the protect form; with `jit` the window form), registers its unwind
/// data before it installs (no hand registration here: the loader does it), installs the
/// AotBaseline pick of `g`; `call_native` enters it, the stub calls `walk` back through T0,
/// and the system unwinder walks from `walk` through the stub frame into the Rust caller.
/// Without the registration the walk stops at the stub frame (the `ep9_platform` self-test
/// check); the `jit`-only form is `native_entry_registered_*`.
#[test]
fn unit_loader_registered_unit_text_runs_and_unwinds_without_jit() {
    let mut iso = defs_isolate(ExecOptions::jitless());
    let text = stub_text();
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: the text at offset 0 is the frame stub (it runs the bytecode of `g`: its call at
    // offset 0), and `stub_unwind` describes it.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &text,
            &stub_unwind(),
            &[],
            0,
            &[g_stub()],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(
        (r.count(Reason::Picked), r.count(Reason::Installed)),
        (1, 1),
        "{:?}",
        r.rows()
    );
    let code = code_of(info(&iso, "g")).unwrap().code_at(0).unwrap();
    let range = CodeRange::process().unwrap();
    assert!(range.contains(code.start));
    if !cfg!(feature = "jit") {
        assert!(!range.is_map_jit());
    }
    assert_eq!(
        info(&iso, "g").slot.kind_byte(),
        DispatchKind::AotBaseline as u8
    );
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
    drop(iso);
}

/// JIT review O3: no unwind data, or unwind data that leaves the text, installs nothing
/// ("no-unwind"): the function keeps its Interp slot and its bytecode runs.
#[test]
fn unit_loader_installs_nothing_without_unwind_data() {
    let text = stub_text();
    let mut outside = stub_unwind();
    #[cfg(unix)]
    {
        // The FDE's pc_range (at its offset 16; the CIE is the first record) past the text.
        let fde = 4 + u32::from_le_bytes(outside[0..4].try_into().unwrap()) as usize;
        outside[fde + 16..fde + 24].copy_from_slice(&(text.len() as u64 + 1).to_le_bytes());
    }
    #[cfg(windows)]
    outside[4..8].copy_from_slice(&(text.len() as u32 + 1).to_le_bytes());
    for (what, unwind) in [("empty", Vec::new()), ("outside", outside)] {
        let iso = defs_isolate(ExecOptions::jitless());
        let lu = info(&iso, "g").loaded_unit().unwrap();
        // SAFETY: as in the test above; nothing is installed.
        let r = unsafe {
            unit_loader::load_and_install(
                &iso.rt,
                lu,
                &text,
                &unwind,
                &[],
                0,
                &[g_stub()],
                Some(&Policy::JITLESS),
                0,
            )
        };
        assert_eq!(
            (r.count(Reason::NoUnwind), r.count(Reason::Installed)),
            (1, 0),
            "{what}: {:?}",
            r.rows()
        );
        assert_eq!(info(&iso, "g").slot.kind_byte(), DispatchKind::Interp as u8);
        assert!(code_of(info(&iso, "g")).unwrap().text.is_none(), "{what}");
    }
}

/// A2-UNIT-SHARE and MC-w34-4 (M7-P5 A2-RW): two isolates that load one text with one unwind
/// data share one mapping (one text per process, keyed by its bytes); the text goes back when
/// the last isolate drops; a load whose policy picks no unit object (the JIT policy) maps
/// nothing; the safe load maps nothing either.
#[test]
fn unit_loader_maps_one_text_per_process_and_only_on_a_pick() {
    // A text of this test alone (other tests map the plain stub text at the same time, and
    // share it: same bytes, same key).
    let mut text = stub_text();
    text.extend(b"maps-one-text");
    let unwind = stub_unwind();
    let load_in = |iso: &Isolate, policy: &Policy| {
        let lu = info(iso, "g").loaded_unit().unwrap();
        // SAFETY: as in `unit_loader_registered_unit_text_runs_and_unwinds_without_jit`; the
        // code does not run here.
        unsafe {
            unit_loader::load_and_install(
                &iso.rt,
                lu,
                &text,
                &unwind,
                &[],
                0,
                &[g_stub()],
                Some(policy),
                0,
            )
        }
    };
    let (a, b) = (
        defs_isolate(ExecOptions::jitless()),
        defs_isolate(ExecOptions::jitless()),
    );
    for iso in [&a, &b] {
        assert_eq!(load_in(iso, &Policy::JITLESS).count(Reason::Installed), 1);
    }
    let ta = code_of(info(&a, "g")).unwrap().text.clone().unwrap();
    let tb = code_of(info(&b, "g")).unwrap().text.clone().unwrap();
    assert!(Arc::ptr_eq(&ta, &tb), "one text for both isolates");
    let w = Arc::downgrade(&ta);
    drop((ta, tb));
    drop(a);
    assert!(w.upgrade().is_some(), "b still holds the text");
    drop(b);
    assert!(
        w.upgrade().is_none(),
        "the text went back with the last isolate"
    );
    // The JIT policy accepts no unit object: no pick, no map.
    let c = defs_isolate(ExecOptions::jitless());
    let r = load_in(&c, &Policy::JIT);
    assert_eq!((r.count(Reason::Picked), r.count(Reason::NoText)), (0, 0));
    assert!(code_of(info(&c, "g")).unwrap().text.is_none());
    // The safe load maps nothing.
    let d = defs_isolate(ExecOptions::jitless());
    let lu = info(&d, "g").loaded_unit().unwrap();
    let r = unit_loader::load(&d.rt, lu, &text, &[g_stub()], Some(&Policy::JITLESS), 0);
    assert_eq!((r.count(Reason::Picked), r.count(Reason::Deferred)), (1, 1));
    assert!(code_of(info(&d, "g")).unwrap().text.is_none());
}

/// MC-w34-4, the hybrid case: a unit text larger than the whole process code range (so it
/// would leave the JIT no room) gives "no-text" before its bytes are hashed, and the function
/// runs its bytecode.
#[test]
fn unit_loader_a_text_larger_than_the_code_range_gives_no_text() {
    let iso = defs_isolate(ExecOptions::jitless());
    let size = CodeRange::process().unwrap().size();
    let mut text = stub_text();
    text.resize(size + 1, 0);
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: nothing is mapped or installed: the text does not fit.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &text,
            &stub_unwind(),
            &[],
            0,
            &[g_stub()],
            Some(&Policy::HYBRID),
            0,
        )
    };
    assert_eq!(
        (r.count(Reason::Picked), r.count(Reason::NoText)),
        (1, 1),
        "{:?}",
        r.rows()
    );
    assert_eq!(info(&iso, "g").slot.kind_byte(), DispatchKind::Interp as u8);
}

// ---- MC-w30-2 and MC-w34-3: JS calls into installed unit code ----------------------------------

/// A host function in the form of `Function.prototype.call` (IsCallable, then the VM steps;
/// the `tests/calls.rs` form).
fn fcall<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    if !this.is_callable() {
        return Err(cx.throw_type_error("fcall: not callable"));
    }
    cx.forward_call(args)
}

/// A host function in the form of `Function.prototype.apply`.
fn fapply<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    if !this.is_callable() {
        return Err(cx.throw_type_error("fapply: not callable"));
    }
    cx.forward_apply(args)
}

/// `fcall` and `fapply` as builtin table definitions, installed as globals: a forwarder
/// records only a function made from a table definition (M7-P0 JIT reviewer F1).
static FORWARDERS: crate::BuiltinTable = crate::BuiltinTable::new(
    "unit-loader-forwarders",
    &[crate::BuiltinModule {
        name: "forwarders",
        block: 106,
        defs: &[
            crate::BuiltinDef::method("fcall", 1, fcall),
            crate::BuiltinDef::method("fapply", 2, fapply),
        ],
        intrinsics: &[],
        props: &[crate::ObjectProps {
            target: crate::IntrinsicId::GlobalObject,
            props: &[
                crate::PropDef::method(crate::PropKey::Str("fcall"), 0),
                crate::PropDef::method(crate::PropKey::Str("fapply"), 1),
            ],
        }],
    }],
);

/// A jitless isolate with the forwarders and `walk` that ran `defs()`, with the frame stub
/// installed for `g` (`g(f) { return f(); }`: the stub runs the call at bytecode offset 0).
fn installed_g() -> Isolate {
    let config = IsolateConfig {
        builtins: Some(&FORWARDERS),
        ..IsolateConfig::default()
    };
    let mut iso = Isolate::new_with(config, ExecOptions::jitless()).unwrap();
    iso.register_host_function("walk", 0, walk);
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
        .unwrap();
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: the text at offset 0 is the frame stub (it runs the bytecode of `g`), and
    // `stub_unwind` describes it.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &stub_text(),
            &stub_unwind(),
            &[],
            0,
            &[g_stub()],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    assert_eq!(
        info(&iso, "g").slot.kind_byte(),
        DispatchKind::AotBaseline as u8
    );
    iso
}

/// Runs the script `u` in `iso`; its completion value as Value bits, or the error text.
fn run_js(iso: &mut Isolate, u: VerifiedUnit) -> Result<u64, String> {
    iso.with_cx_checkpoint(|cx| cx.run_script(u, "calls.js").map(|v| v.bits()))
        .map_err(|e| format!("{e:?}"))
}

/// `return g(walk);` (CallUndefinedReceiver1: T0's `call_fast` sends the native slot cold).
fn call_g_walk() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let a = s.a.get(0);
    s.lda_global("walk");
    s.b.star(a);
    s.lda_global("g");
    s.b.star(s.f);
    let (f, slot) = (s.f, s.call);
    s.b.call_undefined_receiver1(f, a, slot).return_();
    unit("", vec![s.finish()])
}

/// `return fcall.call(g, undefined, walk);` in the call form of the forwarder: `fcall` with
/// `this` = g (the list shape: receiver first).
fn fcall_g_walk() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let l = s.b.new_register_list(3);
    s.lda_global("g");
    s.b.star(l.get(0)).lda_undefined().star(l.get(1));
    s.lda_global("walk");
    s.b.star(l.get(2));
    s.lda_global("fcall");
    s.b.star(s.f);
    let (f, slot) = (s.f, s.call);
    s.b.call_property(f, l, 3, slot).return_();
    unit("", vec![s.finish()])
}

/// `function viaArgs(f) { return g.fapply(undefined, arguments); } return viaArgs(walk);`
/// (`fapply` with `this` = g and an unmapped arguments object: the forward's list form).
fn fapply_g_walk() -> VerifiedUnit {
    let mut via = Asm::new(meta("viaArgs", 1, 0, 0, 0));
    let l = via.b.new_register_list(3);
    via.lda_global("g");
    via.b.star(l.get(0)).lda_undefined().star(l.get(1));
    via.b.create_unmapped_arguments().star(l.get(2));
    via.lda_global("fapply");
    via.b.star(via.f);
    let (f, slot) = (via.f, via.call);
    via.b
        .call_property2(f, l.get(0), l.get(1), l.get(2), slot)
        .return_();
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.b.star(s.f);
    let a = s.a.get(0);
    s.lda_global("walk");
    s.b.star(a);
    let (f, slot) = (s.f, s.call);
    s.b.call_undefined_receiver1(f, a, slot).return_();
    unit("", vec![s.finish(), via.finish()])
}

/// MC-w30-2 (call first): a JS call of a function with installed unit code runs that code.
/// T0's `call_fast` sends the native slot cold, `slow::call` enters it (nested, its frame
/// above the caller's registers), the stub calls `walk` back through T0, and the script
/// gets 42. A call from Rust (`Cx::call`) enters it through the Rust entry. On the old code
/// both throw "not implemented".
#[test]
fn unit_loader_a_js_call_runs_the_installed_unit_code() {
    let mut iso = installed_g();
    assert_eq!(
        run_js(&mut iso, call_g_walk()),
        Ok(Value::int32(42).to_bits())
    );
    let (g, w) = (
        iso.global_raw("g").unwrap(),
        iso.global_raw("walk").unwrap(),
    );
    let v = iso
        .with_cx(|cx| {
            let f: Local = Local::from_bits(g.to_bits());
            let a: Local = Local::from_bits(w.to_bits());
            let u: Local = Local::from_bits(UNDEFINED);
            cx.call(f, u, &[a]).map(|v| v.bits())
        })
        .unwrap();
    assert_eq!(v, Value::int32(42).to_bits());
}

/// MC-w34-3 (DL P5-DL-5): a forwarded `call` and `apply` into an installed unit function give
/// node's result (42). The T0 forward takes only an Interp callee (`call::forward_target`), so
/// the forwarder runs its steps and reaches the unit code through the Rust entry (correct;
/// the forward's speed gain is not there for unit code until the T1 lane, MC-w30-9).
#[test]
fn unit_loader_a_forwarded_call_or_apply_runs_the_installed_unit_code() {
    for (what, u) in [("call", fcall_g_walk()), ("apply", fapply_g_walk())] {
        let mut iso = installed_g();
        assert_eq!(
            run_js(&mut iso, u),
            Ok(Value::int32(42).to_bits()),
            "{what}"
        );
    }
}

/// MC-w34-2 (DL P5-DL-5): the safe `Cx::run_script_with_unit_code` never installs, also for
/// a text and a function table that would install (each pick is "deferred", the slot stays
/// Interp, the JS call runs the bytecode); the unsafe `Cx::run_script_installing_unit_code`
/// with the same unit installs the pick, and the JS call runs the unit code. Both give 42
/// here because the stub runs the bytecode of `g`.
#[test]
fn unit_loader_api_only_the_unsafe_method_installs() {
    let len = call0_bytes().0.len() as u32;
    let vars: &[Var<'_>] = &[(
        DispatchKind::AotBaseline as u8,
        lowering::JITLESS,
        0,
        len,
        0,
        &[],
    )];
    let fns = [(G, digest(G), vars)];
    let (text, unwind) = (stub_text(), stub_unwind());
    for install in [false, true] {
        let mut iso = Isolate::new_with(IsolateConfig::default(), ExecOptions::jitless()).unwrap();
        iso.register_host_function("walk", 0, walk);
        let rows = iso
            .with_cx(|cx| {
                let (rows, r) = if install {
                    // SAFETY: `defs()` with the frame stub and its unwind data stands for a unit
                    // that `ep9_unit::check` accepted (the stub runs the bytecode of `g`).
                    unsafe {
                        cx.run_script_installing_unit_code(
                            defs(),
                            "defs.js",
                            &text,
                            &unwind,
                            &[],
                            0,
                            0,
                            &fns,
                        )
                    }
                } else {
                    cx.run_script_with_unit_code(defs(), "defs.js", &text, 0, &fns)
                };
                r.map(|_| rows)
            })
            .unwrap();
        let get = |n: &str| rows.iter().find(|(k, _)| *k == n).unwrap().1;
        assert_eq!(
            (get("picked"), get("deferred"), get("installed")),
            (1, u32::from(!install), u32::from(install)),
            "install {install}: {rows:?}"
        );
        let kind = if install {
            DispatchKind::AotBaseline
        } else {
            DispatchKind::Interp
        };
        assert_eq!(info(&iso, "g").slot.kind_byte(), kind as u8);
        assert_eq!(
            run_js(&mut iso, call_g_walk()),
            Ok(Value::int32(42).to_bits())
        );
    }
}

/// The unsafe method stores the isolate's unit-data directory in `Cx::unit_data` when the
/// load made an RW copy, and unit code then reads its link cell through it.
#[test]
fn unit_loader_api_the_unsafe_method_sets_the_unit_data_directory() {
    use crate::builtins::IntrinsicId;
    let (text, unwind) = reader(3, 0);
    let tpl = template(
        &[(
            unit_loader::rw::LINK_INTRINSIC,
            IntrinsicId::ObjectPrototype as u32,
        )],
        1,
        0,
    );
    let vars: &[Var<'_>] = &[(
        DispatchKind::AotBaseline as u8,
        lowering::JITLESS,
        0,
        reader_len() as u32,
        0,
        &[],
    )];
    let fns = [(ID, digest(ID), vars)];
    let mut iso = Isolate::new_with(IsolateConfig::default(), ExecOptions::jitless()).unwrap();
    assert_eq!(iso.cx.unit_data, 0);
    iso.with_cx(|cx| {
        // SAFETY: as in the test above; the text is the cell reader.
        let (rows, r) = unsafe {
            cx.run_script_installing_unit_code(defs(), "defs.js", &text, &unwind, &tpl, 3, 0, &fns)
        };
        assert_eq!(rows.iter().find(|(k, _)| *k == "installed").unwrap().1, 1);
        r.map(|_| ())
    })
    .unwrap();
    assert_ne!(iso.cx.unit_data, 0);
    let id = iso.global_raw("id").unwrap();
    let v = call_through_entry(&mut iso, id, &[]).unwrap().unwrap();
    assert_eq!(
        v.to_bits(),
        crate::builtins::intrinsic(&iso.rt, IntrinsicId::ObjectPrototype)
    );
}

// ---- the RW copy and the link table (plan v12 §8.2; H35 S4 step 6) ---------------------------

/// An RW template in form 1 (`unit_loader::rw`): `links` as (kind, index), then `ics` IC
/// cells and `counters` counters.
fn template(links: &[(u8, u32)], ics: u32, counters: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(unit_loader::rw::MAGIC);
    for w in [
        unit_loader::rw::VERSION,
        links.len() as u32,
        ics,
        counters,
        0,
    ] {
        b.extend(w.to_le_bytes());
    }
    for &(kind, index) in links {
        b.extend([kind, 0, 0, 0]);
        b.extend(index.to_le_bytes());
    }
    b
}

/// The cell reader `return [[[cx + CX_UNIT_DATA] + 8 * slot] + 8 * cell]` (the `NativeEntry`
/// ABI, a leaf: the H35-Q9 unit-data load without the x27 save) and its unwind data in the
/// loader's form 1.
fn reader(slot: usize, cell: usize) -> (Vec<u8>, Vec<u8>) {
    #[cfg(target_arch = "aarch64")]
    let code: Vec<u8> = {
        let ldr = |t: u32, n: u32, off: usize| {
            assert!(off.is_multiple_of(8) && off / 8 < 4096);
            0xF940_0000 | (((off / 8) as u32) << 10) | (n << 5) | t
        };
        [
            ldr(9, 0, layout::CX_UNIT_DATA),
            ldr(9, 9, 8 * slot),
            ldr(0, 9, 8 * cell),
            0xD65F_03C0, // ret
        ]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect()
    };
    #[cfg(target_arch = "x86_64")]
    let code: Vec<u8> = {
        let mut v = vec![0x48, 0x8B, 0x87]; // mov rax, [rdi + disp32]
        v.extend((layout::CX_UNIT_DATA as u32).to_le_bytes());
        for off in [8 * slot, 8 * cell] {
            v.extend([0x48, 0x8B, 0x80]); // mov rax, [rax + disp32]
            v.extend((off as u32).to_le_bytes());
        }
        v.push(0xC3); // ret
        v
    };
    let len = code.len();
    #[allow(unused_mut)]
    let mut text = code;
    #[cfg(unix)]
    let unwind = {
        use ep9_platform::unwind::cfi::{CfiProgram, Cie, EhFrameBuilder};
        // A leaf: the CIE rules hold at every instruction.
        let mut b = EhFrameBuilder::new(&Cie::host());
        b.add_fde(0, len, &CfiProgram::new());
        b.finish().bytes().to_vec()
    };
    #[cfg(windows)]
    let unwind = {
        // UNWIND_INFO version 1 with no codes (a leaf), after the code.
        text.resize(UNWIND_INFO_OFF, 0xCC);
        text.extend([1, 0, 0, 0]);
        let mut v = Vec::new();
        for w in [0, len as u32, UNWIND_INFO_OFF as u32] {
            v.extend(w.to_le_bytes());
        }
        v
    };
    (text, unwind)
}

/// The AotBaseline jitless variant of `id` over `len` bytes at text offset 0.
fn id_at0(len: usize) -> FunctionCode {
    FunctionCode {
        function: ID,
        digest: digest(ID),
        variants: vec![variant(
            DispatchKind::AotBaseline,
            lowering::JITLESS,
            0,
            len as u32,
        )],
    }
}

/// The length of the reader's code (the text may be longer on Windows).
fn reader_len() -> usize {
    if cfg!(target_arch = "aarch64") {
        16
    } else {
        22
    }
}

/// RW cells and the link table (plan v12 §8.2): an installing load makes the isolate's RW
/// copy in unit-data slot 5: link cell 0 = the realm's %Object.prototype%, link cell 1 = the
/// atom of unit string 0, 2 IC cells (undefined), 1 counter (0). The directory word of slot 5
/// names the copy, `Cx::unit_data` the directory, and unit code reads link cell 1 through
/// them (the H35-Q9 load). IC cells are weak: after a GC an IC cell whose object nothing else
/// holds is undefined, and one whose object lives keeps it; the link cells keep theirs.
#[test]
fn unit_loader_rw_copy_links_and_unit_code_reads_it() {
    use crate::builtins::IntrinsicId;
    let mut iso = defs_isolate(ExecOptions::jitless());
    let (text, unwind) = reader(5, 1);
    let tpl = template(
        &[
            (
                unit_loader::rw::LINK_INTRINSIC,
                IntrinsicId::ObjectPrototype as u32,
            ),
            (unit_loader::rw::LINK_ATOM, 0),
        ],
        2,
        1,
    );
    let lu = info(&iso, "id").loaded_unit().unwrap();
    // SAFETY: the text at offset 0 is the cell reader (`NativeEntry`), and `unwind` describes it.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &text,
            &unwind,
            &tpl,
            5,
            &[id_at0(reader_len())],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    let atom = lu.string(0);
    let dir = r.unit_data.expect("a directory");
    iso.cx.unit_data = dir;
    let rw = code_of(info(&iso, "id")).unwrap().rw.clone().unwrap();
    let object_prototype = crate::builtins::intrinsic(&iso.rt, IntrinsicId::ObjectPrototype);
    assert_eq!(
        (0..5).map(|i| rw.word(i)).collect::<Vec<_>>(),
        [object_prototype, atom, UNDEFINED, UNDEFINED, 0]
    );
    let (slot, d) = rw.slot();
    assert_eq!((slot, d.address(), d.word(5)), (5, dir, rw.base()));
    // Unit code reads link cell 1 through x28 (the Cx), CX_UNIT_DATA, slot 5.
    let id = iso.global_raw("id").unwrap();
    let v = call_through_entry(&mut iso, id, &[]).unwrap().unwrap();
    assert_eq!(v.to_bits(), atom);
    iso.collect_garbage();
    assert_eq!((rw.word(0), rw.word(1)), (object_prototype, atom));
}

/// Masks the bits of a cell pointer, so the conservative stack scan cannot keep it.
const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Bytes of zeros between the test's frame and the frame that makes the cell (the pattern of
/// `tests/async_gen.rs`; each file has its own copy).
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    std::hint::black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> R {
    let mut pad = [0u8; DEEP];
    std::hint::black_box(&mut pad);
    let r = made();
    std::hint::black_box(&mut pad);
    r
}

/// IC cells are a weak root range (H35 S5; MC-w33-8.2: a unit IC cell holds its keys weakly):
/// after a GC an IC cell whose object nothing else holds is undefined, and an IC cell whose
/// object lives keeps it. The object is made below a pad and its bits are masked, and the
/// test checks that the conservative stack scan did not keep it.
#[test]
fn unit_loader_rw_ic_cells_are_weak() {
    crate::tests::on_big_stack(|| {
        let mut config = IsolateConfig::default();
        config.heap.record_conservative_roots = true;
        let mut iso = Isolate::new_with(config, ExecOptions::jitless()).unwrap();
        iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
            .unwrap();
        let (text, unwind) = reader(5, 0);
        let lu = info(&iso, "id").loaded_unit().unwrap();
        // SAFETY: the reader is installed but not run here.
        let r = unsafe {
            unit_loader::load_and_install(
                &iso.rt,
                lu,
                &text,
                &unwind,
                &template(&[], 2, 0),
                5,
                &[id_at0(reader_len())],
                Some(&Policy::JITLESS),
                0,
            )
        };
        assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
        let rw = code_of(info(&iso, "id")).unwrap().rw.clone().unwrap();
        let rt = &iso.rt;
        let masked = below_a_pad(|| {
            let o = crate::object::new_plain_object(rt);
            rw.set_ic(0, o);
            std::hint::black_box(o) ^ MASK
        });
        let kept = crate::builtins::intrinsic(rt, crate::builtins::IntrinsicId::ObjectPrototype);
        rw.set_ic(1, kept);
        scrub_stack();
        rt.heap.collect();
        let addr = crate::object::cell(masked ^ MASK).addr();
        assert!(
            !rt.heap
                .last_conservative_roots()
                .iter()
                .any(|c| c.addr() == addr),
            "the conservative scan kept the object"
        );
        assert_eq!(
            (rw.word(0), rw.word(1)),
            (UNDEFINED, kept),
            "a dead IC target is undefined; a live one stays"
        );
    });
}

/// MC-w32-8: the loader checks every link-table descriptor before any use: a template that is
/// not form 1, a kind outside the allow-list (0; 3, the fuse kind, is not built), an index out
/// of bounds for its kind, a reserved byte, a cell count over the bound; and a slot of 64 or
/// more, or one held in the isolate. Each gives its reason, maps nothing and installs nothing.
#[test]
fn unit_loader_rw_descriptor_checks_come_before_any_use() {
    use crate::builtins::IntrinsicId;
    use unit_loader::rw::{LINK_ATOM, LINK_INTRINSIC, MAX_CELLS};
    let strings = defs().strings.len() as u32;
    let all = IntrinsicId::ALL.len() as u32;
    let mut bad: Vec<(&str, Vec<u8>)> = vec![
        ("magic", {
            let mut b = template(&[], 0, 0);
            b[0] = b'X';
            b
        }),
        ("version", {
            let mut b = template(&[], 0, 0);
            b[4] = 2;
            b
        }),
        ("reserved", {
            let mut b = template(&[], 0, 0);
            b[20] = 1;
            b
        }),
        ("length", {
            let mut b = template(&[(LINK_ATOM, 0)], 0, 0);
            b.pop();
            b
        }),
        ("kind 0", template(&[(0, 0)], 0, 0)),
        ("kind 3", template(&[(3, 0)], 0, 0)),
        (
            "intrinsic past the list",
            template(&[(LINK_INTRINSIC, all)], 0, 0),
        ),
        (
            "RealmFuses",
            template(&[(LINK_INTRINSIC, IntrinsicId::RealmFuses as u32)], 0, 0),
        ),
        (
            "atom past the strings",
            template(&[(LINK_ATOM, strings)], 0, 0),
        ),
        ("descriptor reserved byte", {
            let mut b = template(&[(LINK_ATOM, 0)], 0, 0);
            b[25] = 1;
            b
        }),
        ("cells", template(&[], MAX_CELLS as u32, 1)),
    ];
    let (text, unwind) = reader(5, 0);
    let run = |iso: &Isolate, tpl: &[u8], slot: u8| {
        let lu = info(iso, "id").loaded_unit().unwrap();
        // SAFETY: nothing is installed in the refused cases; an accepted case installs the
        // reader, which is not run here.
        unsafe {
            unit_loader::load_and_install(
                &iso.rt,
                lu,
                &text,
                &unwind,
                tpl,
                slot,
                &[id_at0(reader_len())],
                Some(&Policy::JITLESS),
                0,
            )
        }
    };
    for (what, tpl) in bad.drain(..) {
        let iso = defs_isolate(ExecOptions::jitless());
        let r = run(&iso, &tpl, 5);
        assert_eq!(
            (r.count(Reason::Link), r.count(Reason::Installed)),
            (1, 0),
            "{what}: {:?}",
            r.rows()
        );
        let uc = code_of(info(&iso, "id")).unwrap();
        assert!(uc.text.is_none() && uc.rw.is_none(), "{what}");
    }
    // Slot 64, then a slot that another unit of the isolate holds.
    let iso = defs_isolate(ExecOptions::jitless());
    let r = run(&iso, &template(&[], 1, 0), 64);
    assert_eq!((r.count(Reason::Slot), r.count(Reason::Installed)), (1, 0));
    let mut iso = defs_isolate(ExecOptions::jitless());
    assert_eq!(
        run(&iso, &template(&[], 1, 0), 7).count(Reason::Installed),
        1
    );
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "again.js").map(|_| ()))
        .unwrap();
    let r = run(&iso, &template(&[], 1, 0), 7);
    assert_eq!((r.count(Reason::Slot), r.count(Reason::Installed)), (1, 0));
    // An empty template makes no copy and keeps the directory.
    let iso = defs_isolate(ExecOptions::jitless());
    let r = run(&iso, &[], 9);
    assert_eq!((r.count(Reason::Installed), r.unit_data), (1, None));
}

/// MC-w30-3-CODE (H20:70; Codex r1 LAZYNS 2): a link cell of a lazy group's intrinsic installs
/// the group first, so the cell holds the object that a later JS read gets.
#[test]
fn unit_loader_a_lazy_intrinsic_link_installs_its_group_first() {
    use crate::builtins::{
        BuiltinModule, BuiltinTable, IntrinsicDef, IntrinsicId, IntrinsicValue, ObjectProps,
        PropAttrs, PropDef, PropKey,
    };
    // One lazy group: the key `Temporal` with its intrinsic (INTL-I1-D8).
    static LAZY: BuiltinTable = BuiltinTable::new(
        "lazy-link",
        &[BuiltinModule {
            name: "temporal-key",
            block: 105,
            defs: &[],
            intrinsics: &[IntrinsicDef {
                id: IntrinsicId::Temporal,
                value: IntrinsicValue::Object {
                    proto: Some(IntrinsicId::ObjectPrototype),
                },
            }],
            props: &[ObjectProps {
                target: IntrinsicId::GlobalObject,
                props: &[PropDef::intrinsic(
                    PropKey::Str("Temporal"),
                    IntrinsicId::Temporal,
                    PropAttrs::METHOD,
                )],
            }],
        }],
    );
    let config = IsolateConfig {
        builtins: Some(&LAZY),
        ..IsolateConfig::default()
    };
    let mut iso = Isolate::new_with(config, ExecOptions::jitless()).unwrap();
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
        .unwrap();
    let rt = &iso.rt;
    assert_eq!(
        crate::builtins::intrinsic(rt, IntrinsicId::Temporal),
        UNDEFINED
    );
    assert!(crate::builtins::lazy_group(rt, IntrinsicId::Temporal).is_some());
    let (text, unwind) = reader(5, 0);
    let tpl = template(
        &[(
            unit_loader::rw::LINK_INTRINSIC,
            IntrinsicId::Temporal as u32,
        )],
        0,
        0,
    );
    let lu = info(&iso, "id").loaded_unit().unwrap();
    // SAFETY: the reader is installed but not run here.
    let r = unsafe {
        unit_loader::load_and_install(
            rt,
            lu,
            &text,
            &unwind,
            &tpl,
            5,
            &[id_at0(reader_len())],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    let v = code_of(info(&iso, "id"))
        .unwrap()
        .rw
        .as_ref()
        .unwrap()
        .word(0);
    assert!(crate::object::is_object(v));
    assert_eq!(v, crate::builtins::intrinsic(rt, IntrinsicId::Temporal));
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
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: the AotOpt pick is refused before any slot write; `K` (a class constructor) is
    // installed over the frame stub only where `installs_class_code` holds, and never runs.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &stub_text(),
            &stub_unwind(),
            &[],
            0,
            &fns,
            Some(&Policy::JITLESS),
            0,
        )
    };
    // MC-w29-8 step 2 (M8-P4 A2-GATE): the class-constructor half flips on purpose: step 8
    // installs `K` as AotBaseline unit code, except in a `spike-native` build, whose T0
    // `call_fast` has no class-constructor check for an AotBaseline slot. AotOpt: never.
    let class_code = crate::native::installs_class_code(DispatchKind::AotBaseline);
    assert_eq!(class_code, !cfg!(feature = "spike-native"));
    assert_eq!(r.count(Reason::Picked), 2);
    let want = if class_code { (1, 1) } else { (0, 2) };
    assert_eq!(
        (r.count(Reason::Installed), r.count(Reason::NotInstallable)),
        want
    );
    let k = if class_code {
        DispatchKind::AotBaseline
    } else {
        DispatchKind::Interp
    };
    assert_eq!(info(&iso, "id").slot.kind(), Some(DispatchKind::Interp));
    assert_eq!(info(&iso, "K").slot.kind(), Some(k));
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

/// MC-w24-3 (M7-P4 integration fixer): the policy of an isolate is the private `ExecOptions`
/// field that the preset constructors set, and the loader reads that field
/// (`unit_loader::isolate_policy`), not a preset table of its own. The feedback setter
/// keeps it, and each isolate gives the loader the policy of its options.
#[test]
fn unit_loader_api_policy_is_the_exec_options_field() {
    use crate::Feedback;
    assert_eq!(ExecOptions::jit().policy(), Some(Policy::JIT));
    assert_eq!(ExecOptions::jitless().policy(), Some(Policy::JITLESS));
    assert_eq!(
        ExecOptions::jit().feedback(Feedback::ICS_ONLY).policy(),
        Some(Policy::JIT)
    );
    assert_eq!(
        ExecOptions::jitless().feedback(Feedback::FULL).policy(),
        Some(Policy::JITLESS)
    );
    let mut presets = vec![ExecOptions::jitless()];
    if cfg!(feature = "jit") {
        presets.push(ExecOptions::jit());
    }
    for exec in presets {
        let iso = Isolate::new_with(IsolateConfig::default(), exec).unwrap();
        assert_eq!(
            unit_loader::isolate_policy(&iso.rt),
            exec.policy(),
            "{}",
            exec.preset().name()
        );
    }
}

// ---- Codex A1 and A2 (M7-P5 fixer r3): a shared text on another thread; the unwind mode ------

/// Codex A1: thread B takes the text that thread A mapped (the hit path of the process text
/// map: the same entry address, while A's isolate keeps the text) and runs it. Each return of
/// `shared_text`, hit or miss, runs `sync_text` on the thread that will run the code (an
/// isolate is not `Send`). The barrier itself cannot fail a test: the test shows that the
/// hit path calls it (the counter of this thread) and that the code then runs.
#[test]
fn unit_loader_a_text_from_another_thread_is_synced_before_it_runs() {
    use std::sync::mpsc;
    use std::time::Duration;
    // Only a guard against a hang (a thread that never answers), not a time bound.
    let deadline = Duration::from_secs(600);
    let syncs = || unit_loader::TEXT_SYNCS.with(core::cell::Cell::get);
    let (to_main, from_a) = mpsc::channel();
    let (done, wait_done) = mpsc::channel::<()>();
    let a = std::thread::spawn(move || {
        let before = syncs();
        let iso = installed_g();
        to_main
            .send((info(&iso, "g").slot.entry(), syncs() - before))
            .unwrap();
        // Keep the text mapped until B ran it.
        let _ = wait_done.recv_timeout(deadline);
        drop(iso);
    });
    let (entry_a, syncs_a) = from_a.recv_timeout(deadline).expect("thread A answers");
    let b = std::thread::spawn(move || {
        let before = syncs();
        let mut iso = installed_g();
        let n = syncs() - before;
        let entry = info(&iso, "g").slot.entry();
        (entry, n, run_js(&mut iso, call_g_walk()))
    });
    let b = b.join();
    let _ = done.send(());
    a.join().unwrap();
    let (entry_b, syncs_b, r) = b.unwrap();
    assert_eq!(
        entry_b, entry_a,
        "B runs the text that A mapped (the hit path)"
    );
    assert_eq!(
        (syncs_a, syncs_b),
        (1, 1),
        "one sync per load, on each thread"
    );
    assert_eq!(r, Ok(Value::int32(42).to_bits()));
}

/// The isolate of [`installed_g`] before the install (the same builtins and globals).
#[cfg(unix)]
fn plain_g() -> Isolate {
    let config = IsolateConfig {
        builtins: Some(&FORWARDERS),
        ..IsolateConfig::default()
    };
    let mut iso = Isolate::new_with(config, ExecOptions::jitless()).unwrap();
    iso.register_host_function("walk", 0, walk);
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
        .unwrap();
    iso
}

/// The name of the child case test (module path from the crate root, for `--exact`).
#[cfg(unix)]
const CHILD_TEST: &str = "tests::unit_loader::unit_loader_child_case";

/// Runs [`unit_loader_child_case`] in a child process (this test binary again) with
/// `EP9_UNIT_LOADER_CHILD=<case>`: a fresh process, so the process unwind mode is not chosen
/// yet and the process code range is free. A child that does not end before the deadline (a
/// hang guard, not a time bound) is killed and the test fails.
#[cfg(unix)]
fn run_child(case: &str) {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
        .env("EP9_UNIT_LOADER_CHILD", case)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let read = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let out = read(Box::new(child.stdout.take().unwrap()));
    let err = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(1200);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child case {case}: no exit before the deadline; killed");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (out, err) = (out.join().unwrap(), err.join().unwrap());
    assert!(
        status.success() && out.contains(&format!("CHILD-OK {case}")),
        "child case {case}: {status}\n{out}\n{err}"
    );
}

/// The child side of [`run_child`]; with no `EP9_UNIT_LOADER_CHILD` it does nothing.
///
/// - `big` (Codex A2, the verifier's V-A2-2): the first text of the process fills the whole
///   free code range. The loader chooses the unwind mode before it maps the text (the probe
///   needs a page of its own), so the text installs, the mode is chosen, and a small text
///   installs after it.
/// - `busy`: `choose_mode` while the code range has no free page fails, and that error is
///   not kept: after the range is free again `choose_mode` passes and a text installs.
#[cfg(unix)]
#[test]
fn unit_loader_child_case() {
    use ep9_platform::unit_text::UnitText;
    use ep9_platform::unwind::{choose_mode, registration_mode};
    let Ok(case) = std::env::var("EP9_UNIT_LOADER_CHILD") else {
        return;
    };
    let range = CodeRange::process().unwrap();
    match case.as_str() {
        "big" => {
            let mut iso = plain_g();
            let free = range.free_bytes();
            let mut big = stub_text();
            big.resize(free, 0);
            let lu = info(&iso, "g").loaded_unit().unwrap();
            // SAFETY: as `installed_g`: offset 0 holds the frame stub, the rest is not code.
            let r = unsafe {
                unit_loader::load_and_install(
                    &iso.rt,
                    lu,
                    &big,
                    &stub_unwind(),
                    &[],
                    0,
                    &[g_stub()],
                    Some(&Policy::JITLESS),
                    0,
                )
            };
            assert_eq!(r.count(Reason::Installed), 1, "big: {:?}", r.rows());
            assert!(registration_mode().is_ok(), "{:?}", registration_mode());
            assert_eq!(
                run_js(&mut iso, call_g_walk()),
                Ok(Value::int32(42).to_bits())
            );
            drop(iso);
            assert_eq!(
                range.free_bytes(),
                free,
                "the big text went back to the range"
            );
        }
        "busy" => {
            let fill = UnitText::map(range, &vec![0u8; range.free_bytes()]).unwrap();
            assert!(choose_mode().is_err(), "no free page for the probe");
            drop(fill);
            assert!(choose_mode().is_ok(), "{:?}", choose_mode());
        }
        other => panic!("unknown child case {other}"),
    }
    // A small text installs and runs (`installed_g` checks the install).
    let mut iso = installed_g();
    assert_eq!(
        run_js(&mut iso, call_g_walk()),
        Ok(Value::int32(42).to_bits())
    );
    println!("CHILD-OK {case}");
}

/// Codex A2: a first text that fills the code range does not stop unwind registration for
/// the process (child case `big`).
#[cfg(unix)]
#[test]
fn unit_loader_a_whole_range_first_text_keeps_the_unwind_mode() {
    run_child("big");
}

/// Codex A2: a probe that could not run is not cached as the process mode (child case
/// `busy`).
#[cfg(unix)]
#[test]
fn unit_loader_a_full_range_at_the_mode_choice_is_tried_again() {
    run_child("busy");
}

// ---- JIT review F1 (M7-P5 fixer r4, M6-ORCH-386), M8-P1 A2-CONSTRUCT (M8-PLAN-D25): constructors -

/// `rconstruct(f, a)`: Construct(f, [a], f) from Rust (`Cx::construct`, the path of
/// `Reflect.construct`: `call::construct_with`, then `call_bytecode` with new.target set).
fn rconstruct<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let f = args.get(0);
    cx.construct(f, &[args.get(1)], f).map(|o| o.value())
}

/// `return new c(walk);` (Construct: the `do_call` construct arm).
fn new_c_walk() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let l = s.b.new_register_list(1);
    s.lda_global("walk");
    s.b.star(l.get(0));
    s.construct_global("c", l);
    s.b.return_();
    unit("", vec![s.finish()])
}

/// `return new g(walk);` (`g` is a method: it has no `[[Construct]]`).
fn new_g_walk() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let l = s.b.new_register_list(1);
    s.lda_global("walk");
    s.b.star(l.get(0));
    s.construct_global("g", l);
    s.b.return_();
    unit("", vec![s.finish()])
}

/// `return rconstruct(c, walk);` (the Rust construct path).
fn rconstruct_c_walk() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (a, b) = (s.b.new_register(), s.b.new_register());
    s.lda_global("c");
    s.b.star(a);
    s.lda_global("walk");
    s.b.star(b);
    s.lda_global("rconstruct");
    s.b.star(s.f);
    let (f, slot) = (s.f, s.call);
    s.b.call_undefined_receiver2(f, a, b, slot).return_();
    unit("", vec![s.finish()])
}

/// `class D extends c { constructor(f) { super(f); } } return new D(walk);` (`super()` is a
/// construct of `c` with new.target `D`: the `do_call` construct arm).
fn super_c_walk() -> VerifiedUnit {
    use ep9_bytecode::{RegisterList, RuntimeId};
    let mut md = meta("D", 1, 0, 0, 0);
    md.kind = FunctionKind::DerivedConstructor;
    md.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
    let mut d = Asm::new(md);
    let a0 = d.b.parameter(0);
    let (this, sup) = (d.b.new_register(), d.b.new_register());
    let l = d.b.new_register_list(1);
    d.b.lda_the_hole().star(this);
    d.b.lda_current_function().get_super_constructor(sup);
    d.b.throw_if_not_super_constructor(sup);
    d.b.ldar(a0).star(l.get(0));
    d.b.lda_new_target().construct(sup, l, 1, d.call);
    d.b.star(this);
    d.b.ldar(this).throw_super_not_called_if_hole().return_();
    let mut s = Asm::script(0);
    let (cls, heritage) = (s.b.new_register(), s.b.new_register());
    let def = RegisterList::new(cls.local_index().unwrap(), 2);
    assert_eq!(heritage.local_index(), Some(cls.local_index().unwrap() + 1));
    let kd = s.b.function_constant(FunctionId(1));
    s.b.create_closure(kd, s.closure, 0).star(cls);
    s.lda_global("c");
    s.b.star(heritage)
        .call_runtime(RuntimeId::DefineClass, def, 2);
    let l = s.b.new_register_list(1);
    s.lda_global("walk");
    s.b.star(l.get(0));
    s.b.ldar(cls).star(s.f);
    let (f, slot) = (s.f, s.call);
    s.b.construct(f, l, 1, slot).return_();
    unit("", vec![s.finish(), d.finish()])
}

/// JIT review F1 (M6-ORCH-386, fix form 2), then M8-P1 A2-CONSTRUCT (M8-PLAN-D25, MC-w29-8 step
/// 1): this test flips on purpose with D25. Form 2 installed no function with `[[Construct]]`;
/// since D25 step 8 installs an ordinary function, and every construct path enters its unit
/// code through `native::native_callee` with new.target in its frame (the class constructor
/// stays refused: `unit_loader_install_refuses_aot_opt_and_a_class_constructor`). One
/// installing load lists the constructor `c` and the method `g` over the same frame stub:
/// both are installed. `new c(walk)`, `rconstruct(c, walk)` (the Rust construct path of
/// `Reflect.construct`) and `super(walk)` from `class D extends c` give the results of an
/// isolate with no install, and each runs the unit code of `c` (`walk` sees the stub's return
/// address of `c` on the native stack). Before form 2 each gave "not implemented ... calling
/// code of this dispatch kind". The call `g(walk)` enters the unit code (42), and `new
/// g(walk)` of the installed method throws the TypeError of an isolate with no install.
#[test]
fn unit_loader_a_constructor_is_installed_and_construct_enters_its_unit_code() {
    let shown = |iso: &mut Isolate, u: VerifiedUnit| {
        iso.with_cx_checkpoint(|cx| cx.run_script(u, "n.js").map(|v| cx.display(v)))
            .map_err(|e| format!("{e:?}"))
    };
    let mut plain = defs_isolate(ExecOptions::jitless());
    plain.register_host_function("rconstruct", 2, rconstruct);
    let mut iso = defs_isolate(ExecOptions::jitless());
    iso.register_host_function("rconstruct", 2, rconstruct);
    // A frame stub of its own for `c` (one code range per function in the code registry).
    let starts = [0, 64];
    let mut c_stub = FunctionCode {
        function: C,
        digest: digest(C),
        ..g_stub()
    };
    c_stub.variants[0].entry = starts[1] as u32;
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: the text holds the frame stub at offsets 0 and 64, and `stub_unwind_at`
    // describes both; each runs the bytecode of `g` and of `c` (the same bytecode: the call at
    // offset 0).
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &stub_text_at(&starts),
            &stub_unwind_at(&starts),
            &[],
            0,
            &[g_stub(), c_stub],
            Some(&Policy::JITLESS),
            0,
        )
    };
    // The return address of the call in the stub of `c` (in the mapped unit text).
    let c_ret = code_of(info(&iso, "c")).map(|u| u.code_at(0).unwrap().start + call0_bytes().1);
    let mut wrong = Vec::new();
    for (name, make) in [
        ("new c(walk)", new_c_walk as fn() -> VerifiedUnit),
        ("rconstruct(c, walk)", rconstruct_c_walk),
        ("super(walk)", super_c_walk),
    ] {
        let before = shown(&mut plain, make());
        WALKED.with(|x| x.borrow_mut().take());
        let after = shown(&mut iso, make());
        let ips = WALKED.with(|x| x.borrow_mut().take()).unwrap_or_default();
        assert!(before.is_ok(), "{name} with no install: {before:?}");
        if after != before {
            wrong.push(format!(
                "{name}: no install {before:?}, after the load {after:?}"
            ));
        }
        if c_ret.is_none_or(|ret| !ips.contains(&ret)) {
            wrong.push(format!(
                "{name}: the unit code of c did not run ({c_ret:x?})"
            ));
        }
    }
    // D25: `c` is installed now (form 2 counted it not installable: [2, 1, 1]).
    let counts = [Reason::Picked, Reason::Installed, Reason::NotInstallable].map(|x| r.count(x));
    if counts != [2, 2, 0] {
        wrong.push(format!("picked, installed, not-installable: {counts:?}"));
    }
    if info(&iso, "c").slot.kind_byte() != DispatchKind::AotBaseline as u8 {
        wrong.push(format!(
            "the slot of c: {} (not AotBaseline)",
            info(&iso, "c").slot.kind_byte()
        ));
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    // The method `g` in the same unit is installed, and the call enters its unit code.
    assert_eq!(
        info(&iso, "g").slot.kind_byte(),
        DispatchKind::AotBaseline as u8
    );
    assert_eq!(
        run_js(&mut iso, call_g_walk()),
        Ok(Value::int32(42).to_bits())
    );
    // `new g(walk)` of the installed method: IsConstructor runs before the slot check, so it
    // is the TypeError of an isolate with no install, not "not implemented" (JIT review r4,
    // R4-4).
    let before = shown(&mut plain, new_g_walk());
    let after = shown(&mut iso, new_g_walk());
    assert_eq!(after, before, "new g(walk)");
    let e = after.expect_err("new g(walk) throws");
    assert!(
        e.contains("TypeError") && e.contains("is not a constructor"),
        "new g(walk): {e}"
    );
    assert!(!e.contains("not implemented"), "new g(walk): {e}");
}

// ---- M8-P1 A2-CONSTRUCT: construct cases and the caller's count (M8-PLAN-D25, D26) ----------

/// Records the native stack in [`WALKED`], as `walk` does.
fn record_walk() {
    let ips = ep9_platform::unwind::native_return_addresses(512);
    WALKED.with(|w| *w.borrow_mut() = Some(ips));
}

/// `mkobj()`: a new object with `tag` "own" (a constructor body that returns an object).
/// Records the native stack, as `walk` does.
fn mkobj<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    record_walk();
    let o = cx.new_object();
    let tag = cx.new_string("own")?;
    cx.set(o, "tag", tag, true)?;
    Ok(o.into())
}

/// `undef()`: undefined (a constructor body that returns nothing). Records the native stack.
fn undef<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    record_walk();
    Ok(Local::from_bits(Value::UNDEFINED.to_bits()))
}

/// `boom()`: throws an Error "boom" (a throw in the constructor body, under its frame).
/// Records the native stack.
fn boom<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    record_walk();
    Err(cx.throw_error(crate::runtime::ErrorKind::Error, "boom"))
}

/// `rconstruct3(f, a, nt)`: Construct(f, [a], nt) from Rust (`Reflect.construct(f, [a], nt)`).
fn rconstruct3<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.construct(args.get(0), &[args.get(1)], args.get(2))
        .map(|o| o.value())
}

/// `f.bindf(t)`: a bound function of `this` with the bound `this` `t` and no bound argument.
fn bindf<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let target = this.as_function().unwrap();
    cx.new_bound_function(target, args.get(0), args.rest(1))
        .map(|f| f.value())
}

/// `c.prototype.tag = "c"; K.prototype.tag = "K"; bc = bindf.call(c, undefined);` and the
/// realm B (`other`) with `c`, `walk`, `mkobj`, `undef` and `boom` of this realm.
fn construct_setup(iso: &mut Isolate) {
    let mut s = Asm::script(0);
    let (o, p) = (s.b.new_register(), s.b.new_register());
    for (ctor, tag) in [("c", "c"), ("K", "K")] {
        s.lda_global(ctor);
        s.b.star(o);
        s.get_named(o, "prototype");
        s.b.star(p);
        s.lda_str(tag);
        s.set_named(p, "tag");
    }
    let l = s.b.new_register_list(2);
    s.lda_global("c");
    s.b.star(l.get(0)).lda_undefined().star(l.get(1));
    s.lda_global("bindf");
    s.b.star(s.f);
    let (f, slot) = (s.f, s.call);
    s.b.call_property1(f, l.get(0), l.get(1), slot);
    s.sta_global("bc");
    s.b.lda_undefined().return_();
    iso.with_cx_checkpoint(|cx| {
        cx.run_script(unit("", vec![s.finish()]), "setup.js")
            .map(|_| ())
    })
    .unwrap();
    iso.with_cx(|cx| {
        let a = cx.global();
        let b = cx.create_realm()?;
        cx.set(a, "other", b, true)?;
        for name in ["c", "walk", "mkobj", "undef", "boom"] {
            let v = cx.get(a, name)?;
            cx.set(b, name, v, true)?;
        }
        Ok(())
    })
    .unwrap();
}

/// A jitless isolate (GC stress `stress`) that ran `defs()` and [`construct_setup`], with the
/// host functions of this section; with `install`, the frame stubs of `g` (text offset 0) and
/// `c` (offset 64) are installed. Returns the isolate and the return address of the call in
/// the stub of `c` (`None` with no install).
fn construct_isolate(install: bool, stress: bool) -> (Isolate, Option<usize>) {
    let config = IsolateConfig {
        heap: ep9_heap::HeapConfig {
            gc_stress: u32::from(stress),
            verify_after_gc: stress,
            ..ep9_heap::HeapConfig::default()
        },
        ..IsolateConfig::default()
    };
    let mut iso = Isolate::new_with(config, ExecOptions::jitless()).unwrap();
    for (name, n, f) in [
        ("walk", 0, walk as crate::host::HostFn),
        ("mkobj", 0, mkobj),
        ("undef", 0, undef),
        ("boom", 0, boom),
        ("rconstruct", 2, rconstruct),
        ("rconstruct3", 3, rconstruct3),
        ("bindf", 1, bindf),
    ] {
        iso.register_host_function(name, n, f);
    }
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
        .unwrap();
    construct_setup(&mut iso);
    if !install {
        return (iso, None);
    }
    let starts = [0, 64];
    let mut c_stub = FunctionCode {
        function: C,
        digest: digest(C),
        ..g_stub()
    };
    c_stub.variants[0].entry = starts[1] as u32;
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: as in `unit_loader_a_constructor_is_installed_and_construct_enters_its_unit_code`:
    // the text holds the frame stub at offsets 0 and 64 (each runs the call at bytecode offset
    // 0 of `g` and of `c`), and `stub_unwind_at` describes both.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &stub_text_at(&starts),
            &stub_unwind_at(&starts),
            &[],
            0,
            &[g_stub(), c_stub],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(r.count(Reason::Installed), 2, "{:?}", r.rows());
    let ret = code_of(info(&iso, "c")).unwrap().code_at(0).unwrap().start + call0_bytes().1;
    (iso, Some(ret))
}

/// `return <new ctor(arg)>.tag;` with `new` by the `Construct` opcode (`ctor` a global).
fn new_tag(ctor: &str, arg: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let l = s.b.new_register_list(1);
    let o = s.b.new_register();
    s.lda_global(arg);
    s.b.star(l.get(0));
    s.construct_global(ctor, l);
    s.b.star(o);
    s.get_named(o, "tag");
    s.b.return_();
    unit("", vec![s.finish()])
}

/// `return rconstruct3(c, walk, K).tag;` (`Reflect.construct(c, [walk], K)`: the receiver's
/// prototype comes from new.target `K`, not from `c`).
fn rconstruct3_tag() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let l = s.b.new_register_list(3);
    let o = s.b.new_register();
    for (i, name) in ["c", "walk", "K"].into_iter().enumerate() {
        s.lda_global(name);
        s.b.star(l.get(i as u32));
    }
    s.lda_global("rconstruct3");
    s.b.star(s.f);
    let (f, slot) = (s.f, s.call);
    s.b.call_undefined_receiver(f, l, 3, slot);
    s.b.star(o);
    s.get_named(o, "tag");
    s.b.return_();
    unit("", vec![s.finish()])
}

/// Runs `u` in `iso` (in realm B, `other`, when `in_b`): the completion value shown, or the
/// exception with its message and stack.
fn shown_in(iso: &mut Isolate, u: VerifiedUnit, in_b: bool) -> Result<String, String> {
    iso.with_cx_checkpoint(|cx| {
        if !in_b {
            return cx.run_script(u, "case.js").map(|v| cx.display(v));
        }
        let a = cx.global();
        let b = cx.get(a, "other")?.as_object().unwrap();
        cx.with_realm(b, |cx| cx.run_script(u, "case-b.js").map(|v| cx.display(v)))
    })
    .map_err(|e| format!("{} | {:?}", e.message, e.stack))
}

/// MC-w36-3 (the 6 construct cases; plan v13 4.2 task 5, the FASTDIFF form of this file:
/// `fastpath_diff.rs` cannot install unit code). Each case runs in an isolate with no install
/// and in one where `c` (and `g`) run their unit code, with and without GC stress, and must
/// give the same result, receiver and error: (1) a base constructor that returns an object,
/// (2) a primitive, (3) nothing; (4) `new` of a bound function whose target is installed; (5)
/// `new` from another realm (`slow::call` sends it to `call::construct_function_object`, the
/// Rust path); (6) `Reflect.construct(c, [walk], K)` with new.target `K` ≠ `c`; and (7) a
/// throw in the constructor body with `Error.stack` through the unit frame (MC-w35-7: the
/// loader-installed text form beside `native_entry.rs`). In the install isolate each case
/// that calls `walk` sees the stub's return address of `c` on the native stack (the unit code
/// ran); before D25 `c` was not installed (form 2).
#[test]
fn unit_loader_construct_of_unit_code_gives_the_bytecode_results() {
    type Case = (
        &'static str,
        fn() -> VerifiedUnit,
        bool,
        Result<&'static str, &'static str>,
    );
    let cases: [Case; 7] = [
        ("object", || new_tag("c", "mkobj"), false, Ok("own")),
        ("primitive", || new_tag("c", "walk"), false, Ok("c")),
        ("nothing", || new_tag("c", "undef"), false, Ok("c")),
        ("bound", || new_tag("bc", "walk"), false, Ok("c")),
        ("cross-realm", || new_tag("c", "walk"), true, Ok("c")),
        ("reflect", rconstruct3_tag, false, Ok("K")),
        ("throw", || new_tag("c", "boom"), false, Err("boom")),
    ];
    for stress in [false, true] {
        let (mut plain, _) = construct_isolate(false, stress);
        let (mut iso, ret) = construct_isolate(true, stress);
        let ret = ret.unwrap();
        let mut wrong = Vec::new();
        for (name, make, in_b, want) in cases {
            let before = shown_in(&mut plain, make(), in_b);
            WALKED.with(|x| x.borrow_mut().take());
            let after = shown_in(&mut iso, make(), in_b);
            let ips = WALKED.with(|x| x.borrow_mut().take()).unwrap_or_default();
            let label = format!("{name} stress={stress}");
            match (&before, want) {
                (Ok(v), Ok(w)) if v == w => {}
                (Err(e), Err(w)) if e.starts_with(&format!("Error: {w}")) => {}
                _ => wrong.push(format!(
                    "{label}: with no install {before:?}, want {want:?}"
                )),
            }
            if after != before {
                wrong.push(format!(
                    "{label}: no install {before:?}, installed {after:?}"
                ));
            }
            if !ips.contains(&ret) {
                wrong.push(format!("{label}: the unit code of c did not run"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
        // The stack of the throw names the frame of `c` (the unit frame; its call `f()`).
        let e = shown_in(&mut iso, new_tag("c", "boom"), false).unwrap_err();
        assert!(e.contains("at c ("), "the stack names c: {e}");
        iso.collect_garbage();
        iso.verify_heap().unwrap();
        drop(plain);
    }
}

/// `g(walk)` `n` times in one script (n T0 calls of `g`); returns the last result.
fn call_g_walk_n(n: u32) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let a = s.a.get(0);
    for _ in 0..n {
        s.lda_global("walk");
        s.b.star(a);
        s.lda_global("g");
        s.b.star(s.f);
        let (f, slot) = (s.f, s.call);
        s.b.call_undefined_receiver1(f, a, slot);
    }
    s.b.return_();
    unit("", vec![s.finish()])
}

/// MC-w29-1 (M8-PLAN-D26): the caller counts a call of unit code, so a unit function gets its
/// vector at the 8th call (threshold 8, `isolate.rs`), as its bytecode does, on each path: T0
/// calls of `g` (`slow::call` before `call_native_cold`), T0 constructs of `c` (before the
/// construct arm's native entry), calls from Rust (`Cx::call`) and constructs from Rust
/// (`Reflect.construct`), both counted at the pushed frame as `slow::count_entry` counts. The
/// unit code counts nothing itself. Before D26 no call of a native slot counted, so the unit
/// function never got a vector (H32-Q5 (2): native slot calls skipped `count_call`). With
/// `spike-native` the T0 call takes the spike entry, which counts nothing.
#[test]
fn unit_loader_construct_and_call_of_unit_code_count_at_the_caller() {
    let has_vector = |iso: &Isolate, name: &str| crate::feedback::vector(info(iso, name)).is_some();
    // T0 calls and T0 constructs.
    let (mut iso, _) = construct_isolate(true, false);
    let v42 = Ok(Value::int32(42).to_bits());
    assert_eq!(run_js(&mut iso, call_g_walk_n(7)), v42);
    assert!(!has_vector(&iso, "g"), "g after 7 T0 calls");
    assert_eq!(run_js(&mut iso, call_g_walk_n(1)), v42);
    // With `spike-native` a T0 call of native code takes the spike entry first
    // (`slow::call` `call_native`), which counts nothing (the `native.rs` module doc; the
    // `tests::native` A-hot test pins it); the default build counts it.
    let t0_calls_count = !cfg!(feature = "spike-native");
    assert_eq!(has_vector(&iso, "g"), t0_calls_count, "g after 8 T0 calls");
    for i in 1..=8 {
        assert_eq!(
            shown_in(&mut iso, new_tag("c", "walk"), false),
            Ok("c".into())
        );
        assert_eq!(has_vector(&iso, "c"), i == 8, "c after {i} T0 constructs");
    }
    // Calls and constructs from Rust.
    let (mut iso, _) = construct_isolate(true, false);
    let [g, c, w] = ["g", "c", "walk"].map(|n| iso.global_raw(n).unwrap());
    for i in 1..=8 {
        let v = iso.call_raw(g, Value::UNDEFINED, &[w]).unwrap();
        assert_eq!(v.to_bits(), Value::int32(42).to_bits());
        assert_eq!(has_vector(&iso, "g"), i == 8, "g after {i} calls from Rust");
        let r = iso.with_cx_checkpoint(|cx| {
            let (c, w): (Local<'_>, Local<'_>) =
                (Local::from_bits(c.to_bits()), Local::from_bits(w.to_bits()));
            let o = cx.construct(c, &[w], c)?;
            cx.get(o, "tag").map(|t| cx.display(t))
        });
        assert_eq!(r.as_deref(), Ok("c"));
        assert_eq!(
            has_vector(&iso, "c"),
            i == 8,
            "c after {i} constructs from Rust"
        );
    }
}

// ---- M8-P2 A2-CUT: C-w22-1 (a text blob at any address) and MC-w23-15 (landing pads) -----

/// Installs `fc` over `text` and `unwind` in a jitless isolate of `defs()` through
/// [`unit_loader::load_and_install_with_pads`]; the isolate and the load counts.
fn padded_isolate(text: &[u8], unwind: &[u8], fc: FunctionCode, pads: bool) -> (Isolate, Report) {
    let iso = defs_isolate(ExecOptions::jitless());
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: each caller's text runs the bytecode of `g` from the variant entry (the frame
    // stub, after a `bti c` or not), and `unwind` describes the stub.
    let r = unsafe {
        unit_loader::load_and_install_with_pads(
            &iso.rt,
            lu,
            text,
            unwind,
            &[],
            0,
            &[fc],
            Some(&Policy::JITLESS),
            0,
            pads,
        )
    };
    (iso, r)
}

/// `g(walk)` through the Rust entry (`blr` on AArch64): 42 when the unit code ran.
fn g_walk_gives_42(iso: &mut Isolate) {
    let g = iso.global_raw("g").unwrap();
    let w = iso.global_raw("walk").unwrap();
    let v = call_through_entry(iso, g, &[w]).unwrap().unwrap();
    assert_eq!(v.to_bits(), Value::int32(42).to_bits());
}

/// C-w22-1: the unit text of an embedder blob may lie at any address (here one byte into a
/// buffer, so not even 2-aligned): the loader copies it into a page of the process code range,
/// installs `g`, and its unit code runs. A blob in memory is never a load error for its
/// address; the 64 KiB rule is the file layout (ep9_unit container v3). This holds on the old
/// code too (the loader always copied); the test pins it.
#[test]
fn unit_loader_a_text_at_an_unaligned_address_is_copied_and_runs() {
    let text = stub_text();
    let mut buf = vec![0u8; text.len() + 1];
    buf[1..].copy_from_slice(&text);
    let blob = &buf[1..];
    let at = blob.as_ptr() as usize;
    assert!(!at.is_multiple_of(2), "{at:#x}");
    let (mut iso, r) = padded_isolate(blob, &stub_unwind(), g_stub(), false);
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    let code = code_of(info(&iso, "g")).unwrap().code_at(0).unwrap();
    assert!(CodeRange::process().unwrap().contains(code.start));
    assert!(code.start.is_multiple_of(4096) && code.start != at);
    g_walk_gives_42(&mut iso);
}

/// MC-w23-15: a unit with the compat flag `has_pads` whose entry is `bti c` maps as BTI
/// guarded pages where the target has them (Linux and Android AArch64 with a BTI CPU and
/// kernel), and its code runs there (a `blr` into `bti c`). With the flag but an entry that is
/// not a pad (the plain frame stub), the loader does not trust the flag and maps plain RX: in
/// guarded pages that call would fault. Other targets: plain RX for both.
#[test]
fn unit_loader_a_padded_text_maps_with_bti_where_the_target_has_it() {
    let guarded = cfg!(all(
        any(target_os = "linux", target_os = "android"),
        target_arch = "aarch64"
    )) && {
        #[cfg(target_arch = "aarch64")]
        {
            std::arch::is_aarch64_feature_detected!("bti")
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            false
        }
    };
    let stub = call0_bytes().0.len() as u32;
    // AArch64: `bti c` at the entry (offset 0), the frame stub at 4. Elsewhere no pad form:
    // the plain stub.
    let (text, unwind, len) = if cfg!(target_arch = "aarch64") {
        let mut t = stub_text_at(&[4]);
        t[..4].copy_from_slice(&0xd503_245fu32.to_le_bytes());
        (t, stub_unwind_at(&[4]), 4 + stub)
    } else {
        (stub_text(), stub_unwind(), stub)
    };
    let fc = FunctionCode {
        function: G,
        digest: digest(G),
        variants: vec![variant(
            DispatchKind::AotBaseline,
            lowering::JITLESS,
            0,
            len,
        )],
    };
    let (mut iso, r) = padded_isolate(&text, &unwind, fc, true);
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    let uc = code_of(info(&iso, "g")).unwrap();
    assert_eq!(unit_loader::text_bti(uc), Some(guarded));
    g_walk_gives_42(&mut iso);
    // A JS call: T0 enters the unit code too.
    assert_eq!(
        run_js(&mut iso, call_g_walk()),
        Ok(Value::int32(42).to_bits())
    );
    // The flag with an entry that is no pad: plain RX, and the code runs.
    let (mut iso, r) = padded_isolate(&stub_text(), &stub_unwind(), g_stub(), true);
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    let uc = code_of(info(&iso, "g")).unwrap();
    assert_eq!(unit_loader::text_bti(uc), Some(false));
    g_walk_gives_42(&mut iso);
}

// The frame stub of `call0_bytes`, linked into this test binary's own executable pages with
// its unwind entries in the binary's own tables (the assembler makes them from the CFI or SEH
// directives): what the static linker does with the text of a `.o` unit (H35 S1; route
// MC-w34-5). A data symbol names it, so no linker thunk stands between the name and the bytes.
#[cfg(all(
    target_arch = "aarch64",
    any(target_os = "linux", target_os = "android")
))]
core::arch::global_asm!(
    ".pushsection .text.ep9_linked_test,\"ax\",%progbits",
    ".p2align 4",
    ".globl ep9_linked_test_text",
    ".hidden ep9_linked_test_text",
    "ep9_linked_test_text:",
    LINKED_A64_BODY!(),
    ".popsection",
    table = const layout::CX_RUNTIME_TABLE,
    entry = const layout::RT_CALL_ANY,
);
#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
core::arch::global_asm!(
    ".section __TEXT,__text,regular,pure_instructions",
    ".p2align 4",
    ".globl _ep9_linked_test_text",
    ".private_extern _ep9_linked_test_text",
    "_ep9_linked_test_text:",
    LINKED_A64_BODY!(),
    table = const layout::CX_RUNTIME_TABLE,
    entry = const layout::RT_CALL_ANY,
);
#[cfg(all(
    target_arch = "aarch64",
    any(target_os = "linux", target_os = "android", target_vendor = "apple")
))]
macro_rules! LINKED_A64_BODY {
    () => {
        concat!(
            ".cfi_startproc\n",
            "stp x29, x30, [sp, #-16]!\n",
            ".cfi_def_cfa_offset 16\n",
            ".cfi_offset w30, -8\n",
            ".cfi_offset w29, -16\n",
            "mov x29, sp\n",
            ".cfi_def_cfa w29, 16\n",
            "ldr x16, [x0, #{table}]\n",
            "ldr x16, [x16, #{entry}]\n",
            "mov w2, #0\n",
            "mov x3, xzr\n",
            "blr x16\n",
            "ldp x29, x30, [sp], #16\n",
            ".cfi_def_cfa wsp, 0\n",
            ".cfi_restore w30\n",
            ".cfi_restore w29\n",
            "ret\n",
            ".cfi_endproc\n",
        )
    };
}
#[cfg(all(
    target_arch = "aarch64",
    any(target_os = "linux", target_os = "android", target_vendor = "apple")
))]
use LINKED_A64_BODY;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "linux", target_os = "android")
))]
core::arch::global_asm!(
    ".pushsection .text.ep9_linked_test,\"ax\",@progbits",
    ".p2align 4",
    ".globl ep9_linked_test_text",
    ".hidden ep9_linked_test_text",
    "ep9_linked_test_text:",
    ".cfi_startproc",
    "push rbp",
    ".cfi_def_cfa_offset 16",
    ".cfi_offset rbp, -16",
    "mov rbp, rsp",
    ".cfi_def_cfa_register rbp",
    "mov rax, qword ptr [rdi + {table}]",
    "xor edx, edx",
    "xor ecx, ecx",
    "call qword ptr [rax + {entry}]",
    "pop rbp",
    ".cfi_def_cfa rsp, 8",
    "ret",
    ".cfi_endproc",
    ".popsection",
    table = const layout::CX_RUNTIME_TABLE,
    entry = const layout::RT_CALL_ANY,
);
#[cfg(all(target_arch = "x86_64", windows))]
core::arch::global_asm!(
    ".section .text$ep9lt,\"xr\"",
    ".p2align 4",
    ".globl ep9_linked_test_text",
    ".def ep9_linked_test_text",
    ".scl 2",
    ".type 32",
    ".endef",
    "ep9_linked_test_text:",
    ".seh_proc ep9_linked_test_text",
    "push rbp",
    ".seh_pushreg rbp",
    ".seh_endprologue",
    "mov rbp, rsp",
    "mov rax, qword ptr [rdi + {table}]",
    "xor edx, edx",
    "xor ecx, ecx",
    "call qword ptr [rax + {entry}]",
    "pop rbp",
    "ret",
    ".seh_endproc",
    ".text",
    table = const layout::CX_RUNTIME_TABLE,
    entry = const layout::RT_CALL_ANY,
);

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    all(target_vendor = "apple", target_arch = "aarch64"),
    all(windows, target_arch = "x86_64")
))]
unsafe extern "C" {
    /// The first byte of the linked frame stub (above).
    static ep9_linked_test_text: u8;
}

/// The linked frame stub: `call0_bytes().0.len()` bytes in this binary's text.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    all(target_vendor = "apple", target_arch = "aarch64"),
    all(windows, target_arch = "x86_64")
))]
fn linked_stub() -> &'static [u8] {
    let len = call0_bytes().0.len();
    // SAFETY: the global_asm above places the stub's bytes, `len` of them, at the symbol, in
    // the binary's read-execute text, never written.
    unsafe { core::slice::from_raw_parts(&raw const ep9_linked_test_text, len) }
}

/// Route MC-w34-5 (the `.o` unit form, H35 S1 and H35-Q3): a registered linked text runs in
/// place. The frame stub is linked into this binary (as the static linker links the text of a
/// `.o` unit) with its unwind entries in the binary's own tables; after static registration
/// (`ep9_platform::linked::register`), an installing load with that text and with no unwind
/// data maps nothing and registers nothing (the code address is the linked text, outside the
/// process code range), installs `g`, and `call_native` runs it; the system unwinder walks from
/// `walk` through the linked frame into the Rust caller. The same bytes copied (not
/// registered) take the blob path: without unwind data they install nothing ("no-unwind").
#[test]
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    all(target_vendor = "apple", target_arch = "aarch64"),
    all(windows, target_arch = "x86_64")
))]
fn unit_loader_a_registered_linked_text_runs_in_place() {
    use ep9_platform::linked::{LinkedUnit, linked_text, register};
    use ep9_platform::unit_text::{Form, UnitText};
    let text = linked_stub();
    assert_eq!(
        text,
        &call0_bytes().0[..],
        "the linked stub is the frame stub"
    );
    // Not registered yet: not a linked text.
    assert!(linked_text(text).is_none() && UnitText::linked(text).is_none());
    let unit: &'static [u8] = Box::leak(vec![0x55u8; 64].into_boxed_slice());
    let rec: &'static LinkedUnit = Box::leak(Box::new(LinkedUnit::new(unit, text)));
    // SAFETY: `text` is in this binary's text, never written, and the binary's unwind tables
    // cover it (the directives above); `unit` lives for the process.
    unsafe { register(rec) }.unwrap();
    assert_eq!(rec.text().as_ptr(), text.as_ptr());
    assert_eq!(linked_text(text).map(<[u8]>::as_ptr), Some(text.as_ptr()));
    // Only the exact range is the linked text.
    assert!(linked_text(&text[1..]).is_none() && linked_text(&text[..4]).is_none());
    assert_eq!(UnitText::linked(text).unwrap().form(), Form::Linked);

    let mut iso = defs_isolate(ExecOptions::jitless());
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: the linked text is the frame stub (it runs the bytecode of `g`), and the binary's
    // tables describe it, so no unwind data is passed.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            text,
            &[],
            &[],
            0,
            &[g_stub()],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(
        (r.count(Reason::Picked), r.count(Reason::Installed)),
        (1, 1),
        "{:?}",
        r.rows()
    );
    let code = code_of(info(&iso, "g")).unwrap().code_at(0).unwrap();
    assert_eq!(code.start, text.as_ptr() as usize, "the code runs in place");
    let range = CodeRange::process().unwrap();
    assert!(!range.contains(code.start));
    let g = iso.global_raw("g").unwrap();
    let w = iso.global_raw("walk").unwrap();
    WALKED.with(|x| x.borrow_mut().take());
    let v = call_through_entry(&mut iso, g, &[w]).unwrap().unwrap();
    assert_eq!(v.to_bits(), Value::int32(42).to_bits());
    let ips = WALKED.with(|x| x.borrow_mut().take()).expect("walk ran");
    let ret = code.start + call0_bytes().1;
    let at = ips.iter().position(|&ip| ip == ret).unwrap_or_else(|| {
        panic!("the walk misses the linked frame (return address {ret:#x}): {ips:x?}")
    });
    assert!(
        ips.len() - at >= 4,
        "the walk stops at the linked frame: {ips:x?}"
    );
    drop(iso);

    // The same bytes, copied: the blob path, which needs unwind data.
    let copy = text.to_vec();
    let iso = defs_isolate(ExecOptions::jitless());
    let lu = info(&iso, "g").loaded_unit().unwrap();
    // SAFETY: as above; with no unwind data the blob path installs nothing.
    let r = unsafe {
        unit_loader::load_and_install(
            &iso.rt,
            lu,
            &copy,
            &[],
            &[],
            0,
            &[g_stub()],
            Some(&Policy::JITLESS),
            0,
        )
    };
    assert_eq!(
        (r.count(Reason::NoUnwind), r.count(Reason::Installed)),
        (1, 0),
        "{:?}",
        r.rows()
    );
}

// ---- M8-P4 A2-GATE: MC-w35-5, the GuardFuse-only install rule -------------------------------

/// MC-w35-5 (H31:138): step 8 installs a pick only when each fuse ref is a GuardFuse read: a
/// static fuse that this build knows and that holds. A dynamic fuse is an invalidation
/// dependency (not installable until H29 A4); a popped or unknown static fuse keeps the
/// bytecode, also when it popped after the pick.
#[test]
fn unit_loader_install_takes_only_guard_fuse_reads() {
    use crate::select::Dep;
    let ok = |index| Dep::Static {
        index,
        intact: true,
    };
    assert!(unit_loader::guard_fuse_reads_only(&[]));
    assert!(unit_loader::guard_fuse_reads_only(&[ok(0), ok(1)]));
    let popped = Dep::Static {
        index: 0,
        intact: false,
    };
    let unknown = ok(u32::MAX);
    let dynamic = Dep::Dynamic {
        kind: 1,
        intact: true,
    };
    for d in [popped, unknown, dynamic] {
        assert!(!unit_loader::guard_fuse_reads_only(&[d]), "{d:?}");
        assert!(!unit_loader::guard_fuse_reads_only(&[ok(0), d]), "{d:?}");
    }
}

/// MC-w35-5: the unit code of a pick with a static fuse ref runs, and a pop after the install
/// leaves it installed: its GuardFuse reads take the slow path at each use (H31:138), so no
/// frame needs a swap. A fuse that popped before the load keeps the bytecode (no pick).
#[test]
fn unit_loader_a_guard_fuse_pick_runs_and_stays_after_a_pop() {
    let install = |iso: &Isolate| {
        let lu = info(iso, "g").loaded_unit().unwrap();
        let mut f = g_stub();
        f.variants[0].fuses = vec![0];
        // SAFETY: the text at offset 0 is the frame stub (it runs the bytecode of `g` and reads
        // no fuse), and `stub_unwind` describes it.
        unsafe {
            unit_loader::load_and_install(
                &iso.rt,
                lu,
                &stub_text(),
                &stub_unwind(),
                &[],
                0,
                &[f],
                Some(&Policy::JITLESS),
                0,
            )
        }
    };
    let mut iso = defs_isolate(ExecOptions::jitless());
    iso.rt.fuses.wire(&iso.rt, 0).unwrap();
    let r = install(&iso);
    assert_eq!(r.count(Reason::Installed), 1, "{:?}", r.rows());
    g_walk_gives_42(&mut iso);
    iso.rt.fuses.pop(&iso.rt, FuseRef::Static(0));
    assert_eq!(info(&iso, "g").slot.kind(), Some(DispatchKind::AotBaseline));
    g_walk_gives_42(&mut iso);
    // Popped before the load: no pick, nothing installed, the bytecode runs.
    let mut iso = defs_isolate(ExecOptions::jitless());
    iso.rt.fuses.wire(&iso.rt, 0).unwrap();
    iso.rt.fuses.pop(&iso.rt, FuseRef::Static(0));
    let r = install(&iso);
    assert_eq!(
        (r.count(Reason::Picked), r.count(Reason::Installed)),
        (0, 0)
    );
    assert_eq!(info(&iso, "g").slot.kind(), Some(DispatchKind::Interp));
    assert_eq!(
        run_js(&mut iso, call_g_walk()),
        Ok(Value::int32(42).to_bits())
    );
}

// ---- M8-P4 A2-GATE: class constructors (MC-w29-8 step 2, the MC-w33-6 contract) -------------

/// The installs of class constructors: not in a `spike-native` build, where `install_code`
/// keeps refusing them (`native::installs_class_code`; that refusal is a `tests::native` test).
#[cfg(not(feature = "spike-native"))]
mod class_ctors {
    use super::*;

    /// The source of the class unit (positions for stack traces).
    const SRC: &str = "class B { constructor(f) { return f(); } }\n\
class D extends B { constructor(f) { return f(); } }\n\
class S extends B { constructor(f) { super(f); } }\n";

    /// Function 1 `B` (a base class constructor), function 2 `D extends B` (a derived
    /// constructor that never calls `super()`), function 3 `S extends B` (T0 only).
    const B: u32 = 1;
    const D: u32 = 2;

    /// The position of the first `f()` after `from` in [`SRC`].
    fn call_at(from: &str) -> u32 {
        let start = pos(SRC, from) as usize;
        (start + SRC[start..].find("f()").unwrap()) as u32
    }

    /// The class unit: `B` and `D` are `return f();` (the call at bytecode offset 0, the
    /// offset the frame stub passes; for `D` an undefined result is the ReferenceError of a
    /// `this` binding that `super()` never set, and 1 the TypeError, as the MC-w33-6 contract
    /// says); `S` is `super(f);` (a construct of `B` with new.target `S`). The script makes
    /// each a global (`D` and `S` by DefineClass with the heritage `B`) and sets
    /// `<C>.prototype.tag` to the class name.
    fn class_defs() -> VerifiedUnit {
        use ep9_bytecode::{RegisterList, RuntimeId};
        let ctor = |name: &str, kind: FunctionKind, from: &str, to: Option<&str>| {
            let end = to.map_or(SRC.len() as u32, |t| pos(SRC, t) - 1);
            let mut m = meta(name, 1, 0, pos(SRC, from), end);
            m.kind = kind;
            m.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
            Asm::new(m)
        };
        let return_f = |a: &mut Asm, from: &str| {
            let a0 = a.b.parameter(0);
            a.b.set_expression_position(call_at(from));
            let slot = a.call;
            a.b.call_undefined_receiver0(a0, slot).return_();
        };
        let mut b = ctor(
            "B",
            FunctionKind::ClassConstructor,
            "class B",
            Some("class D"),
        );
        return_f(&mut b, "class B");
        let mut d = ctor(
            "D",
            FunctionKind::DerivedConstructor,
            "class D",
            Some("class S"),
        );
        return_f(&mut d, "class D");
        let mut s3 = ctor("S", FunctionKind::DerivedConstructor, "class S", None);
        let a0 = s3.b.parameter(0);
        let (this, sup) = (s3.b.new_register(), s3.b.new_register());
        let l = s3.b.new_register_list(1);
        s3.b.lda_the_hole().star(this);
        s3.b.lda_current_function().get_super_constructor(sup);
        s3.b.throw_if_not_super_constructor(sup);
        s3.b.ldar(a0).star(l.get(0));
        let slot = s3.call;
        s3.b.lda_new_target().construct(sup, l, 1, slot);
        s3.b.star(this);
        s3.b.ldar(this).throw_super_not_called_if_hole().return_();
        let mut s = Asm::script(SRC.len());
        let kb = s.b.function_constant(FunctionId(B));
        s.b.create_closure(kb, s.closure, 0);
        s.sta_global("B");
        for (i, name) in [(D, "D"), (3, "S")] {
            let (cls, heritage) = (s.b.new_register(), s.b.new_register());
            let def = RegisterList::new(cls.local_index().unwrap(), 2);
            assert_eq!(heritage.local_index(), Some(cls.local_index().unwrap() + 1));
            let k = s.b.function_constant(FunctionId(i));
            s.b.create_closure(k, s.closure, 0).star(cls);
            s.lda_global("B");
            s.b.star(heritage)
                .call_runtime(RuntimeId::DefineClass, def, 2);
            s.b.ldar(cls);
            s.sta_global(name);
        }
        let (o, p) = (s.b.new_register(), s.b.new_register());
        for name in ["B", "D", "S"] {
            s.lda_global(name);
            s.b.star(o);
            s.get_named(o, "prototype");
            s.b.star(p);
            s.lda_str(name);
            s.set_named(p, "tag");
        }
        s.b.lda_undefined().return_();
        let fns = vec![s.finish(), b.finish(), d.finish(), s3.finish()];
        unit(SRC, fns)
    }

    /// `one()`: 1 (a derived constructor's result that is not an object). Records the native
    /// stack.
    fn one<'s>(
        _cx: &mut Cx<'s>,
        _this: Local<'s>,
        _args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        record_walk();
        Ok(Local::from_bits(Value::int32(1).to_bits()))
    }

    /// The host functions of this section and their arity.
    const HOSTS: [(&str, u32, crate::host::HostFn); 6] = [
        ("walk", 0, walk),
        ("mkobj", 0, mkobj),
        ("undef", 0, undef),
        ("one", 0, one),
        ("boom", 0, boom),
        ("rconstruct3", 3, rconstruct3),
    ];

    /// A jitless isolate (GC stress `stress`) with the forwarders, the host functions and the
    /// realm `other` (with `B`, `D`, `S` and the host functions of this realm), that ran
    /// [`class_defs`]. With `install` the frame stubs of `B` (text offset 0) and `D` (64) are
    /// installed. Returns the isolate and the return address of the call in each stub.
    fn class_isolate(install: bool, stress: bool) -> (Isolate, Option<[usize; 2]>) {
        let config = IsolateConfig {
            builtins: Some(&FORWARDERS),
            heap: ep9_heap::HeapConfig {
                gc_stress: u32::from(stress),
                verify_after_gc: stress,
                ..ep9_heap::HeapConfig::default()
            },
            ..IsolateConfig::default()
        };
        let mut iso = Isolate::new_with(config, ExecOptions::jitless()).unwrap();
        for (name, n, f) in HOSTS {
            iso.register_host_function(name, n, f);
        }
        iso.with_cx_checkpoint(|cx| cx.run_script(class_defs(), "classes.js").map(|_| ()))
            .unwrap();
        iso.with_cx(|cx| {
            let a = cx.global();
            let b = cx.create_realm()?;
            cx.set(a, "other", b, true)?;
            for name in ["B", "D", "S"].into_iter().chain(HOSTS.map(|h| h.0)) {
                let v = cx.get(a, name)?;
                cx.set(b, name, v, true)?;
            }
            Ok(())
        })
        .unwrap();
        if !install {
            return (iso, None);
        }
        let starts = [0, 64];
        let stub = |function: u32, at: usize| FunctionCode {
            function,
            digest: unit_function_digest(&class_defs(), function).unwrap(),
            variants: vec![variant(
                DispatchKind::AotBaseline,
                lowering::JITLESS,
                at as u32,
                call0_bytes().0.len() as u32,
            )],
        };
        let lu = info(&iso, "B").loaded_unit().unwrap();
        // SAFETY: the text holds the frame stub at offsets 0 and 64 (each runs the call at
        // bytecode offset 0 of `B` and of `D` and returns its result: the bytecode of each,
        // whose `return` gives the call's result), and `stub_unwind_at` describes both.
        let r = unsafe {
            unit_loader::load_and_install(
                &iso.rt,
                lu,
                &stub_text_at(&starts),
                &stub_unwind_at(&starts),
                &[],
                0,
                &[stub(B, starts[0]), stub(D, starts[1])],
                Some(&Policy::JITLESS),
                0,
            )
        };
        assert_eq!(
            (r.count(Reason::Installed), r.count(Reason::NotInstallable)),
            (2, 0),
            "{:?}",
            r.rows()
        );
        let ret = |name: &str| {
            let fi = info(&iso, name);
            assert_eq!(fi.slot.kind(), Some(DispatchKind::AotBaseline), "{name}");
            code_of(fi).unwrap().code_at(0).unwrap().start + call0_bytes().1
        };
        let rets = [ret("B"), ret("D")];
        (iso, Some(rets))
    }

    /// `return rconstruct3(<f>, <a>, <nt>).tag;` (`Reflect.construct(f, [a], nt)`, the Rust
    /// construct path).
    fn reflect_tag(f: &str, a: &str, nt: &str) -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(3);
        let o = s.b.new_register();
        for (i, name) in [f, a, nt].into_iter().enumerate() {
            s.lda_global(name);
            s.b.star(l.get(i as u32));
        }
        s.lda_global("rconstruct3");
        s.b.star(s.f);
        let (fr, slot) = (s.f, s.call);
        s.b.call_undefined_receiver(fr, l, 3, slot);
        s.b.star(o);
        s.get_named(o, "tag");
        s.b.return_();
        unit("", vec![s.finish()])
    }

    /// `return <f>(walk);` (a T0 call: `call_fast` sends a class constructor cold).
    fn call_walk(f: &str) -> VerifiedUnit {
        let mut s = Asm::script(0);
        let a = s.a.get(0);
        s.lda_global("walk");
        s.b.star(a);
        s.lda_global(f);
        s.b.star(s.f);
        let (fr, slot) = (s.f, s.call);
        s.b.call_undefined_receiver1(fr, a, slot).return_();
        unit("", vec![s.finish()])
    }

    /// `return fcall.call(<f>, undefined, walk);` (the forwarder form of
    /// `Function.prototype.call`).
    fn fcall_walk(f: &str) -> VerifiedUnit {
        let mut s = Asm::script(0);
        let l = s.b.new_register_list(3);
        s.lda_global(f);
        s.b.star(l.get(0)).lda_undefined().star(l.get(1));
        s.lda_global("walk");
        s.b.star(l.get(2));
        s.lda_global("fcall");
        s.b.star(s.f);
        let (fr, slot) = (s.f, s.call);
        s.b.call_property(fr, l, 3, slot).return_();
        unit("", vec![s.finish()])
    }

    /// MC-w29-8 step 2 under the MC-w33-6 contract (M7-P5-D7): step 8 installs the class
    /// constructors `B` (base) and `D` (derived), and each construct case gives the result,
    /// receiver and error of an isolate with no install, with and without GC stress, and runs
    /// the unit code (the stub's return address is on the native stack). `B`: a primitive
    /// result, an object, nothing, from another realm, `super()` from `S` (a T0 derived
    /// class: the receiver's prototype is `S`'s), and `Reflect.construct(B, [walk], D)`. `D`
    /// (the HOLE is its `this` binding: it never calls `super()`): an object result; nothing
    /// gives the caller's ReferenceError and 1 its TypeError (`call::construct_result`), also
    /// from another realm and through `Reflect.construct`. On the code before M8-P4 nothing is
    /// installed (both picks are not installable).
    #[test]
    fn unit_loader_class_constructors_install_and_construct_gives_the_bytecode_results() {
        type Case = (
            &'static str,
            VerifiedUnit,
            bool,
            Result<&'static str, &'static str>,
            usize,
        );
        let cases = || -> Vec<Case> {
            vec![
                ("B primitive", new_tag("B", "walk"), false, Ok("B"), 0),
                ("B object", new_tag("B", "mkobj"), false, Ok("own"), 0),
                ("B nothing", new_tag("B", "undef"), false, Ok("B"), 0),
                ("B other realm", new_tag("B", "walk"), true, Ok("B"), 0),
                ("super() into B", new_tag("S", "walk"), false, Ok("S"), 0),
                (
                    "Reflect B nt D",
                    reflect_tag("B", "walk", "D"),
                    false,
                    Ok("D"),
                    0,
                ),
                (
                    "B throw",
                    new_tag("B", "boom"),
                    false,
                    Err("Error: boom"),
                    0,
                ),
                ("D object", new_tag("D", "mkobj"), false, Ok("own"), 1),
                (
                    "D no super",
                    new_tag("D", "undef"),
                    false,
                    Err("ReferenceError"),
                    1,
                ),
                (
                    "D result 1",
                    new_tag("D", "one"),
                    false,
                    Err("TypeError"),
                    1,
                ),
                (
                    "D other realm no super",
                    new_tag("D", "undef"),
                    true,
                    Err("ReferenceError"),
                    1,
                ),
                (
                    "D other realm 1",
                    new_tag("D", "one"),
                    true,
                    Err("TypeError"),
                    1,
                ),
                (
                    "Reflect D",
                    reflect_tag("D", "mkobj", "D"),
                    false,
                    Ok("own"),
                    1,
                ),
                (
                    "Reflect D no super",
                    reflect_tag("D", "undef", "D"),
                    false,
                    Err("ReferenceError"),
                    1,
                ),
            ]
        };
        for stress in [false, true] {
            let (mut plain, _) = class_isolate(false, stress);
            let (mut iso, rets) = class_isolate(true, stress);
            let rets = rets.unwrap();
            let mut wrong = Vec::new();
            for ((name, u, in_b, want, ran), (_, u2, ..)) in cases().into_iter().zip(cases()) {
                let before = shown_in(&mut plain, u, in_b);
                WALKED.with(|x| x.borrow_mut().take());
                let after = shown_in(&mut iso, u2, in_b);
                let ips = WALKED.with(|x| x.borrow_mut().take()).unwrap_or_default();
                let label = format!("{name} stress={stress}");
                match (&before, want) {
                    (Ok(v), Ok(w)) if v == w => {}
                    (Err(e), Err(w)) if e.starts_with(w) => {}
                    _ => wrong.push(format!(
                        "{label}: with no install {before:?}, want {want:?}"
                    )),
                }
                if after != before {
                    wrong.push(format!(
                        "{label}: no install {before:?}, installed {after:?}"
                    ));
                }
                if !ips.contains(&rets[ran]) {
                    wrong.push(format!("{label}: the unit code did not run"));
                }
            }
            assert!(wrong.is_empty(), "{wrong:#?}");
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        }
    }

    /// A class constructor with unit code keeps the TypeError of `[[Call]]` (§10.2.1 step 2)
    /// on every call path, as with no install: a T0 call (`slow::call` throws before its
    /// native arms), a forwarded `Function.prototype.call`, a call from Rust (`Cx::call`:
    /// `call_bytecode` throws first) and the Rust native entry (`native_callee` refuses it, so
    /// `call_native` does not enter it). Its unit code never runs on these paths.
    #[test]
    fn unit_loader_a_class_constructor_with_unit_code_still_throws_when_called() {
        let (mut plain, _) = class_isolate(false, false);
        let (mut iso, _) = class_isolate(true, false);
        for name in ["B", "D"] {
            for u in [call_walk as fn(&str) -> VerifiedUnit, fcall_walk] {
                let before = shown_in(&mut plain, u(name), false);
                WALKED.with(|x| x.borrow_mut().take());
                let after = shown_in(&mut iso, u(name), false);
                assert!(WALKED.with(|x| x.borrow_mut().take()).is_none(), "{name}");
                assert_eq!(after, before, "{name}");
                let e = after.unwrap_err();
                assert!(
                    e.starts_with(&format!(
                        "TypeError: Class constructor {name} cannot be invoked without 'new'"
                    )),
                    "{name}: {e}"
                );
            }
            let (f, w) = (
                iso.global_raw(name).unwrap(),
                iso.global_raw("walk").unwrap(),
            );
            let e = iso.call_raw(f, Value::UNDEFINED, &[w]).unwrap_err();
            assert!(
                e.message.contains("cannot be invoked without 'new'"),
                "{name}"
            );
            assert!(call_through_entry(&mut iso, f, &[w]).is_none(), "{name}");
            assert!(
                crate::native::native_callee(f.to_bits()).is_none(),
                "{name}"
            );
            assert!(
                crate::native::native_construct_callee(f.to_bits()).is_some(),
                "{name}"
            );
        }
    }
}
