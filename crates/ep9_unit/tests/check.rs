//! The load check (H35 S4 steps 1-4) on units from the test encoder.
//!
//! The table test mutates every header field of `HEADER_FIELDS` (every bit, plus random and
//! extreme values) and expects the field's listed outcome, which is also what
//! `header-layout.txt` says. A mutation without a recomputed hash is always a LOAD ERROR.

mod common;

use common::*;
use ep9_unit::*;

fn units(host: &HostInfo) -> [(&'static str, Vec<u8>); 2] {
    [
        ("small", TestUnit::small(host).encode()),
        ("native", TestUnit::native(host).encode()),
    ]
}

#[test]
fn a_test_encoded_unit_loads_and_another_abi_gives_source() {
    let host = HostInfo::current();
    let unit = TestUnit::small(&host);
    assert_eq!(verdict(&unit.encode(), &host), Verdict::Load);
    assert_eq!(check(&unit.encode(), &host), Outcome::Load);

    let mut other = unit.clone();
    other.header.versions.bytecode_abi += 1;
    assert_eq!(check(&other.encode(), &host), Outcome::Source);
    assert_eq!(
        verdict(&other.encode(), &host),
        Verdict::Version {
            contract: Contract::BytecodeAbi,
            unit: ep9_bytecode::ABI_VERSION + 1,
            host: ep9_bytecode::ABI_VERSION,
            drop_hints: false,
        }
    );
}

#[test]
fn native_code_needs_every_native_contract_above_version_0() {
    // This build: the deopt-metadata version is still 0, so a unit with a text section runs
    // its bytecode, and a unit without one loads.
    let host = HostInfo::current();
    let first_zero = Contract::ALL
        .into_iter()
        .find(|&c| c.is_native() && host.versions.get(c) == 0);
    assert_eq!(first_zero, Some(Contract::DeoptMeta));
    let native = TestUnit::native(&host).encode();
    assert_eq!(
        verdict(&native, &host),
        Verdict::NoNativeForm(Contract::DeoptMeta)
    );
    assert_eq!(check(&native, &host), Outcome::Bytecode);
    let no_text = TestUnit::native(&host).without(SectionKind::Text).encode();
    assert_eq!(check(&no_text, &host), Outcome::Load);

    // A host with every version above 0 loads native code.
    let mut host = synthetic_host();
    assert_eq!(
        check(&TestUnit::native(&host).encode(), &host),
        Outcome::Load
    );
    // Each native contract at 0 (on both sides) sends a text unit to its bytecode.
    for c in Contract::ALL {
        host.versions = synthetic_host().versions;
        host.versions.set(c, 0);
        let v = verdict(&TestUnit::native(&host).encode(), &host);
        if c.is_native() {
            assert_eq!(v, Verdict::NoNativeForm(c), "{c:?}");
        } else {
            assert_eq!(v, Verdict::Load, "{c:?}");
        }
        assert_eq!(
            check(&TestUnit::small(&host).encode(), &host),
            Outcome::Load,
            "{c:?}"
        );
    }
}

/// The outcome a changed field must give after a recomputed hash.
fn expected(f: &Field, new: &[u8], host: &HostInfo) -> Outcome {
    match f.on_change {
        OnChange::LoadError => Outcome::LoadError,
        OnChange::Source => Outcome::Source,
        OnChange::Bytecode | OnChange::BytecodeDropHints => Outcome::Bytecode,
        OnChange::CpuSubset => {
            let v = u64::from_le_bytes(new.try_into().unwrap());
            if v & !(host.cpu_features & cpu::DEFINED) == 0 {
                Outcome::Load
            } else {
                Outcome::Bytecode
            }
        }
        OnChange::SourceFlags => {
            if source_flags_valid(u32::from_le_bytes(new.try_into().unwrap())) {
                Outcome::Load
            } else {
                Outcome::LoadError
            }
        }
        OnChange::Table => unreachable!("not a header field"),
    }
}

/// New values for a field of `size` bytes: every single-bit flip, all zeros, all ones and 64
/// random values.
fn new_values(old: &[u8], rng: &mut Rng) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for i in 0..old.len() * 8 {
        let mut v = old.to_vec();
        v[i / 8] ^= 1 << (i % 8);
        out.push(v);
    }
    out.push(vec![0; old.len()]);
    out.push(vec![0xff; old.len()]);
    for _ in 0..64 {
        out.push((0..old.len()).map(|_| rng.next() as u8).collect());
    }
    out.retain(|v| v != old);
    out
}

