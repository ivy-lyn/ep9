//! String, atom and property-key tests (H19 v0): the cell layout, interning, the weak atom
//! table, the hash and its seed, array-index keys, the single-character table and the length
//! limit.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::process::Command;

use ep9_bytecode::{FeedbackSlotKind, VerifiedUnit};
use ep9_heap::HeapConfig;
use ep9_value::Value;

use super::{Asm, config, isolate, ok, on_big_stack, run_all, unit};
use crate::atoms::{self, Atom};
use crate::key::{self, Probe};
use crate::kinds::{self, FLAG_ATOM};
use crate::object::cell;
use crate::runtime::{Runtime, val};
use crate::string::{self, JsStr, testing};
use crate::{Dispatch, IntrinsicId, IsolateConfig, Local};

/// Allocated size of the cell of `b`.
fn cell_size(rt: &Runtime, b: u64) -> usize {
    // SAFETY: `b` is a live cell: the caller holds it in a Rust local (conservative scan).
    unsafe { rt.heap.cell_size(cell(b)) }
}

/// A script that returns the string constant `text`.
fn returns(text: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    s.lda_str(text);
    s.b.return_();
    unit("", vec![s.finish()])
}

/// A script that returns `a + b` (a new, non-atom string).
fn concat_of(a: &str, b: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let r = s.b.new_register();
    s.lda_str(a);
    s.b.star(r);
    s.lda_str(b);
    s.b.add(r, s.bin).return_();
    unit("", vec![s.finish()])
}

/// xorshift64*: reproducible test data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

#[test]
fn empty_string_is_16_bytes_and_abc_is_24() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    for (text, size) in [("", 16), ("abc", 24), ("abcdefgh", 24), ("abcdefghi", 32)] {
        let s = string::new_string(rt, text);
        assert_eq!(kinds::cell_kind(s), kinds::SEQ_LATIN1.get());
        assert_eq!(cell_size(rt, s), size, "{text:?}");
        assert_eq!(string::str_len(s), text.len());
    }
    // UTF-16: 16 + 2 len, rounded to 8.
    let s = string::new_string_utf16(rt, &[0x20ac; 3]);
    assert_eq!(kinds::cell_kind(s), kinds::SEQ_UTF16.get());
    assert_eq!(cell_size(rt, s), 24);
    assert_eq!(string::str_len(s), 3);
    // Atoms have the same layout; "" is a static atom.
    let e = atoms::intern_str(rt, "");
    assert_eq!(e, rt.atom(Atom::Empty));
    assert_eq!(cell_size(rt, e), 16);
}

#[test]
fn one_name_from_two_units_and_a_concat_key_are_one_cell() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let a = iso.run_raw(returns("sharedName"), "u1.js").unwrap();
            let a = a.to_bits();
            let b = iso.run_raw(returns("sharedName"), "u2.js").unwrap();
            assert_eq!(a, b.to_bits(), "the same name from two units");
            assert!(string::is_atom(a));
            // "shared" + "Name" is a new cell; ToPropertyKey gives the unit's atom.
            let c = iso.run_raw(concat_of("shared", "Name"), "u3.js").unwrap();
            let c = c.to_bits();
            assert_ne!(c, a);
            assert!(!string::is_atom(c));
            assert_eq!(key::to_property_key(&mut iso.cx, c).unwrap(), a);
            // A text with no atom yet: the string itself becomes the atom (no copy).
            let d = iso.run_raw(concat_of("brand", "New"), "u4.js").unwrap();
            let d = d.to_bits();
            assert_eq!(atoms::lookup(&iso.rt, "brandNew"), None);
            assert_eq!(key::to_property_key(&mut iso.cx, d).unwrap(), d);
            assert!(string::is_atom(d));
            assert_eq!(atoms::lookup(&iso.rt, "brandNew"), Some(d));
            // A unit name equal to a static atom is that atom.
            let l = iso.run_raw(returns("length"), "u5.js").unwrap();
            assert_eq!(l.to_bits(), iso.rt.atom(Atom::Length));
            atoms::verify(&iso.rt).unwrap();
        });
    }
}

#[test]
fn index_strings_are_atoms_with_the_index_cached() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let one = string::new_string(&iso.rt, "1");
    assert_eq!(testing::raw_hash_field(one), 0, "not computed yet");
    let k = key::to_property_key(&mut iso.cx, one).unwrap();
    assert!(string::is_atom(k));
    let f = testing::raw_hash_field(k);
    assert_eq!(f & 3, 3, "computed and array-index bits");
    assert_eq!(f >> 2, 1, "the cached index");
    assert_eq!(key::array_index_of(k), Some(1));
    // The number 1 gives the same key.
    let n = key::to_property_key(&mut iso.cx, Value::number(1.0).to_bits()).unwrap();
    assert_eq!(n, k);
    // Canonical index strings only; long indices are indices but not cached.
    let rt = &*iso.rt;
    let cases: [(&str, Option<u32>, bool); 8] = [
        ("0", Some(0), true),
        ("01", None, false),
        ("16777215", Some(16_777_215), true),
        ("16777216", Some(16_777_216), false),
        ("4294967294", Some(4_294_967_294), false),
        ("4294967295", None, false),
        ("-1", None, false),
        ("1e3", None, false),
    ];
    for (text, index, cached) in cases {
        let a = atoms::intern_str(rt, text);
        assert_eq!(key::array_index_of(a), index, "{text}");
        assert_eq!(testing::raw_hash_field(a) & 2 != 0, cached, "{text}");
        let fresh = string::new_string(rt, text);
        assert_eq!(
            key::array_index_of(fresh),
            index,
            "{text} (no hash field yet)"
        );
        assert_eq!(string::hash(fresh), string::hash(a), "{text}");
    }
}

