//! Probe cases S1-S7 [R05] as tests, with the ep9 rule from 03-gc section 2.
//!
//! | Case | Pointer form at the safepoint | ep9 rule | Expected |
//! |---|---|---|---|
//! | S1 | base pointer, maybe only in a callee-saved register | spill trampoline per ABI | found; without the spill: missed |
//! | S2 | interior pointer only | VO interior lookup | found; exact-only lookup: missed |
//! | S3 | strength-reduced loop pointer; one-past-end pointer | interior lookup + end rule | found; no end rule: missed |
//! | S4 | compressed u32 offset | no pointer compression (N6) | not a root |
//! | S5 | two compressed refs in one word | no pointer compression (N6) | not roots |
//! | S6 | NaN-boxed values | H6: cells untagged, numbers at or above 2^49 | cell found; tagged pointer and numbers not roots |
//! | S7 | reference only in a Rust `Vec` | `ScratchRoots` / `Global<T>` | invisible to the scan; found as a precise root |
//!
//! "Rust frame" tests use plain optimized Rust, so the compiler picks the pointer form. The
//! "register" tests use a naked helper that keeps one word only in a callee-saved register
//! (x19; rbx on SysV; rsi on Win64), so the outcome is deterministic.

use std::hint::black_box;

use ep9_platform::SpillAbi;
use ep9_platform::va::CAGE_LIMIT;

use crate::asm::{hold_in_register, scrub_stack};
use crate::heap::*;
use crate::mutator::{Config, Mutator};
use crate::value;

/// A probe mutator. One padding object first, so probed objects never sit at the cage base.
fn probe() -> Box<Mutator> {
    let mut m = Mutator::new(Config::probe());
    m.new_obj(32, 0);
    m
}

fn describe(f: u8) -> String {
    let names = [
        (F_SPILL, "spill-area"),
        (F_STACK, "stack-slot"),
        (F_EXACT, "exact"),
        (F_INTERIOR, "interior"),
        (F_END, "one-past-end"),
        (F_PRECISE, "precise-root"),
        (F_TRACED, "traced"),
        (F_RESCUED, "rescued"),
    ];
    let v: Vec<&str> = names
        .iter()
        .filter(|(b, _)| f & b != 0)
        .map(|(_, n)| *n)
        .collect();
    if v.is_empty() {
        "NOT FOUND".into()
    } else {
        v.join("+")
    }
}

/// Wipes stale copies of addresses from dead frames below the caller (see `scrub_stack`).
#[inline(always)]
fn scrub() {
    // SAFETY: test threads have 2 MiB stacks; the probes use far less than that.
    unsafe { scrub_stack() }
}

/// Allocates an object of `words` words, watches it, and returns only `base + 8 * offset`.
#[inline(never)]
fn new_value(m: &mut Mutator, words: usize, offset_words: usize) -> usize {
    let o = m.new_obj(words, 0);
    m.watch(o as usize);
    let v = o as usize + offset_words * GRANULE;
    scrub();
    v
}

/// Runs the register helper for `abi` and returns the watched object's flags.
fn register_case(scan: ScanOptions, abi: SpillAbi, offset: usize, spill: bool) -> u8 {
    let mut m = probe();
    m.cfg.scan = scan;
    let v = new_value(&mut m, 8, offset);
    let back = hold_in_register(&mut m, abi, v, spill);
    // SAFETY: `back - offset` is the object `new_value` allocated (no sweep in probes).
    unsafe { verify_any((back - offset * GRANULE) as *const u64) };
    assert_eq!(m.watch_log.len(), 1);
    m.watch_log[0]
}

const EXACT_ONLY: ScanOptions = ScanOptions {
    interior: false,
    one_past_end: false,
};
const NO_END_RULE: ScanOptions = ScanOptions {
    interior: true,
    one_past_end: false,
};

// ---------- S1 ----------

