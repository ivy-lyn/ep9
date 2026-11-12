//! The Atomics object at the JS level (TypedArray TA-4; `src/atomics.rs`; ECMA-262
//! `sec-atomics-object`). R1 (the skeleton) pins `%Atomics%` (its keys in node v26.13.1's
//! order, the attributes, names and lengths, `@@toStringTag`, the global key `Atomics`), the
//! lazy group (the key `Atomics` joins the group of `ArrayBuffer`, TA-D14: realm init makes no
//! `%Atomics%`) and the stub TypeError of each function (R2 ATOMICS flips it). R2 appends. Each
//! script prints the lines of node v26.13.1 for the same script (`print` here, `console.log`
//! there), except the stub lines, and runs with and without GC stress.

mod common;

use std::hint::black_box;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_vm::{BuiltinModule, Cx, IntrinsicId, Isolate, JsResult};

fn lines(v: &[&str]) -> Result<Vec<String>, String> {
    Ok(v.iter().map(|s| s.to_string()).collect())
}

/// `%Atomics%`: an ordinary extensible object with prototype `%Object.prototype%`, the global
/// key `Atomics` {w, c}, the 14 functions {w, c} in node's key order with their names and
/// lengths, and `@@toStringTag` "Atomics" {c}. No function is a constructor.
#[test]
fn atomics_has_the_spec_keys_and_attributes() {
    let src = r#"
const d = (o, k) => {
  const x = Object.getOwnPropertyDescriptor(o, k);
  return ("value" in x ? "d" + (x.writable ? "w" : "-") : "a") + (x.enumerable ? "e" : "-") +
    (x.configurable ? "c" : "-");
};
print(typeof Atomics, d(globalThis, "Atomics"), Object.getPrototypeOf(Atomics) === Object.prototype,
  Object.isExtensible(Atomics), String(Atomics), Atomics[Symbol.toStringTag],
  d(Atomics, Symbol.toStringTag));
print(Reflect.ownKeys(Atomics).map((k) => {
  const v = Atomics[k];
  return String(k) + " " + d(Atomics, k) + (typeof v === "function" ? " " + v.name + " " + v.length : "");
}).join("\n"));
let isCtor = 0;
for (const k of Object.getOwnPropertyNames(Atomics)) {
  try { new Atomics[k](); } catch (e) { if (e instanceof TypeError) isCtor++; }
}
print(isCtor);
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "object dw-c true true [object Atomics] Atomics d--c",
            "load dw-c load 2\n\
             store dw-c store 3\n\
             add dw-c add 3\n\
             sub dw-c sub 3\n\
             and dw-c and 3\n\
             or dw-c or 3\n\
             xor dw-c xor 3\n\
             exchange dw-c exchange 3\n\
             compareExchange dw-c compareExchange 4\n\
             isLockFree dw-c isLockFree 1\n\
             wait dw-c wait 4\n\
             waitAsync dw-c waitAsync 4\n\
             notify dw-c notify 3\n\
             pause dw-c pause 0\n\
             Symbol(Symbol.toStringTag) d--c",
            "14",
        ])
    );
}

/// Each function throws the stub TypeError (m8-stub, TypedArray TA-4 R1) until R2 ATOMICS
/// builds it.
#[test]
fn each_stub_throws_a_type_error() {
    let src = r#"
const names = ["load", "store", "add", "sub", "and", "or", "xor", "exchange",
  "compareExchange", "isLockFree", "wait", "waitAsync", "notify", "pause"];
const ta = new Int32Array(new SharedArrayBuffer(8));
for (const n of names) {
  try {
    Atomics[n](ta, 0, 0, 0);
    print(n, "no throw");
  } catch (e) {
    print(n, e instanceof TypeError, e.message);
  }
}
"#;
    assert_eq!(
        common::run_both(src),
        lines(&[
            "load true Atomics.load: not built yet (m8-stub, TypedArray TA-4)",
            "store true Atomics.store: not built yet (m8-stub, TypedArray TA-4)",
            "add true Atomics.add: not built yet (m8-stub, TypedArray TA-4)",
            "sub true Atomics.sub: not built yet (m8-stub, TypedArray TA-4)",
            "and true Atomics.and: not built yet (m8-stub, TypedArray TA-4)",
            "or true Atomics.or: not built yet (m8-stub, TypedArray TA-4)",
            "xor true Atomics.xor: not built yet (m8-stub, TypedArray TA-4)",
            "exchange true Atomics.exchange: not built yet (m8-stub, TypedArray TA-4)",
            "compareExchange true Atomics.compareExchange: not built yet (m8-stub, TypedArray TA-4)",
            "isLockFree true Atomics.isLockFree: not built yet (m8-stub, TypedArray TA-4)",
            "wait true Atomics.wait: not built yet (m8-stub, TypedArray TA-4)",
            "waitAsync true Atomics.waitAsync: not built yet (m8-stub, TypedArray TA-4)",
            "notify true Atomics.notify: not built yet (m8-stub, TypedArray TA-4)",
            "pause true Atomics.pause: not built yet (m8-stub, TypedArray TA-4)",
        ])
    );
}

