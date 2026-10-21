//! Header codec, golden files and the container history (H35-Q1).
//!
//! - `header-v0.hex`: [`sample`] encoded. Every field has a distinct value, so a moved field
//!   shows up in the diff.
//! - `header-layout.txt`: `layout_text()`, the whole container contract.
//! - `container-history.txt`: one line per container version, `<version> <SHA-256 of the
//!   layout text>`. A layout change without a CONTAINER_VERSION bump fails here, even after
//!   the other two files are re-blessed.
//!
//! `EP9_BLESS=1 cargo test -p ep9_unit --test header` rewrites the first two files and appends
//! a line for a new container version; it never rewrites an existing history line.

mod common;

use common::*;
use ep9_bytecode::{sha256, unit_flags};
use ep9_unit::*;

/// A header with a distinct value in every field.
fn sample() -> UnitHeader {
    UnitHeader {
        total_size: 0x1817_1615_1413_1211,
        section_count: 9,
        header_flags: header_flags::SIGNED,
        content_hash: core::array::from_fn(|i| 0xc0 + i as u8),
        engine_build_id: core::array::from_fn(|i| 0xa0 + i as u8),
        versions: Versions {
            bytecode_abi: 0x21,
            value_encoding: 0x22,
            object_layout: 0x23,
            barrier: 0x24,
            frame: 0x25,
            ic: 0x26,
            deopt_meta: 0x27,
            fact_schema: 0x28,
            builtin_abi: 0x29,
        },
        target_arch: target_arch::AARCH64,
        target_abi: target_abi::WINDOWS,
        heap_page_log2: 18,
        source_flags: 1 // module
            | unit_flags::STRICT
            | unit_flags::TYPESCRIPT
            | unit_flags::TS_USE_DEFINE_FOR_CLASS_FIELDS,
        cpu_required: cpu::A64_LSE | cpu::A64_JSCONV,
    }
}

#[test]
fn header_v0_bytes() {
    check_golden("header-v0.hex", &hexdump(&sample().encode()));
}

#[test]
fn layout_text_matches_the_golden_file() {
    check_golden("header-layout.txt", &layout_text());
}

#[test]
fn decode_round_trips() {
    let bytes = sample().encode();
    assert_eq!(UnitHeader::decode(&bytes), Ok(sample()));
    // Extra bytes after the header are not the header's business.
    let mut longer = bytes.to_vec();
    longer.extend_from_slice(&[0xee; 40]);
    assert_eq!(UnitHeader::decode(&longer), Ok(sample()));

    let valid_flags = [
        0,
        unit_flags::STRICT,
        1 | unit_flags::STRICT,
        2,
        unit_flags::TYPESCRIPT,
        unit_flags::TYPESCRIPT
            | unit_flags::TS_EXPERIMENTAL_DECORATORS
            | unit_flags::TS_EMIT_DECORATOR_METADATA,
    ];
    let mut rng = Rng::new(42);
    for _ in 0..5000 {
        let mut h = sample();
        h.total_size = rng.next();
        h.section_count = MIN_SECTIONS + rng.below((MAX_SECTIONS - MIN_SECTIONS + 1) as u64) as u32;
        h.header_flags = (rng.next() & 1) as u8;
        h.content_hash = core::array::from_fn(|_| rng.next() as u8);
        h.engine_build_id = core::array::from_fn(|_| rng.next() as u8);
        for c in Contract::ALL {
            h.versions.set(c, rng.next() as u32);
        }
        h.target_arch = rng.next() as u8;
        h.target_abi = rng.next() as u8;
        h.heap_page_log2 = rng.next() as u8;
        h.source_flags = valid_flags[rng.below(valid_flags.len() as u64) as usize];
        h.cpu_required = rng.next();
        let b = h.encode();
        assert_eq!(UnitHeader::decode(&b), Ok(h));
        assert_eq!(UnitHeader::decode(&b).unwrap().encode(), b);
    }
}

