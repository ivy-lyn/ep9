//! Far integer keys (M7-P1 OBJ1, route OBJ-FARKEY; H18-Q2 for every object): the bit-flag enum
//! form `o[1 << k] = v` that TypeScript output uses for its flag enums. Each store lands at the
//! capacity of the dense store, so the gap rule never fired, and the store doubled at each one,
//! to 2^28 slots: loading TypeScript 5.6 `typescript.js` took 5.6 GB RSS (RTS FE-3). The density
//! check (`elements::sparse_growth`) stops that on every object kind with an elements store.
//! The expected lines are node v26.13.0's output for the same script.

mod common;

use common::run_both;
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};

/// The flag-enum form on a plain object, an Array, a function, an Error and a class
/// instance, k = 0..27, then a TypeScript enum with flag members.
const FLAG_ENUM: &str = r#"
var o = {}, a = [], f = function () {}, e = new Error("x");
class C {} var c = new C();
for (var k = 0; k < 28; k++) { o[1 << k] = "x" + k; a[1 << k] = k; f[1 << k] = k; e[1 << k] = k; c[1 << k] = k; }
print(Object.keys(o).length, Object.keys(o)[0], Object.keys(o)[27], o[1 << 27], o[3]);
print(a.length, a[1 << 27], Object.keys(a).length, 1 in a, 3 in a);
print(Object.keys(f).length, f[1 << 20], Object.keys(e).length, e[1 << 26], Object.keys(c).join().length);
var TypeFlags;
(function (TypeFlags) {
  TypeFlags[TypeFlags["Any"] = 1] = "Any";
  TypeFlags[TypeFlags["Unknown"] = 2] = "Unknown";
  TypeFlags[TypeFlags["Never"] = 1 << 17] = "Never";
  TypeFlags[TypeFlags["Literal"] = 2944] = "Literal";
  TypeFlags[TypeFlags["Unit"] = 109472] = "Unit";
  TypeFlags[TypeFlags["Narrowable"] = 536624127] = "Narrowable";
})(TypeFlags || (TypeFlags = {}));
print(JSON.stringify(TypeFlags));
"#;

#[test]
fn the_flag_enum_form_gives_the_node_output() {
    assert_eq!(
        run_both(FLAG_ENUM),
        Ok(vec![
            "28 1 134217728 x27 undefined".to_string(),
            "134217729 27 28 true false".into(),
            "28 20 28 26 156".into(),
            concat!(
                r#"{"1":"Any","2":"Unknown","2944":"Literal","109472":"Unit","131072":"Never","#,
                r#""536624127":"Narrowable","Any":1,"Unknown":2,"Never":131072,"Literal":2944,"#,
                r#""Unit":109472,"Narrowable":536624127}"#
            )
            .into(),
        ])
    );
}

/// The plan's memory bound (plan v12 §4.3: under 50 MB RSS, est.), read as the heap's
/// allocated bytes of the whole run (pages as the heap takes them; no GC stress, which
/// inflates that counter): it is a few hundred KiB now. Before the density check the plain
/// object alone allocated about 2 GiB of dense cells.
#[test]
fn the_flag_enum_form_stays_under_50_mb() {
    const LIMIT: u64 = 50 << 20;
    let allocated = common::on_big_stack(|| {
        let unit = ep9_frontend::compile(
            FLAG_ENUM,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .expect("the script compiles");
        let mut iso = common::isolate(false);
        let before = iso.gc_stats().allocated_bytes;
        iso.with_cx_checkpoint(|cx| cx.run_script(unit, "far_keys.js").map(|_| ()))
            .expect("the script runs");
        let allocated = iso.gc_stats().allocated_bytes - before;
        iso.collect_garbage();
        iso.verify_heap().expect("a valid heap");
        allocated
    });
    eprintln!("the flag-enum form allocated {allocated} B");
    assert!(
        allocated < LIMIT,
        "the flag-enum form allocated {allocated} B (limit {LIMIT} B)"
    );
}
