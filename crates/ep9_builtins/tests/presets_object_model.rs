//! The object model reads no execution policy (doc 13 M-R9, MODES-D19, plan v10 rule 40; the
//! w16 O2 test, M5-P2 ELEM2; the vector axis, M5-P3 PROFILE6 for MODES-CHECK-w20 MC-w20-4 (2)
//! and MC-w20-15, orchestrator M5-P3-ORCH-1 option A and ORCH-2). A workload of object and
//! array literals, keyed stores that move arrays through the elements kinds (H18), DICTIONARY
//! arrays, constructor objects and a clone probe (`{...s}` twice, then `q.x = 7`) runs under
//! every preset and feedback setter this build has (`jit` with the `jit` feature, M-R6;
//! `jitless` with `ics_only`, its default, and with `full`) and every heap config (S0, S1, and
//! both with GC stress). At one feedback threshold (0, and the default) every run must make the
//! same shapes and elements kinds: the per-site literal facts (result shape with its slack
//! capacity, ALLOC_SITE elements kind), the set of shapes the IC and clone facts name, and the
//! count of literal sites per elements kind. The ALLOC_SITE cell (V10-D6) is an IC input, so
//! `ics_only` keeps it too. Since M5-P3 `ics_only` runs its own T0 handler table (no profile
//! record), so a failure means an object-model input reads the setter or the table.
//!
//! The threshold is one run value for every preset (ORCH-1 option A), a knob and not a mode:
//! at `u32::MAX` no function gets a vector, so the facts are empty here. Field representation
//! and const bits are not facts (`ShapeDesc` has none, MC-w20-5), and live shapes are not
//! visible through the public API, so the live-shape half of the vector axis (rep/const bits,
//! slack, elements kinds and the ALLOC_SITE kind on live objects, threshold 0 and the default
//! against `u32::MAX`, with the fields that differ named) is the ep9_vm test
//! `tests::profile::every_preset_and_heap_make_the_same_live_object_model_at_one_threshold`.
//!
//! This crate has no `jit` feature (M-R6, `Cargo.toml`): the `jit` preset is an error here
//! unless a workspace build turned the feature on for ep9_vm (feature unification), and the
//! test checks that the two agree.
//!
//! ep9_vm tests cannot compile JS, so this test lives here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ep9_bytecode::FeedbackSlotKind;
use ep9_facts::{ElementsKind, FactSnapshot, ShapeDesc, SlotFacts};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::abi::export_facts;
use ep9_vm::{ExecOptions, Feedback, Isolate, IsolateConfig, Preset};

/// Literals, kind transitions through stores, DICTIONARY arrays, constructors, the clone
/// probe of MC-w20-4 and a fresh array of a generalized literal site (its birth kind); prints a
/// digest so every run is checked for the same answer too. 120 rounds: the functions get
/// their vectors at the default threshold (8) inside the loop, and a GC-stress run stays short.
const WORKLOAD: &str = r#"
function Point(x, y) { this.x = x; this.y = y; }
function cl(s) { return {...s}; }
function leaf(i) { return { array: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9], tag: "leaf " + i }; }
function ints() { return [1, 2, 3]; }
function empty() { return []; }
function grow(n) {
  var a = ints();
  if (n % 3 == 0) a[1] = 1.5;          // the site's arrays become DOUBLE
  if (n % 7 == 0) a[0] = "s";          // ... and ELEMENTS
  return a;
}
var sum = 0, keep = [];
for (var i = 0; i < 120; i++) {
  var p = new Point(i, i + 1);
  var l = leaf(i);
  var g = grow(i);
  var e = empty();
  e.push(i);
  if (i % 5 == 0) e.push(0.25);
  sum += p.x + p.y + l.array[i % 10] + (typeof g[1] == "number" ? g[1] : 0) + e[0];
  if (i % 50 == 0) {
    var d = [];
    d[100000] = i;                      // DICTIONARY (H18-Q2)
    Object.defineProperty(d, "3", { get: function () { return 3; }, configurable: true });
    keep.push(d);
    sum += d[3] + d.length;
  }
  var src = { x: 1, y: i };
  var q = cl(src);
  q = cl(src);
  q.x = 7;                              // the clone probe (MC-w20-4)
  sum += q.x + q.y;
}
var born = ints();                      // the ALLOC_SITE birth kind of the ints() site
print(sum, keep.length, keep[0].length, born.length, typeof born[0]);
"#;

