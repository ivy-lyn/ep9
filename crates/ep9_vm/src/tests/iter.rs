//! Iteration protocol tests (P3-NAT): GetIterator and CreateArrayFromIterable look up and call
//! `@@iterator` (§7.4.3), with the built-in iterator cells as the fast path while the method
//! is the intrinsic; `Array.prototype.values` = `[@@iterator]`,
//! `String.prototype[@@iterator]`, `%Iterator.prototype%[@@iterator]`, the `next` of
//! %ArrayIteratorPrototype% and %StringIteratorPrototype%; `Object.prototype.toString` step 15
//! (`@@toStringTag`).

use ep9_bytecode::{FeedbackSlotKind, Register, RuntimeId, VerifiedUnit};

use super::{Asm, config, isolate, on_big_stack, take_output, unit};
use crate::builtins::IntrinsicId;
use crate::frame::UNDEFINED;
use crate::interp::slow;
use crate::object;
use crate::runtime::Runtime;
use crate::string;
use crate::symbol::{self, WellKnownSymbol};
use crate::{
    Args, Cx, Dispatch, Exception, Isolate, IsolateConfig, JsObject, JsResult, JsSymbol, Local,
    PropertyDescriptor,
};

// ---- test iterables (host functions) ------------------------------------------------------------

fn well_known<'s>(cx: &mut Cx<'s>, w: WellKnownSymbol) -> Local<'s, JsSymbol> {
    let b = symbol::well_known(cx.inner().rt(), w);
    let v: Local<'s> = Local::from_bits(b);
    v.as_symbol().expect("a well-known symbol")
}

fn global_fn<'s>(cx: &mut Cx<'s>, name: &str) -> Local<'s> {
    let g = cx.global();
    cx.get(g, name).unwrap()
}

/// `iterable(n)`: `{ limit: n, [Symbol.iterator]: iterMethod }`.
fn iterable<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let o = cx.new_object();
    cx.set(o, "limit", args.get(0), true)?;
    let m = global_fn(cx, "iterMethod");
    let key = well_known(cx, WellKnownSymbol::Iterator);
    cx.set(o, key, m, true)?;
    Ok(o.value())
}

/// `iterable[Symbol.iterator]()`: `{ next: nextMethod, i: 0, limit: this.limit }`, so the
/// values show that `this` was the iterable.
fn iter_method<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let this = this.as_object().expect("this is the iterable");
    let limit = cx.get(this, "limit")?;
    let it = cx.new_object();
    let next = global_fn(cx, "nextMethod");
    cx.set(it, "next", next, true)?;
    cx.set(it, "i", Local::int32(0), true)?;
    cx.set(it, "limit", limit, true)?;
    Ok(it.value())
}

/// `{ value: 10 * i, done: false }` while `i < limit` (then `i += 1`), else `{ done: true }`.
fn next_method<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let this = this.as_object().expect("this is the iterator");
    let i = cx.get(this, "i")?.as_int32().unwrap();
    let limit = cx.get(this, "limit")?.as_int32().unwrap();
    let r = cx.new_object();
    if i < limit {
        cx.set(this, "i", Local::int32(i + 1), true)?;
        cx.set(r, "value", Local::int32(10 * i), true)?;
        cx.set(r, "done", Local::boolean(false), true)?;
    } else {
        cx.set(r, "done", Local::boolean(true), true)?;
    }
    Ok(r.value())
}

/// A `next` that is done at once.
fn done_next<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let r = cx.new_object();
    cx.set(r, "done", Local::boolean(true), true)?;
    Ok(r.value())
}

/// A `valueOf` that throws.
fn throws<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("boom"))
}

/// `this.it`.
fn own_it<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let this = this.as_object().expect("an object");
    cx.get(this, "it")
}

/// A method that returns its first argument.
fn identity<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(args.get(0))
}

/// A getter that calls `this.it.next()` and prints "inner:" with the result ("value,done") or
/// the error ("Name: message", caught); then returns `this.ret`. With `this.rethrow` true the
/// error propagates instead.
fn reenter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let this = this.as_object().expect("the array-like");
    let it = cx.get(this, "it")?.as_object().expect("an iterator");
    let rethrow = cx.get(this, "rethrow")?.as_bool() == Some(true);
    let next = cx.get(it, "next")?;
    let text = match cx.call(next, it.value(), &[]) {
        Ok(r) => {
            let r = r.as_object().expect("an iterator result");
            let v = cx.get(r, "value")?;
            let d = cx.get(r, "done")?;
            format!("inner:{},{}", cx.to_rust_string(v)?, d.as_bool().unwrap())
        }
        Err(e) if rethrow => return Err(e),
        // Caught: `result` takes the pending exception.
        Err(_) => format!("inner:{}", result(cx, 0).unwrap_err()),
    };
    let print = global_fn(cx, "print");
    let text = cx.new_string(&text)?;
    cx.call(print, Local::undefined(), &[text.value()])?;
    cx.get(this, "ret")
}

