//! Tests of keyed access (owner M4-P3 KEYED; OI-04): the keyed IC state and the
//! `SetKeyedProperty` fast case (`keyed_ic.rs`), with the cold-miss rule of `profile.rs`
//! (MC-w14-6); the integer-index paths of `property.rs` and `arguments.rs` that make no
//! string key; CreateListFromArrayLike's direct element reads (`abi/mod.rs`).
//!
//! The IC programs call `st(p, k, v) { p[k] = v; }` (function 1) and, between calls, the host
//! function `zap(st)`, which returns the state of `st`'s StoreKeyed slot as text
//! (`"<value half> <key>"`, key `U` undefined, `S` a shape, `H` the hole; `"none"` without a
//! vector) and clears the value half. A hit records nothing, so `zap` after a hit reads 0;
//! a cold execution records its bits again.

use ep9_bytecode::{FunctionId, VerifiedUnit};

use super::{Asm, config, isolate, meta, on_big_stack, take_output, unit};
use crate::arguments;
use crate::feedback::{self, fv};
use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::profile::keyed;
use crate::property;
use crate::runtime::val;
use crate::{Args, Cx as AbiCx, JsResult, JsValue, Local};
use crate::{Dispatch, IsolateConfig, kinds, object, string};

/// The StoreKeyed slot of `st` (function 1): `Asm`'s `set_keyed` slot.
fn st_slot() -> u32 {
    Asm::new(meta("st", 3, 0, 0, 0)).set_keyed.index() as u32
}

/// `zap(f)`: the state of `f`'s StoreKeyed slot as text, then its value half is set to 0.
fn zap<'s>(
    cx: &mut AbiCx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let f = args.get(0).bits();
    let fi = object::function_info(f);
    let text = match feedback::vector(fi) {
        None => "none".to_string(),
        Some(v) => {
            let s = st_slot();
            let key = match feedback::key(v, s) {
                UNDEFINED => "U",
                HOLE => "H",
                k if kinds::is_cell_bits(k) => "S",
                _ => "?",
            };
            let value = feedback::value(v, s);
            assert!(val(value).is_int32(), "a keyed value half is an int32");
            // An int32 is not a cell: no barrier (E1).
            let off = fv::SLOTS + fv::SLOT_SIZE * s as usize + fv::VALUE;
            object::init(object::cell(v), off, int32_bits(0));
            format!("{} {key}", value as u32)
        }
    };
    Ok(cx.new_string(&text)?.value())
}

/// `st(p, k, v) { p[k] = v; }` (sloppy; function 1).
fn st_function() -> ep9_bytecode::FunctionCode {
    let mut f = Asm::new(meta("st", 3, 0, 0, 0));
    let (p, k, v) = (f.b.parameter(0), f.b.parameter(1), f.b.parameter(2));
    let s = f.set_keyed;
    f.b.ldar(v)
        .set_keyed_property(p, k, s)
        .lda_undefined()
        .return_();
    f.finish()
}

/// A script with `st` in register `st` and the steps `body` emits; `zap` and `print` are
/// globals. Helpers below emit `st(p, k, v)` and `print(zap(st))`.
struct Prog {
    s: Asm,
    st: ep9_bytecode::Register,
    args: ep9_bytecode::RegisterList,
    key: ep9_bytecode::Register,
}

