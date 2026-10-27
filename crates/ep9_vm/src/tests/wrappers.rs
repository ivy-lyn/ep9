//! Primitive wrapper tests: ToObject for each primitive (the interpreter's `ToObject` and
//! `conv::to_object`), the PRIMITIVE_WRAPPER cell through the fixed-field API, and the own
//! properties of String wrappers (`length` and the indices, §10.4.3).

use ep9_bytecode::{FeedbackSlotKind, HandlerKind, Label, Register, VerifiedUnit};

use super::{Asm, config, isolate, ok, on_big_stack, run_all, unit};
use crate::atoms::{self, Atom};
use crate::conv;
use crate::frame::{FALSE, NULL, TRUE, UNDEFINED};
use crate::interp::Cx;
use crate::key;
use crate::kinds;
use crate::object::{self, attr, cell};
use crate::property;
use crate::runtime::{Root, Runtime};
use crate::string;
use crate::symbol;
use crate::wrapper;
use crate::{Dispatch, Isolate};

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

#[test]
fn the_to_object_opcode_wraps_primitives() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let (ctx, v, w, o) = (
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
            s.b.new_register(),
        );
        let ts = s.b.feedback_slot(FeedbackSlotKind::TypeOf);
        let has = s.b.feedback_slot(FeedbackSlotKind::HasKeyed);
        let cmp = s.cmp;
        // For 1, 1.5, "ab", true: w = ToObject(v); typeof w; w === v
        let loads: [fn(&mut Asm); 4] = [
            |s| {
                s.b.lda_smi(1);
            },
            |s| {
                s.lda_num(1.5);
            },
            |s| {
                s.lda_str("ab");
            },
            |s| {
                s.b.lda_true();
            },
        ];
        for load in loads {
            load(&mut s);
            s.b.star(v).to_object(w);
            s.b.ldar(w).type_of(ts);
            s.print_acc();
            s.b.ldar(w).test_equal_strict(v, cmp);
            s.print_acc();
        }
        // The String wrapper of "ab" has "0", "1" and "length", not "2".
        s.lda_str("ab");
        s.b.to_object(w);
        for k in ["0", "1", "2", "length"] {
            s.lda_str(k);
            s.b.star(v).ldar(w).test_in(v, has);
            s.print_acc();
        }
        // An object is its own ToObject.
        s.b.create_empty_object_literal().star(o).to_object(w);
        s.b.ldar(w).test_equal_strict(o, cmp);
        s.print_acc();
        // undefined and null throw TypeError.
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_undefined().to_object(w);
        });
        try_print_message(&mut s, ctx, |s| {
            s.b.lda_null().to_object(w);
        });
        s.b.lda_undefined().return_();
        unit("", vec![s.finish()])
    }
    let r = run_all(make);
    ok(&r);
    assert_eq!(
        r.output,
        [
            "object",
            "false",
            "object",
            "false",
            "object",
            "false",
            "object",
            "false",
            "true",
            "true",
            "false",
            "true",
            "true",
            "Cannot convert undefined or null to object",
            "Cannot convert undefined or null to object",
        ]
    );
}

/// The isolate's interpreter context and runtime, for tests that call VM internals.
fn parts(iso: &mut Isolate) -> (&mut Cx, &Runtime) {
    let cx: &mut Cx = &mut iso.cx;
    let rt = cx.rt();
    (cx, rt)
}

#[test]
fn to_object_gives_each_primitive_its_prototype() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let (cx, rt) = parts(&mut iso);
        let sym = symbol::new_symbol(rt, atoms::intern_str(rt, "w"));
        let s = atoms::intern_str(rt, "ab");
        let cases = [
            (conv::num(1.0), Root::NumberPrototype),
            (conv::num(-0.0), Root::NumberPrototype),
            (TRUE, Root::BooleanPrototype),
            (FALSE, Root::BooleanPrototype),
            (s, Root::StringPrototype),
        ];
        for (v, root) in cases {
            let w = conv::to_object(cx, v).unwrap();
            assert_eq!(kinds::cell_kind(w), kinds::PRIMITIVE_WRAPPER.get());
            assert!(kinds::is_object(w) && kinds::is_wrapper(w));
            assert_eq!(wrapper::wrapper_value(w), Some(v));
            assert_eq!(object::proto_of(w), rt.root(root));
            assert_eq!(conv::type_of(w), Atom::Object);
            assert!(property::is_extensible(w));
        }
        let w = conv::to_object(cx, sym).unwrap();
        assert_eq!(wrapper::wrapper_value(w), Some(sym));
        assert_eq!(object::proto_of(w), symbol::prototype(rt));
        // Not wrappers: objects (returned as they are), and no ToObject for undefined/null.
        let o = object::new_plain_object(rt);
        assert_eq!(conv::to_object(cx, o).unwrap(), o);
        assert_eq!(wrapper::wrapper_value(o), None);
        for v in [UNDEFINED, NULL] {
            assert!(conv::to_object(cx, v).is_err());
            assert!(kinds::is_error(rt.take_pending()));
        }
        // A wrapper made with any prototype keeps it (realm init makes the prototypes so).
        let w = wrapper::new_wrapper(rt, NULL, conv::num(0.0));
        assert_eq!(object::proto_of(w), NULL);
        assert_eq!(wrapper::wrapper_value(w), Some(conv::num(0.0)));
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