/// A new isolate with `print` and the test iterables.
fn iter_isolate(config: IsolateConfig) -> Isolate {
    let mut iso = isolate(config);
    iso.register_host_function("iterable", 1, iterable);
    iso.register_host_function("iterMethod", 0, iter_method);
    iso.register_host_function("nextMethod", 0, next_method);
    iso.register_host_function("doneNext", 0, done_next);
    iso.register_host_function("throws", 0, throws);
    iso.register_host_function("identity", 1, identity);
    iso.register_host_function("ownIt", 0, own_it);
    iso.register_host_function("reenter", 0, reenter);
    iso
}

/// Runs `check` on a fresh isolate under every dispatch expansion, with and without GC
/// stress; checks the heap after each.
fn each_isolate(check: fn(&mut Isolate)) {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = iter_isolate(config(d, stress));
                take_output();
                check(&mut iso);
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

fn run(iso: &mut Isolate, u: VerifiedUnit) -> (Result<crate::Value, Exception>, Vec<String>) {
    take_output();
    let r = iso.run_raw(u, "iter.js");

    (r, take_output())
}

// ---- bytecode helpers ---------------------------------------------------------------------------

/// `for (x of <acc>) body(x)` as BytecodeGen emits it (GetIterator, `next` once, then
/// IteratorStep until done), without the IteratorClose `finally`.
fn for_of(s: &mut Asm, body: impl FnOnce(&mut Asm, Register)) {
    let (obj, iter, next) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    let res_list = s.b.new_register_list(1);
    let res = res_list.get(0);
    let load = s.b.feedback_slot(FeedbackSlotKind::LoadNamed);
    let call = s.b.feedback_slot(FeedbackSlotKind::Call);
    s.b.star(obj);
    s.b.get_iterator(obj, load, call).star(iter);
    s.get_named(iter, "next");
    s.b.star(next);
    let head = s.b.bound_label();
    let (ok, done) = (s.b.new_label(), s.b.new_label());
    let step = s.b.feedback_slot(FeedbackSlotKind::Call);
    s.b.call_property0(next, iter, step).star(res);
    s.b.jump_if_js_receiver(ok);
    s.b.call_runtime(RuntimeId::ThrowIteratorResultNotAnObject, res_list, 1);
    s.b.bind(ok);
    s.get_named(res, "done");
    s.b.jump_if_to_boolean_true(done);
    s.get_named(res, "value");
    s.b.star(res);
    body(s, res);
    s.b.jump_loop(head, 0, s.loop_);
    s.b.bind(done);
}

/// `acc = iterable(n)`.
fn call_iterable(s: &mut Asm, n: i32) {
    let (f, a0) = (s.f, s.a.get(0));
    s.lda_global("iterable");
    s.b.star(f);
    s.b.lda_smi(n).star(a0);
    s.b.call_undefined_receiver1(f, a0, s.call);
}

/// `print(acc.join())` (clobbers `f`, `a` and the accumulator).
fn print_joined(s: &mut Asm) {
    let arr = s.a.get(1);
    s.b.star(arr);
    s.get_named(arr, "join");
    s.b.star(s.f);
    s.b.call_property0(s.f, arr, s.call);
    s.print_acc();
}

/// A script that collects the values of `for (x of <str>)` into an array and returns it.
fn collect_for_of_string(text: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (arr, idx) = (s.b.new_register(), s.b.new_register());
    let store = s.b.feedback_slot(FeedbackSlotKind::StoreInArrayLiteral);
    s.b.create_empty_array_literal(s.lit).star(arr);
    s.b.lda_zero().star(idx);
    s.lda_str(text);
    let bin = s.bin;
    for_of(&mut s, |s, v| {
        s.b.ldar(v).sta_in_array_literal(arr, idx, store);
        s.b.ldar(idx).inc(bin).star(idx);
    });
    s.b.ldar(arr).return_();
    unit("", vec![s.finish()])
}

// ---- acceptance ---------------------------------------------------------------------------------

/// A user iterable (`[Symbol.iterator]` returning an iterator object) works in for-of, in
/// `[...x]` (CreateArrayFromIterable) and in array destructuring's GetIterator.
#[test]
fn a_symbol_iterator_iterable_works_in_for_of() {
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        // for (x of iterable(3)) print(x)
        call_iterable(&mut s, 3);
        for_of(&mut s, |s, v| {
            s.b.ldar(v);
            s.print_acc();
        });
        // print([...iterable(2)].join())
        call_iterable(&mut s, 2);
        s.b.create_array_from_iterable();
        print_joined(&mut s);
        // An empty iterable: the body never runs.
        call_iterable(&mut s, 0);
        for_of(&mut s, |s, _| {
            s.lda_str("never");
            s.print_acc();
        });
        s.lda_str("end");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    each_isolate(|iso| {
        let (r, out) = run(iso, make());
        assert_eq!(iso.display_raw(r.unwrap()), "end");
        assert_eq!(out, ["0", "10", "20", "0,10"]);
    });
}

/// for-of over a Latin-1 string yields the single-character atoms (the same bits as the atom
/// of that character, W4-STR-SINGLE-CHAR) [I10]; a UTF-16 unit is a new one-unit string and a
/// surrogate pair one two-unit string (§22.1.5.2.1).
#[test]
fn for_of_over_a_latin1_string_yields_single_char_atoms() {
    each_isolate(|iso| {
        let text = "h\u{e9}llo!\u{ff}";
        let (r, _) = run(iso, collect_for_of_string(text));
        let arr = r.unwrap().to_bits();
        let rt: &Runtime = iso.cx.rt();
        let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
        assert_eq!(object::array_len(arr) as usize, bytes.len());
        for (i, &c) in bytes.iter().enumerate() {
            let v = object::array_get(arr, i as u32).unwrap();
            let atom = rt.single_char(c);
            assert_ne!(atom, UNDEFINED, "{c:#x} made");
            assert_eq!(v, atom, "value {i} ({c:#x}) is the single-character atom");
        }
        // "a€😀b": four values; "€" one unit, "😀" a pair.
        let (r, _) = run(iso, collect_for_of_string("a\u{20ac}\u{1f600}b"));
        let arr = r.unwrap().to_bits();
        let rt: &Runtime = iso.cx.rt();
        let texts: Vec<String> = (0..object::array_len(arr))
            .map(|i| string::to_rust_string(object::array_get(arr, i).unwrap()))
            .collect();
        assert_eq!(texts, ["a", "\u{20ac}", "\u{1f600}", "b"]);
        assert_eq!(object::array_get(arr, 0), Some(rt.single_char(b'a')));
        assert_eq!(string::str_len(object::array_get(arr, 2).unwrap()), 2);
    });
}

// ---- GetIterator and CreateArrayFromIterable ---------------------------------------------------

/// GetIterator / CreateArrayFromIterable from Rust (the slow-path entries with no frame).
fn get_iterator<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> Result<Local<'s>, String> {
    let r = slow::get_iterator(
        cx.inner(),
        core::ptr::null_mut(),
        core::ptr::null(),
        v.bits(),
    );
    result(cx, r)
}

