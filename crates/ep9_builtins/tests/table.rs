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
    assert_eq!(
        got,
        [
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
        ]
    );
    assert_eq!(ep9_builtins::object::BLOCK, 1);
    assert_eq!(ep9_builtins::global::BLOCK, 9);
    assert_eq!(ep9_builtins::promise::BLOCK, 10);
    assert_eq!(ep9_builtins::reflect::BLOCK, 11);
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
            "print(Function.prototype.constructor === Function, Object.getPrototypeOf === undefined)"
        ),
        Ok(vec!["true true".to_string()])
    );
    assert_eq!(
        common::run_both("Function('return 1')"),
        Err("Error: not implemented in ep9 v0: Function constructor".to_string())
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
    let our_paths = paths(&ours);
    for p in &root_paths {
        assert!(
            our_paths.contains(p),
            "crates/ep9_builtins/clippy.toml lacks {p}"
        );
    }
    for p in ["ep9_value::Value", "ep9_heap::CellPtr", "ep9_heap::Heap"] {
        assert!(
            our_paths.iter().any(|q| q == p),
            "no disallowed-types entry {p}"
        );
    }
}
