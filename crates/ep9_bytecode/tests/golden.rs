//! Golden files (gates "disasm", "opcode-list", "unit-v0", "determinism").
//!
//! `EP9_BLESS=1 cargo test -p ep9_bytecode --test golden` rewrites the files. Review the diff:
//! a change to `opcodes.txt` or `tiny-unit.hex` is an ABI change and needs an ABI_VERSION
//! bump (tests/abi.rs enforces it).

mod common;

use std::path::PathBuf;

use ep9_bytecode::*;

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

pub fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("EP9_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", path.display()))
        .replace("\r\n", "\n");
    if expected != actual {
        let line = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(expected.lines().count().min(actual.lines().count()));
        panic!(
            "{name} differs from the golden file at line {}:\n  golden: {:?}\n  actual: {:?}\n\
             If the change is intended, run with EP9_BLESS=1 and review the diff.",
            line + 1,
            expected.lines().nth(line),
            actual.lines().nth(line)
        );
    }
}

/// Hex dump, 16 bytes per line, with offsets.
pub fn hexdump(bytes: &[u8]) -> String {
    let mut s = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&format!("{:06x}  {}\n", i * 16, hex.join(" ")));
    }
    s
}

#[test]
fn opcode_table() {
    check_golden("opcodes.txt", &opcode_table_text());
}

#[test]
fn sample_script_disassembly() {
    check_golden(
        "sample-script.txt",
        &disassemble(&common::sample_script_unit()),
    );
}

#[test]
fn sample_module_disassembly() {
    check_golden(
        "sample-module.txt",
        &disassemble(&common::sample_module_unit()),
    );
}

#[test]
fn tiny_unit_bytes() {
    let u = common::tiny_unit();
    check_golden("tiny-unit.hex", &hexdump(&u.to_bytes()));
}

/// Content hashes of the samples. CI runs this on all 4 targets against one file, so it
/// also checks that the bytes do not depend on the target.
#[test]
fn content_hashes() {
    let mut s = String::new();
    for (name, unit) in [
        ("tiny", common::tiny_unit()),
        ("sample-script", common::sample_script_unit()),
        ("sample-module", common::sample_module_unit()),
    ] {
        let bytes = unit.to_bytes();
        s.push_str(&format!(
            "{name} {} {}\n",
            sha256::to_hex(&unit.content_hash()),
            bytes.len()
        ));
    }
    // A digest of 200 random units: pins the generator and the encoder together.
    let mut h = sha256::Sha256::new();
    for seed in 0..200 {
        let u = common::random_unit(seed).build_in_order().unwrap();
        h.update(&u.content_hash());
    }
    s.push_str(&format!("random-0..200 {}\n", sha256::to_hex(&h.finish())));
    check_golden("hashes.txt", &s);
}