impl Prog {
    fn new() -> Prog {
        let mut s = Asm::script(0);
        let st = s.b.new_register();
        let args = s.b.new_register_list(3);
        let key = s.b.new_register();
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0).star(st);
        Prog { s, st, args, key }
    }

    /// `st(p, k, v)` with `k` loaded by `load_key`, `v` an int32.
    fn st(&mut self, p: ep9_bytecode::Register, load_key: impl FnOnce(&mut Asm), v: i32) {
        let a = self.args;
        self.s.b.ldar(p).star(a.get(0));
        load_key(&mut self.s);
        self.s.b.star(a.get(1)).lda_smi(v).star(a.get(2));
        let st = self.st;
        self.s.b.call_undefined_receiver(st, a, 3, self.s.call);
    }

    /// `st(p, i, v)` for an int32 `i`.
    fn st_i(&mut self, p: ep9_bytecode::Register, i: i32, v: i32) {
        self.st(
            p,
            |s| {
                s.b.lda_smi(i);
            },
            v,
        );
    }

    /// `print(zap(st))`.
    fn zap(&mut self) {
        let (f, a0) = (self.s.f, self.args.get(0));
        self.s.b.ldar(self.st).star(a0);
        self.s.lda_global("zap");
        self.s
            .b
            .star(f)
            .call_undefined_receiver1(f, a0, self.s.call);
        self.s.print_acc();
    }

    /// A new array `[v0, v1, ..]` (an empty literal, then keyed stores at the script's own
    /// StoreKeyed slot) in a new register.
    fn array(&mut self, vals: &[i32]) -> ep9_bytecode::Register {
        let a = self.s.b.new_register();
        self.s.b.create_empty_array_literal(self.s.lit).star(a);
        for (i, &v) in vals.iter().enumerate() {
            self.s.b.lda_smi(i as i32).star(self.key).lda_smi(v);
            self.s.b.set_keyed_property(a, self.key, self.s.set_keyed);
        }
        a
    }

    /// `print(p[i])` for each `i`.
    fn print_elements(&mut self, p: ep9_bytecode::Register, is: &[i32]) {
        for &i in is {
            self.s.b.lda_smi(i).get_keyed_property(p, self.s.keyed);
            self.s.print_acc();
        }
    }

    fn finish(mut self) -> VerifiedUnit {
        self.s.b.lda_undefined().return_();
        unit("", vec![self.s.finish(), st_function()])
    }
}