#[test]
fn fields_cover_every_byte_once() {
    let mut at = 0;
    for f in &HEADER_FIELDS {
        assert_eq!(
            f.offset, at,
            "header field {} is not where the last one ended",
            f.name
        );
        assert!(f.size > 0);
        at += f.size;
    }
    assert_eq!(at, HEADER_SIZE);
    let mut at = 0;
    for f in &ENTRY_FIELDS {
        assert_eq!(f.offset, at, "entry field {}", f.name);
        at += f.size;
    }
    assert_eq!(at, SECTION_ENTRY_SIZE);
    let mut names: Vec<&str> = HEADER_FIELDS.iter().map(|f| f.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), HEADER_FIELDS.len());
    // The frozen prefix ends on a field boundary.
    assert!(HEADER_FIELDS.iter().any(|f| f.offset == FROZEN_PREFIX_SIZE));
}

/// Changes one header field by name. False for the fields that have one value (fixed or
/// reserved), which `encode` always writes the same way.
fn perturb(h: &mut UnitHeader, name: &str) -> bool {
    match name {
        "total_size" => h.total_size ^= 0x0101_0101_0101_0101,
        "section_count" => h.section_count = 3,
        "header_flags" => h.header_flags ^= header_flags::SIGNED,
        "content_hash" => h.content_hash = [0x5a; 32],
        "engine_build_id" => h.engine_build_id = [0x6b; 16],
        "target_arch" => h.target_arch ^= 0xff,
        "target_abi" => h.target_abi ^= 0xff,
        "heap_page_log2" => h.heap_page_log2 ^= 0xff,
        "source_flags" => h.source_flags = 0,
        "cpu_required" => h.cpu_required ^= u64::MAX,
        other => match Contract::ALL.into_iter().find(|c| c.name() == other) {
            Some(c) => h.versions.set(c, !h.versions.get(c)),
            None => return false,
        },
    }
    true
}

#[test]
fn layout_table_matches_the_codec() {
    let base = sample();
    let base_bytes = base.encode();
    let mut perturbed = 0;
    for f in &HEADER_FIELDS {
        let range = f.offset..f.offset + f.size;
        let mut h = base;
        if perturb(&mut h, f.name) {
            perturbed += 1;
            let b = h.encode();
            let changed: Vec<usize> = (0..HEADER_SIZE)
                .filter(|&i| b[i] != base_bytes[i])
                .collect();
            assert!(!changed.is_empty(), "{}: no byte changed", f.name);
            assert!(
                changed.iter().all(|i| range.contains(i)),
                "{}: bytes {changed:?} changed, outside {range:?}",
                f.name
            );
            continue;
        }
        let expected: Vec<u8> = match f.name {
            "magic" => MAGIC.to_vec(),
            "container_version" => CONTAINER_VERSION.to_le_bytes().to_vec(),
            "header_size" => (HEADER_SIZE as u32).to_le_bytes().to_vec(),
            "section_table_offset" => (SECTION_TABLE_OFFSET as u32).to_le_bytes().to_vec(),
            "hash_alg" => vec![HASH_SHA256],
            n if n.starts_with("reserved") => vec![0; f.size],
            n => panic!("field {n} has no rule in this test"),
        };
        assert_eq!(&base_bytes[range], &expected[..], "{}", f.name);
    }
    assert_eq!(perturbed, 19);
    assert_eq!(offsets::RESERVED2 + 116, HEADER_SIZE);
}

