//! Symbol tests (H19 "Symbols"): the SYMBOL cell, the well-known symbols, the `Symbol.for`
//! registry, the identity hash and its seed, the conversions (`${s}`, `+s`, `typeof`),
//! symbols as property keys, and `@@toPrimitive` with its hints.
//!
//! JS code cannot make a symbol before the Symbol builtins (P3), so the scripts call two
//! test host functions: `sym(desc)` (a new symbol) and `wks(name)` (a well-known symbol).

use std::hint::black_box;
use std::process::Command;

use ep9_bytecode::{FeedbackSlotKind, HandlerKind, Label, Register, TypeOfLiteral, VerifiedUnit};
use ep9_heap::ScratchRoots;

use super::{Asm, config, isolate, on_big_stack, print, take_output, unit};
use crate::atoms;
use crate::conv::{self, Hint};
use crate::frame::{NULL, UNDEFINED};
use crate::key::{self, Probe};
use crate::kinds;
use crate::object::{self, attr, cell};
use crate::property;
use crate::runtime::{Root, Runtime, val};
use crate::shape::{self, Added};
use crate::string;
use crate::symbol::{self, WellKnownSymbol, flag};
use crate::{Args, Cx, Dispatch, Isolate, JsResult, Local};

// ---- host functions -------------------------------------------------------------------------

/// `sym(desc)`: a new symbol; its description is ToString(desc), or undefined.
fn sym<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let d = args.get(0);
    let desc = if d.is_undefined() {
        UNDEFINED
    } else {
        cx.to_string(d)?.bits()
    };
    let rt = cx.inner().rt();
    Ok(Local::from_bits(symbol::new_symbol(rt, desc)))
}

/// `wks(name)`: the well-known symbol `Symbol[name]`.
fn wks<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let name = cx.to_rust_string(args.get(0))?;
    let w = WellKnownSymbol::ALL
        .into_iter()
        .find(|w| w.name() == name)
        .expect("a well-known symbol name");
    let rt = cx.inner().rt();
    Ok(Local::from_bits(symbol::well_known(rt, w)))
}

/// `recordHint(hint)`: prints its argument (the hint) and returns 7.
fn record_hint<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    nt: Local<'s>,
) -> JsResult<Local<'s>> {
    print(cx, this, args, nt)?;
    Ok(Local::int32(7))
}

/// `returnsThis()`: its receiver (an object, so ToPrimitive throws).
fn returns_this<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

/// A test isolate with `print`, `sym`, `wks`, `recordHint` and `returnsThis`.
fn symbol_isolate(d: Dispatch, stress: bool) -> Isolate {
    let mut iso = isolate(config(d, stress));
    iso.register_host_function("sym", 1, sym);
    iso.register_host_function("wks", 1, wks);
    iso.register_host_function("recordHint", 1, record_hint);
    iso.register_host_function("returnsThis", 0, returns_this);
    iso
}

/// What a script run gave: the displayed result or the exception message, and the output.
type SymRun = (Result<String, String>, Vec<String>);

