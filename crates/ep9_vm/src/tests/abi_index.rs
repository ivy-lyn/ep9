//! Index access and the iterator protocol on the builtin ABI (H20 ABI-1, ABI-3): the index
//! ObjectOps make no atom for a dense element or a missing index and agree with the key path;
//! `get_iterator`, `iterator_step_value` and `iterator_close` follow §7.4.

use super::{config, isolate};
use crate::object;
use crate::runtime::Runtime;
use crate::{Args, Cx, Dispatch, HostFn, Isolate, JsObject, JsResult, Local, PropertyDescriptor};

/// Runs `f` with a builtin-ABI context of a fresh isolate. `stress`: a collection before
/// every allocation, then the heap verifier.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    let out = {
        let iso: &mut Isolate = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

/// The number of atoms in the isolate's table.
fn atom_count(rt: &Runtime) -> usize {
    rt.atoms.borrow().len()
}

/// Takes the pending exception: "Name: message".
fn take_error(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    )
}

/// A host function object `name` (not on the global).
fn host_fn<'s>(rt: &Runtime, f: HostFn, name: &str) -> Local<'s> {
    Local::from_bits(crate::realm::new_host_function(rt, f, name, 0, false))
}

/// A new array of `len` elements `0, 1, ..`, then holes up to `holes_to` (its length).
fn number_array<'s>(rt: &Runtime, len: u32, holes_to: u32) -> Local<'s, JsObject> {
    let a = object::new_array(rt, 0);
    for i in 0..len {
        object::array_set(rt, a, i, crate::conv::num(f64::from(i)));
    }
    object::array_set_length(rt, a, holes_to);
    Local::from_bits(a)
}

/// `! ToString(𝔽(i))` as Rust text (Rust prints an integral f64 below 10^21 as its digits).
fn key_text(i: u64) -> String {
    (i as f64).to_string()
}

// ---- index access (ABI-1) -------------------------------------------------------------------

/// The acceptance check: 10^4 `get_index` calls on dense elements leave the atom table as it
/// was, and so do holes, missing indices and the scan of a small dictionary object.
#[test]
fn get_index_on_dense_elements_makes_no_atom() {
    with_cx(false, |cx, rt| {
        const N: u32 = 10_000;
        let a = number_array(rt, N, 2 * N);
        let al = cx.new_object();
        cx.set(al, "length", Local::int32(5), true).unwrap();
        cx.set(al, "2", Local::int32(7), true).unwrap();
        let before = atom_count(rt);
        let mut sum = 0.0;
        for i in 0..u64::from(N) {
            sum += cx.get_index(a, i).unwrap().as_number().unwrap();
        }
        assert_eq!(sum, f64::from(N) * f64::from(N - 1) / 2.0);
        for i in u64::from(N)..u64::from(2 * N) {
            assert!(!cx.has_index(a, i).unwrap());
            assert!(cx.get_index(a, i).unwrap().is_undefined());
        }
        // An array-like in dictionary mode (an index key): its keys are scanned.
        for i in 0..u64::from(N) {
            let present = i == 2;
            assert_eq!(cx.has_index(al, i).unwrap(), present, "{i}");
            assert_eq!(
                cx.get_index(al, i).unwrap().as_int32(),
                present.then_some(7)
            );
        }
        // Past the array-index range: the digits are compared, not interned.
        assert!(!cx.has_index(a, 4_294_967_296).unwrap());
        assert!(!cx.has_index(al, 9_007_199_254_740_990).unwrap());
        assert_eq!(atom_count(rt), before, "index access made atoms");
        // The key path does make them (the check above can fail).
        let k = cx.key(&key_text(u64::from(2 * N) + 1));
        assert!(!cx.has_property(a, k).unwrap());
        assert_eq!(atom_count(rt), before + 1);
    });
}