/// Interns `n` dynamic atoms that nothing references and returns their addresses (plain
/// numbers in malloc memory, invisible to the GC).
#[inline(never)]
fn make_dynamic_atoms(rt: &Runtime, n: usize) -> Vec<usize> {
    (0..n)
        .map(|i| {
            let s = string::new_string(rt, &format!("dynamic atom {i}"));
            let a = atoms::intern(rt, s);
            assert_eq!(a, s);
            cell(a).addr()
        })
        .collect()
}

/// Overwrites the stack below the caller, so stale copies of dead values are gone.
#[inline(never)]
fn clobber_stack() {
    let mut a = [0u64; 4096];
    std::hint::black_box(&mut a);
}

#[test]
fn unreferenced_dynamic_atoms_go_and_referenced_ones_stay() {
    let iso = isolate(IsolateConfig {
        heap: HeapConfig {
            record_conservative_roots: true,
            ..HeapConfig::default()
        },
        ..IsolateConfig::default()
    });
    let rt = &*iso.rt;
    let kept = atoms::intern_str(rt, "a referenced atom");
    let scope = rt.heap.scratch_roots();
    scope.push(crate::runtime::val(kept));
    let before = rt.atoms.borrow().len();
    let addrs = make_dynamic_atoms(rt, 200);
    assert_eq!(rt.atoms.borrow().len(), before + 200);
    clobber_stack();
    rt.heap.collect();
    let pinned: Vec<usize> = rt
        .heap
        .last_conservative_roots()
        .iter()
        .map(|c| c.addr())
        .collect();
    let mut gone = 0;
    for (i, addr) in addrs.iter().enumerate() {
        let found = atoms::lookup(rt, &format!("dynamic atom {i}"));
        // An atom stays only if the conservative scan found a stale copy of it.
        assert_eq!(found.is_some(), pinned.contains(addr), "dynamic atom {i}");
        gone += usize::from(found.is_none());
    }
    assert!(gone >= 190, "only {gone} of 200 dynamic atoms were dropped");
    assert_eq!(rt.atoms.borrow().len(), before + 200 - gone);
    assert_eq!(atoms::lookup(rt, "a referenced atom"), Some(kept));
    atoms::verify(rt).unwrap();
    drop(scope);
}

#[test]
fn interning_under_gc_stress_keeps_the_table_whole() {
    // A collection before every allocation: interning allocates with no table borrow held,
    // and the weak source checks the table after each collection (debug builds).
    let iso = isolate(config(Dispatch::DEFAULT, true));
    let rt = &*iso.rt;
    let mut kept = Vec::new();
    for i in 0..300 {
        let a = atoms::intern_str(rt, &format!("stress {i}"));
        if i % 3 == 0 {
            // Root every third one in the global object.
            let k = atoms::intern_str(rt, &format!("key {i}"));
            crate::object::own_define(rt, rt.global(), k, a, crate::object::attr::DEFAULT);
            kept.push(i);
        }
    }
    rt.heap.collect();
    for i in kept {
        let k = atoms::lookup(rt, &format!("key {i}")).expect("a rooted key");
        let (v, _) = crate::object::own_get(rt.global(), k).unwrap();
        assert_eq!(string::to_rust_string(v), format!("stress {i}"));
        assert_eq!(atoms::lookup(rt, &format!("stress {i}")), Some(v));
    }
    let n = atoms::verify(rt).unwrap();
    assert!(n >= Atom::ALL.len() + 200, "{n} atoms");
}

#[test]
fn latin1_and_utf16_forms_hash_alike() {
    let keys = testing::keys();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..10_000 {
        let len = (rng.next() % 40) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let units: Vec<u16> = bytes.iter().map(|&b| u16::from(b)).collect();
        assert_eq!(
            testing::sip13_latin1(keys, &bytes),
            testing::sip13(keys, &units),
            "{bytes:?}"
        );
    }
    // The same text in a SEQ_LATIN1 cell and a SEQ_UTF16 cell: one hash, one atom.
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    for text in [
        "",
        "a",
        "caf\u{e9}",
        "12",
        "16777216",
        "a longer text of 4n+3",
    ] {
        let units: Vec<u16> = text.encode_utf16().collect();
        let a = string::new_string(rt, text);
        let b = testing::new_utf16_cell(rt, &units);
        assert_eq!(kinds::cell_kind(a), kinds::SEQ_LATIN1.get());
        assert_eq!(kinds::cell_kind(b), kinds::SEQ_UTF16.get());
        assert_eq!(string::hash(a), string::hash(b), "{text:?}");
        assert_eq!(atoms::intern(rt, b), atoms::intern(rt, a), "{text:?}");
    }
}

#[test]
fn sip13_matches_std_siphash13() {
    // std's DefaultHasher::new() is SipHash-1-3 with zero keys (the algorithm std has used
    // since 1.13). Code units hash as their UTF-16LE bytes.
    use std::hash::Hasher;
    let mut rng = Rng(42);
    for len in 0..64 {
        let units: Vec<u16> = (0..len).map(|_| rng.next() as u16).collect();
        let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        let mut h = std::hash::DefaultHasher::new();
        h.write(&bytes);
        assert_eq!(testing::sip13((0, 0), &units), h.finish(), "length {len}");
    }
}

/// Child of `ep9_hash_seed_gives_the_same_hashes_in_two_runs`: prints hashes of a few texts
/// when `EP9_STR_CHILD=hash`.
#[test]
fn hash_seed_child() {
    if std::env::var("EP9_STR_CHILD").as_deref() != Ok("hash") {
        return;
    }
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    for text in ["", "length", "caf\u{e9}", "\u{20ac}uro", "7", "99999999"] {
        let s = string::new_string(rt, text);
        println!("HASH {text:?} {}", string::hash(s));
    }
}

fn child_hashes(seed: &str) -> Vec<String> {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "tests::strings::hash_seed_child",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("EP9_STR_CHILD", "hash")
        .env("EP9_HASH_SEED", seed)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let lines: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        // libtest prints the test name without a newline before the test's own output.
        .filter_map(|l| l.find("HASH ").map(|i| l[i..].to_owned()))
        .collect();
    assert_eq!(lines.len(), 6, "{lines:?}");
    lines
}