/// Runs the unit from `make` in a [`symbol_isolate`] under every dispatch expansion, with and
/// without GC stress (then the heap verifier); all runs must agree.
fn run_sym(make: fn() -> VerifiedUnit) -> SymRun {
    let mut first: Option<(String, SymRun)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = on_big_stack(move || {
                let mut iso = symbol_isolate(d, stress);
                take_output();
                let r = iso
                    .run_script(make(), "symbols.js")
                    .map(|v| iso.display(v))
                    .map_err(|e| e.message);
                let out = take_output();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (r, out)
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

// ---- bytecode helpers -------------------------------------------------------------------------

/// `acc = name(arg)` for the global function `name` and the string `arg` (clobbers `f`, `a`).
fn call_with_str(s: &mut Asm, name: &str, arg: &str) {
    let (f, a0) = (s.f, s.a.get(0));
    s.lda_str(arg);
    s.b.star(a0);
    s.lda_global(name);
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a0, s.call);
}

/// `try { body } catch (e) { print(e.message) }` around what `body` emits.
fn try_print_message(s: &mut Asm, ctx: Register, body: impl FnOnce(&mut Asm)) {
    let (start, end, handler, done): (Label, Label, Label, Label) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.lda_current_context().star(ctx);
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    body(s);
    s.b.bind(end);
    s.b.jump(done);
    s.b.bind(handler);
    let e = s.a.get(2);
    s.b.star(e);
    s.get_named(e, "message");
    s.print_acc();
    s.b.bind(done);
}

// ---- conversions ------------------------------------------------------------------------------

#[test]
fn template_literals_and_unary_plus_throw_on_a_symbol() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, x, y, t) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let ts = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
        let (bin, cmp) = (s.bin, s.cmp);
        // x = sym("x"); y = sym("x")
        call_with_str(&mut s, "sym", "x");
        s.b.star(x);
        call_with_str(&mut s, "sym", "x");
        s.b.star(y);
        // `${x}` (ToString)
        try_print_message(&mut s, ctx, |s| {
            s.b.ldar(x).to_string();
        });
        // +x (ToNumber)
        try_print_message(&mut s, ctx, |s| {
            s.b.ldar(x).to_number(bin);
        });
        // x + "" and "" + x (string concatenation)
        try_print_message(&mut s, ctx, |s| {
            s.lda_str("");
            s.b.add(x, bin);
        });
        try_print_message(&mut s, ctx, |s| {
            s.lda_str("").b.star(t);
            s.b.ldar(x).add(t, bin);
        });
        // x + 1, x < 1, -x (ToNumeric)
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_smi(1).add(x, bin);
        });
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_smi(1).test_less_than(x, cmp);
        });
        try_print_message(&mut s, ctx, |s| {
            s.b.ldar(x).negate(bin);
        });
        // typeof x, and the TestTypeOf forms
        s.b.ldar(x).type_of(ts);
        s.print_acc();
        s.b.ldar(x).test_type_of(TypeOfLiteral::Symbol);
        s.print_acc();
        s.b.ldar(x).test_type_of(TypeOfLiteral::Object);
        s.print_acc();
        s.lda_str("x").b.test_type_of(TypeOfLiteral::Symbol);
        s.print_acc();
        // Identity: x === x, x == x, x === y, x == y, x == "Symbol(x)", !x
        s.b.ldar(x).test_equal_strict(x, cmp);
        s.print_acc();
        s.b.ldar(x).test_equal(x, cmp);
        s.print_acc();
        s.b.ldar(y).test_equal_strict(x, cmp);
        s.print_acc();
        s.b.ldar(y).test_equal(x, cmp);
        s.print_acc();
        s.lda_str("Symbol(x)").b.test_equal(x, cmp);
        s.print_acc();
        s.b.ldar(x).to_boolean_logical_not();
        s.print_acc();
        // The result: a symbol is displayed as Symbol(desc).
        s.b.ldar(x).return_();
        unit("", vec![s.finish()])
    }
    let (r, out) = run_sym(make);
    assert_eq!(r.as_deref(), Ok("Symbol(x)"));
    let str_err = "Cannot convert a Symbol value to a string";
    let num_err = "Cannot convert a Symbol value to a number";
    assert_eq!(
        out,
        [
            str_err, num_err, str_err, str_err, num_err, num_err, num_err, "symbol", "true",
            "false", "false", "true", "true", "false", "false", "false", "false",
        ]
    );
}

#[test]
fn a_symbol_is_a_32_byte_cell_with_a_description_a_hash_and_flags() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt = cx.rt();
        let d = atoms::intern_str(rt, "d\u{e9}sc");
        let s = symbol::new_symbol(rt, d);
        let u = symbol::new_symbol(rt, UNDEFINED);
        for (b, desc) in [(s, d), (u, UNDEFINED)] {
            assert_eq!(kinds::cell_kind(b), kinds::SYMBOL.get());
            // SAFETY: `b` is a live cell held in a Rust local (conservative scan).
            assert_eq!(unsafe { rt.heap.cell_size(cell(b)) }, 32);
            assert!(kinds::is_symbol(b) && !kinds::is_object(b) && !kinds::is_string(b));
            assert!(key::is_symbol_key(b) && key::is_property_key(b));
            assert_eq!(key::probe(b), Probe::Bits);
            assert_eq!(key::array_index_of(b), None);
            assert_eq!(symbol::description(b), desc);
            assert_eq!(symbol::flags(b), 0);
            assert!(symbol::hash(b) < 1 << 30);
            assert_eq!(key::key_hash(b), symbol::hash(b));
            assert_eq!(key::intern_key(rt, b), b, "a symbol is its own key");
            assert!(!key::key_eq(b, d) && key::key_eq(b, b));
            assert_eq!(symbol::key_for(b), UNDEFINED);
        }
        assert_ne!(s, u);
        // Text: SymbolDescriptiveString, in messages and as a heap string.
        assert_eq!(conv::describe(s), "Symbol(d\u{e9}sc)");
        assert_eq!(conv::describe(u), "Symbol()");
        assert_eq!(key::key_text(rt, s), "Symbol(d\u{e9}sc)");
        assert_eq!(conv::type_of(s).text(), "symbol");
        assert!(conv::to_boolean(u));
        let ds = symbol::descriptive_string(cx, s).unwrap();
        assert_eq!(string::to_rust_string(ds), "Symbol(d\u{e9}sc)");
        let du = symbol::descriptive_string(cx, u).unwrap();
        assert_eq!(string::to_rust_string(du), "Symbol()");
        // ToPropertyKey of a symbol is the symbol; ToString and ToNumber throw TypeError.
        assert_eq!(key::to_property_key(cx, s).unwrap(), s);
        assert!(conv::to_string(cx, s).is_err());
        let e = cx.rt().take_pending();
        assert!(kinds::is_error(e));
        assert!(conv::to_number(cx, s).is_err());
        cx.rt().take_pending();
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