/// Runs `make` under every dispatch expansion, with and without GC stress, with the feedback
/// threshold `threshold`, `print` and `zap`; all runs must print the same lines. Returns them.
fn run_ic(threshold: u32, make: fn() -> VerifiedUnit) -> Vec<String> {
    let mut first: Option<(String, Vec<String>)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let out = on_big_stack(move || {
                let mut iso = isolate(IsolateConfig {
                    feedback_threshold: threshold,
                    ..config(d, stress)
                });
                iso.register_host_function("zap", 1, zap);
                take_output();
                let r = iso.with_cx_checkpoint(|cx| cx.run_script(make(), "keyed.js").map(|_| ()));
                assert!(r.is_ok(), "{r:?}");
                let out = take_output();
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                out
            });
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, out)),
                Some((l0, o0)) => assert_eq!(&out, o0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// The text `zap` gives for the value bits `bits` and the key `key`.
fn z(bits: u32, key: &str) -> String {
    format!("{bits} {key}")
}

/// MC-w14-6, the cold-miss rule: the fast case (a present element of an Array, an int32
/// index) hits only after a cold execution of that case at its site recorded `INDEX |
/// ARRAY`; every other case (a name key, out of bounds, a hole, an object receiver) goes cold
/// and records its bits each time, and leaves the key half alone. A second receiver shape
/// makes the site megamorphic (a cold execution), and then every fast case hits.
#[test]
fn a_fast_store_hits_only_after_its_case_ran_cold() {
    fn make() -> VerifiedUnit {
        let mut p = Prog::new();
        p.zap(); // before the first call: no vector
        let a = p.array(&[1, 2, 3]);
        // A name key on the array: cold, no state change.
        p.st(
            a,
            |s| {
                s.lda_str("x");
            },
            5,
        );
        p.zap();
        // The first fast case: cold, the state records the shape.
        p.st_i(a, 1, 7);
        p.zap();
        // Hits: nothing recorded.
        p.st_i(a, 1, 8);
        p.st_i(a, 2, 9);
        p.zap();
        // Out of bounds (a[5]), then a hole (a[4]): cold each time.
        p.st_i(a, 5, 1);
        p.zap();
        p.st_i(a, 4, 1);
        p.zap();
        // An index string: cold (the fast case needs an int32 key).
        p.st(
            a,
            |s| {
                s.lda_str("0");
            },
            4,
        );
        p.zap();
        // A second array shape: cold once (megamorphic), then a hit for both shapes.
        let b = p.array(&[0]);
        p.s.b.lda_smi(1);
        p.s.set_named(b, "p");
        p.st_i(b, 0, 3);
        p.zap();
        p.st_i(b, 0, 4);
        p.st_i(a, 0, 6);
        p.zap();
        // An object receiver: cold, the key half stays.
        let o = p.s.b.new_register();
        p.s.b.create_empty_object_literal().star(o);
        p.st_i(o, 0, 1);
        p.zap();
        p.print_elements(a, &[0, 1, 2, 3, 4, 5]);
        p.print_elements(b, &[0]);
        p.print_elements(o, &[0]);
        p.finish()
    }
    let ia = keyed::INDEX | keyed::ARRAY;
    let out = run_ic(0, make);
    assert_eq!(
        out,
        [
            "none".to_string(),
            z(keyed::NAME | keyed::ARRAY, "U"),
            z(ia, "S"),
            z(0, "S"),
            z(ia | keyed::OUT_OF_BOUNDS, "S"),
            z(ia | keyed::HOLE, "S"),
            z(ia, "S"),
            z(ia, "H"),
            z(0, "H"),
            z(keyed::INDEX | keyed::OBJECT, "H"),
            "6".into(),
            "8".into(),
            "9".into(),
            "undefined".into(),
            "1".into(),
            "1".into(),
            "4".into(),
            "1".into(),
        ]
    );
}

/// A function without a feedback vector stores a present element in place (the record hooks
/// record nothing there either), and its results equal the run with a vector.
#[test]
fn a_function_without_a_vector_stores_in_place() {
    fn make() -> VerifiedUnit {
        let mut p = Prog::new();
        let a = p.array(&[1, 2, 3]);
        for i in 0..3 {
            p.st_i(a, i, 10 + i);
            p.st_i(a, i, 20 + i);
        }
        p.st_i(a, 4, 7);
        p.zap();
        p.print_elements(a, &[0, 1, 2, 3, 4]);
        p.finish()
    }
    let without = run_ic(u32::MAX, make);
    assert_eq!(
        without,
        ["none", "20", "21", "22", "undefined", "7"].map(String::from)
    );
    let with = run_ic(0, make);
    assert_eq!(with[1..], without[1..]);
    assert_eq!(
        with[0],
        z(keyed::INDEX | keyed::ARRAY | keyed::OUT_OF_BOUNDS, "S")
    );
}

/// A hit stores a cell (a new object each time) into an old array with the write barrier:
/// under GC stress (a collection before every allocation, then the heap verifier) every
/// stored object stays reachable and readable.
#[test]
fn a_hit_stores_cells_with_the_barrier() {
    fn make() -> VerifiedUnit {
        let mut p = Prog::new();
        let a = p.array(&[0, 0]);
        let i = p.s.b.new_register();
        let lit = p.s.b.new_register();
        p.s.b.lda_zero().star(i);
        let head = p.s.b.bound_label();
        let exit = p.s.b.new_label();
        p.s.b
            .lda_smi(40)
            .test_less_than(i, p.s.cmp)
            .jump_if_false(exit);
        // st(a, i & 1, {n: i})
        p.s.b.create_empty_object_literal().star(lit);
        p.s.b.ldar(i);
        p.s.set_named(lit, "n");
        let args = p.args;
        p.s.b.ldar(a).star(args.get(0));
        p.s.b.ldar(i).bitwise_and_smi(1, p.s.bin).star(args.get(1));
        p.s.b.ldar(lit).star(args.get(2));
        let st = p.st;
        p.s.b.call_undefined_receiver(st, args, 3, p.s.call);
        p.s.b
            .ldar(i)
            .inc(p.s.bin)
            .star(i)
            .jump_loop(head, 0, p.s.loop_);
        p.s.b.bind(exit);
        p.zap();
        for k in 0..2 {
            p.s.b.lda_smi(k).get_keyed_property(a, p.s.keyed).star(lit);
            p.s.get_named(lit, "n");
            p.s.print_acc();
        }
        p.finish()
    }
    let out = run_ic(0, make);
    assert_eq!(
        out,
        [
            z(keyed::INDEX | keyed::ARRAY, "S"),
            "38".into(),
            "39".into()
        ]
    );
}

/// The key half is weak (plan rule 8, H16 K7): a shape that only the key half holds dies at a
/// GC and the key half reads undefined. The run that makes the shape happens below a pad of
/// zeros and the stack is scrubbed before the collection, so the conservative scan cannot
/// keep it (the `tests::ic` dead-cell rule, W4-P3FIX-DEADSHAPE-TEST).
#[test]
fn the_key_half_holds_its_shape_weakly() {
    fn make() -> VerifiedUnit {
        let mut p = Prog::new();
        // b: an array with its own shape (an extra named property), dropped after the store.
        let b = p.array(&[0]);
        p.s.b.lda_smi(1);
        p.s.set_named(b, "keyedOnlyHere");
        p.st_i(b, 0, 1);
        p.st_i(b, 0, 2);
        p.s.b.lda_undefined().star(b);
        for i in 0..3 {
            p.s.b.star(p.args.get(i));
        }
        p.finish()
    }
    /// Runs `make` and returns the masked key half of `st`'s StoreKeyed slot.
    #[inline(never)]
    fn made(iso: &mut crate::Isolate) -> usize {
        let r = iso.with_cx_checkpoint(|cx| cx.run_script(make(), "keyed.js").map(|_| ()));
        assert!(r.is_ok(), "{r:?}");
        let k = st_key(iso);
        assert!(kinds::is_cell_bits(k), "the shape after the store");
        core::hint::black_box(k as usize) ^ super::ic::MASK
    }
    on_big_stack(|| {
        let mut iso = isolate(IsolateConfig {
            feedback_threshold: 0,
            ..config(Dispatch::DEFAULT, false)
        });
        let (dead, pad_low) = super::ic::below_a_pad(|| made(&mut iso));
        scrub_stack();
        iso.collect_garbage();
        let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
        let scan_low = high - 8 * iso.rt.heap.stats().last_stack_words;
        assert!(scan_low > pad_low, "the scan read below the pad");
        let addr = dead ^ super::ic::MASK;
        assert!(
            !iso.rt
                .heap
                .last_conservative_roots()
                .iter()
                .any(|c| c.addr() == addr),
            "the conservative scan kept the shape"
        );
        iso.verify_heap().unwrap();
        assert_eq!(st_key(&iso), UNDEFINED, "the dead shape is cleared");
    });
}

/// The key half of `st`'s StoreKeyed slot (function 1 of the last unit).
fn st_key(iso: &crate::Isolate) -> u64 {
    let units = iso.rt.units.borrow();
    let fi = &units.last().unwrap().infos[1];
    let v = feedback::vector(fi).expect("st has a vector");
    feedback::key(v, st_slot())
}

/// Overwrites 256 KiB of stack below the caller (stale words of returned frames).
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    core::hint::black_box(&mut words);
}

