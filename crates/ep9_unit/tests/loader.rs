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

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
                    let vars: Vec<Vec<(u8, u8, u32, u32, u64, &[u32])>> = entries
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
                    let fns: Vec<(u32, [u8; 16], &[(u8, u8, u32, u32, u64, &[u32])])> = entries
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