// ---- roots, registry, hashes ------------------------------------------------------------------

#[test]
fn well_known_symbols_are_isolate_roots_in_order() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let mut seen = Vec::new();
        for w in WellKnownSymbol::ALL {
            let s = symbol::well_known(rt, w);
            assert!(kinds::is_symbol(s), "{w:?}");
            assert!(symbol::is_well_known(s) && !symbol::is_registered(s));
            assert_eq!(symbol::flags(s), flag::WELL_KNOWN);
            let d = symbol::description(s);
            assert_eq!(string::to_rust_string(d), format!("Symbol.{}", w.name()));
            assert!(string::is_atom(d));
            assert!(!seen.contains(&s));
            seen.push(s);
        }
        // %Symbol.prototype%: an ordinary object whose prototype is %Object.prototype%.
        let p = symbol::prototype(rt);
        assert!(kinds::cell_kind(p) == kinds::OBJECT.get());
        assert_eq!(object::proto_of(p), rt.root(Root::ObjectPrototype));
        // Hide the bits from the conservative scan: only the root source keeps them.
        let masked: Vec<u64> = seen.iter().map(|&s| s ^ MASK).collect();
        drop(seen);
        scrub_stack();
        iso.collect_garbage();
        iso.verify_heap().unwrap();
        let rt: &Runtime = &iso.rt;
        for (i, w) in WellKnownSymbol::ALL.into_iter().enumerate() {
            let s = symbol::well_known(rt, w);
            assert_eq!(s, masked[i] ^ MASK, "{w:?} moved");
            assert!(rt.heap.is_cell(cell(s).addr()), "{w:?} died");
        }
        assert!(rt.heap.is_cell(cell(symbol::prototype(rt)).addr()));
    });
}

const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Overwrites stack below the caller, so stale words of returned frames cannot keep a cell
/// alive through the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// `Symbol.for(text)` in its own frame; returns (symbol bits ^ MASK, its hash).
#[inline(never)]
fn symbol_for_masked(cx: &mut crate::interp::Cx, text: &str) -> (u64, u32) {
    let k = string::new_string(cx.rt(), text);
    let s = symbol::symbol_for(cx, k);
    (black_box(s) ^ MASK, symbol::hash(s))
}

