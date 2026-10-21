//! Test-only unit encoder (the writer is A2, not week 4), hosts, a PRNG and golden helpers.
//!
//! The encoder lays sections out and hashes them on its own: it uses the library only for the
//! 256 header bytes and the 32-byte entries, so a layout or hash bug in `check` cannot hide
//! behind the same bug here.
#![allow(dead_code)] // each test file uses a part of this module

use std::path::PathBuf;

use ep9_bytecode::{sha256, unit_flags};
use ep9_unit::*;

/// A unit before encoding. `header.total_size`, `section_count` and `content_hash` are
/// computed by [`TestUnit::encode`]; every other header field is written as given.
#[derive(Clone, Debug)]
pub struct TestUnit {
    pub header: UnitHeader,
    pub sections: Vec<(SectionKind, Vec<u8>)>,
}

/// Deterministic filler bytes.
pub fn filler(seed: u64, n: usize) -> Vec<u8> {
    let mut r = Rng::new(seed);
    (0..n).map(|_| r.next() as u8).collect()
}

/// A header that matches `host`, for a strict script.
pub fn header_for(host: &HostInfo, cpu_required: u64) -> UnitHeader {
    UnitHeader {
        total_size: 0,
        section_count: 0,
        header_flags: 0,
        content_hash: [0; 32],
        engine_build_id: host.engine_build_id,
        versions: host.versions,
        target_arch: host.target_arch,
        target_abi: host.target_abi,
        heap_page_log2: host.heap_page_log2,
        source_flags: unit_flags::STRICT,
        cpu_required,
    }
}

impl TestUnit {
    /// Bytecode, source and hints: no native code. Odd sizes, so every section after the
    /// first has padding before it.
    pub fn small(host: &HostInfo) -> TestUnit {
        TestUnit {
            header: header_for(host, 0),
            sections: vec![
                (SectionKind::Bytecode, filler(1, 37)),
                (SectionKind::Source, b"'use strict'; 1 + 2;\n".to_vec()),
                (SectionKind::Hints, filler(3, 21)),
            ],
        }
    }

    /// Native code and a signature: text, rodata, RW template, bytecode, source, unwind,
    /// signature. Requires the host's LSE bit when the host has it.
    pub fn native(host: &HostInfo) -> TestUnit {
        let mut header = header_for(host, host.cpu_features & cpu::A64_LSE);
        header.header_flags = header_flags::SIGNED;
        TestUnit {
            header,
            sections: vec![
                (SectionKind::Text, filler(10, 29)),
                (SectionKind::Rodata, filler(11, 11)),
                (SectionKind::RwTemplate, filler(12, 17)),
                (SectionKind::Bytecode, filler(13, 37)),
                (SectionKind::Source, b"export const x = 1;\n".to_vec()),
                (SectionKind::Unwind, filler(15, 9)),
                (SectionKind::Signature, filler(16, 64)),
            ],
        }
    }

    pub fn without(mut self, kind: SectionKind) -> TestUnit {
        self.sections.retain(|(k, _)| *k != kind);
        self
    }

    /// The canonical bytes: header, table, sections in kind order at 8-aligned offsets with
    /// zero padding, then the content hash.
    pub fn encode(&self) -> Vec<u8> {
        let mut sections = self.sections.clone();
        sections.sort_by_key(|(k, _)| *k);
        let table_end = SECTION_TABLE_OFFSET + sections.len() * SECTION_ENTRY_SIZE;
        let mut entries = Vec::new();
        let mut body = Vec::new();
        let mut end = table_end;
        for (kind, data) in &sections {
            let align = 1usize << kind.align_log2();
            let start = end.div_ceil(align) * align;
            body.resize(body.len() + (start - end), 0);
            entries.push(SectionEntry {
                kind: *kind,
                offset: start as u64,
                size: data.len() as u64,
            });
            body.extend_from_slice(data);
            end = start + data.len();
        }
        let mut h = self.header;
        h.total_size = end as u64;
        h.section_count = sections.len() as u32;
        h.content_hash = [0; 32];
        let mut out = h.encode().to_vec();
        for e in &entries {
            out.extend_from_slice(&e.encode());
        }
        out.extend_from_slice(&body);
        assert_eq!(out.len(), end);
        rehash(&mut out);
        out
    }

