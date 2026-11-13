//! A unit from bytes to a run (M7-P4 A2-LOADER; plan v12 §7.3; DL M6-ORCH-299): the H35 S4
//! checks `ep9_unit::check` (steps 1-4), then the `ep9_vm` unit loader (steps 5-8,
//! `Cx::run_script_with_unit_code`). `ep9_unit` depends on `ep9_vm`, so this end-to-end test
//! lives here.
//!
//! ```text
//! JS --ep9_frontend--> VerifiedUnit --to_bytes--> bytecode section
//! digests (Cx::unit_function_digest) --functions::encode--> rodata (the C-17 table)
//! text section (never run in M7-P4), source section --TestUnit::encode--> unit bytes
//! check(bytes, HostInfo::current()) == Load --> sections back --> decode, verify
//!   --> run_script_with_unit_code: counts by reason; every function runs its bytecode
//! ```
//!
//! | Test | What it shows |
//! |---|---|
//! | `a_checked_unit_loads_its_code_sets_and_runs_its_bytecode` | the jitless preset picks each accepted variant; the jit-lowering variant is skipped (MC-p0-cx-1); nothing is installed (MC-w30-2); a changed function (another digest) runs its bytecode (MC-w31-8) |
//! | `the_object_model_is_the_same_with_and_without_a_loaded_unit` | doc 13 M-R9 with a loaded unit (route A2-MR9; the w16 O2 workload form, `ep9_builtins/tests/presets_object_model.rs`): the same literal shapes and elements kinds, and the same result |
//! | `unit_class_constructors_from_js_give_the_bytecode_results` | M8-P4 A2-GATE (MC-w29-8 step 2, MC-w33-6): class constructors compiled from JS, installed from checked unit bytes (a counting frame stub each): base, derived with and without `super()`, a result of 1, fields, `Reflect.construct`, another realm, and a call without `new`: the bytecode results, and the unit code each case names ran |
//! | `unit_class_constructors_load_path_parity_has_no_fallback` | the MC-w29-8 load-path parity test: every construct of an installed unit class constructor runs its unit code (0 fallbacks) |

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use common::{TestUnit, header_for};
use ep9_bytecode::{CompiledUnit, UnitString, VerifiedUnit};
use ep9_facts::{ElementsKind, FactSnapshot, ShapeDesc, SlotFacts};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_unit::functions::{self, Entry, NO_META, Variant, VariantKind, lowering};
use ep9_unit::{HostInfo, Outcome, SectionKind, check};
use ep9_vm::abi::export_facts;
use ep9_vm::{ExecOptions, Isolate, IsolateConfig};

/// Objects, arrays that move through elements kinds, constructor objects; no builtin method
/// (this crate links no builtin table). It prints a digest of the run (a host function).
const WORKLOAD: &str = r#"
function Point(x, y) { this.x = x; this.y = y; }
function leaf(i) { return { a: [0, 1, 2, 3], tag: "leaf " + i }; }
function grow(n) {
  var a = [1, 2, 3];
  if (n % 3 == 0) a[1] = 1.5;
  if (n % 7 == 0) a[0] = "s";
  return a;
}
function same() { return "unit text"; }
var sum = 0, keep = [];
for (var i = 0; i < 120; i++) {
  var p = new Point(i, i + 1);
  var l = leaf(i);
  var g = grow(i);
  sum += p.x + p.y + l.a.length + g.length;
  keep[i % 8] = l;
}
print(sum + ":" + same() + ":" + keep[3].tag);
"#;

/// The text section: 64 bytes that never run in M7-P4 (no unit code is installed).
const TEXT_LEN: usize = 64;