#[test]
fn decode_rejects_bad_headers() {
    let good = sample().encode();
    let with = |at: usize, bytes: &[u8]| {
        let mut b = good;
        b[at..at + bytes.len()].copy_from_slice(bytes);
        UnitHeader::decode(&b)
    };
    let le = |v: u32| v.to_le_bytes();
    assert_eq!(UnitHeader::decode(&good[..31]), Err(BadUnit::Truncated));
    assert_eq!(UnitHeader::decode(&good[..255]), Err(BadUnit::Truncated));
    assert_eq!(UnitHeader::decode(&[]), Err(BadUnit::Truncated));
    assert_eq!(with(0, &[0x88]), Err(BadUnit::Magic));
    assert_eq!(with(7, b"\r"), Err(BadUnit::Magic));
    assert_eq!(
        with(offsets::CONTAINER_VERSION, &le(2)),
        Err(BadUnit::ContainerVersion(2))
    );
    assert_eq!(
        with(offsets::HEADER_SIZE, &le(160)),
        Err(BadUnit::HeaderSize)
    );
    assert_eq!(
        with(offsets::SECTION_TABLE_OFFSET, &le(264)),
        Err(BadUnit::SectionTable("table offset"))
    );
    for n in [0, 1, MAX_SECTIONS + 1, u32::MAX] {
        assert_eq!(
            with(offsets::SECTION_COUNT, &le(n)),
            Err(BadUnit::SectionTable("section count")),
            "count {n}"
        );
    }
    assert_eq!(with(offsets::HASH_ALG, &[2]), Err(BadUnit::HashAlg));
    assert_eq!(with(offsets::HASH_ALG, &[0]), Err(BadUnit::HashAlg));
    for bit in 1..8 {
        assert_eq!(
            with(offsets::HEADER_FLAGS, &[1 << bit]),
            Err(BadUnit::HeaderFlags)
        );
    }
    for f in HEADER_FIELDS
        .iter()
        .filter(|f| f.name.starts_with("reserved"))
    {
        for i in f.offset..f.offset + f.size {
            assert_eq!(with(i, &[0x01]), Err(BadUnit::Reserved), "byte {i}");
        }
    }
    let bad_flags = [
        3,                                                           // code kind 3
        unit_flags::TS_USE_DEFINE_FOR_CLASS_FIELDS,                  // TS flag on JavaScript
        unit_flags::TS_EMIT_DECORATOR_METADATA | unit_flags::STRICT, // TS flag on JavaScript
        unit_flags::HAS_SOURCE,                                      // a CompiledUnit section bit
        unit_flags::HAS_HINTS,
        unit_flags::TS_RESERVED,
        1 << 31,
    ];
    for fl in bad_flags {
        assert_eq!(
            with(offsets::SOURCE_FLAGS, &le(fl)),
            Err(BadUnit::SourceFlags),
            "{fl:#x}"
        );
        assert!(!source_flags_valid(fl));
    }
}

#[test]
fn section_entries_round_trip_and_reject_bad_bytes() {
    for k in SectionKind::ALL {
        let e = SectionEntry {
            kind: k,
            offset: 0x0102_0304_0506_0708,
            size: 0x1112_1314_1516_1718,
        };
        let b = e.encode();
        assert_eq!(SectionEntry::decode(&b), Ok(e));
        assert_eq!(SectionKind::from_u32(k as u32), Some(k));
        assert_eq!(b[entry_offsets::ALIGN_LOG2], 3);
    }
    let good = SectionEntry {
        kind: SectionKind::Hints,
        offset: 512,
        size: 8,
    }
    .encode();
    let with = |at: usize, v: u8| {
        let mut b = good;
        b[at] = v;
        SectionEntry::decode(&b)
    };
    for kind in [0u8, 10, 0xff] {
        assert_eq!(
            with(entry_offsets::KIND, kind),
            Err(BadUnit::SectionTable("unknown section kind"))
        );
    }
    assert_eq!(
        with(entry_offsets::FLAGS, 1),
        Err(BadUnit::SectionTable("section flags"))
    );
    assert_eq!(
        with(entry_offsets::ALIGN_LOG2, 4),
        Err(BadUnit::SectionTable("section alignment"))
    );
    for i in entry_offsets::RESERVED..SECTION_ENTRY_SIZE {
        assert_eq!(
            with(i, 1),
            Err(BadUnit::SectionTable("reserved entry bytes"))
        );
    }
    assert_eq!(
        SectionEntry::decode(&good[..31]),
        Err(BadUnit::SectionTable("truncated table"))
    );
}

