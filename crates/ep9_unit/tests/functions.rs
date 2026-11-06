//! The function table (doc 13 C-17; `ep9_unit::functions`) and the SIZE-REPORT model
//! (`ep9_unit::size`).

use ep9_unit::cpu;
use ep9_unit::functions::*;
use ep9_unit::size::{FunctionStats, Policy, size};

fn variant(entry: u32, lowering: u8) -> Variant {
    Variant {
        kind: VariantKind::AotOpt,
        lowering,
        tag: 0,
        entry,
        text_len: 16,
        meta: NO_META,
        meta_len: 0,
        fuse_refs: 0,
        fuse_count: 0,
        cpu_required: 0,
    }
}

/// Three functions: one with no native variant (it runs its bytecode), one with a hybrid and
/// a jitless variant of one function (doc 13 1.4), one baseline variant with deopt metadata,
/// fuse refs and a CPU bit.
fn sample() -> Vec<Entry> {
    vec![
        Entry {
            function: 0,
            digest: [1; 16],
            variants: vec![],
        },
        Entry {
            function: 3,
            digest: [2; 16],
            variants: vec![variant(0, lowering::HYBRID), variant(16, lowering::JITLESS)],
        },
        Entry {
            function: 7,
            digest: [3; 16],
            variants: vec![Variant {
                kind: VariantKind::AotBaseline,
                tag: 2,
                meta: 400,
                meta_len: 40,
                fuse_refs: 440,
                fuse_count: 2,
                cpu_required: cpu::A64_LSE,
                ..variant(32, lowering::JITLESS)
            }],
        },
    ]
}

#[test]
fn the_table_round_trips_and_has_one_encoding() {
    let t = sample();
    let mut rodata = encode(&t);
    assert_eq!(rodata.len(), table_size(3, 3));
    assert_eq!(rodata.len(), 16 + 3 * 32 + 3 * 48);
    assert_eq!(&rodata[..4], b"EPFT");
    rodata.resize(448, 0); // the metadata and the fuse list after the table
    assert_eq!(decode(&rodata, 48), Ok((t.clone(), table_size(3, 3))));
    assert_eq!(encode(&decode(&rodata, 48).unwrap().0), encode(&t));
}

#[test]
fn the_reader_refuses_a_bad_table() {
    let t = sample();
    let mut good = encode(&t);
    good.resize(448, 0);
    let with = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        decode(&b, 48).map(|_| ())
    };
    assert_eq!(with(0, b'X'), Err(BadTable::Magic));
    assert_eq!(with(4, 2), Err(BadTable::Magic));
    assert_eq!(decode(&good[..100], 48).map(|_| ()), Err(BadTable::Size));
    // Entry 1's function 3 -> 0: not ascending.
    assert_eq!(with(16 + 32, 0), Err(BadTable::Order));
    // Entry 1's first variant 0 -> 1.
    assert_eq!(with(16 + 32 + 4, 1), Err(BadTable::VariantRange));
    // Entry flags and reserved bytes.
    assert_eq!(with(16 + 32 + 10, 1), Err(BadTable::Reserved));
    let v0 = 16 + 3 * 32;
    assert_eq!(with(v0, 5), Err(BadTable::Kind));
    for at in [3, 26, 28, 40, 44, 47] {
        assert_eq!(
            with(v0 + at, 1),
            Err(BadTable::Reserved),
            "variant byte {at}"
        );
    }
    // Text out of the section; metadata and fuse lists out of rodata.
    assert_eq!(decode(&good, 47).map(|_| ()), Err(BadTable::Text));
    let v2 = v0 + 2 * 48;
    assert_eq!(with(v2 + 13, 0xff), Err(BadTable::Rodata)); // meta offset past rodata
    assert_eq!(with(v2 + 24, 3), Err(BadTable::Rodata)); // 3 fuse ids past rodata
    // NO_META with a length.
    assert_eq!(with(v0 + 16, 1), Err(BadTable::Rodata));
    // No fuse id, but a fuse-list offset: one table has one encoding (JIT reviewer F6).
    assert_eq!(with(v0 + 20, 8), Err(BadTable::Reserved));
    // An unknown lowering is not a format error: the loader skips that variant.
    assert!(with(v0 + 1, 9).is_ok());
}

#[test]
fn a_variant_runs_only_where_its_lowering_and_cpu_bits_are_accepted() {
    let t = sample();
    let hybrid_preset = [lowering::HYBRID, lowering::JITLESS];
    let jitless_preset = [lowering::JITLESS];
    let pick = |e: &Entry, accept: &[u8], cpu: u64| -> Vec<u32> {
        accepted(e, accept, cpu).map(|v| v.entry).collect()
    };
    assert_eq!(pick(&t[1], &hybrid_preset, 0), [0, 16]);
    assert_eq!(pick(&t[1], &jitless_preset, 0), [16]);
    // No variant: the function runs its bytecode.
    assert!(pick(&t[0], &hybrid_preset, 0).is_empty());
    // A CPU bit the host lacks skips the variant.
    assert!(pick(&t[2], &jitless_preset, 0).is_empty());
    assert_eq!(pick(&t[2], &jitless_preset, cpu::A64_LSE), [32]);
    // `jit` is never stored in a unit: skipped even where the preset lists it.
    let mut j = t[1].clone();
    j.variants[0].lowering = lowering::JIT;
    assert_eq!(
        pick(&j, &[lowering::JIT, lowering::HYBRID, lowering::JITLESS], 0),
        [16]
    );
}

#[test]
fn the_size_model_counts_each_policy() {
    let f = |index, instructions, has_loop, in_profile| FunctionStats {
        index,
        instructions,
        bytecode_bytes: instructions * 3,
        has_loop,
        in_profile,
    };
    let fns = [
        f(0, 10, false, true),
        f(1, 82, false, false),
        f(2, 20, true, true),
        f(3, 5, false, false),
    ];
    let all = size(&fns, Policy::All);
    assert_eq!((all.functions, all.instructions), (4, 117));
    assert_eq!(all.table_bytes, 16 + 4 * (32 + 48));
    let st = size(&fns, Policy::Static);
    assert_eq!((st.functions, st.instructions), (2, 102));
    let pgo = size(&fns, Policy::Pgo);
    assert_eq!((pgo.functions, pgo.instructions), (2, 30));
    // The Spike C sample: 82 instructions -> 2816 B of text and 1448 B of metadata.
    let one = size(&fns[1..2], Policy::All);
    assert_eq!((one.text_bytes, one.meta_bytes), (2816, 1448));
    assert_eq!(size(&[], Policy::All).native_bytes(), 0);
}