fn compile() -> VerifiedUnit {
    ep9_frontend::compile(
        WORKLOAD,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the workload compiles")
}

/// The index of the function named `name` in `cu`.
fn index(cu: &CompiledUnit, name: &str) -> u32 {
    let pos = cu
        .functions
        .iter()
        .position(|f| {
            f.name
                .is_some_and(|id| matches!(&cu.strings[id.index()], UnitString::Latin1(b) if **b == *name.as_bytes()))
        })
        .unwrap_or_else(|| panic!("no function {name}"));
    pos as u32
}

fn isolate(exec: ExecOptions) -> Isolate {
    Isolate::new_with(IsolateConfig::default(), exec).expect("a preset this build runs")
}

/// The writer's step: the binding digest of function `f` (`Cx::unit_function_digest`).
fn digest(unit: &VerifiedUnit, f: u32) -> [u8; 16] {
    isolate(ExecOptions::jitless())
        .with_cx(|cx| Ok(cx.unit_function_digest(unit, f)))
        .unwrap()
        .expect("a function of the unit")
}

fn variant(lowering: u8, entry: u32) -> Variant {
    Variant {
        kind: VariantKind::AotBaseline,
        lowering,
        tag: 0,
        entry,
        text_len: 8,
        meta: NO_META,
        meta_len: 0,
        fuse_refs: 0,
        fuse_count: 0,
        cpu_required: 0,
    }
}

/// The unit bytes: the four named functions, each with a jitless variant; `leaf` also with a
/// variant of lowering jit (never stored in a unit: the loader skips it). `stale` names a
/// function whose digest is taken from another function (its bytecode differs).
fn unit_bytes(unit: &VerifiedUnit, stale: Option<&str>) -> Vec<u8> {
    let cu = unit.unit();
    let mut entries: Vec<Entry> = ["Point", "leaf", "grow", "same"]
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let f = index(cu, name);
            let mut variants = vec![variant(lowering::JITLESS, 8 * k as u32)];
            if *name == "leaf" {
                variants.push(variant(lowering::JIT, 32));
            }
            let from = if stale == Some(*name) { 0 } else { f };
            Entry {
                function: f,
                digest: digest(unit, from),
                variants,
            }
        })
        .collect();
    entries.sort_by_key(|e| e.function);
    let mut header = header_for(&HostInfo::current(), 0);
    header.source_flags = 0; // a sloppy script
    TestUnit {
        header,
        sections: vec![
            (SectionKind::Text, vec![0xCC; TEXT_LEN]),
            (SectionKind::Rodata, functions::encode(&entries)),
            (SectionKind::Bytecode, cu.to_bytes()),
            (SectionKind::Source, WORKLOAD.as_bytes().to_vec()),
        ],
        extensions: Vec::new(),
    }
    .encode()
}

/// The embedder's steps: check, sections back, the bytecode verified, the table decoded.
fn open(bytes: &[u8]) -> (VerifiedUnit, Vec<u8>, Vec<Entry>) {
    assert_eq!(check(bytes, &HostInfo::current()), Outcome::Load);
    let t = TestUnit::parse(bytes).unwrap();
    let section = |k: SectionKind| {
        t.sections
            .iter()
            .find(|(kind, _)| *kind == k)
            .map(|(_, d)| d.clone())
            .unwrap()
    };
    let text = section(SectionKind::Text);
    let cu = ep9_bytecode::decode_unit(&section(SectionKind::Bytecode)).unwrap();
    let unit = ep9_bytecode::verify(cu).unwrap();
    let (entries, _) = functions::decode(&section(SectionKind::Rodata), text.len()).unwrap();
    (unit, text, entries)
}

/// A variant as `Cx::run_script_with_unit_code` takes it: (dispatch kind, lowering, entry,
/// length, CPU bits, fuse ids).
type Var<'a> = (u8, u8, u32, u32, u64, &'a [u32]);