    /// The unit that `bytes` encodes, read field by field (no validation beyond the codecs).
    pub fn parse(bytes: &[u8]) -> Option<TestUnit> {
        let header = UnitHeader::decode(bytes).ok()?;
        let mut sections = Vec::new();
        for i in 0..header.section_count as usize {
            let at = SECTION_TABLE_OFFSET + i * SECTION_ENTRY_SIZE;
            let e = SectionEntry::decode(bytes.get(at..at + SECTION_ENTRY_SIZE)?).ok()?;
            let start = usize::try_from(e.offset).ok()?;
            let end = start.checked_add(usize::try_from(e.size).ok()?)?;
            sections.push((e.kind, bytes.get(start..end)?.to_vec()));
        }
        Some(TestUnit { header, sections })
    }
}

/// The signature section's byte range, read leniently from the table (any in-bounds entry of
/// kind 9). A unit whose table is invalid fails step 1 anyway.
fn signature_range(bytes: &[u8]) -> Option<(usize, usize)> {
    let count = u32::from_le_bytes(bytes.get(24..28)?.try_into().ok()?) as usize;
    for i in 0..count.min(MAX_SECTIONS as usize) {
        let at = SECTION_TABLE_OFFSET + i * SECTION_ENTRY_SIZE;
        let e = bytes.get(at..at + SECTION_ENTRY_SIZE)?;
        if u32::from_le_bytes(e[0..4].try_into().unwrap()) == SectionKind::Signature as u32 {
            let start = u64::from_le_bytes(e[8..16].try_into().unwrap());
            let size = u64::from_le_bytes(e[16..24].try_into().unwrap());
            let end = start.checked_add(size)?;
            if start >= HEADER_SIZE as u64 && end <= bytes.len() as u64 {
                return Some((start as usize, end as usize));
            }
        }
    }
    None
}

/// The content hash by the format rule: SHA-256 of the unit with the hash field and the
/// signature section set to zero.
pub fn content_hash(bytes: &[u8]) -> [u8; 32] {
    let mut copy = bytes.to_vec();
    copy[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].fill(0);
    if let Some((s, e)) = signature_range(bytes) {
        copy[s..e].fill(0);
    }
    sha256::sha256(&copy)
}

/// Writes the content hash of `bytes` into its hash field, as a writer would.
pub fn rehash(bytes: &mut [u8]) {
    if bytes.len() < HEADER_SIZE {
        return;
    }
    let h = content_hash(bytes);
    bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].copy_from_slice(&h);
}

/// A host whose every value differs from this build's, so the tests do not depend on the
/// current constants: all versions above 0, AArch64 Apple, 256 KiB pages, three CPU bits.
pub fn synthetic_host() -> HostInfo {
    HostInfo {
        engine_build_id: *b"ep9-unit-test-01",
        versions: Versions {
            bytecode_abi: 7,
            value_encoding: 3,
            object_layout: 4,
            barrier: 5,
            frame: 6,
            ic: 8,
            deopt_meta: 9,
            fact_schema: 10,
            builtin_abi: 11,
        },
        target_arch: target_arch::AARCH64,
        target_abi: target_abi::APPLE,
        heap_page_log2: 18,
        cpu_features: cpu::A64_LSE | cpu::A64_JSCONV | cpu::A64_FP16,
    }
}

/// splitmix64.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

pub fn blessing() -> bool {
    std::env::var_os("EP9_BLESS").is_some()
}

/// Compares `actual` with a golden file; `EP9_BLESS=1` rewrites the file.
pub fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if blessing() {
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

pub fn read_golden(name: &str) -> String {
    std::fs::read_to_string(golden_path(name))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

pub fn write_golden(name: &str, text: &str) {
    std::fs::write(golden_path(name), text).unwrap();
}

/// Hex dump, 16 bytes per line, with offsets (the CompiledUnit golden format).
pub fn hexdump(bytes: &[u8]) -> String {
    let mut s = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        s.push_str(&format!("{:06x}  {}\n", i * 16, hex.join(" ")));
    }
    s
}