/// `has_index` and `get_index` give what `has_property` and `get` give with the key, on every
/// path: elements and holes, a String wrapper, a dictionary object (stored index keys, an
/// accessor, a non-canonical key), a fast object with an integer key past the array-index
/// range, an object with more keys than the scan reads, a prototype with index keys, and
/// indices up to and past 2^53 - 1.
#[test]
fn index_ops_agree_with_the_key_path() {
    with_cx(true, |cx, rt| {
        let indices: [u64; 14] = [
            0,
            1,
            2,
            3,
            5,
            70,
            4_294_967_294,
            4_294_967_295,
            4_294_967_296,
            9_007_199_254_740_990,
            9_007_199_254_740_991,
            9_007_199_254_740_992,
            9_007_199_254_740_993,
            u64::MAX,
        ];
        let objects: Local<'_, JsObject> = cx.new_array().into();
        let mut n = 0;
        // An array with holes, and one past the array-index range.
        let a = number_array(rt, 3, 6);
        cx.delete_index(a, 1, true).unwrap();
        cx.set(a, "4294967295", Local::int32(-1), true).unwrap();
        push(cx, objects, &mut n, a);
        // A String wrapper.
        let s = cx.new_string("wxyz").unwrap();
        if let Ok(w) = cx.to_object(s) {
            push(cx, objects, &mut n, w);
        } else {
            rt.take_pending();
        }
        // A dictionary object: index keys, an accessor at 3, a non-canonical "05".
        let d = cx.new_object();
        for (k, v) in [("0", 10), ("2", 12), ("05", 15), ("9007199254740990", 99)] {
            cx.set(d, k, Local::int32(v), true).unwrap();
        }
        let getter = host_fn(rt, return_this, "getter");
        let desc = PropertyDescriptor {
            get: Some(getter),
            enumerable: Some(true),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        };
        cx.define_property_or_throw(d, "3", desc).unwrap();
        push(cx, objects, &mut n, d);
        // A fast object with an integer key that is not an array index.
        let f = cx.new_object();
        cx.set(f, "a", Local::int32(1), true).unwrap();
        cx.set(f, "4294967296", Local::int32(2), true).unwrap();
        push(cx, objects, &mut n, f);
        // More keys than the scan reads: the key path.
        let big = cx.new_object();
        for i in 0..100u64 {
            cx.set_index(big, i * 3, Local::number(i as f64), true)
                .unwrap();
            let k = format!("k{i}");
            cx.set(big, k.as_str(), Local::int32(0), true).unwrap();
        }
        push(cx, objects, &mut n, big);
        // An object whose prototype has index keys (`proto_index_keys` on).
        let proto = cx.new_object();
        cx.set(proto, "1", Local::int32(21), true).unwrap();
        cx.set(proto, "4294967296", Local::int32(22), true).unwrap();
        let child = cx.new_object_with_proto(proto);
        cx.set(child, "0", Local::int32(20), true).unwrap();
        push(cx, objects, &mut n, child);
        let child_array = number_array(rt, 1, 4);
        cx.set_prototype_of(child_array, proto).unwrap();
        push(cx, objects, &mut n, child_array);

        assert_eq!(n, 7, "every object kind is in the list");
        for j in 0..n {
            let o = cx.get_index(objects, j).unwrap().as_object().unwrap();
            for &i in &indices {
                let text = key_text(i);
                let key = cx.key(&text);
                let has = cx.has_property(o, key).unwrap();
                assert_eq!(cx.has_index(o, i).unwrap(), has, "object {j}, index {i}");
                let want = cx.get(o, key).unwrap();
                let got = cx.get_index(o, i).unwrap();
                assert!(
                    cx.same_value(got, want),
                    "object {j}, index {i}: {} vs {}",
                    cx.to_rust_string(got).unwrap(),
                    cx.to_rust_string(want).unwrap()
                );
            }
        }
    });
}

/// Appends `o` to the array `objects` at index `*n`.
fn push<'s>(cx: &mut Cx<'s>, objects: Local<'s, JsObject>, n: &mut u64, o: Local<'s, JsObject>) {
    cx.set_index(objects, *n, o, true).unwrap();
    *n += 1;
}

/// `o[k] = text` (a new string).
fn set_text<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, k: &str, text: &str) {
    let s = cx.new_string(text).unwrap();
    cx.set(o, k, s, true).unwrap();
}

/// `getter`: returns its this value.
fn return_this<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