// ---- the lazy group (TA-D14; the TA-3 form of `tests/typed_array.rs`) --------------------------

/// The live cells after the last collection (out of line: the `GcStats` copy stays in this
/// frame, `common::conservative_roots`).
#[inline(never)]
fn live_cells(i: &Isolate) -> usize {
    i.gc_stats().last_marked_cells
}

/// The live cells after the sloppy script `src` (no GC stress) and a scrubbed full collection;
/// `rust` runs first, in the same isolate. Verifies the heap.
fn cells_after(src: &'static str, rust: fn(&mut Cx<'_>) -> JsResult<()>) -> usize {
    common::on_big_stack(move || {
        let unit = ep9_frontend::compile(
            src,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .expect("the script compiles");
        let mut iso = common::isolate(false);
        iso.with_cx(rust).expect("no exception");
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "test.js").map(|_| ()))
            .expect("no exception");
        common::scrub_and_collect(&mut iso);
        let live = live_cells(&iso);
        black_box(&mut iso);
        iso.verify_heap().expect("a valid heap");
        live
    })
}

/// [`cells_after`], the smallest of 3 runs (a stale stack word does not count).
fn min_cells(src: &'static str, rust: fn(&mut Cx<'_>) -> JsResult<()>) -> usize {
    (0..3)
        .map(|_| cells_after(src, rust))
        .min()
        .expect("3 runs")
}

fn nothing(_cx: &mut Cx<'_>) -> JsResult<()> {
    Ok(())
}

/// Realm init makes no `%Atomics%`: a script that reads `Atomics` once leaves more live cells
/// than one that reads nothing, by at least one cell per definition and intrinsic of the six
/// modules of the group (`array_buffer`, `data_view`, `typed_array`, `typed_array_proto`,
/// `base64`, `atomics`), and the read installs the whole group, the same cells as an
/// `ArrayBuffer` read. `Atomics` with the other keys installs the group once.
/// `Cx::has_intrinsic(Atomics)` answers true and installs nothing; `Cx::intrinsic(Atomics)`
/// installs the group.
#[test]
fn the_atomics_key_joins_the_lazy_group() {
    let size: usize = [
        ep9_builtins::array_buffer::MODULE,
        ep9_builtins::data_view::MODULE,
        ep9_builtins::typed_array::MODULE,
        ep9_builtins::typed_array::proto::MODULE,
        ep9_builtins::typed_array::base64::MODULE,
        ep9_builtins::atomics::MODULE,
    ]
    .iter()
    .map(|m: &BuiltinModule| m.defs.len() + m.intrinsics.len())
    .sum();
    let base = min_cells("", nothing);
    let group = min_cells("Atomics;", nothing).saturating_sub(base);
    assert!(
        group >= size,
        "the group adds {group} cells, fewer than its {size} definitions and intrinsics"
    );
    assert_eq!(
        min_cells("ArrayBuffer;", nothing).saturating_sub(base),
        group,
        "an ArrayBuffer read installs the same group"
    );
    // The installs of the group that a run made, rounded.
    let installs = |live: usize| (live.saturating_sub(base) + group / 2) / group;
    for src in [
        "Atomics; Int8Array; ArrayBuffer;",
        "typeof Atomics;",
        "Atomics.load;",
    ] {
        assert_eq!(installs(min_cells(src, nothing)), 1, "{src}");
    }
    fn has(cx: &mut Cx<'_>) -> JsResult<()> {
        assert!(cx.has_intrinsic(IntrinsicId::Atomics));
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", has)),
        0,
        "has_intrinsic installs nothing"
    );
    fn get(cx: &mut Cx<'_>) -> JsResult<()> {
        cx.intrinsic(IntrinsicId::Atomics);
        Ok(())
    }
    assert_eq!(
        installs(min_cells("", get)),
        1,
        "Cx::intrinsic installs the group"
    );
}