/// One run: with `code` the unit's native part goes to the loader; returns the counts (empty
/// without), the printed text and the facts.
fn run(
    exec: ExecOptions,
    unit: VerifiedUnit,
    code: Option<(&[u8], &[Entry])>,
) -> (Vec<(&'static str, u32)>, String, FactSnapshot) {
    let hash = unit.content_hash();
    let mut iso = isolate(exec);
    PRINTED.with(|p| p.borrow_mut().clear());
    iso.register_host_function("print", 1, print);
    let rows = iso
        .with_cx(|cx| {
            let (rows, r) = match code {
                Some((text, entries)) => {
                    let vars: Vec<Vec<Var<'_>>> = entries
                        .iter()
                        .map(|e| {
                            e.variants
                                .iter()
                                .map(|v| {
                                    let none: &[u32] = &[];
                                    (
                                        v.kind.code(),
                                        v.lowering,
                                        v.entry,
                                        v.text_len,
                                        v.cpu_required,
                                        none,
                                    )
                                })
                                .collect()
                        })
                        .collect();
                    let fns: Vec<(u32, [u8; 16], &[Var<'_>])> = entries
                        .iter()
                        .zip(&vars)
                        .map(|(e, v)| (e.function, e.digest, &v[..]))
                        .collect();
                    let cpu = HostInfo::current().cpu_features;
                    cx.run_script_with_unit_code(unit, "unit.js", text, cpu, &fns)
                }
                None => (Vec::new(), cx.run_script(unit, "unit.js")),
            };
            r.map(|_| rows)
        })
        .unwrap_or_else(|e| panic!("the workload threw: {}", e.message));
    iso.collect_garbage();
    iso.verify_heap().expect("a clean heap");
    let facts = export_facts(&iso, &hash).expect("the unit is loaded");
    (rows, PRINTED.with(|p| p.borrow().clone()), facts)
}

thread_local! {
    static PRINTED: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn print<'s>(
    cx: &mut ep9_vm::Cx<'s>,
    _this: ep9_vm::Local<'s>,
    args: ep9_vm::Args<'s>,
    _nt: ep9_vm::Local<'s>,
) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    PRINTED.with(|p| p.borrow_mut().push_str(&parts.join(" ")));
    Ok(ep9_vm::Local::undefined())
}

fn count(rows: &[(&'static str, u32)], name: &str) -> u32 {
    rows.iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no count {name}"))
        .1
}

/// The jitless preset: 4 picks (one per function), the jit variant skipped, nothing
/// installed, and the bytecode result. A stale digest: that function runs its bytecode.
#[test]
fn a_checked_unit_loads_its_code_sets_and_runs_its_bytecode() {
    let unit = compile();
    let (u, text, entries) = open(&unit_bytes(&unit, None));
    assert_eq!(entries.len(), 4);
    let (rows, result, _) = run(ExecOptions::jitless(), u, Some((&text, &entries)));
    let want = [
        ("no-text", 0),
        ("digest", 0),
        ("jit-lowering", 1),
        ("picked", 4),
        ("deferred", 4),
        ("installed", 0),
    ];
    for (name, n) in want {
        assert_eq!(count(&rows, name), n, "{name}: {rows:?}");
    }
    let (_, plain, _) = run(ExecOptions::jitless(), compile(), None);
    assert_eq!(result, plain);
    assert!(result.ends_with(":unit text:leaf 115"), "{result}");
    // A stale digest for `grow`.
    let (u, text, entries) = open(&unit_bytes(&unit, Some("grow")));
    let (rows, stale, _) = run(ExecOptions::jitless(), u, Some((&text, &entries)));
    assert_eq!((count(&rows, "digest"), count(&rows, "picked")), (1, 3));
    assert_eq!(stale, plain);
}

/// A literal site's result shape and elements kind.
type LiteralSite = (Option<Arc<ShapeDesc>>, Option<ElementsKind>);

/// The object-model part of a snapshot (the `presets_object_model.rs` form): per literal site
/// its result shape and elements kind, and the set of every shape the facts name.
fn object_model(facts: &FactSnapshot) -> (BTreeMap<(u32, u32), LiteralSite>, BTreeSet<ShapeDesc>) {
    let mut literals = BTreeMap::new();
    let mut shapes = BTreeSet::new();
    for (key, f) in facts.records() {
        if let SlotFacts::Literal(l) = f {
            literals.insert((key.function, key.slot), (l.shape.clone(), l.elements));
        }
        f.for_each_shape(&mut |s: &ShapeDesc| {
            shapes.insert(s.clone());
        });
    }
    (literals, shapes)
}

/// Doc 13 M-R9 with a loaded unit (A2-MR9): under every preset this build has, a run with the
/// unit's native part loaded makes the same shapes and elements kinds as a run without it.
#[test]
fn the_object_model_is_the_same_with_and_without_a_loaded_unit() {
    let unit = compile();
    let bytes = unit_bytes(&unit, None);
    let mut presets = vec![
        ExecOptions::jitless(),
        ExecOptions::jitless().feedback(ep9_vm::Feedback::FULL),
    ];
    if ep9_vm::Preset::JIT.available() {
        presets.push(ExecOptions::jit());
    }
    for exec in presets {
        let (u, text, entries) = open(&bytes);
        let (rows, with, f1) = run(exec, u, Some((&text, &entries)));
        assert!(count(&rows, "picked") <= 4);
        let (_, without, f0) = run(exec, compile(), None);
        assert_eq!(with, without);
        let (om1, om0) = (object_model(&f1), object_model(&f0));
        assert!(
            !om0.0.is_empty() && !om0.1.is_empty(),
            "the workload makes literal facts"
        );
        assert_eq!(om1, om0, "{}", exec.preset().name());
    }
}

// ---- M8-P4 A2-GATE: unit class constructors compiled from JS (MC-w29-8 step 2) --------------
//
// ```text
// CLASSES --ep9_frontend--> B, D: `Ldar a0; CallUndefinedReceiver0 a0 (offset 2); ...`
// text: a counting frame stub per function (count += 1; return call_any(cx, fp, 2, 0)),
//   unwind form 1 of the host --TestUnit::encode--> unit bytes --check--> Load
//   --Cx::run_script_installing_unit_code--> B and D installed (AotBaseline)
// each case: the same script with no install and with the unit code: the same result, and
//   the count says which unit code ran (the load-path parity of MC-w29-8: 0 fallbacks)
// ```

/// Two class constructors with unit code (`B`, a base class, and `D extends B`, a derived
/// class that never calls `super()`), two T0 classes that call `super()` into `B` (`E`, and
/// `F` with a field that runs on the receiver that `B`'s unit code made), the prototype tags
/// and `var` copies for another realm. No builtin method (this crate links no builtin table).
const CLASSES: &str = r#"
class B { constructor(f) { return f(); } }
class D extends B { constructor(f) { return f(); } }
class E extends B { constructor(f) { super(f); } }
class F extends B { y = 2; constructor(f) { super(f); } }
B.prototype.tag = "B"; D.prototype.tag = "D"; E.prototype.tag = "E"; F.prototype.tag = "F";
var gB = B, gD = D, gE = E, gF = F;
"#;

fn compile_src(src: &str) -> VerifiedUnit {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the script compiles")
}

/// The bytecode offset of the one call of `B` and of `D` (the offset the frame stub passes).
const CALL_AT: u32 = 2;

/// The opcodes of function `f` of `cu`, with their offsets.
fn opcodes(cu: &CompiledUnit, f: u32) -> Vec<(u32, ep9_bytecode::Opcode)> {
    ep9_bytecode::Instructions::new(&cu.functions[f as usize].bytecode)
        .map(|i| i.map(|i| (i.offset, i.opcode)).unwrap())
        .collect()
}

/// The text offset of the stub of function `k` (0: `B`, 1: `D`).
const STUB_AT: [usize; 2] = [0, 64];
/// The Win64 UNWIND_INFO offset in the text.
const WIN_INFO_AT: usize = 256;

/// The counting frame stub at `at`: it adds 1 to the u64 at `counter`, then runs the call at
/// bytecode offset [`CALL_AT`] of its frame through `RuntimeTable::call_any` and returns its
/// result (the `NativeEntry` ABI). Returns the code and its length without the literal.
fn counting_stub(counter: usize) -> (Vec<u8>, usize) {
    let table = ep9_vm::layout::CX_RUNTIME_TABLE;
    let call_any = ep9_vm::layout::RT_CALL_ANY;
    #[cfg(target_arch = "aarch64")]
    {
        let ldr = |t: u32, n: u32, off: usize| {
            assert!(off.is_multiple_of(8) && off / 8 < 4096);
            0xF940_0000 | (((off / 8) as u32) << 10) | (n << 5) | t
        };
        let w = [
            0xA9BF_7BFD,                  // stp x29, x30, [sp, #-16]!
            0x9100_03FD,                  // mov x29, sp
            0x5800_0189,                  // ldr x9, <the literal at 56>
            0xF940_012A,                  // ldr x10, [x9]
            0x9100_054A,                  // add x10, x10, #1
            0xF900_012A,                  // str x10, [x9]
            ldr(16, 0, table),            // ldr x16, [x0, #runtime_table]
            ldr(16, 16, call_any),        // ldr x16, [x16, #call_any]
            0x5280_0002 | (CALL_AT << 5), // mov w2, #CALL_AT
            0xAA1F_03E3,                  // mov x3, xzr
            0xD63F_0200,                  // blr x16
            0xA8C1_7BFD,                  // ldp x29, x30, [sp], #16
            0xD65F_03C0,                  // ret
            0xD503_201F,                  // nop (the literal is 8-aligned)
        ];
        let mut v: Vec<u8> = w.iter().flat_map(|w| w.to_le_bytes()).collect();
        let len = v.len() - 4;
        v.extend((counter as u64).to_le_bytes());
        assert_eq!(v.len(), 64);
        (v, len)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let table = i32::try_from(table).unwrap();
        let e = u8::try_from(call_any).unwrap();
        assert!(table > 127 && e < 128);
        let mut v = vec![0x55, 0x48, 0x89, 0xE5]; // push rbp; mov rbp, rsp
        v.extend([0x48, 0xB8]); // mov rax, imm64
        v.extend((counter as u64).to_le_bytes());
        v.extend([0x48, 0xFF, 0x00]); // inc qword ptr [rax]
        v.extend([0x48, 0x8B, 0x87]); // mov rax, [rdi + disp32]
        v.extend(table.to_le_bytes());
        v.push(0xBA); // mov edx, CALL_AT
        v.extend(CALL_AT.to_le_bytes());
        v.extend([0x31, 0xC9]); // xor ecx, ecx
        v.extend([0xFF, 0x50, e]); // call [rax + disp8]
        v.extend([0x5D, 0xC3]); // pop rbp; ret
        let len = v.len();
        (v, len)
    }
}

/// The unwind section of form 1 for the two stubs (each `len` bytes at [`STUB_AT`]): on unix
/// one CIE and an FDE per stub (the `tests/obj.rs` records: the frame record after the first
/// instruction), on Windows a RUNTIME_FUNCTION per stub over the UNWIND_INFO at
/// [`WIN_INFO_AT`].
fn stub_unwind(len: usize) -> Vec<u8> {
    if cfg!(windows) {
        let mut v = Vec::new();
        for at in STUB_AT {
            for w in [at, at + len, WIN_INFO_AT] {
                v.extend((w as u32).to_le_bytes());
            }
        }
        return v;
    }
    let (cie_tail, fde): (&[u8], &[u8]) = if cfg!(target_arch = "aarch64") {
        (
            &[4, 0x78, 30, 1, 0, 0x0c, 31, 0],
            &[0, 0x41, 0x0e, 16, 0x9e, 1, 0x9d, 2],
        )
    } else {
        (
            &[1, 0x78, 16, 1, 0, 0x0c, 7, 8, 0x90, 1],
            &[0, 0x41, 0x0e, 16, 0x86, 2, 0x43, 0x0d, 6],
        )
    };
    let record = |out: &mut Vec<u8>, body: &[u8]| {
        let n = (4 + body.len()).next_multiple_of(8) - 4;
        out.extend((n as u32).to_le_bytes());
        out.extend(body);
        out.resize(out.len() + n - body.len(), 0);
    };
    let mut out = Vec::new();
    let mut cie = vec![0, 0, 0, 0, 1, b'z', b'R', 0];
    cie.extend(cie_tail);
    record(&mut out, &cie);
    for at in STUB_AT {
        let mut body = ((out.len() + 4) as u32).to_le_bytes().to_vec();
        body.extend((at as u64).to_le_bytes());
        body.extend((len as u64).to_le_bytes());
        body.extend(fde);
        record(&mut out, &body);
    }
    out.extend([0; 4]);
    out
}

/// The unit bytes of [`CLASSES`]: a jitless AotBaseline variant for `B` and `D` over a
/// counting stub that adds to `counters[0]` and `counters[1]`.
fn class_unit_bytes(unit: &VerifiedUnit, counters: &'static [AtomicU64; 2]) -> Vec<u8> {
    let cu = unit.unit();
    let mut text = Vec::new();
    let mut entries = Vec::new();
    let mut len = 0;
    for (k, name) in ["B", "D"].into_iter().enumerate() {
        let (code, n) = counting_stub(counters[k].as_ptr().addr());
        text.resize(STUB_AT[k], 0xCC);
        text.extend(code);
        len = n;
        let f = index(cu, name);
        let mut v = variant(lowering::JITLESS, STUB_AT[k] as u32);
        v.text_len = n as u32;
        entries.push(Entry {
            function: f,
            digest: digest(unit, f),
            variants: vec![v],
        });
    }
    if cfg!(windows) {
        // Version 1, a 1-byte prolog, 1 code: UWOP_PUSH_NONVOL rbp at offset 1.
        text.resize(WIN_INFO_AT, 0xCC);
        text.extend([0x01, 0x01, 0x01, 0x00, 0x01, 0x50, 0x00, 0x00]);
    }
    entries.sort_by_key(|e| e.function);
    let mut header = header_for(&HostInfo::current(), 0);
    header.source_flags = 0; // a sloppy script
    TestUnit {
        header,
        sections: vec![
            (SectionKind::Text, text),
            (SectionKind::Rodata, functions::encode(&entries)),
            (SectionKind::Bytecode, cu.to_bytes()),
            (SectionKind::Source, CLASSES.as_bytes().to_vec()),
            (SectionKind::Unwind, stub_unwind(len)),
        ],
        extensions: Vec::new(),
    }
    .encode()
}

/// `walk()` 42, `mkobj()` a new object with `tag` "own", `undef()` undefined, `one()` 1, and
/// `rconstruct(f, a, nt)` = `Reflect.construct(f, [a], nt)` (the Rust construct path).
fn class_hosts(iso: &mut Isolate) {
    fn walk<'s>(
        _cx: &mut ep9_vm::Cx<'s>,
        _this: ep9_vm::Local<'s>,
        _args: ep9_vm::Args<'s>,
        _nt: ep9_vm::Local<'s>,
    ) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
        Ok(ep9_vm::Local::int32(42))
    }
    fn mkobj<'s>(
        cx: &mut ep9_vm::Cx<'s>,
        _this: ep9_vm::Local<'s>,
        _args: ep9_vm::Args<'s>,
        _nt: ep9_vm::Local<'s>,
    ) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
        let o = cx.new_object();
        let tag = cx.new_string("own")?;
        cx.set(o, "tag", tag, true)?;
        Ok(o.into())
    }
    fn undef<'s>(
        _cx: &mut ep9_vm::Cx<'s>,
        _this: ep9_vm::Local<'s>,
        _args: ep9_vm::Args<'s>,
        _nt: ep9_vm::Local<'s>,
    ) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
        Ok(ep9_vm::Local::undefined())
    }
    fn one<'s>(
        _cx: &mut ep9_vm::Cx<'s>,
        _this: ep9_vm::Local<'s>,
        _args: ep9_vm::Args<'s>,
        _nt: ep9_vm::Local<'s>,
    ) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
        Ok(ep9_vm::Local::int32(1))
    }
    fn rconstruct<'s>(
        cx: &mut ep9_vm::Cx<'s>,
        _this: ep9_vm::Local<'s>,
        args: ep9_vm::Args<'s>,
        _nt: ep9_vm::Local<'s>,
    ) -> ep9_vm::JsResult<ep9_vm::Local<'s>> {
        cx.construct(args.get(0), &[args.get(1)], args.get(2))
            .map(|o| o.into())
    }
    for (name, n, f) in [
        ("walk", 0, walk as ep9_vm::HostFn),
        ("mkobj", 0, mkobj),
        ("undef", 0, undef),
        ("one", 0, one),
        ("rconstruct", 3, rconstruct),
        ("print", 1, print),
    ] {
        iso.register_host_function(name, n, f);
    }
}

