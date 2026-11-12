//! The lazy namespaces of Temporal and Intl (INTL-I1-D8; plan v12 §4.7 LAZYNS; INTL W1): realm
//! init gives the global keys `Temporal` and `Intl` their table places and attributes with a
//! reserved value, and each group (the modules of `ep9_temporal::MODULES`, of
//! `ep9_intl::MODULES`) installs into a realm at the first read of its key or the first use
//! of one of its intrinsics. Every script runs on a fresh
//! isolate with `ep9_builtins::TABLE`, with and without GC stress (a collection before every
//! allocation, then the heap verifier); both runs must agree.
//!
//! What a script sees is the eager result: the expected lines are node v26.13.0's output for
//! the same scripts (`print` = `console.log`; `createRealm()` =
//! `vm.runInContext('globalThis', vm.createContext({}))` with `print` copied in), except the
//! newTarget case, where node 26.13.0 crashes (V8 b5666f7d85 is the fix): its line is the
//! spec result (GetPrototypeFromConstructor takes the newTarget realm's
//! %Temporal.PlainDate.prototype%). Whether a script installed the group is read from the
//! live cells after a scrubbed full collection: the group adds more than one cell per
//! definition and intrinsic of its modules.

mod common;

use std::cell::RefCell;
use std::hint::black_box;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{
    Args, BuiltinModule, Cx, HostFn, IntrinsicId, IntrinsicValue, Isolate, IsolateConfig, JsResult,
    Local, PropAttrs, PropDef, PropKey, PropValue, PropertyDescriptor,
};

thread_local! {
    static OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut parts = Vec::new();
    for v in args.iter() {
        parts.push(cx.to_rust_string(v)?);
    }
    OUT.with(|o| o.borrow_mut().push(parts.join(" ")));
    Ok(Local::undefined())
}

/// The host functions of every realm.
const HOST: [(&str, u32, HostFn); 3] = [
    ("print", 1, print),
    ("createRealm", 0, create_realm),
    ("intrinsic", 1, intrinsic),
];

/// `intrinsic(name)`: the intrinsic of the current realm whose spec name is `name` (INTL W2:
/// for a group intrinsic with no path from `globalThis`, such as `%IntlSegmentsPrototype%`).
/// The use of a group intrinsic installs its group first (C3).
fn intrinsic<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let name = cx.to_rust_string(args.get(0))?;
    let id = IntrinsicId::ALL
        .into_iter()
        .find(|id| id.name() == name)
        .unwrap_or_else(|| panic!("{name} is an intrinsic name"));
    Ok(cx.intrinsic(id).into())
}

/// `createRealm()`: a new realm with the host functions, made in it; returns its global.
fn create_realm<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let global = cx.create_realm()?;
    cx.with_realm(global, |cx| {
        for (name, length, f) in HOST {
            let func = cx.new_builtin_closure(f, name, length, Local::undefined());
            let desc = PropertyDescriptor::data(func.into(), true, false, true);
            cx.define_property_or_throw(global, name, desc)?;
        }
        Ok(())
    })?;
    Ok(global.into())
}

/// An isolate with the table and [`HOST`].
fn isolate(stress: bool) -> Isolate {
    let mut iso = Isolate::new(IsolateConfig {
        heap: HeapConfig {
            gc_stress: u32::from(stress),
            verify_after_gc: stress,
            ..HeapConfig::default()
        },
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    for (name, length, f) in HOST {
        iso.register_host_function(name, length, f);
    }
    iso
}

/// The live cells after the last collection (out of line: the `GcStats` copy stays in this
/// frame, `common::conservative_roots`).
#[inline(never)]
fn live_cells(i: &Isolate) -> usize {
    i.gc_stats().last_marked_cells
}

/// One run of the sloppy script `src`: the printed lines or the uncaught exception as "Name:
/// message", and the live cells after a scrubbed full collection. Verifies the heap.
fn run(src: &str, stress: bool) -> (Result<Vec<String>, String>, usize) {
    let unit = ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the script compiles");
    let mut iso = isolate(stress);
    OUT.with(|o| o.borrow_mut().clear());
    let r = iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()));
    let out = OUT.with(|o| core::mem::take(&mut *o.borrow_mut()));
    let r = r.map(|_| out).map_err(|e| e.message);
    common::scrub_and_collect(&mut iso);
    let live = live_cells(&iso);
    black_box(&mut iso);
    iso.verify_heap().expect("a valid heap");
    (r, live)
}