#[inline(never)]
fn s1_frame(m: &mut Mutator) -> (u8, u64) {
    let o = m.new_obj(8, 0);
    // SAFETY: `o` was just allocated and is used after the safepoint (so it is live).
    let id = unsafe { obj_id(o) };
    // SAFETY: as above.
    let before = unsafe { verify(o, id) };
    m.safepoint();
    let f = m.heap.flags_of(o as usize);
    // SAFETY: as above.
    (f, before ^ unsafe { verify(o, id) })
}

#[test]
fn s1_base_pointer_in_rust_frame_is_found() {
    let mut m = probe();
    let (f, _) = s1_frame(&mut m);
    println!("S1 rust frame ({}): {}", SpillAbi::HOST.name(), describe(f));
    assert!(f & F_EXACT != 0, "S1 base pointer missed: {}", describe(f));
}

#[test]
fn s1_register_only_ref_needs_the_spill_trampoline() {
    for &abi in SpillAbi::available() {
        let with = register_case(ScanOptions::FULL, abi, 0, true);
        let without = register_case(ScanOptions::FULL, abi, 0, false);
        println!(
            "S1 register-only ({}): trampoline -> {}; no spill -> {}",
            abi.name(),
            describe(with),
            describe(without)
        );
        assert_eq!(
            with & (F_SPILL | F_EXACT),
            F_SPILL | F_EXACT,
            "{}",
            abi.name()
        );
        assert_eq!(without, 0, "{}: control must miss it", abi.name());
    }
}

// ---------- S2 ----------

#[inline(never)]
fn make_interior(m: &mut Mutator, k: usize) -> *const u64 {
    let o = m.new_obj(8, 0);
    m.watch(o as usize);
    // SAFETY: `k < 8`, inside the object.
    unsafe { o.add(k) }
}

#[inline(never)]
fn s2_frame(m: &mut Mutator, k: usize) -> (u64, *const u64) {
    let p = make_interior(m, k);
    m.safepoint();
    // SAFETY: `p` points into the watched object.
    (unsafe { p.read() }, p)
}

#[test]
fn s2_interior_pointer_in_rust_frame_is_found() {
    let mut m = probe();
    let k = black_box(5);
    let (v, p) = s2_frame(&mut m, k);
    // SAFETY: `p - k` is the object base.
    let base = unsafe { p.sub(k) };
    // SAFETY: as above.
    assert_eq!(v, payload_word(unsafe { obj_id(base) }, k));
    let f = m.watch_log[0];
    println!("S2 rust frame: {}", describe(f));
    assert!(
        f & F_INTERIOR != 0,
        "S2 interior pointer missed: {}",
        describe(f)
    );
}

#[test]
fn s2_register_only_interior_ref_needs_interior_lookup() {
    let abi = SpillAbi::HOST;
    let with = register_case(ScanOptions::FULL, abi, 5, true);
    let exact_only = register_case(EXACT_ONLY, abi, 5, true);
    println!(
        "S2 register-only: interior lookup -> {}; exact only -> {}",
        describe(with),
        describe(exact_only)
    );
    assert_eq!(with & (F_SPILL | F_INTERIOR), F_SPILL | F_INTERIOR);
    assert_eq!(
        exact_only, 0,
        "exact-only lookup must miss an interior pointer"
    );
}

// ---------- S3 ----------

#[inline(never)]
fn s3_frame(m: &mut Mutator) -> u64 {
    let o = m.new_obj(32, 0);
    m.watch(o as usize);
    // SAFETY: a 32-word object with 0 children: the payload is words 2..32.
    let base = unsafe { o.add(2) } as *const u64;
    let mut sum = 0u64;
    for i in 0..30 {
        m.safepoint();
        // SAFETY: `i + 2 < 32`, inside the watched object.
        sum = sum.wrapping_add(unsafe { base.add(i).read() });
    }
    sum
}