#[test]
fn wrapper_length_and_indices_are_right() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let (cx, rt) = parts(&mut iso);
        let k = |t: &str| atoms::intern_str(rt, t);
        // "a€c": a UTF-16 string, so index 1 makes a new one-character string.
        let text = string::new_string(rt, "a\u{20ac}c");
        let w = conv::to_object(cx, text).unwrap();
        let len = wrapper::string_own_property(rt, w, k("length"));
        assert_eq!(
            len,
            Some((conv::num(3.0), 0)),
            "length: {{w: f, e: f, c: f}}"
        );
        for (i, want) in ["a", "\u{20ac}", "c"].into_iter().enumerate() {
            let (v, a) = wrapper::string_own_property(rt, w, k(&i.to_string())).unwrap();
            assert_eq!(string::to_rust_string(v), want);
            assert_eq!(a, attr::ENUMERABLE, "index: {{w: f, e: t, c: f}}");
        }
        // Not own properties of the wrapper: past the end, non-canonical numbers, other
        // names and symbols.
        let sym = symbol::new_symbol(rt, k("0"));
        for key in [
            k("3"),
            k("01"),
            k("-0"),
            k("1.0"),
            k("4294967295"),
            k("x"),
            sym,
        ] {
            assert_eq!(wrapper::string_own_property(rt, w, key), None);
            assert_eq!(wrapper::string_own_attrs(rt, w, key), None);
        }
        for (key, a) in [
            (k("0"), attr::ENUMERABLE),
            (k("2"), attr::ENUMERABLE),
            (k("length"), 0),
        ] {
            assert_eq!(wrapper::string_own_attrs(rt, w, key), Some(a));
        }
        // The ordinary internal methods see them (property.rs asks the wrapper first).
        let d = property::get_own_property(cx, w, k("length"))
            .unwrap()
            .unwrap();
        assert_eq!(d.value, Some(conv::num(3.0)));
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(false))
        );
        let d = property::get_own_property(cx, w, k("1")).unwrap().unwrap();
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(true), Some(false))
        );
        let keys = property::own_property_keys(cx, w);
        let texts: Vec<String> = (0..object::array_len(keys))
            .map(|i| key::key_text(rt, object::array_get(keys, i).unwrap()))
            .collect();
        assert_eq!(texts, ["0", "1", "2", "length"]);
        // Number, Boolean and Symbol wrappers have no string own properties.
        for v in [conv::num(12.0), TRUE, sym] {
            let o = conv::to_object(cx, v).unwrap();
            assert_eq!(wrapper::string_own_property(rt, o, k("length")), None);
            assert_eq!(wrapper::string_own_property(rt, o, k("0")), None);
        }
        // An empty String wrapper has length 0 and no index.
        let e = conv::to_object(cx, rt.atom(Atom::Empty)).unwrap();
        assert_eq!(
            wrapper::string_own_property(rt, e, k("length")),
            Some((conv::num(0.0), 0))
        );
        assert_eq!(wrapper::string_own_property(rt, e, k("0")), None);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// A String wrapper of a fresh (non-atom) string, made in its own frame. Returns the wrapper
/// only, so the string lives on only through the wrapper's fixed field.
#[inline(never)]
fn wrap_fresh(cx: &mut Cx, rt: &Runtime) -> u64 {
    let s = string::concat(
        rt,
        atoms::intern_str(rt, "kept "),
        atoms::intern_str(rt, "text"),
    )
    .unwrap();
    conv::to_object(cx, s).unwrap()
}

#[test]
fn a_wrapper_keeps_its_value_across_gc() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let (cx, rt) = parts(&mut iso);
        let roots = rt.heap.scratch_roots();
        let i = roots.push(crate::runtime::val(wrap_fresh(cx, rt)));
        for _ in 0..8 {
            rt.heap.collect();
        }
        let w = roots.get(i).to_bits();
        let v = wrapper::wrapper_value(w).unwrap();
        assert!(rt.heap.is_cell(cell(v).addr()));
        assert_eq!(string::to_rust_string(v), "kept text");
        drop(roots);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

