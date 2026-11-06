//! ABI version guard (H2: versioned ABI).
//!
//! The ABI fingerprint is SHA-256 over the opcode table text, the bytes of the tiny unit and
//! (from ABI 2) the runtime function table text.
//! `tests/golden/abi-history.txt` has one line per ABI version: `<version> <fingerprint>`.
//! - The last line must be the current (ABI_VERSION, fingerprint).
//! - A version never appears twice. So an opcode or layout change without an ABI_VERSION bump
//!   fails here, even after the other golden files are re-blessed.
//!
//! `EP9_BLESS=1` appends a line for a new version; it refuses to rewrite an existing one.
//!
//! The second guard (MC-P4-SLOTABI, option B; H02 "opcode and section appends"): the
//! fingerprint above does not hold the `FeedbackSlotKind` table, so
//! `tests/golden/slot-kind-history.txt` has one line per ABI version from ABI 5:
//! `<ABI_VERSION> <SHA-256 of slot_kind_table_text>`. The same rules hold: the last line is
//! the current (ABI_VERSION, fingerprint), and a version never maps to two fingerprints. So a
//! slot-kind append, rename or renumber without an ABI_VERSION bump fails here. Every ABI bump
//! adds a line to both files (`EP9_BLESS=1` writes both). A unit with an unknown kind byte
//! (0xff, the top of the byte) gives `LoadError::Malformed`, not `AbiMismatch`: this guard is
//! what sends a unit with a later slot kind to SOURCE (H35).

mod common;

use ep9_bytecode::*;

fn fingerprint() -> String {
    let mut h = sha256::Sha256::new();
    h.update(opcode_table_text().as_bytes());
    h.update(b"\n");
    h.update(&common::tiny_unit().to_bytes());
    h.update(b"\n");
    h.update(runtime_table_text().as_bytes());
    sha256::to_hex(&h.finish())
}

#[test]
fn abi_version_matches_history() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/abi-history.txt");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    let mut entries: Vec<(u32, String)> = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut parts = line.split_whitespace();
        let v: u32 = parts.next().unwrap().parse().expect("version number");
        let fp = parts.next().expect("fingerprint").to_string();
        assert!(
            entries.iter().all(|(old, _)| *old != v),
            "ABI version {v} appears twice in abi-history.txt"
        );
        entries.push((v, fp));
    }
    let fp = fingerprint();
    let current = (ABI_VERSION, fp.clone());
    if entries.last() == Some(&current) {
        return;
    }
    if let Some(&(v, _)) = entries.iter().find(|(v, _)| *v == ABI_VERSION) {
        panic!(
            "the bytecode ABI changed (fingerprint {fp}) but ABI_VERSION is still {v}. \
             Bump ABI_VERSION in crates/ep9_bytecode/src/unit.rs, re-bless the golden files, \
             and record the change in docs/adr/H02-bytecode-abi.md."
        );
    }
    if std::env::var_os("EP9_BLESS").is_some() {
        let mut out = if text.is_empty() {
            "# ep9 bytecode ABI history: <ABI_VERSION> <SHA-256 of opcodes.txt + tiny unit + runtime table>\n"
                .to_string()
        } else {
            text.clone()
        };
        out.push_str(&format!("{ABI_VERSION} {fp}\n"));
        std::fs::write(&path, out).unwrap();
        return;
    }
    panic!("ABI_VERSION {ABI_VERSION} is not in abi-history.txt; run with EP9_BLESS=1");
}

// ---- MC-P4-SLOTABI (option B) and MC-P4-SLOTTOP: the slot-kind table guard ---------------

/// The slot-kind table as stable text: one line `<byte> <name>` per kind, in byte order.
fn slot_kind_table_text(kinds: &[(u8, &str)]) -> String {
    kinds.iter().map(|(b, n)| format!("{b} {n}\n")).collect()
}

/// The (byte, name) list of the slot kinds of this build.
fn slot_kinds() -> Vec<(u8, &'static str)> {
    FeedbackSlotKind::ALL
        .iter()
        .map(|&k| (k as u8, k.name()))
        .collect()
}