#[test]
fn s3_strength_reduced_loop_pointer_is_found() {
    let mut m = probe();
    let sum = s3_frame(&mut m);
    let log = m.watch_log.clone();
    assert_eq!(log.len(), 30);
    let exact = log.iter().filter(|f| **f & F_EXACT != 0).count();
    let interior = log.iter().filter(|f| **f & F_INTERIOR != 0).count();
    let missed = log.iter().filter(|f| **f == 0).count();
    println!(
        "S3 loop, 30 safepoints: exact {exact}, interior {interior}, missed {missed}; first {}, last {}",
        describe(log[0]),
        describe(log[29])
    );
    assert_eq!(missed, 0);
    black_box(sum);
}

#[test]
fn s3_register_only_one_past_end_ref_needs_the_end_rule() {
    let abi = SpillAbi::HOST;
    // An 8-word object: offset 8 words is one past its end.
    let with = register_case(ScanOptions::FULL, abi, 8, true);
    let without = register_case(NO_END_RULE, abi, 8, true);
    println!(
        "S3 one-past-end: end rule -> {}; no end rule -> {}",
        describe(with),
        describe(without)
    );
    assert_eq!(with & (F_SPILL | F_END), F_SPILL | F_END);
    assert_eq!(
        without, 0,
        "without the end rule a one-past-end pointer is missed"
    );
}

// ---------- S4, S5 ----------

#[inline(never)]
fn make_compressed(m: &mut Mutator) -> u32 {
    let o = m.new_obj(8, 0);
    let c = (o as usize - m.heap.used().start) as u32;
    scrub();
    c
}

#[inline(never)]
fn s4_frame(m: &mut Mutator) -> (u8, u64) {
    let c = make_compressed(m);
    scrub();
    m.safepoint();
    let o = m.heap.used().start + c as usize;
    // SAFETY: probes never sweep, so the object is still allocated.
    (m.heap.flags_of(o), unsafe { verify_any(o as *const u64) })
}

#[test]
fn s4_compressed_u32_ref_is_not_a_root() {
    let mut m = probe();
    let (f, _) = s4_frame(&mut m);
    println!(
        "S4 compressed u32: {} (ep9 rule N6: no compressed references)",
        describe(f)
    );
    assert_eq!(f, 0, "the scan reads full 64-bit words only");
}

#[inline(never)]
fn make_pair(m: &mut Mutator) -> u64 {
    let a = m.new_obj(8, 0) as usize - m.heap.used().start;
    let b = m.new_obj(8, 0) as usize - m.heap.used().start;
    let pair = a as u64 | (b as u64) << 32;
    scrub();
    pair
}

#[inline(never)]
fn s5_frame(m: &mut Mutator) -> (u8, u8) {
    let pair = make_pair(m);
    scrub();
    m.safepoint();
    let start = m.heap.used().start;
    let (a, b) = (
        start + (pair as u32) as usize,
        start + (pair >> 32) as usize,
    );
    (m.heap.flags_of(a), m.heap.flags_of(b))
}

#[test]
fn s5_two_compressed_refs_in_one_word_are_not_roots() {
    let mut m = probe();
    let (fa, fb) = s5_frame(&mut m);
    println!("S5 packed pair: {} / {}", describe(fa), describe(fb));
    assert_eq!((fa, fb), (0, 0));
}

// ---------- S6 ----------

#[inline(never)]
fn make_boxed(m: &mut Mutator) -> [u64; 3] {
    let cell = m.new_obj(8, 0) as usize;
    let tagged = m.new_obj(8, 0) as usize;
    let as_number = m.new_obj(8, 0) as usize;
    let v = [
        value::cell(cell),
        tagged as u64 | value::PROBE_HIGH_TAG,
        // The address bits as a double: decodes back to the address, but the encoded word is
        // at or above 2^49.
        value::double(f64::from_bits(as_number as u64)),
    ];
    scrub();
    v
}