// ---- integer-index access without a string key (property.rs, arguments.rs) --------------------

/// Runs `make`, then `check` with the interpreter `Cx`, the global `A` (held by a global
/// handle) and `print`, under GC stress `stress`; checks the heap after.
fn with_global(make: fn() -> VerifiedUnit, stress: bool, check: fn(&mut Cx, u64, u64)) {
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        iso.with_cx_checkpoint(|cx| cx.run_script(make(), "test.js").map(|_| ()))
            .unwrap();
        let a = iso.global_raw("A").expect("A").to_bits();
        let print = iso.global_raw("print").expect("print").to_bits();
        let h = iso.rt.heap.new_global(val(a));
        check(&mut iso.cx, a, print);
        iso.rt.heap.drop_global(h);
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// The text of `v` (no JS): a string's characters, else its description.
fn text(v: u64) -> String {
    if kinds::is_string(v) {
        string::to_rust_string(v)
    } else {
        crate::conv::describe(v)
    }
}

/// `A = (function (a, b) { return arguments; })(1, 2, 3)` (sloppy, `a` and `b` mapped).
fn mapped_arguments() -> VerifiedUnit {
    super::arguments::mapped_a()
}

/// An int32 index of an arguments object reads and writes its elements (a mapped one
/// through its parameter) and answers `in`, as the string key does; an index that is not an
/// element takes the key path (ordinary storage, then the chain).
#[test]
fn int_index_access_on_arguments_objects() {
    for stress in [false, true] {
        with_global(mapped_arguments, stress, |cx, a, _| {
            let i = int32_bits;
            let key = |cx: &Cx, n: u32| crate::key::index_key(cx.rt(), n);
            let get = |cx: &mut Cx, n: i32| text(property::get_keyed(cx, a, i(n)).unwrap());
            let by_key = |cx: &mut Cx, n: u32| {
                let k = key(cx, n);
                text(property::get(cx, a, k).unwrap())
            };
            assert_eq!(
                (0..4).map(|n| get(cx, n)).collect::<Vec<_>>(),
                ["1", "2", "3", "undefined"]
            );
            // A mapped index reads its parameter.
            super::arguments::set_param(cx, a, 0, i(10));
            assert_eq!((get(cx, 0), by_key(cx, 0)), ("10".into(), "10".into()));
            // A write to a mapped index writes the parameter too; index 2 is not mapped.
            property::set_keyed(cx, a, i(1), i(20), false).unwrap();
            assert_eq!(text(super::arguments::param(a, 1)), "20");
            property::set_keyed(cx, a, i(2), i(30), true).unwrap();
            assert_eq!((get(cx, 1), get(cx, 2)), ("20".into(), "30".into()));
            assert_eq!(arguments::element_value(a, 2), Some(i(30)));
            // `in`.
            let has = |cx: &mut Cx, n: i32| property::has(cx, a, i(n)).unwrap();
            assert_eq!(
                (0..4).map(|n| has(cx, n)).collect::<Vec<_>>(),
                [true, true, true, false]
            );
            // A new index past the elements is an ordinary property (the key path).
            property::set_keyed(cx, a, i(5), i(50), true).unwrap();
            assert_eq!((get(cx, 5), by_key(cx, 5)), ("50".into(), "50".into()));
            assert!(has(cx, 5));
            assert_eq!(arguments::element_value(a, 5), None);
            // A deleted element: the index is gone (no prototype has it).
            let k0 = key(cx, 0);
            assert!(property::delete(cx, a, k0, true).unwrap());
            assert_eq!((get(cx, 0), has(cx, 0)), ("undefined".into(), false));
            assert!(!arguments::set_element_value(cx.rt(), a, 0, i(1)));
            // Set again: an ordinary property now, not mapped.
            property::set_keyed(cx, a, i(0), i(7), true).unwrap();
            assert_eq!(get(cx, 0), "7");
            super::arguments::set_param(cx, a, 0, i(99));
            assert_eq!(get(cx, 0), "7", "unmapped after the delete");
        });
    }
}

/// `A = w.call("abc")` for a sloppy `w() { return this; }`: a String wrapper.
fn string_wrapper() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    let (w, recv) = (s.b.new_register(), s.b.new_register());
    s.b.create_closure(k, s.closure, 0).star(w);
    s.lda_str("abc");
    s.b.star(recv).call_property0(w, recv, s.call);
    s.sta_global("A");
    s.b.lda_undefined().return_();
    let mut f = Asm::new(meta("w", 0, 0, 0, 0));
    let this = f.b.receiver();
    f.b.ldar(this).return_();
    unit("", vec![s.finish(), f.finish()])
}