fn slot_kind_fingerprint(kinds: &[(u8, &str)]) -> String {
    let mut h = sha256::Sha256::new();
    h.update(slot_kind_table_text(kinds).as_bytes());
    sha256::to_hex(&h.finish())
}

/// The `<version> <fingerprint>` lines of a history file, in file order.
fn history_lines(history: &str) -> Result<Vec<(u32, String)>, String> {
    let mut entries: Vec<(u32, String)> = Vec::new();
    for line in history
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut parts = line.split_whitespace();
        let v: u32 = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("bad history line {line:?}"))?;
        let fp = parts
            .next()
            .ok_or_else(|| format!("bad history line {line:?}"))?;
        if let Some(&(last, _)) = entries.last()
            && v <= last
        {
            return Err(format!("ABI version {v} follows {last} in the history"));
        }
        entries.push((v, fp.to_string()));
    }
    Ok(entries)
}

/// `Ok(true)` when the last line of the slot-kind history is (`abi`, `fingerprint`),
/// `Ok(false)` when `abi` is new (a bless appends it), and `Err` when the slot-kind table
/// changed under an ABI version the history already has, or the file is malformed.
fn slot_history_state(history: &str, abi: u32, fingerprint: &str) -> Result<bool, String> {
    let entries = history_lines(history)?;
    match entries.iter().position(|(v, _)| *v == abi) {
        Some(i) if entries[i].1 != fingerprint => Err(format!(
            "the FeedbackSlotKind table changed (fingerprint {fingerprint}) but ABI_VERSION is \
             still {abi}. A slot-kind append bumps ABI_VERSION in crates/ep9_bytecode/src/unit.rs \
             (H02 \"opcode and section appends\"); re-bless the golden files and record the \
             change in docs/adr/H02-bytecode-abi.md."
        )),
        Some(i) if i + 1 != entries.len() => Err(format!(
            "ABI_VERSION {abi} is not the newest version in slot-kind-history.txt"
        )),
        Some(_) => Ok(true),
        None => match entries.last() {
            Some(&(last, _)) if abi < last => Err(format!(
                "ABI_VERSION {abi} is older than {last} in slot-kind-history.txt"
            )),
            _ => Ok(false),
        },
    }
}

