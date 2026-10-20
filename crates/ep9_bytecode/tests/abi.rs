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