#[test]
fn the_registry_is_stable() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt = cx.rt();
        let n0 = symbol::registry_len(rt);
        // One symbol per key text, whatever cell carries the text.
        let a = symbol::symbol_for(cx, atoms::intern_str(rt, "app"));
        let fresh =
            string::concat(rt, atoms::intern_str(rt, "a"), atoms::intern_str(rt, "pp")).unwrap();
        assert!(!string::is_atom(fresh));
        assert_eq!(symbol::symbol_for(cx, fresh), a);
        let utf16 = string::testing::new_utf16_cell(rt, &[0x61, 0x70, 0x70]);
        assert_eq!(
            symbol::symbol_for(cx, utf16),
            a,
            "UTF-16 storage, same text"
        );
        let b = symbol::symbol_for(cx, atoms::intern_str(rt, "other"));
        assert_ne!(a, b);
        let e = symbol::symbol_for(cx, rt.atom(crate::runtime::Atom::Empty));
        assert_ne!(e, a);
        assert_eq!(symbol::registry_len(rt), n0 + 3);
        // KeyForSymbol: the key of a registered symbol; undefined for any other symbol.
        assert!(symbol::is_registered(a) && !symbol::is_well_known(a));
        assert_eq!(string::to_rust_string(symbol::key_for(a)), "app");
        assert_eq!(string::to_rust_string(symbol::key_for(e)), "");
        assert_eq!(conv::describe(a), "Symbol(app)");
        let plain = symbol::new_symbol(rt, atoms::intern_str(rt, "app"));
        assert_ne!(plain, a);
        assert_eq!(symbol::key_for(plain), UNDEFINED);
        let iter = symbol::well_known(rt, WellKnownSymbol::Iterator);
        assert_eq!(symbol::key_for(iter), UNDEFINED);
        // Only the registry keeps "kept" alive: same cell and hash after a full GC, and
        // many other registrations (growth) in between.
        let (kept, kept_hash) = symbol_for_masked(cx, "kept");
        for i in 0..100 {
            symbol_for_masked(cx, &format!("filler {i}"));
        }
        scrub_stack();
        cx.rt().heap.collect();
        assert!(
            cx.rt().heap.is_cell(cell(kept ^ MASK).addr()),
            "the registry keeps it"
        );
        let again = symbol_for_masked(cx, "kept");
        assert_eq!(again, (kept, kept_hash));
        assert_eq!(symbol::registry_len(cx.rt()), n0 + 3 + 1 + 100);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// The hashes of the well-known symbols and of four new symbols in a new isolate.
fn hashes_of_a_new_isolate() -> Vec<u32> {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt: &Runtime = &iso.rt;
    let mut h: Vec<u32> = WellKnownSymbol::ALL
        .into_iter()
        .map(|w| symbol::hash(symbol::well_known(rt, w)))
        .collect();
    for _ in 0..4 {
        h.push(symbol::hash(symbol::new_symbol(rt, UNDEFINED)));
    }
    h
}

/// Child of `two_isolates_with_one_seed_give_the_same_symbol_hashes`: prints the symbol
/// hashes of a new isolate when `EP9_SYM_CHILD=hash`.
#[test]
fn symbol_hash_seed_child() {
    if std::env::var("EP9_SYM_CHILD").as_deref() != Ok("hash") {
        return;
    }
    println!("SYMHASH {:?}", hashes_of_a_new_isolate());
}

fn child_symbol_hashes(seed: &str) -> String {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "tests::symbols::symbol_hash_seed_child",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("EP9_SYM_CHILD", "hash")
        .env("EP9_HASH_SEED", seed)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    let i = text.find("SYMHASH ").expect("the child printed its hashes");
    text[i..].lines().next().unwrap().to_owned()
}

#[test]
fn two_isolates_with_one_seed_give_the_same_symbol_hashes() {
    // One process (one seed): two isolates, the same hashes in the same order.
    let a = hashes_of_a_new_isolate();
    let b = hashes_of_a_new_isolate();
    assert_eq!(a, b);
    let mut d = a.clone();
    d.sort_unstable();
    d.dedup();
    assert_eq!(d.len(), a.len(), "distinct hashes: {a:?}");
    // Two processes with one EP9_HASH_SEED agree; another seed gives other hashes.
    #[cfg(not(miri))]
    {
        let one = child_symbol_hashes("1");
        assert_eq!(one, child_symbol_hashes("1"), "EP9_HASH_SEED=1 twice");
        assert_ne!(one, child_symbol_hashes("2"), "another seed, other hashes");
    }
}

// ---- symbols as property keys -----------------------------------------------------------------

