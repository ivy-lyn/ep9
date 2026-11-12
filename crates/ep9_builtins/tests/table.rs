//! The builtin table of `ep9_builtins` (H20): the checks, the module blocks, the id goldens
//! (`tests/golden/ids-<module>.txt`, append-only; `EP9_BLESS=1 cargo test -p ep9_builtins
//! --test table` rewrites them after a review), property order, the crate's clippy.toml, and
//! the realm that `TABLE` gives.

mod common;

use std::path::PathBuf;

use ep9_builtins::{MODULES, TABLE};
use ep9_vm::internals::VM_BUILTINS;
use ep9_vm::{BuiltinModule, Cx, IntrinsicId, check_tables};

fn golden_path(module: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/ids-{module}.txt"))
}

fn read_golden(module: &str) -> String {
    let p = golden_path(module);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", p.display()))
        .replace("\r\n", "\n")
}

#[test]
fn the_table_passes_the_checks() {
    assert_eq!(check_tables(&[VM_BUILTINS, &TABLE]), Ok(()));
    assert_eq!(TABLE.name, "ep9_builtins");
}

#[test]
fn modules_have_fixed_blocks_in_a_fixed_order() {
    let got: Vec<(&str, u8)> = MODULES.iter().map(|m| (m.name, m.block)).collect();
    // Append-safe (README §3.4): each listed module keeps its place and block, and every block is
    // its place + 1, so a new module at the end passes; no pin on the length of the list.
    let fixed = [
        ("object", 1),
        ("function", 2),
        ("array", 3),
        ("number", 4),
        ("boolean", 5),
        ("string", 6),
        ("symbol", 7),
        ("math", 8),
        ("global", 9),
        ("promise", 10),
        ("reflect", 11),
        ("date", 12),
        ("collections", 13),
        ("json", 14),
        ("proxy", 15),
        ("regexp", 16),
        ("weakref", 17),
        ("bigint", 18),
        ("temporal", 19),
        ("duration", 20),
        ("instant", 21),
        ("plain_time", 22),
        ("plain_date", 23),
        ("plain_date_time", 24),
        ("plain_year_month", 25),
        ("plain_month_day", 26),
        ("zoned_date_time", 27),
    ];
    assert_eq!(&got[..fixed.len()], &fixed[..]);
    for (i, &(name, block)) in got.iter().enumerate() {
        assert_eq!(usize::from(block), i + 1, "{name}");
    }
    assert_eq!(ep9_builtins::object::BLOCK, 1);
    assert_eq!(ep9_builtins::global::BLOCK, 9);
    assert_eq!(ep9_builtins::promise::BLOCK, 10);
    assert_eq!(ep9_builtins::reflect::BLOCK, 11);
    assert_eq!(ep9_builtins::date::BLOCK, 12);
    assert_eq!(ep9_builtins::collections::BLOCK, 13);
    assert_eq!(ep9_builtins::json::BLOCK, 14);
    assert_eq!(ep9_builtins::proxy::BLOCK, 15);
    assert_eq!(ep9_builtins::regexp::BLOCK, 16);
    assert_eq!(ep9_builtins::weakref::BLOCK, 17);
    assert_eq!(ep9_builtins::bigint::BLOCK, 18);
    // The Temporal modules are one run, in the order of ep9_temporal::MODULES (not a pin on the
    // end of the table: a later module can follow them).
    // The Intl modules (INTL-I1-D1) are one run too, in the order of ep9_intl::MODULES.
    for crate_modules in [ep9_temporal::MODULES, ep9_intl::MODULES] {
        let start = got
            .iter()
            .position(|&(name, _)| name == crate_modules[0].name)
            .unwrap();
        let run = &MODULES[start..start + crate_modules.len()];
        assert_eq!(
            run.iter().map(|m| (m.name, m.block)).collect::<Vec<_>>(),
            crate_modules
                .iter()
                .map(|m| (m.name, m.block))
                .collect::<Vec<_>>()
        );
    }
    // The Intl blocks follow disposable (block 29; M7-P3 SPEC-ERM), after iterator (block 28).
    assert_eq!(ep9_builtins::disposable::BLOCK, 29);
    assert_eq!(ep9_intl::intl::BLOCK, 30);
    assert_eq!(ep9_intl::locale::BLOCK, 31);
    assert_eq!(ep9_intl::collator::BLOCK, 32);
    assert_eq!(ep9_builtins::typed_array::BLOCK, 35);
    assert_eq!(ep9_builtins::typed_array::proto::BLOCK, 36);
    assert_eq!(ep9_builtins::typed_array::base64::BLOCK, 37);
    assert_eq!(ep9_builtins::atomics::BLOCK, 38);
}

#[test]
fn ids_match_the_goldens() {
    let bless = std::env::var_os("EP9_BLESS").is_some();
    let mut failures = Vec::new();
    for m in MODULES {
        let actual = m.ids_text();
        if bless {
            std::fs::write(golden_path(m.name), &actual).unwrap();
            continue;
        }
        if actual != read_golden(m.name) {
            failures.push(format!("--- ids-{}.txt now:\n{actual}", m.name));
        }
    }
    assert!(
        failures.is_empty(),
        "builtin ids changed (append-only; if only lines were appended, run with EP9_BLESS=1 \
         and review the diff):\n{}",
        failures.join("\n")
    );
}