/// A getter on a prototype's index runs with the receiver as this; an index on a prototype
/// shows through an array's hole; a failed `set_index` is false or a TypeError.
#[test]
fn index_ops_walk_the_prototype_chain() {
    with_cx(false, |cx, rt| {
        let a = number_array(rt, 1, 3);
        let proto = cx.new_object();
        let getter = host_fn(rt, return_this, "getter");
        let desc = PropertyDescriptor {
            get: Some(getter),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        };
        cx.define_property_or_throw(proto, "1", desc).unwrap();
        cx.set(proto, "2", Local::int32(9), true).unwrap();
        cx.set_prototype_of(a, proto).unwrap();
        assert!(cx.has_index(a, 1).unwrap() && cx.has_index(a, 2).unwrap());
        assert!(cx.get_index(a, 1).unwrap().is_same_bits(&a));
        assert_eq!(cx.get_index(a, 2).unwrap().as_int32(), Some(9));
        // An inherited getter without a setter: the store fails.
        assert!(!cx.set_index(a, 1, Local::int32(5), false).unwrap());
        assert!(cx.set_index(a, 1, Local::int32(5), true).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot assign to read only property '1' of object"
        );
        // An inherited data property: the store adds an element to the receiver.
        assert!(cx.set_index(a, 2, Local::int32(6), true).unwrap());
        let stored = object::array_get(a.bits(), 2).map(crate::runtime::val);
        assert_eq!(stored.and_then(|v| v.as_int32()), Some(6));
        assert_eq!(cx.get_index(proto, 2).unwrap().as_int32(), Some(9));
    });
}

/// `set_index` appends to a dense array without a key; a non-extensible array refuses a new
/// element. `delete_index` makes a hole, gives true for a missing index without a key, and
/// false or a TypeError for a non-configurable property.
#[test]
fn set_and_delete_index() {
    with_cx(false, |cx, rt| {
        let a = number_array(rt, 0, 0);
        let before = atom_count(rt);
        for i in 0..1000u64 {
            assert!(cx.set_index(a, i, Local::number(i as f64), true).unwrap());
        }
        assert_eq!(object::array_len(a.bits()), 1000);
        assert!(cx.delete_index(a, 500, true).unwrap());
        assert!(!cx.has_index(a, 500).unwrap());
        assert!(cx.delete_index(a, 5000, true).unwrap());
        let plain = cx.new_object();
        assert!(cx.delete_index(plain, 123_456, true).unwrap());
        assert_eq!(
            atom_count(rt),
            before,
            "set_index or delete_index made atoms"
        );

        cx.prevent_extensions(a).unwrap();
        assert!(cx.set_index(a, 1, Local::int32(-1), true).unwrap());
        assert!(!cx.set_index(a, 2000, Local::int32(-1), false).unwrap());
        assert!(cx.set_index(a, 2000, Local::int32(-1), true).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot assign to read only property '2000' of object"
        );

        let o = cx.new_object();
        let desc = PropertyDescriptor::data(Local::int32(1), true, true, false);
        cx.define_property_or_throw(o, "3", desc).unwrap();
        assert!(!cx.delete_index(o, 3, false).unwrap());
        assert!(cx.delete_index(o, 3, true).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot delete property '3' of object"
        );
        assert!(cx.has_index(o, 3).unwrap());
    });
}

/// Values stored and read by index survive a collection before every allocation.
#[test]
fn index_ops_under_gc_stress() {
    with_cx(true, |cx, _| {
        let a = cx.new_array();
        let o = cx.new_object();
        for i in 0..40u64 {
            let s = cx.new_string(&format!("v{i}")).unwrap();
            cx.set_index(a, i, s, true).unwrap();
            let s = cx.new_string(&format!("w{i}")).unwrap();
            cx.set_index(o, i * 7, s, true).unwrap();
        }
        cx.collect_garbage();
        for i in 0..40u64 {
            let v = cx.get_index(a, i).unwrap();
            assert_eq!(cx.to_rust_string(v).unwrap(), format!("v{i}"));
            let w = cx.get_index(o, i * 7).unwrap();
            assert_eq!(cx.to_rust_string(w).unwrap(), format!("w{i}"));
        }
    });
}

// ---- the iterator protocol (ABI-3) ----------------------------------------------------------