fn array_from_iterable<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> Result<Local<'s>, String> {
    let r = slow::array_from_iterable(
        cx.inner(),
        core::ptr::null_mut(),
        core::ptr::null(),
        v.bits(),
    );
    result(cx, r)
}

/// The value, or "Name: message" of the pending exception (taken).
fn result<'s>(cx: &mut Cx<'s>, r: u64) -> Result<Local<'s>, String> {
    if r != 0 {
        return Ok(Local::from_bits(r));
    }
    let e: Local<'_> = Local::from_bits(cx.inner().rt().take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    Err(format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    ))
}

/// `[...v]` joined with ",".
fn spread_text<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> Result<String, String> {
    let a = array_from_iterable(cx, v)?;
    let a = a.as_object().unwrap();
    let join = cx.get(a, "join").unwrap();
    let s = cx.call(join, a.value(), &[]).unwrap();
    Ok(cx.to_rust_string(s).unwrap())
}

fn new_array<'s>(cx: &mut Cx<'s>, items: &[Local<'s>]) -> Local<'s, JsObject> {
    cx.create_array_from_list(items).unwrap().into()
}

#[test]
fn get_iterator_calls_at_iterator_and_checks_the_result() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let iter = well_known(cx, WellKnownSymbol::Iterator);
            // Not iterable: undefined, null, a number, an object without @@iterator, a
            // non-callable @@iterator.
            let plain = cx.new_object();
            let bad = cx.new_object();
            cx.set(bad, iter, Local::int32(1), true)?;
            for (v, text) in [
                (Local::undefined(), "undefined"),
                (Local::null(), "null"),
                (Local::int32(5), "5"),
                (plain.value(), "object"),
                (bad.value(), "object"),
            ] {
                let e = get_iterator(cx, v).unwrap_err();
                assert_eq!(e, format!("TypeError: {text} is not iterable"));
                let e = array_from_iterable(cx, v).unwrap_err();
                assert_eq!(e, format!("TypeError: {text} is not iterable"));
            }
            // A method whose result is not an object.
            let prim = cx.new_object();
            let identity = global_fn(cx, "identity");
            cx.set(prim, iter, identity, true)?;
            let e = get_iterator(cx, prim.value()).unwrap_err();
            assert_eq!(
                e,
                "TypeError: Result of the Symbol.iterator method is not an object"
            );
            // A user iterable: GetIterator calls the method with `this` = the iterable.
            let f = global_fn(cx, "iterable");
            let user = cx.call(f, Local::undefined(), &[Local::int32(2)])?;
            let it = get_iterator(cx, user).unwrap().as_object().unwrap();
            let limit = cx.get(it, "limit")?;
            assert_eq!(limit.as_int32(), Some(2));
            assert_eq!(spread_text(cx, user).unwrap(), "0,10");
            // IteratorStep in CreateArrayFromIterable: a result that is not an object, and a
            // missing `next`.
            let holder = cx.new_object();
            let own_it = global_fn(cx, "ownIt");
            cx.set(holder, iter, own_it, true)?;
            let it = cx.new_object();
            cx.set(holder, "it", it, true)?;
            cx.set(it, "next", identity, true)?;
            let e = array_from_iterable(cx, holder.value()).unwrap_err();
            assert_eq!(e, "TypeError: Iterator result undefined is not an object");
            let it = cx.new_object();
            cx.set(holder, "it", it, true)?;
            let e = array_from_iterable(cx, holder.value()).unwrap_err();
            assert_eq!(e, "TypeError: undefined is not a function");
            Ok(())
        })
        .unwrap();
    });
}