#[test]
fn has_own_property_on_a_string_wrapper_allocates_nothing() {
    on_big_stack(|| {
        // GC stress 1: a collection before every allocation, so the count shows allocations.
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let (cx, rt) = parts(&mut iso);
        let k = |t: &str| atoms::intern_str(rt, t);
        // "a€c": a UTF-16 string, so reading index 1 would make a new one-character string.
        let text = string::new_string(rt, "a\u{20ac}c");
        let w = conv::to_object(cx, text).unwrap();
        let keys = [k("0"), k("1"), k("2"), k("3"), k("length"), k("x")];
        let before = rt.heap.stats().collections;
        let own = keys.map(|key| property::has_own_property(cx, w, key));
        let has = keys.map(|key| property::has_property(cx, w, key));
        assert_eq!(
            rt.heap.stats().collections,
            before,
            "an allocation happened"
        );
        assert_eq!(own, [true, true, true, false, true, false]);
        assert_eq!(has, own);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

#[test]
fn for_in_lists_a_string_wrapper_s_indices_first() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let (cx, rt) = parts(&mut iso);
        let k = |t: &str| atoms::intern_str(rt, t);
        let texts = |cx: &mut Cx, o: u64| -> Vec<String> {
            let keys = property::for_in_keys(cx, o).unwrap();
            (0..object::array_len(keys))
                .map(|i| key::key_text(rt, object::array_get(keys, i).unwrap()))
                .collect()
        };
        let w = conv::to_object(cx, k("ab")).unwrap();
        assert_eq!(texts(cx, w), ["0", "1"]);
        // Own keys added later: an index past the length, a name, a smaller index and a
        // symbol (skipped).
        let sym = symbol::new_symbol(rt, k("s"));
        for key in [k("5"), k("x"), k("2"), sym] {
            property::define(cx, w, key, conv::num(1.0)).unwrap();
        }
        assert_eq!(texts(cx, w), ["0", "1", "2", "5", "x"]);
        // Inherited keys come after the own ones; an enumerable `length` on
        // %Object.prototype% is hidden by the wrapper's own `length` (not enumerable).
        let op = rt.root(Root::ObjectPrototype);
        let sp = rt.root(Root::StringPrototype);
        property::define(cx, op, k("length"), conv::num(1.0)).unwrap();
        property::define(cx, op, k("y"), conv::num(1.0)).unwrap();
        property::define(cx, sp, k("z"), conv::num(1.0)).unwrap();
        property::define(cx, sp, sym, conv::num(1.0)).unwrap();
        assert_eq!(texts(cx, w), ["0", "1", "2", "5", "x", "z", "y"]);
        // An empty String wrapper has no index keys; a UTF-16 one has one per code unit.
        let e = conv::to_object(cx, rt.atom(Atom::Empty)).unwrap();
        assert_eq!(texts(cx, e), ["z", "y"]);
        let u = conv::to_object(cx, string::new_string(rt, "a\u{20ac}c")).unwrap();
        assert_eq!(texts(cx, u), ["0", "1", "2", "z", "y"]);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

#[test]
fn stores_to_a_string_wrapper_s_own_properties_change_nothing() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let (cx, rt) = parts(&mut iso);
        let k = |t: &str| atoms::intern_str(rt, t);
        let w = conv::to_object(cx, k("ab")).unwrap();
        let z = k("z");
        // [[Get]] reads `length` and the indices (exotic.rs asks the wrapper first).
        assert_eq!(property::get(cx, w, k("length")).unwrap(), conv::num(2.0));
        assert_eq!(property::get(cx, w, k("1")).unwrap(), k("b"));
        assert_eq!(property::get(cx, w, k("2")).unwrap(), UNDEFINED);
        // [[Set]] fails on them (§10.4.3: not writable): false, so sloppy code drops the
        // store and strict code throws; nothing goes into the property storage.
        for key in [k("length"), k("0"), k("1")] {
            assert!(!property::ordinary_set(cx, w, key, z).unwrap());
            property::set(cx, w, key, z, false).unwrap();
            assert!(property::set(cx, w, key, z, true).is_err());
            assert!(kinds::is_error(rt.take_pending()));
        }
        assert_eq!(object::own_count(w), 0);
        assert_eq!(property::get(cx, w, k("0")).unwrap(), k("a"));
        // An index at or past the length is an ordinary property.
        assert!(property::ordinary_set(cx, w, k("2"), z).unwrap());
        assert_eq!(object::own_count(w), 1);
        assert_eq!(property::get(cx, w, k("2")).unwrap(), z);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}