#[cfg(not(miri))]
#[test]
fn ep9_hash_seed_gives_the_same_hashes_in_two_runs() {
    let a = child_hashes("1");
    let b = child_hashes("1");
    assert_eq!(a, b, "EP9_HASH_SEED=1 twice");
    assert_ne!(a, child_hashes("2"), "another seed, other hashes");
    assert_eq!(child_hashes("0x1"), a, "hexadecimal seed");
    assert_eq!(
        string::parse_seed("not a number"),
        string::parse_seed(" not a number ")
    );
}

#[test]
fn the_atoms_list_is_append_only() {
    // Ids are stable (H13 index list): only appends may change this table.
    let want: &[(u16, &str)] = &[
        (0, ""),
        (1, "length"),
        (2, "prototype"),
        (3, "constructor"),
        (4, "name"),
        (5, "message"),
        (6, "stack"),
        (7, "toString"),
        (8, "valueOf"),
        (9, "raw"),
        (10, "value"),
        (11, "done"),
        (12, "undefined"),
        (13, "null"),
        (14, "true"),
        (15, "false"),
        (16, "object"),
        (17, "boolean"),
        (18, "number"),
        (19, "string"),
        (20, "function"),
        (21, "symbol"),
        (22, "bigint"),
        (23, "NaN"),
        (24, "Infinity"),
        (25, ","),
        (26, "globalThis"),
        (27, "get"),
        (28, "set"),
        (29, "writable"),
        (30, "enumerable"),
        (31, "configurable"),
        (32, "next"),
        (33, "return"),
        (34, "throw"),
    ];
    let got: Vec<(u16, &str)> = Atom::ALL.iter().map(|a| (a.id(), a.text())).collect();
    assert_eq!(&got[..want.len()], want);
    for (i, a) in Atom::ALL.iter().enumerate() {
        assert_eq!(usize::from(a.id()), i);
    }
    // Every static atom is the table's atom of its text and a root.
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    rt.heap.collect();
    for &a in Atom::ALL {
        assert_eq!(atoms::intern_str(rt, a.text()), rt.atom(a), "{a:?}");
        assert!(string::is_atom(rt.atom(a)));
    }
}

#[test]
fn atoms_verify_runs_and_finds_a_broken_table() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    let n = atoms::verify(rt).unwrap();
    assert!(n >= Atom::ALL.len(), "{n} atoms");
    let a = atoms::intern_str(rt, "an atom to break");
    // SAFETY: `a` is live (a Rust local).
    let h = unsafe { cell(a).header() };
    h.set_flags(h.flags() & !FLAG_ATOM);
    let e = atoms::verify(rt).unwrap_err();
    assert!(e.contains("no ATOM flag"), "{e}");
    h.set_flags(h.flags() | FLAG_ATOM);
    let f = testing::raw_hash_field(a);
    testing::set_raw_hash_field(a, f ^ (1 << 7));
    let e = atoms::verify(rt).unwrap_err();
    assert!(e.contains("stored hash"), "{e}");
    testing::set_raw_hash_field(a, f);
    atoms::verify(rt).unwrap();
}

#[test]
fn char_at_gives_single_character_atoms() {
    let iso = isolate(config(Dispatch::DEFAULT, true));
    let rt = &*iso.rt;
    let s = string::new_string(rt, "abca\u{e9}");
    let a0 = string::char_at(rt, s, 0);
    let a3 = string::char_at(rt, s, 3);
    assert_eq!(a0, a3);
    assert!(string::is_atom(a0));
    assert_eq!(a0, rt.single_char(b'a'));
    assert_eq!(a0, atoms::intern_str(rt, "a"));
    let e = string::char_at(rt, s, 4);
    assert_eq!(e, rt.single_char(0xe9));
    assert_eq!(string::to_rust_string(e), "\u{e9}");
    // A unit above 0xFF is a new string each time (no table).
    let u = string::new_string(rt, "\u{20ac}");
    let c1 = string::char_at(rt, u, 0);
    let c2 = string::char_at(rt, u, 0);
    assert_ne!(c1, c2);
    assert!(string::equals(c1, c2));
    // A unit string "b" and s[1] are one atom.
    let b = atoms::intern_str(rt, "b");
    assert_eq!(string::char_at(rt, s, 1), b);
    rt.heap.collect();
    atoms::verify(rt).unwrap();
}

#[test]
fn string_view_reads_script_strings_and_the_copy_outlives_gc() {
    // The view itself cannot outlive the next isolate call (compile_fail doctests on
    // Isolate::string_view); the copied text can.
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    for (a, b) in [("abc", "def"), ("\u{20ac}x", "y")] {
        let v = iso.run_raw(concat_of(a, b), "view.js").unwrap();
        let text = iso.string_view(v).map(|s| s.to_string_lossy());
        iso.heap().collect();
        let _ = iso.global_raw("a name interned after the view");
        assert_eq!(text, Some(format!("{a}{b}")));
    }
    assert!(iso.string_view(Value::UNDEFINED).is_none());
}

/// The text of the `i`th string of [`unrooted_strings`].
fn unrooted_text(i: usize) -> String {
    format!("w4-1-unrooted-{i:04}-0123456789")
}

/// `n` new strings, kept as raw `Value`s only on the Rust heap (the buffer of a `Vec`, which the
/// conservative scan does not read): nothing roots them.
#[inline(never)]
fn unrooted_strings(iso: &crate::Isolate, n: usize) -> Vec<Value> {
    let mut kept = Vec::with_capacity(n);
    for i in 0..n {
        let b = string::new_string(&iso.rt, &unrooted_text(i));
        // SAFETY: `b` is a new live string cell; the test reads it only through the checked
        // `Isolate::string_view`.
        kept.push(unsafe { Value::from_bits_unchecked(b) });
    }
    kept
}