/// The fast paths make the same cells as the natives, and stop when the method or the
/// prototype's `next` is not the intrinsic any more.
#[test]
fn the_fast_paths_follow_the_intrinsics() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let iter = well_known(cx, WellKnownSymbol::Iterator);
            let aip = cx.intrinsic(IntrinsicId::ArrayIteratorPrototype);
            let sip = cx.intrinsic(IntrinsicId::StringIteratorPrototype);
            // An array and a string get iterator cells with the realm's prototypes.
            let a = new_array(cx, &[Local::int32(1), Local::int32(2)]);
            let it = get_iterator(cx, a.value()).unwrap().as_object().unwrap();
            assert!(cx.get_prototype_of(it)?.is_same_bits(&aip));
            let s = cx.new_string("ab")?;
            let it = get_iterator(cx, s.value()).unwrap().as_object().unwrap();
            assert!(cx.get_prototype_of(it)?.is_same_bits(&sip));
            assert_eq!(spread_text(cx, a.value()).unwrap(), "1,2");
            assert_eq!(spread_text(cx, s.value()).unwrap(), "a,b");
            // A hole reads through [[Get]]: Array.prototype[1] shows.
            let holes = cx.new_array();
            cx.set(holes, "0", Local::int32(1), true)?;
            cx.set(holes, "2", Local::int32(3), true)?;
            let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
            cx.set(ap, "1", Local::int32(9), true)?;
            assert_eq!(spread_text(cx, holes.value()).unwrap(), "1,9,3");
            cx.delete(ap, "1")?;
            // An own @@iterator on the array wins over Array.prototype's.
            let user = global_fn(cx, "iterMethod");
            cx.set(a, iter, user, true)?;
            cx.set(a, "limit", Local::int32(3), true)?;
            assert_eq!(spread_text(cx, a.value()).unwrap(), "0,10,20");
            // A `next` on %ArrayIteratorPrototype% that is not the intrinsic: the spread calls
            // it (done at once), for an Array and a String wrapper alike.
            let b = new_array(cx, &[Local::int32(1), Local::int32(2)]);
            let next = cx.get(aip, "next")?;
            let done = global_fn(cx, "doneNext");
            cx.set(aip, "next", done, true)?;
            assert_eq!(spread_text(cx, b.value()).unwrap(), "");
            cx.set(aip, "next", next, true)?;
            assert_eq!(spread_text(cx, b.value()).unwrap(), "1,2");
            // The same for %StringIteratorPrototype%.next.
            let snext = cx.get(sip, "next")?;
            cx.set(sip, "next", done, true)?;
            assert_eq!(spread_text(cx, s.value()).unwrap(), "");
            cx.set(sip, "next", snext, true)?;
            assert_eq!(spread_text(cx, s.value()).unwrap(), "a,b");
            // A replaced Array.prototype[@@iterator] is called (here: the user iterable).
            let values = cx.get(ap, "values")?;
            cx.set(ap, iter, user, true)?;
            cx.set(b, "limit", Local::int32(1), true)?;
            assert_eq!(spread_text(cx, b.value()).unwrap(), "0");
            cx.set(ap, iter, values, true)?;
            assert_eq!(spread_text(cx, b.value()).unwrap(), "1,2");
            Ok(())
        })
        .unwrap();
    });
}