fn golden(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

/// MC-P4-SLOTABI (option B): the slot-kind table of this build is the last line of
/// `slot-kind-history.txt`, under the current ABI_VERSION; every version of that file is also
/// in `abi-history.txt`.
#[test]
fn slot_kind_table_matches_history() {
    let path = golden("slot-kind-history.txt");
    let text = read(&path);
    let fp = slot_kind_fingerprint(&slot_kinds());
    let abi: Vec<u32> = history_lines(&read(&golden("abi-history.txt")))
        .unwrap()
        .into_iter()
        .map(|(v, _)| v)
        .collect();
    for (v, _) in history_lines(&text).unwrap() {
        assert!(
            abi.contains(&v),
            "slot-kind-history.txt has ABI version {v}, abi-history.txt does not"
        );
    }
    match slot_history_state(&text, ABI_VERSION, &fp) {
        Ok(true) => {}
        Ok(false) if std::env::var_os("EP9_BLESS").is_some() => {
            let mut out = if text.is_empty() {
                "# ep9 FeedbackSlotKind table history (MC-P4-SLOTABI, from ABI 5): <ABI_VERSION> \
                 <SHA-256 of the slot-kind table text, one line \"<byte> <name>\" per kind>\n"
                    .to_string()
            } else {
                text
            };
            out.push_str(&format!("{ABI_VERSION} {fp}\n"));
            std::fs::write(&path, out).unwrap();
        }
        Ok(false) => panic!(
            "ABI_VERSION {ABI_VERSION} is not in slot-kind-history.txt; run with EP9_BLESS=1"
        ),
        Err(e) => panic!("{e}"),
    }
}

/// MC-P4-SLOTABI: the guard fails on a slot-only change with no ABI bump. The tables here are
/// scratch tables in memory (a kind no build has), so the test needs no change of
/// `FeedbackSlotKind` and no bless run can race it.
#[test]
fn a_slot_kind_append_without_an_abi_bump_fails() {
    let kinds = slot_kinds();
    let fp = slot_kind_fingerprint(&kinds);
    let history = format!("# history\n{ABI_VERSION} {fp}\n");
    assert_eq!(slot_history_state(&history, ABI_VERSION, &fp), Ok(true));

    // A scratch kind at the next free byte, a renamed kind and two kinds that swap bytes.
    let mut added = kinds.clone();
    added.push((kinds.len() as u8, "ScratchKind"));
    let mut renamed = kinds.clone();
    renamed[13].1 = "TypeOfValue";
    let mut swapped = kinds.clone();
    (swapped[1].0, swapped[2].0) = (swapped[2].0, swapped[1].0);
    swapped.swap(1, 2);
    for changed in [added, renamed, swapped] {
        assert_ne!(slot_kind_table_text(&changed), slot_kind_table_text(&kinds));
        let changed_fp = slot_kind_fingerprint(&changed);
        // No bump: refused.
        let e = slot_history_state(&history, ABI_VERSION, &changed_fp).unwrap_err();
        assert!(e.contains("ABI_VERSION is still"), "{e}");
        // With the bump, the new version is accepted (a bless appends it), and the file
        // with that line is current.
        assert_eq!(
            slot_history_state(&history, ABI_VERSION + 1, &changed_fp),
            Ok(false)
        );
        let next = format!("{history}{} {changed_fp}\n", ABI_VERSION + 1);
        assert_eq!(
            slot_history_state(&next, ABI_VERSION + 1, &changed_fp),
            Ok(true)
        );
        // A later slot-kind change under that same new version is refused again.
        assert!(slot_history_state(&next, ABI_VERSION + 1, &fp).is_err());
    }
    // An ABI bump with the same table (an opcode append) needs its line too.
    assert_eq!(
        slot_history_state(&history, ABI_VERSION + 1, &fp),
        Ok(false)
    );
    // Going back to an old version, or a file out of order, is refused.
    let two = format!("{history}{} aa\n", ABI_VERSION + 1);
    assert!(slot_history_state(&two, ABI_VERSION, &fp).is_err());
    assert!(slot_history_state("6 aa\n5 bb\n", 6, "aa").is_err());
    assert!(slot_history_state("5 aa\n5 aa\n", 5, "aa").is_err());
    assert!(slot_history_state("5\n", 5, "aa").is_err());
}

/// MC-P4-SLOTTOP: no kind has the top byte 0xff, or any byte past the last kind; a unit
/// whose slot-kind byte is 0xff is refused with `LoadError::Malformed` (outcome LOAD ERROR).
#[test]
fn the_top_slot_kind_byte_is_no_kind() {
    assert_eq!(FeedbackSlotKind::from_u8(0xff), None);
    for b in FeedbackSlotKind::ALL.len()..=0xff {
        assert_eq!(FeedbackSlotKind::from_u8(b as u8), None, "byte {b}");
    }
    // The slot-kind list of a function in the bytes: its u32 count, then one byte per kind.
    let unit = common::sample_script_unit();
    let bytes = unit.to_bytes();
    let f = unit
        .functions
        .iter()
        .find(|f| f.feedback_slots.len() > 1)
        .expect("a function with feedback slots");
    let mut list = (f.feedback_slots.len() as u32).to_le_bytes().to_vec();
    list.extend(f.feedback_slots.iter().map(|&k| k as u8));
    let at = bytes
        .windows(list.len())
        .position(|w| w == list)
        .expect("the slot-kind list is in the bytes")
        + 4;
    for b in [0xff, FeedbackSlotKind::ALL.len() as u8] {
        let mut m = bytes.clone();
        m[at] = b;
        let h = compute_content_hash(&m);
        m[header_offsets::CONTENT_HASH..header_offsets::CONTENT_HASH + 32].copy_from_slice(&h);
        let e = CompiledUnit::from_bytes(&m).unwrap_err();
        assert!(
            matches!(
                e,
                LoadError::Malformed {
                    what: "feedback slot kind",
                    ..
                }
            ),
            "byte {b:#x}: {e:?}"
        );
        assert_eq!(e.outcome(), LoadOutcome::LoadError);
    }
    // The control: the same bytes with the kind byte unchanged load.
    assert!(CompiledUnit::from_bytes(&bytes).is_ok());
}