/// An int32 index of a String wrapper reads its character and answers `in` without a key; a
/// store to a character is refused (strict: TypeError), and an index past the string is an
/// ordinary property.
#[test]
fn int_index_access_on_string_wrappers() {
    for stress in [false, true] {
        with_global(string_wrapper, stress, |cx, a, _| {
            assert_eq!(kinds::cell_kind(a), kinds::PRIMITIVE_WRAPPER.get());
            let i = int32_bits;
            let get = |cx: &mut Cx, n: i32| text(property::get_keyed(cx, a, i(n)).unwrap());
            assert_eq!(
                (0..4).map(|n| get(cx, n)).collect::<Vec<_>>(),
                ["\"a\"", "\"b\"", "\"c\"", "undefined"].map(|t| t.trim_matches('"').to_string())
            );
            let has = |cx: &mut Cx, n: i32| property::has(cx, a, i(n)).unwrap();
            assert_eq!(
                (0..4).map(|n| has(cx, n)).collect::<Vec<_>>(),
                [true, true, true, false]
            );
            property::set_keyed(cx, a, i(0), i(1), false).unwrap();
            assert_eq!(get(cx, 0), "a");
            assert!(property::set_keyed(cx, a, i(0), i(1), true).is_err());
            cx.rt().take_pending();
            property::set_keyed(cx, a, i(3), i(4), true).unwrap();
            assert_eq!((get(cx, 3), has(cx, 3)), ("4".into(), true));
        });
    }
}