// ---- the natives --------------------------------------------------------------------------------

/// `it.next()` as (value text, done).
fn step<'s>(cx: &mut Cx<'s>, it: Local<'s, JsObject>) -> JsResult<(String, bool)> {
    let next = cx.get(it, "next")?;
    let r = cx.call(next, it.value(), &[])?.as_object().unwrap();
    let v = cx.get(r, "value")?;
    let d = cx.get(r, "done")?;
    Ok((cx.to_rust_string(v)?, d.as_bool().unwrap()))
}

fn data_attrs(d: &PropertyDescriptor<'_>) -> (Option<bool>, Option<bool>, Option<bool>) {
    (d.writable, d.enumerable, d.configurable)
}

#[test]
fn values_is_at_iterator_and_the_iterator_prototypes_are_built() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let iter = well_known(cx, WellKnownSymbol::Iterator);
            let tag = well_known(cx, WellKnownSymbol::ToStringTag);
            let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
            let values = cx.get(ap, "values")?;
            let at_iter = cx.get(ap, iter)?;
            assert!(values.is_callable() && values.is_same_bits(&at_iter));
            let d = cx.get_own_property(ap, iter)?.unwrap();
            assert_eq!(data_attrs(&d), (Some(true), Some(false), Some(true)));
            let values = values.as_object().unwrap();
            let name = cx.get(values, "name")?;
            assert_eq!(cx.to_rust_string(name)?, "values");
            assert_eq!(cx.get(values, "length")?.as_int32(), Some(0));
            assert!(!cx.is_constructor(values));
            // String.prototype[@@iterator] and %Iterator.prototype%[@@iterator].
            let sp = cx.intrinsic(IntrinsicId::StringPrototype);
            let si = cx.get(sp, iter)?.as_object().unwrap();
            let name = cx.get(si, "name")?;
            assert_eq!(cx.to_rust_string(name)?, "[Symbol.iterator]");
            assert_eq!(cx.get(si, "length")?.as_int32(), Some(0));
            let ip = cx.intrinsic(IntrinsicId::IteratorPrototype);
            let ii = cx.get(ip, iter)?;
            let o = cx.new_object();
            assert!(cx.call(ii, o.value(), &[])?.is_same_bits(&o));
            assert_eq!(cx.call(ii, Local::int32(5), &[])?.as_int32(), Some(5));
            // %ArrayIteratorPrototype% and %StringIteratorPrototype%: two objects on
            // %Iterator.prototype%, each with its own `next` and @@toStringTag {c}.
            let aip = cx.intrinsic(IntrinsicId::ArrayIteratorPrototype);
            let sip = cx.intrinsic(IntrinsicId::StringIteratorPrototype);
            assert!(!aip.is_same_bits(&sip));
            assert!(cx.get_prototype_of(sip)?.is_same_bits(&ip));
            let an = cx.get(aip, "next")?;
            let sn = cx.get(sip, "next")?;
            assert!(an.is_callable() && sn.is_callable() && !an.is_same_bits(&sn));
            for (p, want) in [(aip, "Array Iterator"), (sip, "String Iterator")] {
                let d = cx.get_own_property(p, tag)?.unwrap();
                assert_eq!(cx.to_rust_string(d.value.unwrap())?, want);
                assert_eq!(data_attrs(&d), (Some(false), Some(false), Some(true)));
            }
            Ok(())
        })
        .unwrap();
    });
}