#[test]
fn a_symbol_works_as_a_key() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, x, y, o, o2, t) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let has = s.b.feedback_slot(FeedbackSlotKind::HasKeyed);
        let def = s.b.feedback_slot(FeedbackSlotKind::DefineKeyedOwn);
        let (keyed, set_keyed) = (s.keyed, s.set_keyed);
        call_with_str(&mut s, "sym", "k");
        s.b.star(x);
        call_with_str(&mut s, "sym", "k");
        s.b.star(y);
        s.b.create_empty_object_literal().star(o);
        // o[x] = 1; o["Symbol(k)"] = 2
        s.b.lda_smi(1).set_keyed_property(o, x, set_keyed);
        s.lda_str("Symbol(k)");
        s.b.star(t).lda_smi(2).set_keyed_property(o, t, set_keyed);
        // o[x], o[y], o["Symbol(k)"]
        s.b.ldar(x).get_keyed_property(o, keyed);
        s.print_acc();
        s.b.ldar(y).get_keyed_property(o, keyed);
        s.print_acc();
        s.b.ldar(t).get_keyed_property(o, keyed);
        s.print_acc();
        // x in o, y in o
        s.b.ldar(o).test_in(x, has);
        s.print_acc();
        s.b.ldar(o).test_in(y, has);
        s.print_acc();
        // o[x] = 3 replaces; then delete o[x]; x in o
        s.b.lda_smi(3).set_keyed_property(o, x, set_keyed);
        s.b.ldar(x).get_keyed_property(o, keyed);
        s.print_acc();
        s.b.ldar(x).delete_property_sloppy(o);
        s.print_acc();
        s.b.ldar(o).test_in(x, has);
        s.print_acc();
        // A computed key in a literal: o2 = {[y]: 4}; o2[y]
        s.b.create_empty_object_literal().star(o2);
        s.b.lda_smi(4).define_keyed_own_property(o2, y, 0, def);
        s.b.ldar(y).get_keyed_property(o2, keyed);
        s.print_acc();
        // ToName of a symbol is the symbol: o2[ToName(y)]
        s.b.ldar(y).to_name().get_keyed_property(o2, keyed);
        s.print_acc();
        // null[x] and undefined[x] = 1: the key text in the message.
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_null().star(t);
            s.b.ldar(x).get_keyed_property(t, keyed);
        });
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_undefined().star(t);
            s.b.lda_smi(1).set_keyed_property(t, x, set_keyed);
        });
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let (r, out) = run_sym(make);
    assert_eq!(r.as_deref(), Ok("undefined"));
    assert_eq!(
        out,
        [
            "1",
            "undefined",
            "2",
            "true",
            "false",
            "3",
            "true",
            "false",
            "4",
            "4",
            "Cannot read properties of null (reading 'Symbol(k)')",
            "Cannot set properties of undefined (setting 'Symbol(k)')",
        ]
    );
}

#[test]
fn own_property_keys_put_symbols_last_in_creation_order() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt = cx.rt();
        let o = object::new_plain_object(rt);
        let s1 = symbol::new_symbol(rt, atoms::intern_str(rt, "s1"));
        let s2 = symbol::new_symbol(rt, UNDEFINED);
        for (k, v) in [
            (atoms::intern_str(rt, "b"), 1),
            (s1, 2),
            (atoms::intern_str(rt, "1"), 3),
            (atoms::intern_str(rt, "a"), 4),
            (s2, 5),
        ] {
            property::define(cx, o, k, conv::num(f64::from(v))).unwrap();
        }
        let keys = property::own_property_keys(cx, o);
        let texts: Vec<String> = (0..object::array_len(keys))
            .map(|i| key::key_text(rt, object::array_get(keys, i).unwrap()))
            .collect();
        assert_eq!(texts, ["1", "b", "a", "Symbol(s1)", "Symbol()"]);
        assert_eq!(object::array_get(keys, 3), Some(s1));
        assert_eq!(object::array_get(keys, 4), Some(s2));
        // [[Get]] and [[GetOwnProperty]] by symbol.
        assert_eq!(property::get(cx, o, s2).unwrap(), conv::num(5.0));
        let d = property::get_own_property(cx, o, s1).unwrap();
        assert_eq!(d.value, Some(conv::num(2.0)));
        let other = symbol::new_symbol(rt, atoms::intern_str(rt, "s1"));
        assert!(property::get_own_property(cx, o, other).is_none());
        assert!(property::has_property(cx, o, s1) && !property::has_property(cx, o, other));
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

#[test]
fn symbol_keys_in_shapes_compare_by_bits_and_hash_by_identity() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let roots: ScratchRoots<'_> = rt.heap.scratch_roots();
        // Twelve keys, symbols and strings mixed: up to 8 a lookup scans, above 8 it uses the
        // DESC_INDEX hash index (key::key_hash: a symbol's identity hash).
        let mut keys = Vec::new();
        for i in 0..12 {
            let k = if i % 3 == 1 {
                atoms::intern_str(rt, &format!("k{i}"))
            } else {
                // The same description for all: only identity tells them apart.
                symbol::new_symbol(rt, atoms::intern_str(rt, "same"))
            };
            keys.push(roots.push(val(k)));
        }
        let root = shape::root_shape_for(rt, shape::null_proto_info(rt), kinds::OBJECT, 4);
        let mut s = roots.push(val(root));
        let first = s;
        for &ki in &keys {
            let k = roots.get(ki).to_bits();
            let Added::Shape(c) = shape::add(rt, roots.get(s).to_bits(), k, attr::DEFAULT) else {
                panic!("unexpected dictionary mode");
            };
            shape::check(rt, c).unwrap();
            s = roots.push(val(c));
        }
        let stranger = symbol::new_symbol(rt, atoms::intern_str(rt, "same"));
        for n in [7, 8, 9, 12] {
            let sh = roots.get(first + n).to_bits();
            for (j, &ki) in keys.iter().enumerate() {
                let k = roots.get(ki).to_bits();
                let want = (j < n).then_some(j);
                assert_eq!(
                    shape::lookup(rt, sh, k).map(|f| f.index),
                    want,
                    "{n} keys, key {j}"
                );
            }
            assert_eq!(shape::lookup(rt, sh, stranger).map(|f| f.index), None);
        }
        drop(roots);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

