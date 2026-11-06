//! The relocation kinds against the golden file and the H34 text: the codes, the order and
//! the append-only rule (H34 Freeze table, part "Relocation kinds (append-only)").

use ep9_masm::{KIND_COUNT, RelocKind};

const GOLDEN: &str = include_str!("golden/reloc-kinds.txt");
const H34: &str = include_str!("../../../docs/adr/H34-relocation-kinds.md");

fn golden_lines() -> Vec<&'static str> {
    GOLDEN
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect()
}

fn now() -> Vec<String> {
    RelocKind::ALL
        .iter()
        .map(|k| format!("{} {}", k.code(), k.name()))
        .collect()
}

/// The golden file holds one line per code, and only those. A kind appended to the code
/// needs its line; a changed code or name fails.
#[test]
fn the_kinds_are_the_golden_lines() {
    assert_eq!(golden_lines(), now());
    assert_eq!(golden_lines().len(), KIND_COUNT);
}

/// The kind names of the Freeze table row "Relocation kinds (append-only)", in its order
/// ("Local, Stub, ..., HeapConst (JIT only), in this order").
fn h34_freeze_list() -> Vec<String> {
    let row = H34
        .lines()
        .find(|l| l.starts_with("| Relocation kinds (append-only) |"))
        .expect("H34 has the Freeze table row");
    let cell = row
        .split('|')
        .nth(2)
        .expect("the What-it-holds cell")
        .trim();
    let list = cell
        .split(", in this order")
        .next()
        .expect("the list ends with \", in this order\"");
    list.split(", ")
        .map(|n| n.trim_end_matches(" (JIT only)").to_string())
        .collect()
}

/// The first cells of the H34 Decision kinds table ("| Kind (04 §5) | Meaning | ..."), with
/// the operand cut off: "Stub(id)" -> "Stub".
fn h34_decision_list() -> Vec<String> {
    let mut lines = H34
        .lines()
        .skip_while(|l| !l.starts_with("| Kind (04 §5) |"));
    lines.next().expect("H34 has the Decision kinds table");
    lines
        .skip(1)
        .take_while(|l| l.starts_with('|'))
        .map(|l| {
            let cell = l.split('|').nth(1).expect("a Kind cell").trim();
            cell.split('(').next().expect("a name").to_string()
        })
        .collect()
}

/// The code order is the H34 order, in the Freeze table row and in the Decision table, so
/// code i is the i-th H34 kind (MODES-CHECK w32: "ep9_masm keeps the H34 order and meaning").
#[test]
fn the_codes_follow_the_h34_order() {
    let names: Vec<String> = RelocKind::ALL
        .iter()
        .map(|k| k.name().to_string())
        .collect();
    assert_eq!(h34_freeze_list(), names);
    assert_eq!(h34_decision_list(), names);
    assert_eq!(
        names[..8],
        [
            "Local",
            "Stub",
            "Runtime",
            "LinkSlot",
            "IcCell",
            "RootIndex",
            "DeoptExit",
            "HeapConst"
        ]
    );
}

/// The append-only rule on the golden file: the lines of the 8 kinds of M1 exit stay the
/// first 8 lines, unchanged. A new kind adds line 9, never a line before it.
#[test]
fn the_golden_file_is_append_only() {
    let first = [
        "0 Local",
        "1 Stub",
        "2 Runtime",
        "3 LinkSlot",
        "4 IcCell",
        "5 RootIndex",
        "6 DeoptExit",
        "7 HeapConst",
    ];
    assert_eq!(golden_lines()[..first.len()], first);
}

/// The unknown codes start at the top value.
#[test]
fn every_code_from_the_top_value_is_unknown() {
    for code in KIND_COUNT as u8..=u8::MAX {
        assert_eq!(RelocKind::from_code(code), None, "code {code}");
    }
}