#[test]
fn the_array_iterator_reads_array_likes_and_stays_done() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
            let values = cx.get(ap, "values")?;
            // An array-like object: the length is read at each step.
            let o = cx.new_object();
            cx.set(o, "length", Local::int32(1), true)?;
            let a = cx.new_string("a")?;
            let b = cx.new_string("b")?;
            cx.set(o, "0", a, true)?;
            cx.set(o, "1", b, true)?;
            let it = cx.call(values, o.value(), &[])?.as_object().unwrap();
            assert_eq!(step(cx, it)?, ("a".into(), false));
            cx.set(o, "length", Local::int32(2), true)?;
            assert_eq!(step(cx, it)?, ("b".into(), false));
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            // Done for good, even when the object grows.
            cx.set(o, "length", Local::int32(5), true)?;
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            // A String wrapper is an array-like of code units (not code points).
            let s = cx.new_string("h\u{1f600}")?;
            let w = cx.to_object(s)?;
            let it = cx.call(values, w.value(), &[])?.as_object().unwrap();
            let mut n = 0;
            while !step(cx, it)?.1 {
                n += 1;
            }
            assert_eq!(n, 3);
            // A primitive this is boxed; undefined throws.
            let it = cx.call(values, Local::int32(1), &[])?.as_object().unwrap();
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            assert!(cx.call(values, Local::undefined(), &[]).is_err());
            let e = take_message(cx);
            assert_eq!(e, "Cannot convert undefined or null to object");
            // A step that throws completes the iterator (§27.5.3.3 GeneratorStart).
            let bad = cx.new_object();
            let len = cx.new_object();
            let throws = global_fn(cx, "throws");
            cx.set(len, "valueOf", throws, true)?;
            cx.set(bad, "length", len, true)?;
            let it = cx.call(values, bad.value(), &[])?.as_object().unwrap();
            assert!(step(cx, it).is_err());
            assert_eq!(take_message(cx), "boom");
            cx.set(bad, "length", Local::int32(0), true)?;
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            Ok(())
        })
        .unwrap();
    });
}

/// What [`reenter`] prints when `next` finds its iterator executing.
const RUNNING: &str = "inner:TypeError: Array Iterator is already running";

/// An accessor descriptor with the getter [`reenter`].
fn reenter_getter<'s>(cx: &mut Cx<'s>) -> PropertyDescriptor<'s> {
    PropertyDescriptor {
        get: Some(global_fn(cx, "reenter")),
        set: Some(Local::undefined()),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::default()
    }
}

/// `Array.prototype.values.call(o)`, also stored as `o.it`.
fn values_of<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<Local<'s, JsObject>> {
    let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
    let values = cx.get(ap, "values")?;
    let it = cx.call(values, o.value(), &[])?.as_object().unwrap();
    cx.set(o, "it", it, true)?;
    Ok(it)
}

/// `o = { length: 3, get 0() { reenter; return "a" }, 1: "b", 2: "c" }` and `o.it`.
fn reentrant_array_like<'s>(
    cx: &mut Cx<'s>,
) -> JsResult<(Local<'s, JsObject>, Local<'s, JsObject>)> {
    let o = cx.new_object();
    cx.set(o, "length", Local::int32(3), true)?;
    let get = reenter_getter(cx);
    cx.define_property_or_throw(o, "0", get)?;
    for (k, v) in [("1", "b"), ("2", "c"), ("ret", "a")] {
        let v = cx.new_string(v)?;
        cx.set(o, k, v, true)?;
    }
    let it = values_of(cx, o)?;
    Ok((o, it))
}