/// `next()` of the test iterators: `{value: this.n, done: this.n >= this.limit}`, then
/// `this.n += 1`; `this.mode` "bad" returns 1, "throw" throws.
fn test_next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let it = this.as_object().unwrap();
    let calls = cx.get(it, "calls")?;
    let calls = cx.to_number(calls)?;
    cx.set(it, "calls", Local::number(calls + 1.0), true)?;
    let mode = cx.get(it, "mode")?;
    match cx.to_rust_string(mode)?.as_str() {
        "bad" => return Ok(Local::int32(1)),
        "throw" => return Err(cx.throw_range_error("next threw")),
        _ => {}
    }
    let n = cx.get(it, "n")?;
    let n = cx.to_number(n)?;
    let limit = cx.get(it, "limit")?;
    let limit = cx.to_number(limit)?;
    cx.set(it, "n", Local::number(n + 1.0), true)?;
    let r = cx.new_object();
    cx.create_data_property_or_throw(r, "value", Local::number(n))?;
    cx.create_data_property_or_throw(r, "done", Local::boolean(n >= limit))?;
    Ok(r.value())
}

/// `return()` of the test iterators: counts calls in `this.closed`; `this.ret` "bad" returns
/// 1, "throw" throws, else `{}`.
fn test_return<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let it = this.as_object().unwrap();
    let closed = cx.get(it, "closed")?;
    let closed = cx.to_number(closed)?;
    cx.set(it, "closed", Local::number(closed + 1.0), true)?;
    let mode = cx.get(it, "ret")?;
    match cx.to_rust_string(mode)?.as_str() {
        "bad" => Ok(Local::int32(1)),
        "throw" => Err(cx.throw_type_error("return threw")),
        _ => Ok(cx.new_object().value()),
    }
}

/// An iterable whose `@@iterator` returns itself (`return_this`), with `test_next`, the
/// `return` method `ret_method` (if any) and the counters at 0.
fn test_iterable<'s>(
    cx: &mut Cx<'s>,
    rt: &Runtime,
    limit: i32,
    ret_method: Option<Local<'s>>,
) -> Local<'s, JsObject> {
    let it = cx.new_object();
    let sym = crate::symbol::well_known(rt, crate::symbol::WellKnownSymbol::Iterator);
    let key: Local<'s, crate::JsSymbol> = Local::from_bits(sym);
    let me = host_fn(rt, return_this, "[Symbol.iterator]");
    cx.set(it, key, me, true).unwrap();
    let next = host_fn(rt, test_next, "next");
    cx.set(it, "next", next, true).unwrap();
    if let Some(r) = ret_method {
        cx.set(it, "return", r, true).unwrap();
    }
    for (k, v) in [("n", 0), ("limit", limit), ("calls", 0), ("closed", 0)] {
        cx.set(it, k, Local::int32(v), true).unwrap();
    }
    it
}

fn int_prop<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, k: &str) -> i32 {
    cx.get(o, k).unwrap().as_int32().unwrap()
}

#[test]
fn get_iterator_steps_an_array_and_a_custom_iterator() {
    with_cx(true, |cx, rt| {
        let mut vals = [Local::undefined(); 3];
        for (i, v) in vals.iter_mut().enumerate() {
            *v = cx.new_string(&format!("s{i}")).unwrap().value();
        }
        let a = cx.create_array_from_list(&vals).unwrap();
        let mut rec = cx.get_iterator(a).unwrap();
        let mut seen = Vec::new();
        while let Some(v) = cx.iterator_step_value(&mut rec).unwrap() {
            seen.push(cx.to_rust_string(v).unwrap());
        }
        assert_eq!(seen, ["s0", "s1", "s2"]);
        assert!(rec.is_done());
        assert!(cx.iterator_step_value(&mut rec).unwrap().is_none());

        let it = test_iterable(cx, rt, 4, None);
        let mut rec = cx.get_iterator(it).unwrap();
        assert!(rec.iterator().is_same_bits(&it));
        assert!(rec.next_method().is_callable());
        let mut n = 0;
        while let Some(v) = cx.iterator_step_value(&mut rec).unwrap() {
            assert_eq!(v.as_number(), Some(f64::from(n)));
            n += 1;
        }
        assert_eq!(n, 4);
        // A done record makes no more calls.
        assert!(cx.iterator_step_value(&mut rec).unwrap().is_none());
        assert_eq!(int_prop(cx, it, "calls"), 5);
    });
}

