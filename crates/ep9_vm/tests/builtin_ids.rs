//! The id golden of the VM's own builtin table (H20, W4-BUILTIN-IDS):
//! `tests/golden/vm-builtin-ids.txt` lists every definition of block 0 with its `BuiltinId`,
//! then every `IntrinsicId`. Both lists are append-only: an id never moves, so units, snapshots
//! and other tables can name it (H13). `EP9_BLESS=1 cargo test -p ep9_vm --test builtin_ids`
//! rewrites the file; review the diff: only appended lines are allowed.

use std::path::PathBuf;

use ep9_vm::internals::VM_BUILTINS;
use ep9_vm::{BuiltinModule, IntrinsicId, check_tables};

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/vm-builtin-ids.txt")
}

/// The golden text of the VM table with `module` as its module.
fn text(module: &BuiltinModule) -> String {
    let mut s = module.ids_text();
    s.push_str("# IntrinsicId (append-only)\n");
    for id in IntrinsicId::ALL {
        s.push_str(&format!("intrinsic {:>3}  {}\n", id as u16, id.name()));
    }
    s
}

fn read_golden() -> String {
    std::fs::read_to_string(golden_path())
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", golden_path().display()))
        .replace("\r\n", "\n")
}

#[test]
fn vm_builtin_ids_match_the_golden() {
    assert_eq!(
        VM_BUILTINS.modules.len(),
        1,
        "the VM table is one module (block 0)"
    );
    let actual = text(&VM_BUILTINS.modules[0]);
    if std::env::var_os("EP9_BLESS").is_some() {
        std::fs::write(golden_path(), &actual).unwrap();
        return;
    }
    let want = read_golden();
    assert!(
        actual == want,
        "the VM builtin ids changed (ids are append-only).\n--- golden\n{want}\n--- now\n{actual}\n\
         If only lines were appended, run with EP9_BLESS=1 and review the diff."
    );
}

#[test]
fn swapping_two_entries_fails_the_golden() {
    let m = VM_BUILTINS.modules[0];
    assert!(m.defs.len() >= 4);
    for (a, b) in [(0, 1), (2, 3), (m.defs.len() - 2, m.defs.len() - 1)] {
        let mut defs = m.defs.to_vec();
        defs.swap(a, b);
        let swapped = BuiltinModule {
            defs: Box::leak(defs.into_boxed_slice()),
            ..m
        };
        assert_ne!(text(&swapped), read_golden(), "swap {a} and {b}");
    }
}

#[test]
fn the_vm_table_passes_the_checks() {
    assert_eq!(check_tables(&[VM_BUILTINS]), Ok(()));
}