#[test]
fn every_header_field_mutation_gives_its_listed_outcome() {
    let host = synthetic_host();
    let mut rng = Rng::new(7);
    let mut counts = [0usize; 4];
    for (name, base) in units(&host) {
        assert_eq!(verdict(&base, &host), Verdict::Load, "{name}");
        for f in &HEADER_FIELDS {
            let range = f.offset..f.offset + f.size;
            for new in new_values(&base[range.clone()], &mut rng) {
                let mut m = base.clone();
                m[range.clone()].copy_from_slice(&new);
                // Without a recomputed hash: always a load error.
                assert_eq!(
                    check(&m, &host),
                    Outcome::LoadError,
                    "{name}: {} = {new:02x?} without rehash",
                    f.name
                );
                if f.name == "content_hash" {
                    continue; // a rehash would undo the change
                }
                rehash(&mut m);
                let v = verdict(&m, &host);
                let want = expected(f, &new, &host);
                assert_eq!(v.outcome(), want, "{name}: {} = {new:02x?}: {v}", f.name);
                assert_eq!(
                    v.drops_hints(),
                    f.on_change == OnChange::BytecodeDropHints,
                    "{name}: {}",
                    f.name
                );
                counts[want as usize] += 1;
            }
        }
    }
    println!("header mutations: load/bytecode/source/load-error = {counts:?}");
    assert!(counts.iter().all(|&n| n > 0));
}

#[test]
fn a_fact_schema_mismatch_drops_the_hints_whatever_else_differs() {
    // Step 4 reports the first difference, an object-layout one here, but the hints still go.
    let host = synthetic_host();
    let mut u = TestUnit::small(&host);
    u.header.versions.object_layout += 1;
    u.header.versions.fact_schema += 1;
    let v = verdict(&u.encode(), &host);
    assert!(matches!(
        v,
        Verdict::Version {
            contract: Contract::ObjectLayout,
            ..
        }
    ));
    assert_eq!(v.outcome(), Outcome::Bytecode);
    assert!(v.drops_hints(), "{v}");

    // Every other field mutated on top of a fact-schema mismatch: BYTECODE always drops the
    // hints; SOURCE and LOAD ERROR (steps 1-3 run first) use no section at all.
    let mut rng = Rng::new(13);
    let mut counts = [0usize; 4];
    for (name, mut u) in [
        ("small", TestUnit::small(&host)),
        ("native", TestUnit::native(&host)),
    ] {
        u.header.versions.fact_schema += 1;
        let base = u.encode();
        for f in &HEADER_FIELDS {
            if f.name == "content_hash" || f.name == "fact_schema" {
                continue;
            }
            let range = f.offset..f.offset + f.size;
            for new in new_values(&base[range.clone()], &mut rng) {
                let mut m = base.clone();
                m[range.clone()].copy_from_slice(&new);
                rehash(&mut m);
                let v = verdict(&m, &host);
                let want = match expected(f, &new, &host) {
                    Outcome::Load => Outcome::Bytecode,
                    other => other,
                };
                assert_eq!(v.outcome(), want, "{name}: {} = {new:02x?}: {v}", f.name);
                assert_eq!(
                    v.drops_hints(),
                    want == Outcome::Bytecode,
                    "{name}: {} = {new:02x?}: {v}",
                    f.name
                );
                counts[want as usize] += 1;
            }
        }
    }
    println!("with a fact-schema mismatch: load/bytecode/source/load-error = {counts:?}");
    assert!(counts[1..].iter().all(|&n| n > 0));
}

#[test]
fn every_entry_mutation_is_a_load_error_or_another_canonical_unit() {
    let host = synthetic_host();
    let mut rng = Rng::new(11);
    let (mut errors, mut others) = (0, 0);
    for (name, base) in units(&host) {
        let count = UnitHeader::decode(&base).unwrap().section_count as usize;
        for i in 0..count {
            let entry = SECTION_TABLE_OFFSET + i * SECTION_ENTRY_SIZE;
            for f in &ENTRY_FIELDS {
                let range = entry + f.offset..entry + f.offset + f.size;
                for new in new_values(&base[range.clone()], &mut rng) {
                    let mut m = base.clone();
                    m[range.clone()].copy_from_slice(&new);
                    assert_eq!(
                        check(&m, &host),
                        Outcome::LoadError,
                        "{name}: entry {i} {} without rehash",
                        f.name
                    );
                    rehash(&mut m);
                    if check(&m, &host) == Outcome::LoadError {
                        errors += 1;
                        continue;
                    }
                    // Still canonical: the unit it describes encodes to exactly these bytes.
                    others += 1;
                    let u = TestUnit::parse(&m).expect("a unit that passes step 1 parses");
                    assert_eq!(u.encode(), m, "{name}: entry {i} {} = {new:02x?}", f.name);
                }
            }
        }
    }
    println!("entry mutations: {errors} load errors, {others} other canonical units");
    assert!(errors > 0 && others > 0);
}