/// H20-Q4 (Codex W4-1): `Isolate::string_view` reads only a live string cell of its own heap.
/// Raw `Value`s of strings that nothing roots: after a GC the sweep frees most of them, and
/// the view of each freed cell is `None`, also with zap off (the release default, where the
/// freed text is still in memory); a cell that a stale stack word kept is still read. Bits
/// inside a live cell and bits outside the cage give `None` too.
#[test]
fn string_view_reads_only_live_cells_of_its_heap() {
    for zap in [false, true] {
        let mut cfg = config(Dispatch::DEFAULT, false);
        cfg.heap.zap = zap;
        let mut iso = isolate(cfg);
        let n = 64;
        let gcs = iso.gc_stats().collections;
        let kept = unrooted_strings(&iso, n);
        // No GC ran while they were made, so no address was used twice.
        assert_eq!(iso.gc_stats().collections, gcs);
        for (i, v) in kept.iter().enumerate() {
            let text = iso.string_view(*v).map(|s| s.to_string_lossy());
            assert_eq!(text, Some(unrooted_text(i)), "before the GC, string {i}");
        }
        clobber_stack();
        iso.collect_garbage();
        let mut freed = 0;
        for (i, v) in kept.iter().enumerate() {
            let live = iso.heap().is_cell(v.to_bits() as usize);
            let text = iso.string_view(*v).map(|s| s.to_string_lossy());
            if live {
                assert_eq!(text, Some(unrooted_text(i)), "zap {zap}: kept string {i}");
            } else {
                freed += 1;
                assert_eq!(text, None, "zap {zap}: freed string {i}");
            }
        }
        assert!(
            freed > n / 2,
            "zap {zap}: only {freed} of {n} cells were freed"
        );
        // A live string: bits 8 bytes into its cell, and a cell address outside the cage.
        let live = iso
            .run_raw(concat_of("live-", "string"), "view.js")
            .unwrap();
        assert_eq!(
            iso.string_view(live).map(|s| s.to_string_lossy()),
            Some("live-string".to_string())
        );
        for bits in [live.to_bits() + 8, 8 << 20] {
            // SAFETY: the bits are only given to the checked `Isolate::string_view`.
            let v = unsafe { Value::from_bits_unchecked(bits) };
            assert!(iso.string_view(v).is_none(), "{bits:#x}");
        }
    }
}

#[test]
fn key_compares_and_probes() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    let a = atoms::intern_str(rt, "k");
    let j = atoms::intern_str(rt, "j");
    let plain = string::new_string(rt, "k");
    let count = || rt.atoms.borrow().non_atom_compares();
    let c0 = count();
    assert!(key::key_eq(a, a));
    assert!(!key::key_eq(a, j));
    assert_eq!(key::probe(a), Probe::Bits);
    assert_eq!(count(), c0, "atoms compare by bits");
    assert!(key::key_eq(a, plain));
    assert_eq!(key::probe(plain), Probe::Content);
    if cfg!(debug_assertions) {
        assert_eq!(count(), c0 + 2, "a content compare and a content probe");
    }
    assert_eq!(key::intern_key(rt, plain), a);
    assert_eq!(key::key_hash(a), key::key_hash(plain));
    assert_eq!(key::key_text(rt, plain), "k");
    // The compares above were on purpose; the EP9_ASSERT_ATOM_KEYS gate checks the rest.
    rt.atoms.borrow_mut().reset_non_atom_compares();
}

#[test]
fn index_and_number_keys_are_the_atoms_of_their_text() {
    for stress in [false, true] {
        let iso = isolate(config(Dispatch::DEFAULT, stress));
        let rt = &*iso.rt;
        // Single digits, the cached-index limit (2^24) and the u32 ends.
        let ends = [0, 9, 10, 16_777_215, 16_777_216, 4_294_967_294, u32::MAX];
        for i in ends.into_iter().chain(1000..1100) {
            // A new atom may allocate (a GC point under stress); then it is the text's atom.
            let k = key::index_key(rt, i);
            assert!(string::is_atom(k), "{i}");
            assert_eq!(k, atoms::intern_str(rt, &i.to_string()), "{i}");
            assert_eq!(key::array_index_of(k), (i != u32::MAX).then_some(i), "{i}");
        }
        let cases: [(f64, &str); 7] = [
            (0.0, "0"),
            (-0.0, "0"),
            (12.0, "12"),
            (4_294_967_296.0, "4294967296"),
            (1.5, "1.5"),
            (-1.0, "-1"),
            (f64::NAN, "NaN"),
        ];
        for (d, text) in cases {
            let k = key::number_key(rt, d);
            assert!(string::is_atom(k), "{text}");
            assert_eq!(k, atoms::intern_str(rt, text), "{text}");
        }
        atoms::verify(rt).unwrap();
    }
}

/// The non-atom key compares of `iso`'s thread since its atom table was made (W4-KEY-EQ;
/// always 0 in release builds).
fn non_atom_compares(iso: &crate::Isolate) -> u64 {
    iso.rt.atoms.borrow().non_atom_compares()
}

#[test]
fn own_property_keys_hands_out_index_keys_as_atoms() {
    // o = {bar: 2, 1: 0}; a = [5, , 6]; w = new String("ab"): every key of [[OwnPropertyKeys]]
    // is an atom, so the lookups a caller makes with them (CopyDataProperties) compare bits.
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let texts = iso
        .with_cx(|cx| {
            let o = cx.new_object();
            cx.set(o, "bar", Local::number(2.0), true)?;
            cx.set(o, "1", Local::number(0.0), true)?;
            let a = cx.new_array();
            cx.set(a, "0", Local::number(5.0), true)?;
            cx.set(a, "2", Local::number(6.0), true)?;
            let sp = cx.intrinsic(IntrinsicId::StringPrototype);
            let ab = cx.new_string("ab")?;
            let w = cx.new_wrapper(sp, ab)?;
            let mut texts = Vec::new();
            for obj in [o, a.into(), w] {
                let keys = cx.own_property_keys(obj)?.bits();
                for i in 0..crate::object::array_len(keys) {
                    let k = crate::object::array_get(keys, i).unwrap();
                    let text = string::to_rust_string(k);
                    assert!(string::is_atom(k), "{text}");
                    assert!(
                        crate::property::get_own_property(cx.inner(), obj.bits(), k)
                            .unwrap()
                            .is_some()
                    );
                    texts.push(text);
                }
            }
            Ok(texts)
        })
        .unwrap();
    assert_eq!(texts, ["1", "bar", "0", "2", "length", "0", "1", "length"]);
    if cfg!(debug_assertions) {
        assert_eq!(non_atom_compares(&iso), 0, "W4-KEY-EQ");
    }
}