#[inline(never)]
fn s6_frame(m: &mut Mutator) -> [u8; 3] {
    let v = make_boxed(m);
    scrub();
    m.safepoint();
    let v = black_box(v);
    let cell = v[0] as usize;
    let tagged = (v[1] & !value::PROBE_HIGH_TAG) as usize;
    let as_number = value::as_double(v[2]).unwrap().to_bits() as usize;
    [
        m.heap.flags_of(cell),
        m.heap.flags_of(tagged),
        m.heap.flags_of(as_number),
    ]
}

#[test]
fn s6_nan_boxed_cell_is_found_tagged_and_numbers_are_not() {
    let mut m = probe();
    let [cell, tagged, number] = s6_frame(&mut m);
    println!(
        "S6: cell value -> {}; high-tagged pointer -> {}; address bits as a double -> {}",
        describe(cell),
        describe(tagged),
        describe(number)
    );
    assert!(cell & F_EXACT != 0, "an H6 cell value is the raw pointer");
    assert_eq!(
        tagged, 0,
        "H6 never tags cell pointers; a tagged pointer is not a root"
    );
    assert_eq!(number, 0, "a number never looks like a cage address");
}

#[test]
fn s6_encoded_numbers_never_fall_below_2_49() {
    let mut x = 0x1234_5678_9ABC_DEF0u64;
    let specials = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        f64::MIN_POSITIVE,
        5e-324,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::MAX,
        f64::MIN,
    ];
    let mut min = u64::MAX;
    for i in 0..1_000_000u32 {
        x = splitmix64(x);
        let d = if (i as usize) < specials.len() {
            specials[i as usize]
        } else {
            f64::from_bits(x)
        };
        min = min.min(value::double(d)).min(value::int32(x as i32));
    }
    println!(
        "S6: smallest encoded number of 2 x 10^6 = {min:#x} (2^49 = {:#x})",
        1u64 << 49
    );
    assert!(min >= 1 << 49 && min as usize >= CAGE_LIMIT);
}

// ---------- S7 ----------

#[inline(never)]
fn make_vec(m: &mut Mutator, rooted: bool) -> Vec<*mut u64> {
    let o = m.new_obj(8, 0);
    if rooted {
        m.push_scratch(o as usize);
    }
    let v = vec![o];
    scrub();
    v
}

#[inline(never)]
fn s7_frame(m: &mut Mutator, rooted: bool) -> u8 {
    let v = make_vec(m, rooted);
    scrub();
    m.safepoint();
    let f = m.heap.flags_of(v[0] as usize);
    // SAFETY: probes never sweep, so the object is still allocated.
    black_box(unsafe { verify_any(v[0]) });
    if rooted {
        m.pop_scratch();
    }
    f
}

#[test]
fn s7_ref_only_in_rust_vec_needs_scratch_roots() {
    let mut m = probe();
    let unrooted = s7_frame(&mut m, false);
    let rooted = s7_frame(&mut m, true);
    println!(
        "S7 Vec only: {}; Vec + ScratchRoots: {}",
        describe(unrooted),
        describe(rooted)
    );
    assert_eq!(unrooted, 0, "the scan cannot see malloc memory");
    assert!(rooted & F_PRECISE != 0);
}

// ---------- a short stress, so `cargo test` covers the random mutator too ----------

#[test]
fn short_stress_host_abi_misses_nothing() {
    use crate::mutator::{Spill, stress};
    let cfg = Config::stress(Spill::Trampoline(SpillAbi::HOST), ScanOptions::FULL, 7);
    let (s, h) = stress(cfg, 20_000);
    println!(
        "short stress: {} safepoints, {} live checks, {} missed, max depth {}, {} allocated, {} freed",
        s.safepoints, s.checked, s.missed, s.max_depth, h.allocated, h.freed
    );
    assert_eq!(s.missed, 0, "{:?}", s.first_misses);
    assert!(h.freed > 0 && s.checked > 0);
}
