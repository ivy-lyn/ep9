//! Property tests (gate "roundtrip"): random well-formed units round-trip through
//! serialize -> deserialize -> verify, and every corrupted input is rejected without a panic.
//!
//! The PRNG is SplitMix64 with fixed seeds, so a failure names a seed that reproduces it on
//! every target. `EP9_ROUNDTRIP_CASES=<n>` runs more cases (default 400).

mod common;

use common::{Rng, random_unit};
use ep9_bytecode::*;

fn cases() -> u64 {
    std::env::var("EP9_ROUNDTRIP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400)
}

/// Rewrites the content hash so that a mutation gets past the hash check.
fn rehash(bytes: &mut [u8]) {
    let h = compute_content_hash(bytes);
    bytes[header_offsets::CONTENT_HASH..header_offsets::CONTENT_HASH + 32].copy_from_slice(&h);
}

#[test]
fn random_units_round_trip() {
    let mut stats = [0usize; 4]; // functions, instructions, wide instructions, bytes
    for seed in 0..cases() {
        let r = random_unit(seed);
        let unit = r
            .build_in_order()
            .unwrap_or_else(|e| panic!("seed {seed}: builder output fails: {e}"));
        let bytes = unit.to_bytes();
        let back = CompiledUnit::from_bytes(&bytes)
            .unwrap_or_else(|e| panic!("seed {seed}: does not load: {e}"));
        assert_eq!(*back, *unit, "seed {seed}: loaded unit differs");
        assert_eq!(back.to_bytes(), bytes, "seed {seed}: second write differs");
        assert_eq!(back.content_hash(), unit.content_hash(), "seed {seed}");
        // The disassembler handles every unit and is a pure function of it.
        assert_eq!(disassemble(&back), disassemble(&unit), "seed {seed}");
        stats[0] += unit.functions.len();
        for f in &unit.functions {
            for i in Instructions::new(&f.bytecode) {
                let i = i.unwrap();
                stats[1] += 1;
                stats[2] += usize::from(i.scale != OperandScale::Single);
            }
        }
        stats[3] += bytes.len();
    }
    // The generator must reach the interesting cases, or the test proves little.
    println!(
        "{} units: {} functions, {} instructions ({} with a prefix), {} bytes",
        cases(),
        stats[0],
        stats[1],
        stats[2],
        stats[3]
    );
    assert!(stats[2] > 0, "no Wide/ExtraWide instruction generated");
}

#[test]
fn random_units_cover_every_emittable_opcode() {
    let mut seen = [false; OPCODE_COUNT];
    for seed in 0..cases() {
        let unit = random_unit(seed).build_in_order().unwrap();
        for f in &unit.functions {
            for i in Instructions::new(&f.bytecode) {
                seen[i.unwrap().opcode as usize] = true;
            }
        }
    }
    let missing: Vec<&str> = OPCODES
        .iter()
        .filter(|i| {
            !i.flags.contains(OpcodeFlags::RESERVED)
                && !i.flags.contains(OpcodeFlags::PREFIX)
                && !seen[i.opcode as usize]
        })
        .map(|i| i.name)
        .collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}

#[test]
fn function_order_does_not_change_bytes() {
    for seed in 0..cases() / 4 {
        let r = random_unit(seed);
        let a = r.build_in_order().unwrap().to_bytes();
        let mut order: Vec<usize> = (0..r.functions.len()).collect();
        let mut rng = Rng::new(seed ^ 0xabcd);
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let b = r.build(&order).unwrap().to_bytes();
        assert_eq!(a, b, "seed {seed}: order {order:?} changed the bytes");
    }
}

#[test]
fn corrupted_bytes_are_rejected_without_panic() {
    for seed in 0..cases() / 2 {
        let unit = random_unit(seed).build_in_order().unwrap();
        let bytes = unit.to_bytes();
        let mut rng = Rng::new(seed.wrapping_mul(31) ^ 0x5eed);
        for _ in 0..8 {
            let mut m = bytes.clone();
            let at = rng.below(m.len() as u64) as usize;
            let flip = 1 + rng.below(255) as u8;
            m[at] ^= flip;
            let r = CompiledUnit::from_bytes(&m);
            let in_build_id =
                (header_offsets::BUILD_ID..header_offsets::BUILD_ID + 16).contains(&at);
            if in_build_id {
                // The build id is outside the hash (H35 checks it separately).
                let u = r.unwrap_or_else(|e| panic!("seed {seed}: build id flip: {e}"));
                assert_eq!(u.to_bytes(), m);
            } else {
                assert!(r.is_err(), "seed {seed}: flip at {at} was accepted");
            }
        }
    }
}

#[test]
fn rehashed_mutations_never_panic_and_stay_canonical() {
    let mut accepted = 0usize;
    let mut total = 0usize;
    for seed in 0..cases() / 2 {
        let unit = random_unit(seed).build_in_order().unwrap();
        let bytes = unit.to_bytes();
        let mut rng = Rng::new(seed ^ 0x0123_4567);
        for _ in 0..16 {
            let mut m = bytes.clone();
            // Mutate 1-3 bytes after the header (the body), then fix the hash.
            for _ in 0..1 + rng.below(3) {
                let at = HEADER_SIZE + rng.below((m.len() - HEADER_SIZE) as u64) as usize;
                m[at] = rng.next() as u8;
            }
            rehash(&mut m);
            total += 1;
            if let Ok(u) = CompiledUnit::from_bytes(&m) {
                accepted += 1;
                // Canonical form: an input that loads is the only encoding of its unit.
                assert_eq!(
                    u.to_bytes(),
                    m,
                    "seed {seed}: accepted a non-canonical input"
                );
            }
        }
    }
    println!("rehashed mutations: {accepted} of {total} still valid");
}

#[test]
fn truncated_and_extended_inputs_are_rejected() {
    for seed in 0..20 {
        let bytes = random_unit(seed).build_in_order().unwrap().to_bytes();
        for n in 0..bytes.len() {
            assert!(
                CompiledUnit::from_bytes(&bytes[..n]).is_err(),
                "seed {seed} len {n}"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(CompiledUnit::from_bytes(&longer).is_err());
    }
}

#[test]
fn header_errors_map_to_h35_outcomes() {
    let bytes = common::tiny_unit().to_bytes();
    // Bad magic: load error.
    let mut m = bytes.clone();
    m[0] = 0;
    let e = CompiledUnit::from_bytes(&m).unwrap_err();
    assert_eq!(e, LoadError::BadMagic);
    assert_eq!(e.outcome(), LoadOutcome::LoadError);
    // Another ABI version: compile the source instead, even before the hash check.
    let mut m = bytes.clone();
    m[header_offsets::ABI_VERSION] = 99;
    let e = CompiledUnit::from_bytes(&m).unwrap_err();
    assert_eq!(e, LoadError::AbiMismatch { found: 99 });
    assert_eq!(e.outcome(), LoadOutcome::Source);
    // Hash mismatch: load error.
    let mut m = bytes.clone();
    m[HEADER_SIZE] ^= 1;
    assert_eq!(
        CompiledUnit::from_bytes(&m).unwrap_err(),
        LoadError::HashMismatch
    );
    // Reserved header bytes must be zero, even with a correct hash.
    let mut m = bytes.clone();
    m[header_offsets::RESERVED0] = 1;
    rehash(&mut m);
    assert_eq!(
        CompiledUnit::from_bytes(&m).unwrap_err(),
        LoadError::BadHeader("reserved bytes")
    );
    // TS flag bits on a JavaScript unit.
    let mut m = bytes.clone();
    let fl = u32::from_le_bytes(m[20..24].try_into().unwrap())
        | unit_flags::TS_USE_DEFINE_FOR_CLASS_FIELDS;
    m[20..24].copy_from_slice(&fl.to_le_bytes());
    rehash(&mut m);
    assert_eq!(
        CompiledUnit::from_bytes(&m).unwrap_err(),
        LoadError::BadHeader("TS flags on a JavaScript unit")
    );
    // peek_header reads the build id without trusting the rest.
    let p = peek_header(&bytes).unwrap();
    assert_eq!(p.abi_version, ABI_VERSION);
    assert_eq!(p.total_size as usize, bytes.len());
}

/// `cargo test --release -p ep9_bytecode --test roundtrip -- --ignored --nocapture load_speed`
#[test]
#[ignore = "timing only"]
fn load_speed() {
    let units: Vec<Vec<u8>> = (0..3000)
        .map(|s| random_unit(s).build_in_order().unwrap().to_bytes())
        .collect();
    let total: usize = units.iter().map(Vec::len).sum();
    let time = |f: &dyn Fn(&[u8])| {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            for u in &units {
                f(u);
            }
            best = best.min(t.elapsed().as_secs_f64());
        }
        best
    };
    let hash = time(&|b| {
        std::hint::black_box(compute_content_hash(b));
    });
    let decode = time(&|b| {
        std::hint::black_box(decode_unit(b).unwrap());
    });
    let full = time(&|b| {
        std::hint::black_box(CompiledUnit::from_bytes(b).unwrap());
    });
    let mb = total as f64 / 1e6;
    println!(
        "{:.1} MB in {} units: hash {:.0} MB/s, hash+decode {:.0} MB/s, hash+decode+verify {:.0} MB/s",
        mb,
        units.len(),
        mb / hash,
        mb / decode,
        mb / full
    );
}