/// A `next` called by the user code of its own step (an element getter, the `length` getter)
/// throws a TypeError: the iterator is executing (GeneratorValidate step 6; V8 runs the call,
/// W4-P3FIX-ITER-REENTRY). A getter that catches it lets the outer step go on; one that does
/// not completes the iterator.
#[test]
fn a_reentrant_next_throws_and_the_outer_step_goes_on() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let (_, it) = reentrant_array_like(cx)?;
            assert_eq!(step(cx, it)?, ("a".into(), false));
            assert_eq!(take_output(), [RUNNING]);
            assert_eq!(step(cx, it)?, ("b".into(), false));
            assert_eq!(step(cx, it)?, ("c".into(), false));
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            assert!(take_output().is_empty());
            // The length getter runs at each step, so each step re-enters once.
            let o = cx.new_object();
            let get = reenter_getter(cx);
            cx.define_property_or_throw(o, "length", get)?;
            cx.set(o, "ret", Local::int32(1), true)?;
            let x = cx.new_string("x")?;
            cx.set(o, "0", x, true)?;
            let it = values_of(cx, o)?;
            assert_eq!(step(cx, it)?, ("x".into(), false));
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            assert_eq!(take_output(), [RUNNING, RUNNING]);
            // Not caught: the outer `next` throws it too, and the iterator is done.
            let (o, it) = reentrant_array_like(cx)?;
            cx.set(o, "rethrow", Local::boolean(true), true)?;
            assert!(step(cx, it).is_err());
            assert_eq!(take_message(cx), "Array Iterator is already running");
            assert_eq!(step(cx, it)?, ("undefined".into(), true));
            assert!(take_output().is_empty());
            Ok(())
        })
        .unwrap();
    });
}

/// `for (x of <global name>) print(x)`.
fn for_of_global(name: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    s.lda_global(name);
    for_of(&mut s, |s, v| {
        s.b.ldar(v);
        s.print_acc();
    });
    s.b.lda_undefined().return_();
    unit("", vec![s.finish()])
}

/// for-of and spread (interp/slow/iter.rs) step an iterator object through its `next`, so a
/// getter that re-enters it gets the TypeError there too. Over the array-like itself, the
/// fast path's cell is out of reach; the getter's call then runs a step of `o.it`, whose own
/// getter call re-enters `o.it`.
#[test]
fn for_of_and_spread_see_a_running_array_iterator() {
    fn o_global<'s>(cx: &mut Cx<'s>) -> JsResult<Local<'s, JsObject>> {
        let g = cx.global();
        Ok(cx.get(g, "o")?.as_object().unwrap())
    }
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let (o, it) = reentrant_array_like(cx)?;
            // o[Symbol.iterator] = Array.prototype.values, so GetIterator(o) takes the fast path.
            let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
            let values = cx.get(ap, "values")?;
            let iter = well_known(cx, WellKnownSymbol::Iterator);
            cx.set(o, iter, values, true)?;
            let g = cx.global();
            cx.set(g, "o", o, true)?;
            cx.set(g, "it", it, true)
        })
        .unwrap();
        // GetIterator(it) calls %Iterator.prototype%[@@iterator], then the loop calls `next`.
        let (r, out) = run(iso, for_of_global("it"));
        r.unwrap();
        assert_eq!(out, [RUNNING, "a", "b", "c"]);
        iso.with_cx(|cx| {
            let o = o_global(cx)?;
            values_of(cx, o).map(|_| ())
        })
        .unwrap();
        let (r, out) = run(iso, for_of_global("o"));
        r.unwrap();
        assert_eq!(out, [RUNNING, "inner:a,false", "a", "b", "c"]);
        // [...it] and [...o]: the generic IteratorStep loop of CreateArrayFromIterable.
        iso.with_cx(|cx| {
            let o = o_global(cx)?;
            let it = values_of(cx, o)?;
            take_output();
            assert_eq!(spread_text(cx, it.value()).unwrap(), "a,b,c");
            assert_eq!(take_output(), [RUNNING]);
            values_of(cx, o)?;
            assert_eq!(spread_text(cx, o.value()).unwrap(), "a,b,c");
            assert_eq!(take_output(), [RUNNING, "inner:a,false"]);
            Ok(())
        })
        .unwrap();
    });
}

/// The message of the pending exception (taken).
fn take_message(cx: &mut Cx<'_>) -> String {
    let e: Local<'_> = Local::from_bits(cx.inner().rt().take_pending());
    let e = e.as_object().expect("an Error object");
    let m = cx.get(e, "message").unwrap();
    cx.to_rust_string(m).unwrap()
}