/// [`run`] with and without GC stress; the results must agree. Gives the lines and the live
/// cells of the plain run.
fn run_both(src: &str) -> (Result<Vec<String>, String>, usize) {
    let a = src.to_string();
    let b = src.to_string();
    let (plain, live) = common::on_big_stack(move || run(&a, false));
    let (stress, _) = common::on_big_stack(move || run(&b, true));
    assert_eq!(plain, stress, "GC stress changed the result of {src:?}");
    (plain, live)
}

/// The live cells after `src`: the smallest of 3 runs, so a stale stack word does not count.
fn min_cells(src: &'static str) -> usize {
    (0..3)
        .map(|_| common::on_big_stack(move || run(src, false)).1)
        .min()
        .expect("3 runs")
}

/// The live cells of an isolate that ran nothing, and the cells that the Temporal group adds
/// (a script that reads `Temporal` once, minus that).
fn group_cells() -> (usize, usize) {
    let base = min_cells("");
    let one = min_cells("Temporal;");
    assert!(one > base, "reading Temporal installs the group");
    (base, one - base)
}

/// The number of definitions and intrinsics of `modules`: the function objects and intrinsic
/// objects one install makes, at least.
fn size_of(modules: &[BuiltinModule]) -> usize {
    modules
        .iter()
        .map(|m| m.defs.len() + m.intrinsics.len())
        .sum()
}

/// The number of definitions and intrinsics of the Temporal modules.
fn group_size() -> usize {
    size_of(ep9_temporal::MODULES)
}

/// The cases whose script also installs every other lazy group ("Intl" and the TypedArray
/// group): `Object.keys` and `Object.freeze` of globalThis ask `[[GetOwnProperty]]` of every
/// key. Their count starts from the base plus [`other_group_cells`]: since the TA sync of
/// m8-p2 (INTL W2 and TA-4) those cells (218 + 318) are more than half of the Temporal group
/// (718), so the rounding of [`installs`] alone counts one install too many.
const EVERY_GROUP_CASES: [&str; 2] = [
    "a define with a value first",
    "Object.freeze(globalThis) first",
];

/// The cells that the lazy groups other than Temporal add (one read of a key of each).
fn other_group_cells(base: usize) -> usize {
    min_cells("Intl; ArrayBuffer;").saturating_sub(base)
}

/// Whether the script that left `live` cells installed the Temporal group in `realms` realms
/// (0, 1 or 2): the live cells above `base`, in units of the group's cells, rounded.
fn installs(live: usize, base: usize, group: usize) -> usize {
    (live.saturating_sub(base) + group / 2) / group
}

/// The text of a table key as the scripts print it: `[Symbol.x]` for a well-known symbol.
fn key_text(k: &PropKey) -> String {
    k.text()
}

/// A script helper: the own keys of `o` in [[OwnPropertyKeys]] order, as [`key_text`] gives
/// them, with "d" or "a" (data or accessor) and the w, e, c flags.
const KEYS_JS: &str = r#"
const keyText = (k) => (typeof k === "symbol" ? "[" + k.description + "]" : k);
const keys = (o) => {
  return Reflect.ownKeys(o).map((k) => {
    const d = Object.getOwnPropertyDescriptor(o, k);
    const kind = "value" in d ? "d" : "a";
    const w = "value" in d ? (d.writable ? "w" : "-") : "-";
    return keyText(k) + ":" + kind + w + (d.enumerable ? "e" : "-") + (d.configurable ? "c" : "-");
  }).join(",");
};
"#;