#[test]
fn required_sections_and_the_signature_flag() {
    let host = synthetic_host();
    for k in [SectionKind::Bytecode, SectionKind::Source] {
        for unit in [TestUnit::small(&host), TestUnit::native(&host)] {
            assert_eq!(
                verdict(&unit.without(k).encode(), &host),
                Verdict::LoadError(BadUnit::MissingSection(k))
            );
        }
    }
    // Optional sections may go.
    assert_eq!(
        check(
            &TestUnit::small(&host).without(SectionKind::Hints).encode(),
            &host
        ),
        Outcome::Load
    );
    for k in [
        SectionKind::Text,
        SectionKind::Rodata,
        SectionKind::RwTemplate,
        SectionKind::Unwind,
    ] {
        assert_eq!(
            check(&TestUnit::native(&host).without(k).encode(), &host),
            Outcome::Load,
            "{k:?}"
        );
    }
    // The signed flag and the signature section must agree.
    let mut signed_without = TestUnit::small(&host);
    signed_without.header.header_flags = header_flags::SIGNED;
    assert_eq!(
        verdict(&signed_without.encode(), &host),
        Verdict::LoadError(BadUnit::Signature)
    );
    let mut section_without_flag = TestUnit::native(&host);
    section_without_flag.header.header_flags = 0;
    assert_eq!(
        verdict(&section_without_flag.encode(), &host),
        Verdict::LoadError(BadUnit::Signature)
    );
}

#[test]
fn the_hash_skips_only_itself_and_the_signature() {
    let host = synthetic_host();
    let base = TestUnit::native(&host).encode();
    let sig = TestUnit::parse(&base)
        .unwrap()
        .sections
        .iter()
        .position(|(k, _)| *k == SectionKind::Signature)
        .unwrap();
    let count = TestUnit::parse(&base).unwrap().sections.len();
    let at = SECTION_TABLE_OFFSET + sig * SECTION_ENTRY_SIZE;
    let start = u64::from_le_bytes(base[at + 8..at + 16].try_into().unwrap()) as usize;
    assert_eq!(sig + 1, count, "the signature is the last section");
    for i in 0..base.len() {
        let mut m = base.clone();
        m[i] ^= 0x01;
        let want = if i >= start {
            Outcome::Load // signature bytes: outside the hash, not checked in v0
        } else {
            Outcome::LoadError
        };
        assert_eq!(check(&m, &host), want, "byte {i}");
    }
}

#[test]
fn padding_and_trailing_bytes_are_load_errors() {
    let host = synthetic_host();
    let base = TestUnit::small(&host).encode();
    // The first section ends at an odd offset: the byte after it is padding.
    let table_end = SECTION_TABLE_OFFSET + 3 * SECTION_ENTRY_SIZE;
    let pad = table_end + 37;
    assert_eq!(base[pad], 0);
    let mut m = base.clone();
    m[pad] = 1;
    rehash(&mut m);
    assert_eq!(
        verdict(&m, &host),
        Verdict::LoadError(BadUnit::SectionTable("padding not zero"))
    );
    let mut m = base.clone();
    m.extend_from_slice(&[0; 8]);
    let len = (m.len() as u64).to_le_bytes();
    m[offsets::TOTAL_SIZE..offsets::TOTAL_SIZE + 8].copy_from_slice(&len);
    rehash(&mut m);
    assert_eq!(
        verdict(&m, &host),
        Verdict::LoadError(BadUnit::SectionTable("bytes after the last section"))
    );
}