#[test]
fn get_iterator_errors() {
    with_cx(false, |cx, rt| {
        assert!(cx.get_iterator(Local::undefined()).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: undefined is not iterable");
        let o = cx.new_object();
        assert!(cx.get_iterator(o).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: object is not iterable");
        assert!(cx.get_iterator(Local::int32(5)).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: 5 is not iterable");
        // @@iterator returns a primitive.
        let f = host_fn(rt, return_this, "f");
        assert!(cx.get_iterator_from_method(Local::int32(1), f).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Result of the Symbol.iterator method is not an object"
        );
    });
}

/// IteratorStepValue sets `[[Done]]` when `next` throws or gives a non-object.
#[test]
fn a_throwing_step_marks_the_record_done() {
    with_cx(false, |cx, rt| {
        let it = test_iterable(cx, rt, 10, None);
        set_text(cx, it, "mode", "bad");
        let mut rec = cx.get_iterator(it).unwrap();
        assert!(cx.iterator_step_value(&mut rec).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Iterator result 1 is not an object"
        );
        assert!(rec.is_done());
        assert!(cx.iterator_step_value(&mut rec).unwrap().is_none());
        assert_eq!(int_prop(cx, it, "calls"), 1);

        let it = test_iterable(cx, rt, 10, None);
        set_text(cx, it, "mode", "throw");
        let mut rec = cx.get_iterator(it).unwrap();
        assert!(cx.iterator_step_value(&mut rec).is_err());
        assert_eq!(take_error(cx, rt), "RangeError: next threw");
        assert!(rec.is_done());
    });
}

/// IteratorClose with a normal completion: `return` runs once with the iterator as this; its
/// throw or a non-object result replaces the completion; no `return` leaves it.
#[test]
fn iterator_close_with_a_normal_completion() {
    with_cx(true, |cx, rt| {
        let ret = host_fn(rt, test_return, "return");
        let it = test_iterable(cx, rt, 10, Some(ret));
        let rec = cx.get_iterator(it).unwrap();
        assert_eq!(cx.iterator_close(&rec, Ok(7)).unwrap(), 7);
        assert_eq!(int_prop(cx, it, "closed"), 1);

        set_text(cx, it, "ret", "bad");
        assert!(cx.iterator_close(&rec, Ok(7)).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Iterator result 1 is not an object"
        );
        set_text(cx, it, "ret", "throw");
        assert!(cx.iterator_close(&rec, Ok(7)).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: return threw");
        assert_eq!(int_prop(cx, it, "closed"), 3);

        // No `return`: nothing runs.
        let plain = test_iterable(cx, rt, 10, None);
        let rec = cx.get_iterator(plain).unwrap();
        assert_eq!(cx.iterator_close(&rec, Ok(1)).unwrap(), 1);
        cx.set(plain, "return", Local::null(), true).unwrap();
        assert_eq!(cx.iterator_close(&rec, Ok(2)).unwrap(), 2);
        // A `return` that is not callable: TypeError.
        cx.set(plain, "return", Local::int32(5), true).unwrap();
        assert!(cx.iterator_close(&rec, Ok(3)).is_err());
        assert_eq!(take_error(cx, rt), "TypeError: 5 is not a function");
    });
}

/// IteratorClose with a throw completion: `return` runs without a pending exception, and the
/// first exception is the result whatever `return` does.
#[test]
fn iterator_close_with_a_throw_completion_keeps_the_exception() {
    with_cx(true, |cx, rt| {
        let ret = host_fn(rt, test_return, "return");
        let it = test_iterable(cx, rt, 10, Some(ret));
        let rec = cx.get_iterator(it).unwrap();
        for mode in ["ok", "bad", "throw"] {
            set_text(cx, it, "ret", mode);
            let first = cx.throw_range_error("first");
            let r: JsResult<()> = cx.iterator_close(&rec, Err(first));
            assert!(r.is_err());
            assert_eq!(take_error(cx, rt), "RangeError: first", "{mode}");
        }
        assert_eq!(int_prop(cx, it, "closed"), 3);
        // A `return` that is not callable does not replace it either.
        cx.set(it, "return", Local::int32(5), true).unwrap();
        let first = cx.throw_range_error("first");
        let r: JsResult<()> = cx.iterator_close(&rec, Err(first));
        assert!(r.is_err());
        assert_eq!(take_error(cx, rt), "RangeError: first");
    });
}