#[test]
fn swapping_two_entries_fails_a_golden() {
    for m in MODULES.iter().filter(|m| m.defs.len() >= 2) {
        let mut defs = m.defs.to_vec();
        defs.swap(0, 1);
        let swapped = BuiltinModule {
            defs: Box::leak(defs.into_boxed_slice()),
            ..*m
        };
        assert_ne!(swapped.ids_text(), read_golden(m.name), "{}", m.name);
    }
}

/// Every installed intrinsic with its own keys, in [[OwnPropertyKeys]] order.
fn intrinsic_keys(cx: &mut Cx<'_>) -> ep9_vm::JsResult<Vec<(IntrinsicId, Vec<String>)>> {
    let mut out = Vec::new();
    for id in IntrinsicId::ALL {
        if !cx.has_intrinsic(id) {
            continue;
        }
        let o = cx.intrinsic(id);
        let keys = cx.own_property_keys(o)?;
        let n = cx.get(keys, "length")?;
        let n = cx.to_length(n)?;
        let mut texts = Vec::new();
        for i in 0..n {
            let k = cx.get(keys, i.to_string().as_str())?;
            let text = match k.as_symbol() {
                Some(sym) => cx.descriptive_string(sym)?.value(),
                None => k,
            };
            texts.push(cx.to_rust_string(text)?);
        }
        out.push((id, texts));
    }
    Ok(out)
}

#[test]
fn two_isolates_have_the_same_intrinsic_property_order() {
    let a = common::on_big_stack(|| common::isolate(false).with_cx(intrinsic_keys));
    let b = common::on_big_stack(|| common::isolate(true).with_cx(intrinsic_keys));
    let a = a.expect("no exception");
    assert_eq!(Ok(a.clone()), b);
    let global = &a
        .iter()
        .find(|(id, _)| *id == IntrinsicId::GlobalObject)
        .unwrap()
        .1;
    assert_eq!(global.first().map(String::as_str), Some("globalThis"));
    assert_eq!(global.last().map(String::as_str), Some("print"));
}

#[test]
fn the_table_reaches_the_isolate() {
    let iso = common::isolate(false);
    let t = iso.builtin_table().expect("installed");
    assert!(core::ptr::eq(t, &TABLE));
}

#[test]
fn function_prototype_is_a_function() {
    assert_eq!(
        common::run_both("print(typeof Function.prototype, Function.prototype())"),
        Ok(vec!["function undefined".to_string()])
    );
    assert_eq!(
        common::run_both(
            "print(Function.prototype.constructor === Function, Object.getPrototypeOf(Function.prototype) === Object.prototype)"
        ),
        Ok(vec!["true true".to_string()])
    );
    assert_eq!(
        common::run_both("Function('return 1')"),
        // The common isolates have no compiler hook (O2, P3-EVAL-VM).
        Err(
            "EvalError: code generation from strings is not available (no compiler hook)"
                .to_string()
        )
    );
    assert_eq!(
        common::run_both(
            "class C extends Object {} var c = new C(); print(c instanceof C, typeof Object())"
        ),
        Ok(vec!["true object".to_string()])
    );
}

/// The crate clippy.toml repeats every ban of the root clippy.toml (clippy reads only the
/// nearest file) and bans raw `Value` and the heap cell types.
#[test]
fn clippy_toml_repeats_the_root_bans() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ours = std::fs::read_to_string(dir.join("clippy.toml")).unwrap();
    let root = std::fs::read_to_string(dir.join("../../clippy.toml")).unwrap();
    let paths = |text: &str| -> Vec<String> {
        text.lines()
            .filter_map(|l| l.split("path = \"").nth(1))
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_string)
            .collect()
    };
    let root_paths = paths(&root);
    assert!(root_paths.len() > 50, "the root bans: {}", root_paths.len());
    // ep9_temporal (design SPEC-T §3.1) and ep9_intl (INTL-I1-D1) keep a copy of this file
    // plus a libm ban.
    let temporal = std::fs::read_to_string(dir.join("../ep9_temporal/clippy.toml")).unwrap();
    let intl = std::fs::read_to_string(dir.join("../ep9_intl/clippy.toml")).unwrap();
    for (name, text) in [
        ("crates/ep9_builtins/clippy.toml", &ours),
        ("crates/ep9_temporal/clippy.toml", &temporal),
        ("crates/ep9_intl/clippy.toml", &intl),
    ] {
        let our_paths = paths(text);
        for p in &root_paths {
            assert!(our_paths.contains(p), "{name} lacks {p}");
        }
        for p in ["ep9_value::Value", "ep9_heap::CellPtr", "ep9_heap::Heap"] {
            assert!(
                our_paths.iter().any(|q| q == p),
                "{name}: no disallowed-types entry {p}"
            );
        }
    }
    assert!(paths(&temporal).iter().any(|q| q == "libm::fma"));
    assert!(paths(&intl).iter().any(|q| q == "libm::fma"));
}