/// `A = [0, , 2]` (a hole at 1, from a store at 2 on `[0]`... built by keyed stores).
fn holey_array() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (a, key) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_array_literal(s.lit).star(a);
    for (i, v) in [(0, 0), (2, 2)] {
        s.b.lda_smi(i).star(key).lda_smi(v);
        s.b.set_keyed_property(a, key, s.set_keyed);
    }
    s.b.ldar(a);
    s.sta_global("A");
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// A hole of an Array reads through the chain without a key: undefined while no prototype
/// has an index property, the prototype's element once %Array.prototype% has one (the
/// `proto_index` flag); `in` answers the same; CreateListFromArrayLike reads the same values
/// as `[[Get]]`.
#[test]
fn holes_read_through_the_chain_without_a_key() {
    for stress in [false, true] {
        with_global(holey_array, stress, |cx, a, print| {
            let i = int32_bits;
            let get = |cx: &mut Cx, n: i32| text(property::get_keyed(cx, a, i(n)).unwrap());
            let has = |cx: &mut Cx, n: i32| property::has(cx, a, i(n)).unwrap();
            assert_eq!((get(cx, 1), has(cx, 1)), ("undefined".into(), false));
            let list = |cx: &mut Cx, a: u64| {
                let mut acx = AbiCx::new(cx);
                let l = acx
                    .create_list_from_array_like(Local::<JsValue>::from_bits(a))
                    .unwrap();
                assert_eq!(l.len(), 3);
                take_output();
                acx.call_built(Local::<JsValue>::from_bits(print), Local::undefined(), l)
                    .unwrap();
                take_output()
            };
            assert_eq!(list(cx, a), ["0 undefined 2"]);
            // %Array.prototype%[1] = "p": now the hole reads it.
            let proto = object::proto_of(a);
            let p = string::new_string(cx.rt(), "p");
            property::set_keyed(cx, proto, i(1), p, true).unwrap();
            assert_eq!((get(cx, 1), has(cx, 1)), ("p".into(), true));
            assert_eq!(list(cx, a), ["0 p 2"]);
            // Out of bounds: no element anywhere.
            assert_eq!((get(cx, 3), has(cx, 3)), ("undefined".into(), false));
        });
    }
}

/// CreateListFromArrayLike on an arguments object reads each element (a mapped one through
/// its parameter), honours a changed `length`, and reads a deleted element through the chain
/// (`[[Get]]` order and results).
#[test]
fn create_list_from_array_like_reads_arguments_elements() {
    for stress in [false, true] {
        with_global(mapped_arguments, stress, |cx, a, print| {
            let list = |cx: &mut Cx| {
                let mut acx = AbiCx::new(cx);
                let l = acx
                    .create_list_from_array_like(Local::<JsValue>::from_bits(a))
                    .unwrap();
                take_output();
                acx.call_built(Local::<JsValue>::from_bits(print), Local::undefined(), l)
                    .unwrap();
                take_output()
            };
            assert_eq!(list(cx), ["1 2 3"]);
            super::arguments::set_param(cx, a, 1, int32_bits(20));
            assert_eq!(list(cx), ["1 20 3"]);
            let k0 = crate::key::index_key(cx.rt(), 0);
            assert!(property::delete(cx, a, k0, true).unwrap());
            assert_eq!(list(cx), ["undefined 20 3"]);
            // length = 4: the list reads index 3 (missing: undefined).
            let length = cx.rt().atom(crate::runtime::Atom::Length);
            property::set(cx, a, length, int32_bits(4), true).unwrap();
            assert_eq!(list(cx), ["undefined 20 3 undefined"]);
            property::set(cx, a, length, int32_bits(1), true).unwrap();
            assert_eq!(list(cx), ["undefined"]);
        });
    }
}
