//! The object model reads no execution policy (doc 13 M-R9, MODES-D19, plan v10 rule 40; the
//! w16 O2 test, M5-P2 ELEM2). A workload of object and array literals, keyed stores that
//! move arrays through the elements kinds (H18), DICTIONARY arrays and constructor objects
//! runs on the `jitless` preset under both type-feedback setters, `full` and `ics_only`
//! (the axis MODES-D19 is about). Both runs must make the same shapes and the same elements
//! kinds: the per-site literal facts (result shape, ALLOC_SITE elements kind), the set of
//! shapes the IC facts name, and the count of literal sites per elements kind. The ALLOC_SITE
//! cell (V10-D6) is an IC input, so `ics_only` keeps it too.
//!
//! This crate has no `jit` feature (M-R6, `Cargo.toml`): the `jit` preset is an error here
//! unless a workspace build turned the feature on for ep9_vm (feature unification), and the
//! test checks that the two agree. PROFILE6 (M5-P3) makes `ics_only` take effect and re-runs
//! this test then; until then both setters run the same T0 table, so a failure now means an
//! object-model input reads the setter.
//!
//! ep9_vm tests cannot compile JS, so this test lives here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ep9_facts::{ElementsKind, FactSnapshot, ShapeDesc, SlotFacts};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::abi::export_facts;
use ep9_vm::{ExecOptions, Feedback, Isolate, IsolateConfig, Preset};

/// Literals, kind transitions through stores, DICTIONARY arrays, constructors; prints a
/// digest so both runs are checked for the same answer too.
const WORKLOAD: &str = r#"
function Point(x, y) { this.x = x; this.y = y; }
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
for (var i = 0; i < 400; i++) {
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
}
print(sum, keep.length, keep[0].length);
"#;

/// The jitless preset with the feedback setter `f`.
fn options(f: Feedback) -> ExecOptions {
    ExecOptions::jitless().feedback(f)
}

/// Runs the workload on a fresh isolate with `exec` (eager vectors: threshold 0, so every
/// site records from its first run); GC stress 1 if `stress`. The printed digest and the facts.
fn run(exec: ExecOptions, stress: bool) -> (String, FactSnapshot) {
    let unit = ep9_frontend::compile(
        WORKLOAD,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the workload compiles");
    let mut iso = Isolate::new_with(
        IsolateConfig {
            builtins: Some(&ep9_builtins::TABLE),
            feedback_threshold: 0,
            heap: HeapConfig {
                gc_stress: u32::from(stress),
                verify_after_gc: stress,
                ..HeapConfig::default()
            },
            ..IsolateConfig::default()
        },
        exec,
    )
    .expect("a jitless isolate");
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

/// The w16 O2 test (M-R9): equal shapes and elements kinds under `full` and `ics_only`, with
/// and without GC stress; the ALLOC_SITE facts show the generalized literal kinds.
#[test]
fn feedback_setters_make_the_same_shapes_and_elements_kinds() {
    for stress in [false, true] {
        let (out_full, full) = run(options(Feedback::FULL), stress);
        let (out_ics, ics) = run(options(Feedback::ICS_ONLY), stress);
        assert_eq!(out_full, out_ics, "the same answer (stress {stress})");
        let (a, b) = (object_model(&full), object_model(&ics));
        assert!(!a.literals.is_empty() && !a.shapes.is_empty());
        assert_eq!(
            a.shapes.len(),
            b.shapes.len(),
            "shape counts (stress {stress})"
        );
        assert_eq!(a.kinds, b.kinds, "elements-kind counts (stress {stress})");
        assert_eq!(a, b, "the object model (stress {stress})");
        // The ALLOC_SITE kinds (V10-D6): `ints()` met DOUBLE and ELEMENTS stores, so its site
        // makes PACKED_ELEMENTS arrays; `empty()` met a push of 0.25: DOUBLE; the nested array
        // of `leaf` has no site (no slot of its own).
        let kinds: BTreeSet<ElementsKind> = a.kinds.keys().flatten().copied().collect();
        assert!(
            kinds.contains(&ElementsKind::PackedElements),
            "{:?}",
            a.kinds
        );
        assert!(
            kinds.contains(&ElementsKind::PackedDouble)
                || kinds.contains(&ElementsKind::HoleyDouble),
            "{:?}",
            a.kinds
        );
    }
}