#[test]
fn next_checks_its_receiver() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let iter = well_known(cx, WellKnownSymbol::Iterator);
            let aip = cx.intrinsic(IntrinsicId::ArrayIteratorPrototype);
            let sip = cx.intrinsic(IntrinsicId::StringIteratorPrototype);
            let an = cx.get(aip, "next")?;
            let sn = cx.get(sip, "next")?;
            let a = cx.new_array();
            let ai = cx.get(a, iter)?;
            let ai = cx.call(ai, a.value(), &[])?;
            let s = cx.new_string("x")?;
            let sp = cx.intrinsic(IntrinsicId::StringPrototype);
            let si = cx.get(sp, iter)?;
            let si = cx.call(si, s.value(), &[])?;
            let plain = cx.new_object_with_proto(ai);
            // Each `next` takes only its own kind; an object that inherits from an iterator
            // is not one.
            for (next, this, ok) in [
                (an, ai, true),
                (sn, si, true),
                (an, si, false),
                (sn, ai, false),
                (an, plain.value(), false),
                (sn, Local::undefined(), false),
            ] {
                let r = cx.call(next, this, &[]);
                assert_eq!(r.is_ok(), ok);
                if !ok {
                    let m = take_message(cx);
                    assert!(m.contains("called on incompatible receiver"), "{m}");
                }
            }
            // Done iterators keep their kind: the string iterator holds "" once done (its
            // one value went to the first call above).
            let si = si.as_object().unwrap();
            assert_eq!(step(cx, si)?, ("undefined".into(), true));
            assert!(cx.call(an, si.value(), &[]).is_err());
            take_message(cx);
            assert_eq!(step(cx, si)?, ("undefined".into(), true));
            let ai = ai.as_object().unwrap();
            assert_eq!(step(cx, ai)?, ("undefined".into(), true));
            assert!(cx.call(sn, ai.value(), &[]).is_err());
            take_message(cx);
            // String.prototype[@@iterator] needs a this that is not undefined or null.
            let f = cx.get(sp, iter)?;
            assert!(cx.call(f, Local::null(), &[]).is_err());
            assert_eq!(
                take_message(cx),
                "String.prototype[Symbol.iterator] called on null or undefined"
            );
            Ok(())
        })
        .unwrap();
    });
}

// ---- Object.prototype.toString ------------------------------------------------------------------

/// `Object.prototype.toString.call(this)`.
fn text<'s>(cx: &mut Cx<'s>, this: Local<'s>) -> String {
    let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
    let ts = cx.get(op, "toString").unwrap();
    let r = cx.call(ts, this, &[]).unwrap();
    cx.to_rust_string(r).unwrap()
}

#[test]
fn object_to_string_reads_to_string_tag() {
    each_isolate(|iso| {
        iso.with_cx(|cx| {
            let tag = well_known(cx, WellKnownSymbol::ToStringTag);
            assert_eq!(text(cx, Local::undefined()), "[object Undefined]");
            assert_eq!(text(cx, Local::null()), "[object Null]");
            // Step 15: a String tag wins (any code units); another value gives builtinTag.
            let o = cx.new_object();
            let zed = cx.new_string("Z\u{e9}d\u{20ac}")?;
            cx.set(o, tag, zed, true)?;
            assert_eq!(text(cx, o.value()), "[object Z\u{e9}d\u{20ac}]");
            let a = cx.new_array();
            cx.set(a, tag, Local::int32(7), true)?;
            assert_eq!(text(cx, a.value()), "[object Array]");
            let empty = cx.new_string("")?;
            cx.set(a, tag, empty, true)?;
            assert_eq!(text(cx, a.value()), "[object ]");
            // A primitive: Get(ToObject(v), @@toStringTag) sees its prototype's tag.
            let np = cx.intrinsic(IntrinsicId::NumberPrototype);
            let num = cx.new_string("Num")?;
            cx.set(np, tag, num, true)?;
            assert_eq!(text(cx, Local::int32(1)), "[object Num]");
            cx.delete(np, tag)?;
            assert_eq!(text(cx, Local::int32(1)), "[object Number]");
            // The iterator prototypes' tags.
            let ap = cx.intrinsic(IntrinsicId::ArrayPrototype);
            let values = cx.get(ap, "values")?;
            let it = cx.call(values, a.value(), &[])?;
            assert_eq!(text(cx, it), "[object Array Iterator]");
            let s = cx.new_string("s")?;
            let sp = cx.intrinsic(IntrinsicId::StringPrototype);
            let iter = well_known(cx, WellKnownSymbol::Iterator);
            let si = cx.get(sp, iter)?;
            let it = cx.call(si, s.value(), &[])?;
            assert_eq!(text(cx, it), "[object String Iterator]");
            let plain = cx.new_object();
            assert_eq!(text(cx, plain.value()), "[object Object]");
            Ok(())
        })
        .unwrap();
    });
}