/// A jitless isolate with [`class_hosts`] that ran [`CLASSES`] (with `counters`: from checked
/// unit bytes, with `B` and `D` installed; else as plain bytecode), and the realm `other` with
/// the `var` copies and the host functions.
fn class_isolate(counters: Option<&'static [AtomicU64; 2]>) -> Isolate {
    let mut iso = isolate(ExecOptions::jitless());
    class_hosts(&mut iso);
    let unit = compile_src(CLASSES);
    // The stub runs the bytecode of B and D: the call at CALL_AT and its result (B: then
    // `Return`; D: `JumpIfUndefined`, `Return`, and for undefined `LdaTheHole; Return`: the
    // stub returns undefined there, and the caller's check reads undefined and the hole the
    // same way, `interp::derived_result_error`: the ReferenceError of step 12).
    {
        use ep9_bytecode::Opcode as O;
        let cu = unit.unit();
        let head = |name| opcodes(cu, index(cu, name))[..3].to_vec();
        let want = |third| {
            vec![
                (0, O::Ldar),
                (CALL_AT, O::CallUndefinedReceiver0),
                (5, third),
            ]
        };
        assert_eq!(head("B"), want(O::Return));
        assert_eq!(head("D"), want(O::JumpIfUndefined));
    }
    let rows = iso
        .with_cx(|cx| {
            let Some(counters) = counters else {
                return cx.run_script(unit, "classes.js").map(|_| Vec::new());
            };
            let bytes = class_unit_bytes(&unit, counters);
            let (u, text, entries) = open(&bytes);
            let t = TestUnit::parse(&bytes).unwrap();
            let unwind = t
                .sections
                .iter()
                .find(|(k, _)| *k == SectionKind::Unwind)
                .map(|(_, d)| d.clone())
                .unwrap();
            let vars: Vec<Vec<Var<'_>>> = entries
                .iter()
                .map(|e| {
                    let v = &e.variants[0];
                    let none: &[u32] = &[];
                    vec![(v.kind.code(), v.lowering, v.entry, v.text_len, 0, none)]
                })
                .collect();
            let fns: Vec<(u32, [u8; 16], &[Var<'_>])> = entries
                .iter()
                .zip(&vars)
                .map(|(e, v)| (e.function, e.digest, &v[..]))
                .collect();
            let cpu = HostInfo::current().cpu_features;
            // SAFETY: `check` gave Load on `bytes` (`open`), and `u`, `text`, `unwind` and
            // `fns` are its sections, unchanged. Each variant's code is the counting stub of
            // its function: it runs the bytecode of `B` and `D` (the asserts above) and adds
            // 1 to a test counter, and `unwind` describes both stubs. No RW data.
            let (rows, r) = unsafe {
                cx.run_script_installing_unit_code(
                    u,
                    "classes.js",
                    &text,
                    &unwind,
                    &[],
                    0,
                    cpu,
                    &fns,
                )
            };
            r.map(|_| rows)
        })
        .unwrap_or_else(|e| panic!("the classes threw: {}", e.message));
    if counters.is_some() {
        assert_eq!(count(&rows, "installed"), 2, "{rows:?}");
        assert_eq!(count(&rows, "not-installable"), 0, "{rows:?}");
    }
    iso.with_cx(|cx| {
        let a = cx.global();
        let b = cx.create_realm()?;
        cx.set(a, "other", b, true)?;
        for name in [
            "gB",
            "gD",
            "gE",
            "gF",
            "walk",
            "mkobj",
            "undef",
            "one",
            "rconstruct",
            "print",
        ] {
            let v = cx.get(a, name)?;
            cx.set(b, name, v, true)?;
        }
        Ok(())
    })
    .unwrap();
    iso
}

/// Runs `print(<expr>)` in `iso` (in the realm `other` when `in_other`): the printed text,
/// or the exception message.
fn shown(iso: &mut Isolate, expr: &str, in_other: bool) -> Result<String, String> {
    let unit = compile_src(&format!("print({expr});"));
    PRINTED.with(|p| p.borrow_mut().clear());
    iso.with_cx(|cx| {
        if in_other {
            let a = cx.global();
            let b = cx.get(a, "other")?.as_object().unwrap();
            cx.with_realm(b, |cx| cx.run_script(unit, "case.js"))?;
        } else {
            cx.run_script(unit, "case.js")?;
        }
        Ok(())
    })
    .map_err(|e| e.message)?;
    Ok(PRINTED.with(|p| p.borrow().clone()))
}

static CASE_COUNTS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static PARITY_COUNTS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

/// MC-w29-8 step 2 under the MC-w33-6 contract (M7-P5-D7), from JS: a unit base class `B` and
/// a unit derived class `D` give, for each construct case, the result, receiver and error of
/// the same classes as bytecode, and the case runs the unit code it names (B 0, D 1). `E` and
/// `F` call `super()` into `B` (F's field `y` is defined on the receiver that `B`'s unit code
/// returns, and on the object a body returns); `D` with no `super()` gives the caller's
/// ReferenceError, with 1 its TypeError, also from another realm and through
/// `Reflect.construct`; a call of `B` or `D` throws the TypeError and runs no unit code.
#[test]
fn unit_class_constructors_from_js_give_the_bytecode_results() {
    type Case = (
        &'static str,
        bool,
        Result<&'static str, &'static str>,
        [u64; 2],
    );
    let cases: [Case; 17] = [
        ("new B(walk).tag", false, Ok("B"), [1, 0]),
        ("new B(mkobj).tag", false, Ok("own"), [1, 0]),
        ("new B(undef).tag", false, Ok("B"), [1, 0]),
        ("new E(walk).tag", false, Ok("E"), [1, 0]),
        ("new F(walk).tag + new F(walk).y", false, Ok("F2"), [2, 0]),
        (
            "new F(mkobj).tag + new F(mkobj).y",
            false,
            Ok("own2"),
            [2, 0],
        ),
        ("rconstruct(B, walk, D).tag", false, Ok("D"), [1, 0]),
        ("new gB(walk).tag", true, Ok("B"), [1, 0]),
        ("new gE(walk).tag", true, Ok("E"), [1, 0]),
        ("new D(mkobj).tag", false, Ok("own"), [0, 1]),
        ("new D(undef)", false, Err("ReferenceError"), [0, 1]),
        ("new D(one)", false, Err("TypeError"), [0, 1]),
        (
            "rconstruct(D, undef, D)",
            false,
            Err("ReferenceError"),
            [0, 1],
        ),
        ("new gD(undef)", true, Err("ReferenceError"), [0, 1]),
        ("new gD(one)", true, Err("TypeError"), [0, 1]),
        (
            "B(walk)",
            false,
            Err("TypeError: Class constructor B cannot be invoked without 'new'"),
            [0, 0],
        ),
        (
            "D(walk)",
            false,
            Err("TypeError: Class constructor D cannot be invoked without 'new'"),
            [0, 0],
        ),
    ];
    let mut plain = class_isolate(None);
    let mut iso = class_isolate(Some(&CASE_COUNTS));
    let read = || CASE_COUNTS.each_ref().map(|c| c.load(Relaxed));
    let mut wrong = Vec::new();
    for (src, in_other, want, ran) in cases {
        let before = shown(&mut plain, src, in_other);
        let n0 = read();
        let after = shown(&mut iso, src, in_other);
        let n1 = read();
        match (&before, want) {
            (Ok(v), Ok(w)) if v == w => {}
            (Err(e), Err(w)) if e.starts_with(w) => {}
            _ => wrong.push(format!("{src}: with no install {before:?}, want {want:?}")),
        }
        if after != before {
            wrong.push(format!("{src}: no install {before:?}, installed {after:?}"));
        }
        let delta = [n1[0] - n0[0], n1[1] - n0[1]];
        if delta != ran {
            wrong.push(format!("{src}: unit code runs {delta:?}, want {ran:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    iso.collect_garbage();
    iso.verify_heap().expect("a clean heap");
}

/// The MC-w29-8 load-path parity test (JSC c823243598c form): with `B` and `D` installed from
/// checked unit bytes, every construct of them runs their unit code, with 0 fallbacks to the
/// bytecode: 300 constructs of `B` (100 `new B`, 100 through `super()` from `E`, 100 through
/// `Reflect.construct`) and 100 of `D` count 300 and 100, and the results equal a run with no
/// install.
#[test]
fn unit_class_constructors_load_path_parity_has_no_fallback() {
    let src = r#"(function () {
  var s = 0;
  for (var i = 0; i < 100; i++) {
    s += new B(walk).tag.length + new E(walk).tag.length + rconstruct(B, walk, B).tag.length;
    s += new D(mkobj).tag.length;
  }
  return s;
})()"#;
    let mut plain = class_isolate(None);
    let mut iso = class_isolate(Some(&PARITY_COUNTS));
    let before = shown(&mut plain, src, false);
    let after = shown(&mut iso, src, false);
    assert_eq!(before, Ok("600".to_string()));
    assert_eq!(after, before);
    let ran = PARITY_COUNTS.each_ref().map(|c| c.load(Relaxed));
    assert_eq!(ran, [300, 100], "constructs that ran unit code (B, D)");
}