/// Realm init makes no object of the Temporal group: an isolate that reads `Temporal` once
/// has more live cells than one that does not, by at least one cell per definition and
/// intrinsic of the group (every function object and intrinsic object comes at the read).
/// `Cx::has_intrinsic` answers true for a Temporal intrinsic before the install and installs
/// nothing (M6-ORCH-132).
#[test]
fn realm_init_makes_no_temporal_object() {
    let (base, group) = group_cells();
    assert!(
        group >= group_size(),
        "the group adds {group} cells, fewer than its {} definitions and intrinsics",
        group_size()
    );
    let after = common::on_big_stack(|| {
        let mut iso = isolate(false);
        let r = iso.with_cx(|cx| {
            for id in [
                IntrinsicId::Temporal,
                IntrinsicId::TemporalNow,
                IntrinsicId::TemporalPlainDatePrototype,
                IntrinsicId::TemporalZonedDateTime,
            ] {
                assert!(cx.has_intrinsic(id), "{}", id.name());
            }
            Ok(())
        });
        assert_eq!(r, Ok(()));
        common::scrub_and_collect(&mut iso);
        live_cells(&iso)
    });
    assert_eq!(
        installs(after, base, group),
        0,
        "has_intrinsic installs nothing"
    );
}

/// The global keys before any install, after the install and the table walk order of the
/// `%GlobalObject%` properties (the VM table, then `ep9_builtins::TABLE`; then the host
/// functions) are one list: the key `Temporal` keeps its place.
#[test]
fn global_keys_keep_the_table_order() {
    let mut want: Vec<String> = Vec::new();
    for t in [ep9_vm::internals::VM_BUILTINS, &ep9_builtins::TABLE] {
        for m in t.modules {
            for op in m
                .props
                .iter()
                .filter(|op| op.target == IntrinsicId::GlobalObject)
            {
                want.extend(op.props.iter().map(|p| key_text(&p.key)));
            }
        }
    }
    want.extend(HOST.iter().map(|(n, _, _)| n.to_string()));
    assert!(want.iter().any(|k| k == "Temporal"));
    let want = want.join(",");
    let src = format!(
        "{KEYS_JS}
const before = Reflect.ownKeys(globalThis).map(keyText).join(\",\");
print(before);
print(typeof Temporal);
print(Reflect.ownKeys(globalThis).map(keyText).join(\",\") === before);"
    );
    let (r, _) = run_both(&src);
    assert_eq!(r, Ok(vec![want, "object".to_string(), "true".to_string()]));
}

/// Each script with its node lines and the number of realms whose group it installs.
const CASES: &[(&str, &str, usize)] = &[
    (
        "descriptor",
        r#"const d = Object.getOwnPropertyDescriptor(globalThis, "Temporal");
print(d.writable, d.enumerable, d.configurable, d.value === Temporal, Object.prototype.toString.call(d.value));"#,
        1,
    ),
    (
        "global load, a monomorphic site",
        r#"function g() { return Temporal; }
const a = g(), b = g(), c = g();
print(Object.prototype.toString.call(a), a === b, b === c, a === globalThis.Temporal);"#,
        1,
    ),
    (
        "typeof",
        r#"function t() { return typeof Temporal; }
print(t(), t(), typeof Temporal);"#,
        1,
    ),
    (
        "globalThis.Temporal, a monomorphic site",
        r#"function m(o) { return o.Temporal; }
const a = m(globalThis), b = m(globalThis), c = m(globalThis);
print(Object.prototype.toString.call(a), a === b, b === c, a === Temporal);"#,
        1,
    ),
    (
        "globalThis.Temporal, a polymorphic site",
        r#"function p(o) { return o.Temporal; }
p({ Temporal: 1 }); p({ x: 0, Temporal: 2 }); p({ y: 0, z: 0, Temporal: 3 });
const a = p(globalThis), b = p(globalThis);
print(Object.prototype.toString.call(a), a === b, a === Temporal);"#,
        1,
    ),
    (
        "globalThis.Temporal and a prototype holder, a megamorphic site",
        r#"function q(o) { return o.Temporal; }
for (let i = 0; i < 12; i++) { const o = {}; o["k" + i] = i; o.Temporal = i; q(o); q(o); }
const c = Object.create(globalThis);
const a = q(globalThis), b = q(c), d = q(globalThis);
print(Object.prototype.toString.call(a), a === b, a === d, a === Temporal);"#,
        1,
    ),
    (
        "globalThis[key]",
        r#"const k = "Tem" + "poral";
const a = globalThis[k];
print(Object.prototype.toString.call(a), a === Temporal);"#,
        1,
    ),
    (
        "Object.create(globalThis).Temporal",
        r#"const o = Object.create(globalThis);
const a = o.Temporal;
print(Object.prototype.toString.call(a), a === Temporal, Reflect.get(globalThis, "Temporal") === a);"#,
        1,
    ),
    (
        "with (globalThis)",
        r#"let a;
with (globalThis) { a = Temporal; }
print(Object.prototype.toString.call(a), a === globalThis.Temporal);"#,
        1,
    ),
    (
        "a proxy over globalThis",
        r#"const p = new Proxy(globalThis, {});
const a = p.Temporal;
const d = Object.getOwnPropertyDescriptor(new Proxy(globalThis, {}), "Temporal");
print(Object.prototype.toString.call(a), d.value === a, d.writable, d.enumerable, d.configurable, a === Temporal);"#,
        1,
    ),
    (
        "a proxy's [[GetOwnProperty]] first",
        r#"const d = Reflect.getOwnPropertyDescriptor(new Proxy(globalThis, {}), "Temporal");
print(Object.prototype.toString.call(d.value), d.value === Temporal);"#,
        1,
    ),
    (
        "a write first, then an intrinsic use",
        r#"Temporal = 5;
print(Temporal, typeof globalThis.Temporal);
print(new Date(0).toTemporalInstant().toString());
const d = Object.getOwnPropertyDescriptor(globalThis, "Temporal");
print(Temporal, d.value, d.writable, d.enumerable, d.configurable);"#,
        1,
    ),
    (
        "a strict write first",
        r#""use strict";
Temporal = 1;
print(Temporal, new Date(0).toTemporalInstant().epochMilliseconds, Temporal);"#,
        1,
    ),
    (
        "a define with a value first",
        r#"Object.defineProperty(globalThis, "Temporal", { value: 7, enumerable: true });
print(new Date(0).toTemporalInstant().toString());
const d = Object.getOwnPropertyDescriptor(globalThis, "Temporal");
print(d.value, d.writable, d.enumerable, d.configurable, Object.keys(globalThis).includes("Temporal"));"#,
        1,
    ),
    (
        "a define of attributes first",
        r#"Object.defineProperty(globalThis, "Temporal", { enumerable: true });
const d = Object.getOwnPropertyDescriptor(globalThis, "Temporal");
print(Object.prototype.toString.call(d.value), d.writable, d.enumerable, d.configurable);"#,
        1,
    ),
    (
        "a delete first: an intrinsic use does not bring the key back",
        r#"print(delete globalThis.Temporal, typeof Temporal, "Temporal" in globalThis);
print(new Date(0).toTemporalInstant().toString());
print(typeof Temporal, "Temporal" in globalThis, Object.getOwnPropertyNames(globalThis).includes("Temporal"));"#,
        1,
    ),
    (
        "var Temporal keeps the value",
        r#"var Temporal;
print(typeof Temporal, Object.prototype.toString.call(Temporal));"#,
        1,
    ),
    (
        "Object.freeze(globalThis) first",
        r#"Object.freeze(globalThis);
const d = Object.getOwnPropertyDescriptor(globalThis, "Temporal");
print(Object.prototype.toString.call(Temporal), d.writable, d.configurable, d.value === Temporal);"#,
        1,
    ),
    (
        "for-in, [[OwnPropertyKeys]] and HasProperty do not install",
        r#"let n = 0;
for (const k in globalThis) n++;
const keys = Reflect.ownKeys(globalThis);
print(keys.includes("Temporal"), Object.getOwnPropertyNames(globalThis).includes("Temporal"));
print("Temporal" in globalThis, Object.prototype.hasOwnProperty.call(globalThis, "Temporal"), Object.hasOwn(globalThis, "Temporal"));"#,
        0,
    ),
    (
        "a second realm installs its own group only",
        r#"const B = createRealm();
const t = B.Temporal;
print(Object.prototype.toString.call(t), Object.getPrototypeOf(t) === B.Object.prototype, Object.getPrototypeOf(t) !== Object.prototype);
print(Object.getPrototypeOf(t.PlainDate) === B.Function.prototype, t.PlainDate.prototype.constructor === t.PlainDate);"#,
        1,
    ),
    (
        "two realms, two groups",
        r#"const B = createRealm();
const t = B.Temporal;
print(t !== Temporal, Temporal.PlainDate !== t.PlainDate, Object.getPrototypeOf(Temporal) === Object.prototype);"#,
        2,
    ),
    (
        "MC-w30-6 (V8 c37f5f4c3c): a function of realm A reads B.Temporal first",
        r#"const B = createRealm();
function readB() { return B.Temporal; }
const t = readB();
print(Object.getPrototypeOf(t) === B.Object.prototype, Object.getPrototypeOf(t.Now) === B.Object.prototype, t === readB());
print(Object.getPrototypeOf(t.Now.instant) === B.Function.prototype, t.Now.instant() instanceof t.Instant);"#,
        1,
    ),
    (
        "MC-w30-6 (V8 b5666f7d85): a newTarget of a realm whose group is not installed",
        r#"const B = createRealm();
const nt = Function.prototype.bind.call(B.Object, null);
const r = Reflect.construct(Temporal.PlainDate, [2020, 1, 2], nt);
print(r.toString(), Object.getPrototypeOf(r) === B.Temporal.PlainDate.prototype, Object.getPrototypeOf(r) !== Temporal.PlainDate.prototype);"#,
        2,
    ),
];

/// The node lines of [`CASES`], in order (the spec line for the newTarget case).
const NODE: &[&[&str]] = &[
    &["true false true true [object Temporal]"],
    &["[object Temporal] true true true"],
    &["object object object"],
    &["[object Temporal] true true true"],
    &["[object Temporal] true true"],
    &["[object Temporal] true true true"],
    &["[object Temporal] true"],
    &["[object Temporal] true true"],
    &["[object Temporal] true"],
    &["[object Temporal] true true false true true"],
    &["[object Temporal] true"],
    &["5 number", "1970-01-01T00:00:00Z", "5 5 true false true"],
    &["1 0 1"],
    &["1970-01-01T00:00:00Z", "7 true true true true"],
    &["[object Temporal] true true true"],
    &[
        "true undefined false",
        "1970-01-01T00:00:00Z",
        "undefined false false",
    ],
    &["object [object Temporal]"],
    &["[object Temporal] false false true"],
    &["true true", "true true true"],
    &["[object Temporal] true true", "true true"],
    &["true true true"],
    &["true true true", "true true"],
    &["2020-01-02 true true"],
];

/// Every way to reach the namespace object gives it (a global load and a named load at a
/// monomorphic, a polymorphic and a megamorphic site, a keyed load, a prototype holder,
/// `with`, `[[GetOwnProperty]]`, a proxy's target); a write, a define and a delete before the
/// first read keep the eager result, and a later intrinsic use does not bring a deleted key
/// back; for-in, `[[OwnPropertyKeys]]` and HasProperty do not install; a second realm installs
/// its own group (MC-w30-6: a cross-realm first read installs in the holder's realm, and a
/// newTarget's realm installs for GetPrototypeFromConstructor).
#[test]
fn every_read_gives_the_eager_result() {
    assert_eq!(CASES.len(), NODE.len());
    let (base, group) = group_cells();
    // `createRealm()` alone installs no group: it is the base of the realm cases.
    let realm_base = common::on_big_stack(|| run("const B = createRealm();", false).1);
    let others = other_group_cells(base);
    for ((name, src, realms), want) in CASES.iter().zip(NODE) {
        let (r, live) = run_both(src);
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(r, Ok(want), "{name}");
        let from = if src.contains("createRealm") {
            realm_base
        } else if EVERY_GROUP_CASES.contains(name) {
            base + others
        } else {
            base
        };
        assert_eq!(
            installs(live, from, group),
            *realms,
            "{name}: groups installed"
        );
    }
}

/// Spread, `Object.assign` and `JSON.stringify` of globalThis give the eager result. (They
/// ask `[[GetOwnProperty]]` for every key, whose result holds the value, so they install the
/// group; only for-in reads the attributes alone.)
#[test]
fn copies_of_globalthis_give_the_eager_result() {
    let (r, _) = run_both(
        r#"const s = { ...globalThis };
const a = Object.assign({}, globalThis);
print("Temporal" in s, "Temporal" in a, JSON.stringify(Object.keys(s).filter((k) => k === "Temporal")));
print(typeof JSON.stringify(globalThis, (k, v) => (k === "" ? { t: typeof v.Temporal } : v)));"#,
    );
    assert_eq!(
        r,
        Ok(vec!["false false []".to_string(), "string".to_string()])
    );
}

/// The path of each Temporal intrinsic from `globalThis` (the first table property that names
/// it, in walk order).
fn paths() -> Vec<(IntrinsicId, String)> {
    let mut paths: Vec<(IntrinsicId, String)> =
        vec![(IntrinsicId::GlobalObject, "globalThis".into())];
    loop {
        let before = paths.len();
        for m in ep9_builtins::TABLE.modules {
            for op in m.props {
                let Some(from) = paths
                    .iter()
                    .find(|(id, _)| *id == op.target)
                    .map(|p| p.1.clone())
                else {
                    continue;
                };
                for p in op.props {
                    if let PropValue::Intrinsic(id) = p.value
                        && !matches!(p.key, PropKey::Symbol(_))
                        && !paths.iter().any(|(i, _)| *i == id)
                    {
                        paths.push((id, format!("{from}.{}", key_text(&p.key))));
                    }
                }
            }
        }
        if paths.len() == before {
            break;
        }
    }
    paths
}

/// The attribute text of a table property, as `KEYS_JS` prints it.
fn attrs_text(p: &PropDef) -> String {
    let PropAttrs {
        writable,
        enumerable,
        configurable,
    } = p.attrs;
    let accessor = matches!(p.value, PropValue::Accessor { .. });
    format!(
        "{}{}{}{}",
        if accessor { "a" } else { "d" },
        if writable && !accessor { "w" } else { "-" },
        if enumerable { "e" } else { "-" },
        if configurable { "c" } else { "-" },
    )
}

/// The group is complete: after the install, every object of every module of
/// `ep9_temporal::MODULES` has the key list of the eager install (the table walk: a function
/// intrinsic's `length` and `name` first, then its table properties in module order; the
/// symbol keys after the string keys, §10.1.11.1), with the table's attributes.
#[test]
fn the_group_is_complete() {
    group_is_complete(ep9_temporal::MODULES);
}

/// [`the_group_is_complete`] for the Intl group (INTL W1; design §4.6 case 8): every object
/// of every module of `ep9_intl::MODULES` has the key list of the eager install.
#[test]
fn the_intl_group_is_complete() {
    group_is_complete(ep9_intl::MODULES);
}

/// Every object of every module of `modules` (one lazy group) has the key list of the eager
/// install, read after the group installs.
fn group_is_complete(modules: &[BuiltinModule]) {
    let paths = paths();
    let mut src = KEYS_JS.to_string();
    let mut want = Vec::new();
    for m in modules {
        for d in m.intrinsics {
            // An intrinsic with no path from `globalThis` (INTL W2: %IntlSegmentsPrototype%,
            // %IntlSegmentIteratorPrototype%) is read with the host function `intrinsic`.
            let path = &paths
                .iter()
                .find(|(id, _)| *id == d.id)
                .map_or_else(|| format!("intrinsic({:?})", d.id.name()), |p| p.1.clone());
            let mut keys = Vec::new();
            if let IntrinsicValue::Builtin(_) = d.value {
                keys.push("length:d--c".to_string());
                keys.push("name:d--c".to_string());
            }
            let mut symbols = Vec::new();
            for t in ep9_builtins::TABLE.modules {
                for op in t.props.iter().filter(|op| op.target == d.id) {
                    for p in op.props {
                        let text = format!("{}:{}", key_text(&p.key), attrs_text(p));
                        match p.key {
                            PropKey::Symbol(_) => symbols.push(text),
                            _ => keys.push(text),
                        }
                    }
                }
            }
            keys.extend(symbols);
            src.push_str(&format!("print({path:?}, keys({path}));\n"));
            want.push(format!("{path} {}", keys.join(",")));
        }
    }
    assert_eq!(
        want.len(),
        modules.iter().map(|m| m.intrinsics.len()).sum::<usize>()
    );
    let (r, _) = run_both(&src);
    assert_eq!(r, Ok(want));
}

/// The Intl ids of INTL W1 and W2 (the group of the key `Intl`).
const INTL_IDS: [IntrinsicId; 17] = [
    IntrinsicId::Intl,
    IntrinsicId::IntlLocale,
    IntrinsicId::IntlLocalePrototype,
    IntrinsicId::IntlCollator,
    IntrinsicId::IntlCollatorPrototype,
    IntrinsicId::IntlPluralRules,
    IntrinsicId::IntlPluralRulesPrototype,
    IntrinsicId::IntlNumberFormat,
    IntrinsicId::IntlNumberFormatPrototype,
    IntrinsicId::IntlListFormat,
    IntrinsicId::IntlListFormatPrototype,
    IntrinsicId::IntlSegmenter,
    IntrinsicId::IntlSegmenterPrototype,
    IntrinsicId::IntlSegmentsPrototype,
    IntrinsicId::IntlSegmentIteratorPrototype,
    IntrinsicId::IntlDisplayNames,
    IntrinsicId::IntlDisplayNamesPrototype,
];

/// Design §4.6 case 1 for the Intl ids (INTL W1, C1): realm init makes no object of the Intl
/// group (a script that reads `Intl` once has more live cells than one that does not, by at
/// least one cell per definition and intrinsic of `ep9_intl::MODULES`), and every Intl id is
/// in that group. `Cx::has_intrinsic` answers true for an Intl id before the install and
/// installs nothing (M6-ORCH-132).
#[test]
fn realm_init_makes_no_intl_object() {
    let base = min_cells("");
    let intl = min_cells("Intl;") - base;
    assert!(
        intl >= size_of(ep9_intl::MODULES),
        "the Intl group adds {intl} cells, fewer than its {} definitions and intrinsics",
        size_of(ep9_intl::MODULES)
    );
    let made: Vec<IntrinsicId> = ep9_intl::MODULES
        .iter()
        .flat_map(|m| m.intrinsics.iter().map(|d| d.id))
        .collect();
    assert_eq!(
        made, INTL_IDS,
        "the Intl group makes the W1 and W2 Intl ids"
    );
    let after = common::on_big_stack(|| {
        let mut iso = isolate(false);
        let r = iso.with_cx(|cx| {
            for id in INTL_IDS {
                assert!(cx.has_intrinsic(id), "{}", id.name());
            }
            Ok(())
        });
        assert_eq!(r, Ok(()));
        common::scrub_and_collect(&mut iso);
        live_cells(&iso)
    });
    assert_eq!(
        installs(after, base, intl),
        0,
        "has_intrinsic installs nothing"
    );
}

/// Design §4.6 case 3: `typeof Temporal` installs the Temporal group only (the Intl group
/// stays out), then `typeof Intl` installs Intl; each group's cells add up.
#[test]
fn one_namespace_installs_without_the_other() {
    let base = min_cells("");
    let temporal = min_cells("typeof Temporal;") - base;
    let intl = min_cells("typeof Intl;") - base;
    let both = min_cells("typeof Temporal; typeof Intl;") - base;
    assert!(temporal >= group_size() && intl >= size_of(ep9_intl::MODULES));
    // `typeof Temporal` alone leaves the Intl group out: fewer than half its cells more.
    assert!(both >= temporal + intl / 2, "{both} {temporal} {intl}");
    assert!(
        both.abs_diff(temporal + intl) < intl.min(temporal) / 2,
        "{both} {temporal} {intl}"
    );
    let (r, _) = run_both(
        "print(typeof Temporal, typeof Intl, Object.prototype.toString.call(Intl), Intl.Locale.prototype.constructor === Intl.Locale);",
    );
    assert_eq!(r, Ok(vec!["object object [object Intl] true".to_string()]));
}

/// Design §4.6 case 7 (C4): the global `Intl` is an ordinary data property {writable: true,
/// enumerable: false, configurable: true} with the value %Intl% (node v26.13.0 lines); a
/// delete before the first read keeps the eager result.
#[test]
fn the_intl_global_is_a_data_property() {
    let (r, live) = run_both(
        r#"const d = Object.getOwnPropertyDescriptor(globalThis, "Intl");
print(d.writable, d.enumerable, d.configurable, d.value === Intl, Object.prototype.toString.call(d.value));"#,
    );
    assert_eq!(
        r,
        Ok(vec!["true false true true [object Intl]".to_string()])
    );
    let base = min_cells("");
    assert!(live > base);
    let (r, _) = run_both(
        r#"print(delete globalThis.Intl, typeof Intl, "Intl" in globalThis);
print(typeof Temporal, Reflect.ownKeys(globalThis).includes("Intl"));"#,
    );
    assert_eq!(
        r,
        Ok(vec![
            "true undefined false".to_string(),
            "object false".to_string()
        ])
    );
}