#[test]
fn push_through_set_uses_atom_keys() {
    // Array.prototype[7] = "p" (so push takes the [[Set]] path); a = []; a.push(1, 2, 3).
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let len = iso
        .with_cx(|cx| {
            let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
            let p = cx.new_string("p")?;
            cx.set(ap, "7", p, true)?;
            let push = cx.get(ap, "push")?;
            let a = cx.new_array();
            let args = [Local::number(1.0), Local::number(2.0), Local::number(3.0)];
            let n = cx.call(push, a, &args)?;
            let n = cx.to_number(n)?;
            let last = cx.get(a, "2")?;
            assert_eq!(cx.to_number(last)?, 3.0);
            Ok(n)
        })
        .unwrap();
    assert_eq!(len, 3.0);
    if cfg!(debug_assertions) {
        assert_eq!(non_atom_compares(&iso), 0, "W4-KEY-EQ");
    }
}

/// Child of `the_atom_key_gate_fails_on_a_non_atom_compare`: one non-atom key compare when
/// `EP9_STR_CHILD=non_atom`.
#[test]
fn non_atom_key_child() {
    if std::env::var("EP9_STR_CHILD").as_deref() != Ok("non_atom") {
        return;
    }
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &*iso.rt;
    let a = string::new_string(rt, "x");
    let b = string::new_string(rt, "x");
    assert!(key::key_eq(a, b));
}

#[cfg(all(debug_assertions, not(miri)))]
#[test]
fn the_atom_key_gate_fails_on_a_non_atom_compare() {
    let run = |gate: bool| {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.args([
            "tests::strings::non_atom_key_child",
            "--exact",
            "--test-threads",
            "1",
        ])
        .env("EP9_STR_CHILD", "non_atom");
        if gate {
            c.env("EP9_ASSERT_ATOM_KEYS", "1");
        } else {
            c.env_remove("EP9_ASSERT_ATOM_KEYS");
        }
        c.output().unwrap()
    };
    assert!(run(false).status.success());
    let out = run(true);
    assert!(!out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("W4-KEY-EQ"), "{text}");
}

#[test]
fn over_limit_strings_throw_range_error() {
    // s = "ab"; for (i = 0; i < 40; i++) s = s + s; -- the 28th doubling passes 2^29 - 24.
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (str_, i) = (s.b.new_register(), s.b.new_register());
        s.lda_str("ab");
        s.b.star(str_).lda_zero().star(i);
        let head = s.b.bound_label();
        let exit = s.b.new_label();
        s.b.lda_smi(40).test_less_than(i, s.cmp).jump_if_false(exit);
        s.b.ldar(str_).add(str_, s.bin).star(str_);
        s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
        s.b.bind(exit);
        s.b.ldar(str_).return_();
        unit("", vec![s.finish()])
    }
    let e = on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        iso.run_raw(make(), "limit.js").unwrap_err()
    });
    assert_eq!(e.constructor.as_deref(), Some("RangeError"), "{e}");
    assert!(e.message.contains("Invalid string length"), "{e}");
    assert_eq!(string::MAX_STRING_LENGTH, (1 << 29) - 24);
}

#[test]
fn computed_keys_and_string_indexing_under_gc_stress() {
    // var o = {}; var k = "na" + "me"; o[k] = 1; print(o.name);
    // o[5] = "five"; print(o["5"]); print("abc"[1]); print(k in o);
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (o, k, t) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        s.b.create_empty_object_literal().star(o);
        s.lda_str("na");
        s.b.star(t);
        s.lda_str("me");
        s.b.add(t, s.bin).star(k);
        s.b.lda_smi(1).set_keyed_property(o, k, s.set_keyed);
        s.get_named(o, "name");
        s.print_acc();
        s.b.lda_smi(5).star(t);
        s.lda_str("five");
        s.b.set_keyed_property(o, t, s.set_keyed);
        s.lda_str("5");
        s.b.get_keyed_property(o, s.keyed);
        s.print_acc();
        s.lda_str("abc");
        s.b.star(t).lda_smi(1).get_keyed_property(t, s.keyed);
        s.print_acc();
        let hs = s.b.feedback_slot(FeedbackSlotKind::HasKeyed);
        s.b.ldar(o).test_in(k, hs);
        s.print_acc();
        s.b.ldar(k).return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    assert_eq!(ok(&r), "name");
    assert_eq!(r.output, ["1", "five", "b", "true"]);
}

// ---- ropes and slices (M4-P1 STR1; H19) ------------------------------------------------------

/// The allocator of the `ep9_vm` unit-test binary: `System`, plus a per-thread count of Rust
/// heap allocations, so a test can show that a call copies nothing into a Rust `Vec` (tests
/// run on many threads; each counts only its own). The JS heap does not use it (`ep9_heap`
/// maps its own pages).
struct CountingAlloc;

