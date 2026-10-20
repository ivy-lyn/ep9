//! Determinism (gate "determinism", H2): same input -> same bytes and hash.
//!
//! Cross-target equality comes from `golden.rs::content_hashes`, which every CI target runs
//! against one committed file.

mod common;

use ep9_bytecode::*;

#[test]
fn samples_build_to_identical_bytes_twice() {
    let a = common::sample_script_unit();
    let b = common::sample_script_unit();
    assert_eq!(a.to_bytes(), b.to_bytes());
    assert_eq!(a.content_hash(), b.content_hash());
    let a = common::sample_module_unit();
    let b = common::sample_module_unit();
    assert_eq!(a.to_bytes(), b.to_bytes());
    assert_eq!(disassemble(&a), disassemble(&b));
}

#[test]
fn random_units_build_to_identical_bytes_twice() {
    for seed in 0..200 {
        let a = common::random_unit(seed).build_in_order().unwrap();
        let b = common::random_unit(seed).build_in_order().unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes(), "seed {seed}");
    }
}

/// Builds run on other threads give the same bytes (no thread-local or hash-seed state
/// reaches the output: std's HashMap uses a random seed per map).
#[test]
fn builds_on_other_threads_agree() {
    let here: Vec<Vec<u8>> = (0..50)
        .map(|s| common::random_unit(s).build_in_order().unwrap().to_bytes())
        .collect();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                (0..50)
                    .map(|s| common::random_unit(s).build_in_order().unwrap().to_bytes())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), here);
    }
}

#[test]
fn content_hash_excludes_build_id_and_covers_ts_flags() {
    let unit = common::sample_module_unit().into_inner();
    let mut other = unit.clone();
    other.header.engine_build_id = [7; 16];
    assert_ne!(unit.to_bytes(), other.to_bytes());
    assert_eq!(unit.content_hash(), other.content_hash());
    // TS flags are part of module identity (H3): another flag value, another hash.
    let mut ts = unit.clone();
    ts.header.typescript = Some(TsFlags::DEFAULT);
    assert_ne!(unit.content_hash(), ts.content_hash());
    // So are goal and strictness.
    let script = common::sample_script_unit().into_inner();
    let mut strict = script.clone();
    strict.header.strict = true;
    assert_ne!(script.content_hash(), strict.content_hash());
}

#[test]
fn source_hash_is_sha256_of_the_decoded_text() {
    let u = common::sample_script_unit();
    assert_eq!(
        u.header.source_hash,
        sha256::sha256(common::SAMPLE_SCRIPT.as_bytes())
    );
    assert_eq!(u.header.source_len as usize, common::SAMPLE_SCRIPT.len());
}