/// Every preset and feedback setter this build runs: `jit` with `full` (its default) and
/// `ics_only` (only with the `jit` feature, M-R6), `jitless` with `ics_only` (its default) and
/// `full`.
fn presets() -> Vec<ExecOptions> {
    let mut v = Vec::new();
    if Preset::JIT.available() {
        v.push(ExecOptions::jit());
        v.push(ExecOptions::jit().feedback(Feedback::ICS_ONLY));
    }
    v.push(ExecOptions::jitless());
    v.push(ExecOptions::jitless().feedback(Feedback::FULL));
    v
}

/// The heap configs (MC-w20-15): S0, S1, and both with GC stress (a collection before every
/// allocation, then the heap verifier; S1 stress: minor GCs, a major one every 64th).
fn heaps() -> [(&'static str, HeapConfig); 4] {
    let h = |generational: bool, stress: bool| HeapConfig {
        generational,
        gc_stress: u32::from(stress),
        verify_after_gc: stress,
        ..HeapConfig::default()
    };
    [
        ("S0", h(false, false)),
        ("S1", h(true, false)),
        ("S0+stress", h(false, true)),
        ("S1+stress", h(true, true)),
    ]
}

/// Runs `run` for each (options, heap name, heap, threshold), 8 at a time, each on its own
/// thread (an isolate belongs to its thread; a GC-stress run takes seconds, so the runs go in
/// parallel). Results in input order, each with its label.
fn run_all(
    cases: Vec<(ExecOptions, &'static str, HeapConfig, u32)>,
) -> Vec<(String, String, FactSnapshot)> {
    let mut out = Vec::new();
    for chunk in cases.chunks(8) {
        let handles: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|(exec, heap_name, heap, threshold)| {
                std::thread::Builder::new()
                    .stack_size(64 << 20)
                    .spawn(move || {
                        let (printed, facts) = run(exec, heap, threshold);
                        (label(exec, heap_name, threshold), printed, facts)
                    })
                    .expect("a test thread")
            })
            .collect();
        out.extend(handles.into_iter().map(|h| h.join().expect("the run")));
    }
    out
}

/// A label for one run.
fn label(exec: ExecOptions, heap: &str, threshold: u32) -> String {
    format!(
        "{} {} {heap} threshold {threshold}",
        exec.preset().name(),
        exec.feedback_mode().name()
    )
}

/// Runs the workload on a fresh isolate with `exec`, the heap config `heap` and the feedback
/// threshold `threshold`. The printed digest and the facts.
fn run(exec: ExecOptions, heap: HeapConfig, threshold: u32) -> (String, FactSnapshot) {
    run_src(WORKLOAD, exec, heap, threshold)
}

/// [`run`] with the script `src` in place of the workload.
fn run_src(
    src: &str,
    exec: ExecOptions,
    heap: HeapConfig,
    threshold: u32,
) -> (String, FactSnapshot) {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the workload compiles");
    let mut iso = Isolate::new_with(
        IsolateConfig {
            builtins: Some(&ep9_builtins::TABLE),
            feedback_threshold: threshold,
            heap,
            ..IsolateConfig::default()
        },
        exec,
    )
    .expect("a preset this build runs");
    PRINTED.with(|p| p.borrow_mut().clear());
    iso.register_host_function("print", 1, print);
    let u = unit.clone();
    iso.with_cx(|cx| cx.run_script(u, "om.js").map(|_| ()))
        .unwrap_or_else(|e| panic!("the workload threw: {}", e.message));
    let printed = PRINTED.with(|p| p.borrow().clone());
    iso.collect_garbage();
    iso.verify_heap().expect("a clean heap");
    let facts = export_facts(&iso, &unit.content_hash()).expect("the unit is loaded");
    (printed, facts)
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

/// The object-model part of a snapshot: per literal site (function, slot) its result shape
/// and elements kind; the set of every shape a slot's facts name; the literal sites per
/// elements kind. Counts of cold runs and every profile bit (type sets, keyed bits, call
/// targets) are left out: `ics_only` drops the record hooks that write them.
/// A literal site's result shape and elements kind.
type LiteralSite = (Option<Arc<ShapeDesc>>, Option<ElementsKind>);

#[derive(Debug, PartialEq, Eq)]
struct ObjectModel {
    literals: BTreeMap<(u32, u32), LiteralSite>,
    shapes: BTreeSet<ShapeDesc>,
    kinds: BTreeMap<Option<ElementsKind>, usize>,
}

fn object_model(facts: &FactSnapshot) -> ObjectModel {
    let mut literals = BTreeMap::new();
    let mut shapes = BTreeSet::new();
    let mut kinds = BTreeMap::new();
    for (key, f) in facts.records() {
        if let SlotFacts::Literal(l) = f {
            literals.insert((key.function, key.slot), (l.shape.clone(), l.elements));
            *kinds.entry(l.elements).or_insert(0) += 1;
        }
        f.for_each_shape(&mut |s: &ShapeDesc| {
            shapes.insert(s.clone());
        });
    }
    ObjectModel {
        literals,
        shapes,
        kinds,
    }
}

/// M-R6: the jitless preset runs in every build; the jit preset only where ep9_vm has the
/// `jit` feature, and then the isolate says so, never a silent downgrade.
#[test]
fn the_jit_preset_is_refused_without_the_feature() {
    assert!(Preset::JITLESS.available());
    let r = Isolate::new_with(IsolateConfig::default(), ExecOptions::jit());
    assert_eq!(
        r.is_ok(),
        Preset::JIT.available(),
        "the jit preset runs exactly when the build has the feature"
    );
}

/// The w16 O2 test on the vector axis (M-R9; MC-w20-4 (2), MC-w20-15): at threshold 0 and at
/// the default, every preset and feedback setter and every heap config make the same shapes
/// (slack capacities included) and elements kinds, and the same answer; the ALLOC_SITE facts
/// show the generalized literal kinds.
#[test]
fn every_preset_and_heap_make_the_same_shapes_and_elements_kinds_at_one_threshold() {
    for threshold in [0, IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD] {
        let mut first: Option<(String, String, ObjectModel)> = None;
        let cases = presets()
            .into_iter()
            .flat_map(|exec| heaps().map(|(n, h)| (exec, n, h, threshold)))
            .collect();
        for (l, out, facts) in run_all(cases) {
            let om = object_model(&facts);
            assert!(!om.literals.is_empty() && !om.shapes.is_empty(), "{l}");
            // The ALLOC_SITE kinds (V10-D6): `ints()` met DOUBLE and ELEMENTS stores, so
            // its site makes PACKED_ELEMENTS arrays; `empty()` met a push of 0.25: DOUBLE;
            // the nested array of `leaf` has no site (no slot of its own).
            let kinds: BTreeSet<ElementsKind> = om.kinds.keys().flatten().copied().collect();
            assert!(
                kinds.contains(&ElementsKind::PackedElements),
                "{l}: {:?}",
                om.kinds
            );
            assert!(
                kinds.contains(&ElementsKind::PackedDouble)
                    || kinds.contains(&ElementsKind::HoleyDouble),
                "{l}: {:?}",
                om.kinds
            );
            match &first {
                None => first = Some((l, out, om)),
                Some((l0, out0, om0)) => {
                    assert_eq!(&out, out0, "the same answer: {l} vs {l0}");
                    assert_eq!(
                        om.shapes.len(),
                        om0.shapes.len(),
                        "shape counts: {l} vs {l0}"
                    );
                    assert_eq!(om.kinds, om0.kinds, "elements-kind counts: {l} vs {l0}");
                    assert_eq!(&om, om0, "the object model: {l} vs {l0}");
                }
            }
        }
    }
}

/// The threshold `u32::MAX` (no vector) is a knob, not a mode: every preset gives the same
/// answer, and no function has facts, so the facts cannot show the object model there (the
/// ep9_vm live-shape test names what differs from threshold 0).
#[test]
fn with_no_vector_every_preset_gives_the_same_answer_and_no_facts() {
    let mut first: Option<(String, String)> = None;
    let cases = presets()
        .into_iter()
        .map(|exec| {
            let (n, h) = heaps()[0].clone();
            (exec, n, h, u32::MAX)
        })
        .collect();
    for (l, out, facts) in run_all(cases) {
        assert_eq!(facts.records().count(), 0, "{l}: no vector, no facts");
        match &first {
            None => first = Some((l, out)),
            Some((l0, out0)) => assert_eq!(&out, out0, "{l} vs {l0}"),
        }
    }
}

/// A script with a site of every record kind (MC-w21-1): binary, unary and compare operators,
/// `typeof`, keyed loads, stores, `in` and a computed literal key, an array literal with
/// computed elements, `for-in`, a loop, a closure and calls; and the IC and object-model kinds
/// (named loads and stores, globals, a construct site, `instanceof`, literals, a clone).
const RECORD_KINDS: &str = r#"
function Point(x, y) { this.x = x; this.y = y; }
function f(o, a, n) {
  var s = 0;
  for (var i = 0; i < n; i++) {
    s = s + a[i % a.length] * 1.5;
    a[i % a.length] = s;
    if (typeof o.x == "number") s += o.x;
    if (i in a) s++;
    s = -s;
    var ty = typeof a[0];
    var dk = { ["k" + (i % 2)]: ty.length };
    s += dk.k0 === undefined ? 1 : 2;
  }
  for (var k in o) s += k.length;
  var g = function () { return s; };
  var arr = [s, o];
  var c = {...o};
  return g() + arr.length + c.x + (o instanceof Point ? 1 : 0);
}
var t = 0;
for (var j = 0; j < 40; j++) t += f(new Point(j, 2), [1, 2, 3], 5);
print(t);
"#;

/// The slot kinds whose facts come only from the records of `profile.rs` (MC-w21-1): an
/// `ics_only` isolate writes none of them, so its export leaves them out (M5-P3-ORCH-14; H16
/// Change log 2026-11-01). The Call kind is both: `Call*` sites are records, `Construct*` sites
/// an IC (`construct_ic.rs`).
const RECORD_ONLY: [FeedbackSlotKind; 11] = [
    FeedbackSlotKind::LoadKeyed,
    FeedbackSlotKind::StoreKeyed,
    FeedbackSlotKind::DefineKeyedOwn,
    FeedbackSlotKind::HasKeyed,
    FeedbackSlotKind::StoreInArrayLiteral,
    FeedbackSlotKind::BinaryOp,
    FeedbackSlotKind::CompareOp,
    FeedbackSlotKind::TypeOf,
    FeedbackSlotKind::ForIn,
    FeedbackSlotKind::JumpLoop,
    FeedbackSlotKind::CreateClosure,
];

/// MC-w21-1 (M5-P3-ORCH-14): an `ics_only` export has no record-kind entry, so no BinaryOp or
/// CompareOp site reads as "int32 or never ran" (F-R9); it keeps the IC and object-model kinds.
/// A `full` export of the same script has a record of every kind the script uses (the control),
/// and more Call records (its `Call*` sites).
#[test]
fn an_ics_only_export_has_no_record_kind() {
    let kinds = |facts: &FactSnapshot| -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for (_, f) in facts.records() {
            *m.entry(format!("{:?}", f.kind())).or_insert(0) += 1;
        }
        m
    };
    let cases: Vec<(ExecOptions, &'static str, HeapConfig, u32)> = presets()
        .into_iter()
        .map(|exec| {
            let (n, h) = heaps()[0].clone();
            (exec, n, h, 0)
        })
        .collect();
    let handles: Vec<_> = cases
        .into_iter()
        .map(|(exec, n, h, threshold)| {
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    let (printed, facts) = run_src(RECORD_KINDS, exec, h, threshold);
                    (exec, label(exec, n, threshold), printed, facts)
                })
                .expect("a test thread")
        })
        .collect();
    let mut calls = BTreeMap::new();
    let mut ic_kinds = None;
    let mut answer: Option<String> = None;
    for (exec, l, printed, facts) in handles.into_iter().map(|h| h.join().expect("the run")) {
        assert!(!printed.is_empty(), "{l}");
        assert_eq!(
            answer.get_or_insert_with(|| printed.clone()),
            &printed,
            "{l}"
        );
        let k = kinds(&facts);
        let has = |kind: FeedbackSlotKind| k.contains_key(&format!("{kind:?}"));
        let ics_only = exec.feedback_mode() == Feedback::ICS_ONLY;
        for kind in RECORD_ONLY {
            assert_eq!(has(kind), !ics_only, "{l}: {kind:?} in {k:?}");
        }
        // The IC and object-model kinds are there in every mode, with the same counts.
        let ic: BTreeMap<String, usize> = k
            .iter()
            .filter(|(name, _)| {
                name.as_str() != "Call" && !RECORD_ONLY.iter().any(|r| format!("{r:?}") == **name)
            })
            .map(|(name, n)| (name.clone(), *n))
            .collect();
        for kind in [
            FeedbackSlotKind::LoadNamed,
            FeedbackSlotKind::StoreNamed,
            FeedbackSlotKind::LoadGlobal,
            FeedbackSlotKind::Literal,
            FeedbackSlotKind::CloneObject,
            FeedbackSlotKind::InstanceOf,
        ] {
            assert!(has(kind), "{l}: {kind:?} in {k:?}");
        }
        match &ic_kinds {
            None => ic_kinds = Some((l.clone(), ic)),
            Some((l0, ic0)) => assert_eq!(&ic, ic0, "the IC kinds: {l} vs {l0}"),
        }
        calls.insert(ics_only, k.get("Call").copied().unwrap_or(0));
    }
    // `ics_only` keeps the Construct-site Call records (an IC) and leaves out the `Call*` ones.
    assert!(
        calls[&true] >= 1 && calls[&true] < calls[&false],
        "{calls:?}"
    );
}