#[test]
fn steps_run_in_order() {
    let host = synthetic_host();
    let unit = |f: &dyn Fn(&mut UnitHeader)| {
        let mut u = TestUnit::small(&host);
        f(&mut u.header);
        u.encode()
    };
    // Step 2 before step 3: a bad hash wins over another build id.
    let mut m = unit(&|h| h.engine_build_id = [9; 16]);
    m[offsets::CONTENT_HASH] ^= 1;
    assert_eq!(verdict(&m, &host), Verdict::LoadError(BadUnit::Hash));
    // Step 3 before step 4.
    let m = unit(&|h| {
        h.engine_build_id = [9; 16];
        h.versions.barrier += 1;
    });
    assert_eq!(verdict(&m, &host), Verdict::BuildId);
    let m = unit(&|h| {
        h.versions.bytecode_abi += 1;
        h.target_arch = target_arch::X86_64;
    });
    assert_eq!(check(&m, &host), Outcome::Source);
    // Step 4 reports the first difference in check order.
    let m = unit(&|h| {
        h.versions.barrier += 1;
        h.cpu_required = 1 << 63;
    });
    assert!(matches!(
        verdict(&m, &host),
        Verdict::Version {
            contract: Contract::Barrier,
            ..
        }
    ));
    // A fact-schema mismatch also drops the hints.
    let m = unit(&|h| h.versions.fact_schema += 1);
    assert!(verdict(&m, &host).drops_hints());
    assert_eq!(check(&m, &host), Outcome::Bytecode);
}

#[test]
fn unknown_cpu_bits_and_targets_never_match() {
    let mut host = synthetic_host();
    // The host claims an undefined bit: a unit that requires it still runs bytecode.
    host.cpu_features |= 1 << 63;
    let mut u = TestUnit::small(&host);
    u.header.cpu_required = 1 << 63 | cpu::A64_LSE;
    assert_eq!(
        verdict(&u.encode(), &host),
        Verdict::Cpu { missing: 1 << 63 }
    );
    // A required bit of the other architecture is missing too.
    u.header.cpu_required = cpu::X86_AVX2;
    assert_eq!(check(&u.encode(), &host), Outcome::Bytecode);
    u.header.cpu_required = cpu::A64_LSE | cpu::A64_FP16;
    assert_eq!(check(&u.encode(), &host), Outcome::Load);
    // An unknown host target matches nothing, not even an unknown unit target.
    host.target_arch = 0;
    host.target_abi = 0;
    let u = TestUnit::small(&host);
    assert_eq!(
        verdict(&u.encode(), &host),
        Verdict::Target {
            unit: (0, 0),
            host: (0, 0)
        }
    );
}

#[test]
fn host_info_reads_the_contract_constants() {
    let h = HostInfo::current();
    assert_eq!(h.engine_build_id, ENGINE_BUILD_ID);
    assert_eq!(h.versions.bytecode_abi, ep9_bytecode::ABI_VERSION);
    assert_eq!(h.versions.value_encoding, ep9_value::encoding::VERSION);
    assert_eq!(h.versions.barrier, ep9_heap::BARRIER_VERSION);
    assert_eq!(h.versions.fact_schema, ep9_facts::FACT_SCHEMA_VERSION);
    assert_eq!(
        h.versions.object_layout,
        ep9_vm::versions::OBJECT_LAYOUT_VERSION
    );
    assert_eq!(h.versions.frame, ep9_vm::versions::FRAME_VERSION);
    assert_eq!(h.versions.ic, ep9_vm::versions::IC_LAYOUT_VERSION);
    assert_eq!(h.versions.deopt_meta, ep9_vm::versions::DEOPT_META_VERSION);
    assert_eq!(
        h.versions.builtin_abi,
        ep9_vm::versions::BUILTIN_ABI_VERSION
    );
    assert_eq!(1usize << h.heap_page_log2, ep9_heap::layout::PAGE_SIZE);
    // The 4 tier-1 targets all have known codes.
    assert_ne!(h.target_arch, 0);
    assert_ne!(h.target_abi, 0);
    // Detection only sets defined bits of the host's own architecture.
    for (bit, _, arch) in cpu::BITS {
        if arch != h.target_arch {
            assert_eq!(h.cpu_features & bit, 0);
        }
    }
    assert_eq!(h.cpu_features & !cpu::DEFINED, 0);
    println!("host: {h:x?}");
}

#[test]
fn verdicts_name_their_reason() {
    let host = synthetic_host();
    let mut u = TestUnit::small(&host);
    u.header.versions.barrier = 4;
    let text = verdict(&u.encode(), &host).to_string();
    assert_eq!(text, "bytecode: barrier version 4, this engine has 5");
    assert_eq!(
        Verdict::LoadError(BadUnit::Hash).to_string(),
        "load error: content hash mismatch"
    );
}