thread_local! {
    static RUST_ALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn count_rust_alloc() {
    // `try_with`: a thread that is tearing down its locals still allocates.
    let _ = RUST_ALLOCS.try_with(|n| n.set(n.get() + 1));
}

/// Rust heap allocations made so far on this thread.
fn rust_allocs() -> u64 {
    RUST_ALLOCS.with(Cell::get)
}

// SAFETY: every method forwards to `System` with the same arguments, so `System`'s contract
// holds; the count is a thread-local `Cell` with no destructor and allocates nothing.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_rust_alloc();
        // SAFETY: the caller keeps `GlobalAlloc::alloc`'s contract (non-zero size).
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_rust_alloc();
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator, so from `System`, with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_rust_alloc();
        // SAFETY: as `dealloc`, and the caller keeps `realloc`'s size contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

/// `a + b` through the runtime concat (the string Add path).
fn cat(rt: &Runtime, a: u64, b: u64) -> u64 {
    string::concat(rt, a, b).expect("under the limit")
}

/// The text of a string through the rope-safe reader.
fn text(b: u64) -> String {
    string::to_rust_string(b)
}

/// The rope and slice tests run on a stressed isolate: a collection before every allocation
/// and the heap verifier after each one (plan v9 §3.6 acceptance).
fn stressed() -> crate::Isolate {
    isolate(config(Dispatch::DEFAULT, true))
}

#[test]
fn concat_makes_a_cons_from_13_units_and_a_flat_string_below() {
    let iso = stressed();
    let rt = &*iso.rt;
    let a = string::new_string(rt, "abcdef");
    let b = string::new_string(rt, "ghijkl");
    let short = cat(rt, a, b);
    assert_eq!(kinds::cell_kind(short), kinds::SEQ_LATIN1.get());
    assert_eq!(text(short), "abcdefghijkl");
    let m = string::new_string(rt, "m");
    let long = cat(rt, short, m);
    assert_eq!(kinds::cell_kind(long), kinds::CONS.get());
    assert_eq!(cell_size(rt, long), 32);
    assert_eq!(string::str_len(long), string::MIN_CONS_LENGTH);
    assert_eq!(text(long), "abcdefghijklm");
    // "" + s and s + "" are s.
    let e = rt.atom(Atom::Empty);
    assert_eq!(cat(rt, e, long), long);
    assert_eq!(cat(rt, long, e), long);
    rt.heap.collect();
    assert_eq!(text(long), "abcdefghijklm");
}

#[test]
fn flatten_writes_the_flat_form_once_into_the_cons() {
    let mut iso = stressed();
    let rt: &Runtime = &iso.rt;
    let mut s = string::new_string(rt, "x");
    for i in 0..200 {
        let p = string::new_string(rt, &format!("{i},"));
        s = cat(rt, s, p);
    }
    let want: String = std::iter::once("x".to_string())
        .chain((0..200).map(|i| format!("{i},")))
        .collect();
    assert_eq!(kinds::cell_kind(s), kinds::CONS.get());
    let f = string::flatten(&mut iso.cx, s);
    let rt: &Runtime = &iso.rt;
    assert_eq!(kinds::cell_kind(f), kinds::SEQ_LATIN1.get());
    assert_eq!(text(f), want);
    // The CONS is now (flat, ""): a second flatten returns the same cell and allocates
    // nothing (under GC stress an allocation would collect); the CONS reads as its flat form.
    let before = rt.heap.stats().collections;
    assert_eq!(string::flatten(&mut iso.cx, s), f);
    assert_eq!(iso.rt.heap.stats().collections, before);
    assert_eq!(kinds::cell_kind(s), kinds::CONS.get());
    assert_eq!(string::view(&iso.cx, f).len(), want.len());
    assert_eq!(text(s), want);
    assert!(string::equals(s, f));
    assert_eq!(string::hash(s), string::hash(f));
}

#[test]
fn deep_ropes_read_flatten_and_compare_without_recursion() {
    // A 100k-deep left rope (s += x) and right rope (s = x + s) on the test thread's normal
    // stack: the walks keep their work on a Rust Vec, not the native stack. GC stress runs a
    // smaller depth (a collection per allocation is quadratic).
    for (stress, n) in [(false, 100_000), (true, 1_500)] {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        let rt: &Runtime = &iso.rt;
        // The parts live in scratch roots: a Rust Vec buffer is not scanned (rule 8).
        let roots = rt.heap.scratch_roots();
        for d in 0..10 {
            roots.push(val(string::new_string(rt, &format!("<{d}>"))));
        }
        let part = |i: usize| roots.get(i % 10).to_bits();
        let (mut left, mut right) = (rt.atom(Atom::Empty), rt.atom(Atom::Empty));
        for i in 0..n {
            left = cat(rt, left, part(i));
            right = cat(rt, part(n - 1 - i), right);
        }
        let want: String = (0..n).map(|i| format!("<{}>", i % 10)).collect();
        assert_eq!(text(left), want);
        assert_eq!(text(right), want);
        assert!(string::equals(left, right));
        assert_eq!(string::compare(left, right), core::cmp::Ordering::Equal);
        assert_eq!(string::hash(left), string::hash(right));
        assert!(string::equals_text(left, JsStr::Latin1(want.as_bytes())));
        // One part more on the left: greater, not equal.
        let bigger = cat(rt, left, part(0));
        assert_eq!(string::compare(bigger, right), core::cmp::Ordering::Greater);
        assert_eq!(string::compare(right, bigger), core::cmp::Ordering::Less);
        assert!(!string::equals(bigger, right));
        drop(roots);
        let fl = string::flatten(&mut iso.cx, left);
        let fr = string::flatten(&mut iso.cx, right);
        assert_eq!(text(fl), want);
        assert!(string::equals(fl, fr));
        iso.collect_garbage();
        assert_eq!(text(left), want);
        assert_eq!(text(bigger), format!("{want}<0>"));
    }
}

#[test]
fn a_rope_hashes_and_compares_like_its_flat_text() {
    let iso = stressed();
    let rt = &*iso.rt;
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for round in 0..200 {
        // Random parts, some with a unit above 0xFF, glued in a random tree shape. The parts
        // live in scratch roots; `pieces` holds their root indices.
        let roots = rt.heap.scratch_roots();
        let mut pieces: Vec<usize> = Vec::new();
        let mut all: Vec<u16> = Vec::new();
        for _ in 0..(1 + rng.next() % 6) {
            let len = (rng.next() % 9) as usize;
            let wide = rng.next().is_multiple_of(4);
            let units: Vec<u16> = (0..len)
                .map(|_| {
                    let u = (rng.next() % 0x7f) as u16 + 1;
                    if wide && rng.next().is_multiple_of(3) {
                        u + 0x400
                    } else {
                        u
                    }
                })
                .collect();
            all.extend(&units);
            pieces.push(roots.push(val(string::new_string_utf16(rt, &units))));
        }
        while pieces.len() > 1 {
            let i = (rng.next() as usize) % (pieces.len() - 1);
            let (x, y) = (
                roots.get(pieces[i]).to_bits(),
                roots.get(pieces[i + 1]).to_bits(),
            );
            let joined = roots.push(val(cat(rt, x, y)));
            pieces.splice(i..i + 2, [joined]);
        }
        let r = roots.get(pieces[0]).to_bits();
        let flat = string::new_string_utf16(rt, &all);
        assert_eq!(string::str_len(r), all.len());
        assert!(string::equals(r, flat), "round {round}");
        assert!(string::equals(flat, r), "round {round}");
        assert_eq!(string::hash(r), string::hash(flat), "round {round}");
        assert_eq!(string::compare(r, flat), core::cmp::Ordering::Equal);
        assert!(string::equals_text(r, JsStr::Utf16(&all)), "round {round}");
        assert_eq!(text(r), String::from_utf16_lossy(&all));
        for (i, &u) in all.iter().enumerate() {
            assert_eq!(string::unit_at(r, i), u, "round {round} unit {i}");
        }
        assert_eq!(string::array_index(r), JsStr::Utf16(&all).array_index());
        let wide = all.iter().any(|&u| u > 0xFF);
        string::with_text(r, |t| {
            assert_eq!(matches!(t, JsStr::Utf16(_)), wide, "round {round}");
            assert_eq!(t.to_utf16(), all);
        });
    }
}

#[test]
fn rope_hash_matches_the_slice_hash() {
    // The streaming SipHash over a rope walk equals the slice form for every tail length.
    let iso = stressed();
    let rt = &*iso.rt;
    for len in 13..40 {
        let units: Vec<u16> = (0..len as u16).map(|i| 0x61 + i % 26).collect();
        let (a, b) = units.split_at(len / 3);
        let r = cat(
            rt,
            string::new_string_utf16(rt, a),
            string::new_string_utf16(rt, b),
        );
        assert_eq!(kinds::cell_kind(r), kinds::CONS.get());
        assert_eq!(string::hash(r), string::hash_text(JsStr::Utf16(&units)).1);
    }
}

#[test]
fn a_cons_with_a_utf16_leaf_flattens_to_utf16() {
    let mut iso = stressed();
    let rt: &Runtime = &iso.rt;
    let a = string::new_string(rt, "latin-one ");
    let b = string::new_string_utf16(rt, &[0x20ac, 0x41, 0x42]);
    let r = cat(rt, a, b);
    assert_eq!(kinds::cell_kind(r), kinds::CONS.get());
    let f = string::flatten(&mut iso.cx, r);
    assert_eq!(kinds::cell_kind(f), kinds::SEQ_UTF16.get());
    assert_eq!(text(f), "latin-one \u{20ac}AB");
    // A Latin-1-only rope flattens to Latin-1.
    let rt: &Runtime = &iso.rt;
    let r2 = cat(rt, a, a);
    let f2 = string::flatten(&mut iso.cx, r2);
    assert_eq!(kinds::cell_kind(f2), kinds::SEQ_LATIN1.get());
}

#[test]
fn substring_makes_a_slice_from_13_units_over_the_flat_parent() {
    let mut iso = stressed();
    let rt: &Runtime = &iso.rt;
    let p = string::new_string(rt, "0123456789abcdefghijklmnopqrstuvwxyz");
    let s = string::substring(rt, p, 3, 20);
    assert_eq!(kinds::cell_kind(s), kinds::SLICED.get());
    assert_eq!(cell_size(rt, s), 32);
    assert_eq!(text(s), "3456789abcdefghij");
    assert_eq!(string::view(&iso.cx, s).len(), 17);
    // A slice of a slice points at the same parent.
    let rt: &Runtime = &iso.rt;
    let s2 = string::substring(rt, s, 2, 15);
    assert_eq!(kinds::cell_kind(s2), kinds::SLICED.get());
    assert_eq!(text(s2), "56789abcdefgh");
    // Below 13 units: a flat copy; the whole string: itself; empty: "".
    let s3 = string::substring(rt, s, 0, 12);
    assert_eq!(kinds::cell_kind(s3), kinds::SEQ_LATIN1.get());
    assert_eq!(text(s3), "3456789abcde");
    assert_eq!(string::substring(rt, p, 0, 36), p);
    assert_eq!(string::substring(rt, p, 5, 5), rt.atom(Atom::Empty));
    // flatten leaves a slice as is; hash and compare see its text.
    assert_eq!(string::flatten(&mut iso.cx, s), s);
    let rt: &Runtime = &iso.rt;
    let same = string::new_string(rt, "3456789abcdefghij");
    assert!(string::equals(s, same));
    assert_eq!(string::hash(s), string::hash(same));
    // A substring of a rope flattens it first; a UTF-16 parent gives a UTF-16 slice.
    let w = string::new_string_utf16(rt, &[0x3b1; 20]);
    let r = cat(rt, p, w);
    let s4 = string::substring(rt, r, 30, 45);
    assert_eq!(kinds::cell_kind(s4), kinds::SLICED.get());
    let mut want: Vec<u16> = "uvwxyz".encode_utf16().collect();
    want.extend([0x3b1; 9]);
    assert!(string::equals_text(s4, JsStr::Utf16(&want)));
    // The slice keeps its parent alive across collections.
    drop(want);
    iso.collect_garbage();
    assert_eq!(text(s2), "56789abcdefgh");
}

#[test]
fn char_at_and_string_view_flatten_a_rope() {
    let mut iso = stressed();
    let rt: &Runtime = &iso.rt;
    let a = string::new_string(rt, "abcdefghij");
    let r = cat(rt, a, a);
    assert_eq!(kinds::cell_kind(r), kinds::CONS.get());
    let c = string::char_at(rt, r, 13);
    assert_eq!(text(c), "d");
    // SAFETY: `r` is a live string cell of this isolate (a Rust local: conservative scan).
    let v = unsafe { Value::from_bits_unchecked(r) };
    let got = iso.string_view(v).map(|s| s.to_string_lossy());
    assert_eq!(got.as_deref(), Some("abcdefghijabcdefghij"));
}

#[test]
fn an_interned_rope_is_one_key_with_its_flat_text() {
    // atoms::intern takes any string kind (it is not a GC point, so it keeps the CONS): a
    // rope atom and the same text from Rust are one key.
    let iso = stressed();
    let rt = &*iso.rt;
    let a = string::new_string(rt, "property-");
    let b = string::new_string(rt, "name-long");
    let r = cat(rt, a, b);
    let k = atoms::intern(rt, r);
    assert_eq!(k, r);
    assert!(string::is_atom(r));
    assert_eq!(atoms::intern_str(rt, "property-name-long"), r);
    assert_eq!(atoms::lookup(rt, "property-name-long"), Some(r));
    rt.heap.collect();
    atoms::verify(rt).unwrap();
}

#[test]
fn ropes_from_bytecode_add_print_and_compare() {
    // var s = ""; for (i = 0; i < 40; i++) s = s + "ab"; print(s); return s === s + "";
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (str_, i, t) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
        s.lda_str("");
        s.b.star(str_).lda_zero().star(i);
        let head = s.b.bound_label();
        let exit = s.b.new_label();
        s.b.lda_smi(40).test_less_than(i, s.cmp).jump_if_false(exit);
        s.lda_str("ab");
        s.b.add(str_, s.bin).star(str_);
        s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
        s.b.bind(exit);
        s.b.ldar(str_);
        s.print_acc();
        s.lda_str("");
        s.b.add(str_, s.bin).star(t);
        s.b.ldar(t).test_equal_strict(str_, s.cmp).return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    assert_eq!(ok(&r), "true");
    assert_eq!(r.output, ["ab".repeat(40)]);
}

#[test]
fn array_index_reads_ropes_and_slices_without_a_flatten() {
    // `profile.rs` (the keyed `KeySet` bits) and `key::array_index_of` call
    // `string::array_index` on keys that can be a CONS not yet flattened or a SLICED string
    // (M4-P1 PROFILE need 5). It reads them in place: no flatten, no JS-heap allocation (under
    // GC stress an allocation would collect), before and after the hash field is cached. A
    // rope or slice has at least 13 units, more than any array index (10 digits), so the
    // answer is always `None`.
    let iso = stressed();
    let rt = &*iso.rt;
    let digits = |s: &str| string::new_string(rt, s);
    let cases = [
        cat(rt, digits("1234567"), digits("890123")),
        cat(rt, digits("0"), digits("000000000000")),
        cat(rt, digits("4294967"), digits("294000")),
        cat(rt, digits("abcdefg"), digits("hijklm")),
    ];
    let parent = digits("12345678901234567890");
    let slice = string::substring(rt, parent, 1, 15);
    assert_eq!(kinds::cell_kind(slice), kinds::SLICED.get());
    for (i, &s) in cases.iter().chain([&slice]).enumerate() {
        let before = rt.heap.stats().collections;
        assert_eq!(string::array_index(s), None, "case {i}");
        let _ = string::hash(s);
        assert_eq!(string::array_index(s), None, "case {i}, hash cached");
        assert_eq!(key::array_index_of(s), None, "case {i}");
        assert_eq!(
            rt.heap.stats().collections,
            before,
            "case {i}: no allocation"
        );
        if i < cases.len() {
            assert_eq!(kinds::cell_kind(s), kinds::CONS.get(), "case {i}");
            assert!(string::str_len(s) >= string::MIN_CONS_LENGTH);
        }
    }
}

#[test]
fn array_index_copies_no_rope_text_to_the_rust_heap() {
    // Codex r2 code F2 (M4-P1): the test above counts collections, which a Rust `Vec` copy
    // does not make. `string::array_index` answers a key longer than 10 units without
    // `with_text` (a974ad4), so a rope key (Latin-1 or two-byte), before or after its hash is
    // cached, costs no Rust heap allocation; a slice key is flat and never needed the copy.
    // The control shows the counter sees the copy that the guard skips.
    let iso = stressed();
    let rt = &*iso.rt;
    let ropes = [
        cat(
            rt,
            string::new_string(rt, "1234567"),
            string::new_string(rt, "890123"),
        ),
        cat(
            rt,
            string::new_string(rt, "12345\u{3b1}7"),
            string::new_string(rt, "890123"),
        ),
    ];
    let parent = string::new_string(rt, "12345678901234567890");
    let slice = string::substring(rt, parent, 1, 15);
    assert_eq!(kinds::cell_kind(slice), kinds::SLICED.get());
    for (i, &s) in ropes.iter().chain([&slice]).enumerate() {
        for cached in [false, true] {
            if cached {
                let _ = string::hash(s);
            }
            let before = rust_allocs();
            assert_eq!(string::array_index(s), None, "case {i}");
            assert_eq!(key::array_index_of(s), None, "case {i}");
            let made = rust_allocs() - before;
            assert_eq!(
                made, 0,
                "case {i}, hash cached {cached}: Rust heap allocations"
            );
        }
        if i < ropes.len() {
            assert_eq!(kinds::cell_kind(s), kinds::CONS.get(), "case {i}");
        }
    }
    // Control: the copy that the guard skips is a Rust heap allocation the counter sees.
    let before = rust_allocs();
    string::with_text(ropes[0], |t| assert_eq!(t.array_index(), None));
    assert!(
        rust_allocs() > before,
        "with_text copies an unflattened rope"
    );
}
