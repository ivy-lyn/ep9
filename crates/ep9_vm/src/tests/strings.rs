//! String, atom and property-key tests (H19 v0): the cell layout, interning, the weak atom
//! table, the hash and its seed, array-index keys, the single-character table and the length
//! limit.

use std::process::Command;

use ep9_bytecode::{FeedbackSlotKind, VerifiedUnit};
use ep9_heap::HeapConfig;
use ep9_value::Value;

use super::{Asm, config, isolate, ok, on_big_stack, run_all, unit};
use crate::atoms::{self, Atom};
use crate::key::{self, Probe};
use crate::kinds::{self, FLAG_ATOM};
use crate::object::cell;
use crate::runtime::Runtime;
use crate::string::{self, testing};
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
            let a = iso.run_script(returns("sharedName"), "u1.js").unwrap();
            let a = a.to_bits();
            let b = iso.run_script(returns("sharedName"), "u2.js").unwrap();
            assert_eq!(a, b.to_bits(), "the same name from two units");
            assert!(string::is_atom(a));
            // "shared" + "Name" is a new cell; ToPropertyKey gives the unit's atom.
            let c = iso
                .run_script(concat_of("shared", "Name"), "u3.js")
                .unwrap();
            let c = c.to_bits();
            assert_ne!(c, a);
            assert!(!string::is_atom(c));
            assert_eq!(key::to_property_key(&mut iso.cx, c).unwrap(), a);
            // A text with no atom yet: the string itself becomes the atom (no copy).
            let d = iso.run_script(concat_of("brand", "New"), "u4.js").unwrap();
            let d = d.to_bits();
            assert_eq!(atoms::lookup(&iso.rt, "brandNew"), None);
            assert_eq!(key::to_property_key(&mut iso.cx, d).unwrap(), d);
            assert!(string::is_atom(d));
            assert_eq!(atoms::lookup(&iso.rt, "brandNew"), Some(d));
            // A unit name equal to a static atom is that atom.
            let l = iso.run_script(returns("length"), "u5.js").unwrap();
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
        let v = iso.run_script(concat_of(a, b), "view.js").unwrap();
        let text = iso.string_view(v).map(|s| s.to_string_lossy());
        iso.heap().collect();
        let _ = iso.global("a name interned after the view");
        assert_eq!(text, Some(format!("{a}{b}")));
    }
    assert!(iso.string_view(Value::UNDEFINED).is_none());
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
                    assert!(crate::property::get_own_property(cx.inner(), obj.bits(), k).is_some());
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
        iso.run_script(make(), "limit.js").unwrap_err()
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