/// The history state for (`version`, `fingerprint`): `Ok(true)` when the last line is exactly
/// that pair, `Ok(false)` when `version` is new (a bless appends it), and `Err` when the layout
/// changed under a version that the history already has, or the file is malformed.
fn history_state(history: &str, version: u32, fingerprint: &str) -> Result<bool, String> {
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
            return Err(format!(
                "container version {v} follows {last} in container-history.txt"
            ));
        }
        entries.push((v, fp.to_string()));
    }
    match entries.iter().position(|(v, _)| *v == version) {
        Some(i) if entries[i].1 != fingerprint => Err(format!(
            "the container layout changed (fingerprint {fingerprint}) but CONTAINER_VERSION is \
             still {version}. Bump CONTAINER_VERSION in crates/ep9_unit/src/header.rs, re-bless \
             the golden files, and record the change in docs/adr/H35-aot-unit-format.md."
        )),
        Some(i) if i + 1 != entries.len() => Err(format!(
            "CONTAINER_VERSION {version} is not the newest version in container-history.txt"
        )),
        Some(_) => Ok(true),
        None => match entries.last() {
            Some(&(last, _)) if version < last => Err(format!(
                "CONTAINER_VERSION {version} is older than {last} in container-history.txt"
            )),
            _ => Ok(false),
        },
    }
}

#[test]
fn container_history_pins_the_layout() {
    let fp = sha256::to_hex(&sha256::sha256(layout_text().as_bytes()));
    let text = read_golden("container-history.txt");
    match history_state(&text, CONTAINER_VERSION, &fp) {
        Ok(true) => {}
        Ok(false) if blessing() => {
            let mut out = if text.is_empty() {
                "# ep9 unit container history: <CONTAINER_VERSION> <SHA-256 of header-layout.txt>\n"
                    .to_string()
            } else {
                text
            };
            out.push_str(&format!("{CONTAINER_VERSION} {fp}\n"));
            write_golden("container-history.txt", &out);
        }
        Ok(false) => panic!(
            "CONTAINER_VERSION {CONTAINER_VERSION} is not in container-history.txt; run with \
             EP9_BLESS=1"
        ),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn a_layout_change_without_a_version_bump_fails_the_history_check() {
    // An in-memory history like the golden file (which container_history_pins_the_layout
    // checks), so a bless run that rewrites the file cannot race this test.
    let fp = sha256::to_hex(&sha256::sha256(layout_text().as_bytes()));
    let text = format!("# history\n{CONTAINER_VERSION} {fp}\n");
    assert_eq!(history_state(&text, CONTAINER_VERSION, &fp), Ok(true));
    // One changed byte in the layout text (here: one more reserved byte), same version.
    let changed = layout_text().replace("140 116 reserved2", "140 117 reserved2");
    assert_ne!(changed, layout_text());
    let changed_fp = sha256::to_hex(&sha256::sha256(changed.as_bytes()));
    let e = history_state(&text, CONTAINER_VERSION, &changed_fp).unwrap_err();
    assert!(e.contains("Bump CONTAINER_VERSION"), "{e}");
    // With a bump, the new version is accepted (a bless appends it).
    assert_eq!(
        history_state(&text, CONTAINER_VERSION + 1, &changed_fp),
        Ok(false)
    );
    // Going back to an old version, or rewriting the order, is refused.
    let two = format!("{text}{} {changed_fp}\n", CONTAINER_VERSION + 1);
    assert!(history_state(&two, CONTAINER_VERSION, &fp).is_err());
    assert!(history_state("2 aa\n1 bb\n", 2, "aa").is_err());
    assert!(history_state("1 aa\n1 aa\n", 1, "aa").is_err());
}