// ---- @@toPrimitive ---------------------------------------------------------------------------

#[test]
fn at_at_to_primitive_gets_the_right_hint() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, tp, o, o2, t) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let (bin, cmp, keyed, set_keyed) = (s.bin, s.cmp, s.keyed, s.set_keyed);
        // tp = wks("toPrimitive"); o = {}; o[tp] = recordHint
        call_with_str(&mut s, "wks", "toPrimitive");
        s.b.star(tp);
        s.b.create_empty_object_literal().star(o);
        s.lda_global("recordHint");
        s.b.set_keyed_property(o, tp, set_keyed);
        // +o (number), `${o}` (string), o + 1 (default), o == 7 (default), o < 8 (number)
        s.b.ldar(o).to_number(bin);
        s.print_acc();
        s.b.ldar(o).to_string();
        s.print_acc();
        s.b.lda_smi(1).add(o, bin);
        s.print_acc();
        s.b.lda_smi(7).test_equal(o, cmp);
        s.print_acc();
        s.b.lda_smi(8).test_less_than(o, cmp);
        s.print_acc();
        // o2[o] = 1 (ToPropertyKey: string), then o2[7]
        s.b.create_empty_object_literal().star(o2);
        s.b.lda_smi(1).set_keyed_property(o2, o, set_keyed);
        s.b.lda_smi(7).get_keyed_property(o2, keyed);
        s.print_acc();
        // A non-callable @@toPrimitive throws; so does one that returns an object.
        try_print_message(&mut s, ctx, |s| {
            s.b.create_empty_object_literal().star(t);
            s.b.lda_smi(1).set_keyed_property(t, tp, set_keyed);
            s.b.ldar(t).to_number(bin);
        });
        try_print_message(&mut s, ctx, |s| {
            s.b.create_empty_object_literal().star(t);
            s.lda_global("returnsThis");
            s.b.set_keyed_property(t, tp, set_keyed);
            s.b.ldar(t).to_string();
        });
        // undefined and null mean "none": OrdinaryToPrimitive.
        s.b.create_empty_object_literal().star(t);
        s.b.lda_undefined().set_keyed_property(t, tp, set_keyed);
        s.b.ldar(t).to_string();
        s.print_acc();
        s.b.create_empty_object_literal().star(t);
        s.b.lda_null().set_keyed_property(t, tp, set_keyed);
        s.b.ldar(t).to_string();
        s.print_acc();
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let (r, out) = run_sym(make);
    assert_eq!(r.as_deref(), Ok("undefined"));
    assert_eq!(
        out,
        [
            "number",
            "7",
            "string",
            "7",
            "default",
            "8",
            "default",
            "true",
            "number",
            "true",
            "string",
            "1",
            "1 is not a function",
            "Cannot convert object to primitive value",
            "[object Object]",
            "[object Object]",
        ]
    );
}

#[test]
fn to_primitive_passes_each_hint_from_rust() {
    on_big_stack(|| {
        let mut iso = symbol_isolate(Dispatch::DEFAULT, true);
        take_output();
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt = cx.rt();
        let o = object::new_plain_object(rt);
        let tp = symbol::well_known(rt, WellKnownSymbol::ToPrimitive);
        let f = crate::realm::new_host_function(rt, record_hint, "[Symbol.toPrimitive]", 1, false);
        object::own_define(rt, o, tp, f, attr::HIDDEN);
        for hint in [Hint::Default, Hint::Number, Hint::String] {
            assert_eq!(conv::to_primitive(cx, o, hint).unwrap(), conv::num(7.0));
        }
        // A primitive is returned as it is, without a lookup.
        assert_eq!(conv::to_primitive(cx, NULL, Hint::Number).unwrap(), NULL);
        assert_eq!(take_output(), ["default", "number", "string"]);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}
