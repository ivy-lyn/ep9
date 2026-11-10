//! TypedArray TA-1 fixer r3 (design study, judge §4.5): safe calls of the builtin ABI cannot
//! write the internal slots of an ArrayBuffer, SharedArrayBuffer or DataView that
//! `abi/typed.rs` made (an engine object), and a slot path never takes a word of another cell
//! for a slots cell. Public API only, no `unsafe`, one isolate per case.
//!
//! Run these tests in debug AND with `--release`: the release build had no kind check in the
//! slot path (`object::slot_count`), so some of the old failures show only there.
//!
//! | Test | Path (safe calls only) | Before the fix |
//! |---|---|---|
//! | `a_promise_method_refuses_a_plain_object` | A2: a Promise method on a plain object whose second in-object slot holds an engine buffer | release: the buffer's class word changed |
//! | `the_slot_path_refuses_a_forged_chain` | a promise whose rejected-list entry is a plain object with a view in its second in-object slot, then "handle" | release: a write into the view's buffer |
//! | `a_forged_resolving_record_cannot_settle_an_engine_buffer` | N1: the record of `create_resolving_functions` gets a buffer for its promise (`set_slot`), then the resolve function runs | release: the byte length 8 became 1000000 |
//! | `a_forged_rejected_record_cannot_write_an_engine_buffer` | N2: a rejected promise's list entry gets a buffer for its promise, then `take_unhandled_rejections` | debug and release: slot 2 (the maximum) cleared |
//! | `the_promise_methods_cannot_write_an_engine_object` | A1: `fulfill_promise`, `reject_promise`, `perform_promise_then` on each kind of engine object | release: slots 0-4 written |
#![forbid(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use ep9_vm::{Cx, Isolate, IsolateConfig, JsObject, Local, ObjectClass};

/// The panic text of a slot access of a value that is not an object with internal slots.
const NOT_SLOTS: &str = "is not an object with internal slots";

/// The panic text of the Promise methods for an argument that is not a promise: their
/// `debug_assert!` (`abi/jobs.rs`) runs first in a debug build; a release build has only the
/// slot path check.
const NOT_SLOTS_OR_NOT_PROMISE: &str = if cfg!(debug_assertions) {
    "self.is_promise(p)"
} else {
    NOT_SLOTS
};

/// The panic text of the `object::slot_set` choke on an engine object.
const BRAND: &str = "belongs to abi/typed.rs";

/// The `debug_assert!` text of the Promise operations for a value that is not a promise: a
/// debug build stops there, before the choke.
const NOT_PROMISE: &str = "is_promise(";

/// `perform_promise_then` reads `[[PromiseState]]` (slot 0) before it writes: on an object
/// whose slot 0 is not an int32 it panics there, before any write.
const STATE: &str = "[[PromiseState]] is an int32";

fn isolate() -> Isolate {
    Isolate::new(IsolateConfig::default()).unwrap()
}

/// The panic text of `f`, or `None` if it returns.
fn panic_text(f: impl FnOnce()) -> Option<String> {
    let p = catch_unwind(AssertUnwindSafe(f)).err()?;
    Some(
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default(),
    )
}

/// Asserts that `t` is a panic whose text holds one of `want`.
#[track_caller]
fn assert_panic_any(t: Option<String>, want: &[&str], case: &str) {
    assert!(
        t.as_deref()
            .is_some_and(|t| want.iter().any(|w| t.contains(w))),
        "{case}: want a panic with one of {want:?}, got {t:?}"
    );
}

/// Asserts that `t` is a panic whose text holds `want`.
#[track_caller]
fn assert_panic(t: Option<String>, want: &str, case: &str) {
    assert!(
        t.as_deref().is_some_and(|t| t.contains(want)),
        "{case}: want a panic with {want:?}, got {t:?}"
    );
}

/// A2: `fulfill_promise`, `reject_promise` and `perform_promise_then` on `{a: 1, b: buf}`. The
/// object is not an object with internal slots, so the slot path panics before any write (a
/// debug build: the `debug_assert!` of the method, before it); the buffer keeps its class and
/// its length.
#[test]
fn a_promise_method_refuses_a_plain_object() {
    for op in 0..3 {
        let mut iso = isolate();
        iso.with_cx(|cx| {
            let buf = cx.allocate_array_buffer(Local::null(), 64, None)?;
            let o = cx.new_object();
            cx.set(o, "a", Local::number(1.0), true)?;
            cx.set(o, "b", buf, true)?;
            let sab = Local::int32(i32::from(ObjectClass::SharedArrayBuffer.id()));
            let t = panic_text(|| match op {
                0 => cx.fulfill_promise(o, sab),
                1 => cx.reject_promise(o, sab),
                _ => {
                    cx.perform_promise_then(o, Local::undefined(), Local::undefined(), None);
                }
            });
            assert_panic(t, NOT_SLOTS_OR_NOT_PROMISE, &format!("op {op}"));
            assert_eq!(cx.object_class(buf), Some(ObjectClass::ArrayBuffer));
            assert_eq!(cx.array_buffer_byte_length(buf)?, 64);
            assert!(!cx.is_detached_buffer(buf));
            Ok(())
        })
        .unwrap();
    }
}

/// The chain form (design study authority.md §1.2): a real rejected promise gets a plain object
/// `{a: 1, b: view}` as its rejected-list entry (slot 2), then `perform_promise_then` runs
/// "handle", which unlinks that entry through the slot path. It panics at the first slot read
/// of the entry; the view, its buffer and its bytes stay as they were.
#[test]
fn the_slot_path_refuses_a_forged_chain() {
    let mut iso = isolate();
    iso.with_cx(|cx| {
        let b = cx.allocate_array_buffer(Local::null(), 8, None)?;
        let v = cx.new_typed_array(Local::null(), 1, b, 0, None)?;
        for i in 0..8 {
            cx.set_value_in_buffer(b, i, 1, Local::int32(0x10 + i as i32), true)?;
        }
        let e = cx.new_object();
        cx.set(e, "a", Local::number(1.0), true)?;
        cx.set(e, "b", v, true)?;
        let p = cx.new_promise(Local::null());
        cx.reject_promise(p, Local::number(1.0));
        cx.set_slot(p, ObjectClass::Promise, 2, e)?;
        let t = panic_text(|| {
            cx.perform_promise_then(p, Local::undefined(), Local::undefined(), None);
        });
        assert_panic(t, NOT_SLOTS, "handle");
        assert_eq!(cx.typed_array_length(v)?, Some(8));
        for i in 0..8 {
            let x = cx.get(v, i.to_string().as_str())?;
            assert_eq!(x.as_number(), Some(f64::from(0x10 + i)), "element {i}");
        }
        assert_eq!(cx.array_buffer_byte_length(b)?, 8);
        assert!(cx.slot(b, ObjectClass::ArrayBuffer, 2)?.is_undefined());
        Ok(())
    })
    .unwrap();
}

/// N1 (design study brand.md §2): the resolving functions of a real promise share a record
/// (`closure_data`); `set_slot` makes its promise an engine buffer; the resolve function then
/// settles that buffer. The choke panics at the first slot write; the buffer keeps its
/// length (a debug build stops earlier, at the `debug_assert!` of the Promise operation).
#[test]
fn a_forged_resolving_record_cannot_settle_an_engine_buffer() {
    let want = if cfg!(debug_assertions) {
        NOT_PROMISE
    } else {
        BRAND
    };
    let mut iso = isolate();
    iso.with_cx(|cx| {
        let b = cx.allocate_array_buffer(Local::null(), 8, None)?;
        let p = cx.new_promise(Local::null());
        let (res, _rej) = cx.create_resolving_functions(p);
        let state = cx.closure_data(res).as_object().expect("a record");
        cx.set_slot(state, ObjectClass::PromiseReaction, 0, b)?;
        let g = cx.global();
        cx.set(g, "b", b, true)?;
        cx.set(g, "res", res, true)?;
        Ok(())
    })
    .unwrap();
    // The panic unwinds through the call of a builtin closure: this `with_cx` does not return.
    let t = panic_text(|| {
        let _ = iso.with_cx(|cx| {
            let g = cx.global();
            let res = cx.get(g, "res")?;
            cx.call(res, Local::undefined(), &[Local::number(1e6)])
                .map(|_| ())
        });
    });
    assert_panic(t, want, "resolve");
    iso.with_cx(|cx| {
        let g = cx.global();
        let b = cx.get(g, "b")?;
        assert_eq!(cx.array_buffer_byte_length(b)?, 8);
        let past = cx.get_value_from_buffer(b, 1000, 1, true).map(|_| ());
        let e = cx.catch_exception(past.unwrap_err()).unwrap();
        let name = cx.get(e.as_object().expect("an error"), "name")?;
        assert_eq!(cx.to_rust_string(name)?, "RangeError");
        Ok(())
    })
    .unwrap();
}

/// N2 (design study brand.md §2): a rejected promise's entry in the rejected list (`slot(p,
/// Promise, 2)`) gets an engine buffer for its promise (`set_slot`); then
/// `take_unhandled_rejections` clears slot 2 of "its promise". The choke panics there (no
/// `debug_assert!` is on this path, so in both builds); slot 2 (the maximum) stays 64 and the
/// buffer still resizes.
#[test]
fn a_forged_rejected_record_cannot_write_an_engine_buffer() {
    let mut iso = isolate();
    iso.with_cx(|cx| {
        let b = cx.allocate_array_buffer(Local::null(), 8, Some(64))?;
        let p = cx.new_promise(Local::null());
        cx.reject_promise(p, Local::number(1.0));
        let e = cx
            .slot(p, ObjectClass::Promise, 2)?
            .as_object()
            .expect("an entry");
        cx.set_slot(e, ObjectClass::PromiseReaction, 0, b)?;
        let g = cx.global();
        cx.set(g, "b", b, true)?;
        Ok(())
    })
    .unwrap();
    let t = panic_text(|| {
        let _ = iso.take_unhandled_rejections();
    });
    assert_panic(t, BRAND, "take_unhandled_rejections");
    iso.with_cx(|cx| {
        let g = cx.global();
        let b = cx.get(g, "b")?;
        let max = cx.slot(b, ObjectClass::ArrayBuffer, 2)?;
        assert_eq!(max.as_number(), Some(64.0));
        cx.array_buffer_resize(b, 16)?;
        assert_eq!(cx.array_buffer_byte_length(b)?, 16);
        Ok(())
    })
    .unwrap();
}

/// Every byte length, readable slot (buffer 1-5, DataView 0-2) and byte of `bufs` and `dv`.
fn snapshot<'s>(
    cx: &mut Cx<'s>,
    bufs: [(Local<'s, JsObject>, ObjectClass); 4],
    dv: Local<'s, JsObject>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (b, class) in bufs {
        out.push(format!("{:?}", cx.array_buffer_byte_length(b)));
        for i in 1..=5 {
            out.push(format!(
                "{:?}",
                cx.slot(b, class, i).map(|x| format!("{x:?}"))
            ));
        }
        let len = cx.array_buffer_byte_length(b).unwrap_or(0);
        for i in 0..len {
            let x = cx.get_value_from_buffer(b, i, 1, true);
            out.push(format!("{:?}", x.map(|x| x.as_number())));
        }
    }
    for i in 0..=2 {
        let x = cx.slot(dv, ObjectClass::DataView, i);
        out.push(format!("{:?}", x.map(|x| format!("{x:?}"))));
    }
    out
}

/// A1 (check.md): `fulfill_promise`, `reject_promise` and `perform_promise_then` on a fixed
/// buffer with a view, a zero-length buffer, a resizable buffer, a growable SAB and a
/// DataView. Each call panics before it writes a slot: at the choke, or where
/// `perform_promise_then` reads a `[[PromiseState]]` that is not an int32 (a debug build: at
/// the `debug_assert!` of the operation). After each call every byte length, readable slot and
/// byte is as before, also after a minor GC, and after a full GC the view still reads its
/// bytes.
#[test]
fn the_promise_methods_cannot_write_an_engine_object() {
    let want: &[&str] = if cfg!(debug_assertions) {
        &[NOT_PROMISE]
    } else {
        &[BRAND, STATE]
    };
    for which in 0..5 {
        for op in 0..3 {
            let mut iso = isolate();
            let minors = iso.gc_stats().minor_collections;
            iso.with_cx(|cx| {
                let fixed = cx.allocate_array_buffer(Local::null(), 8, None)?;
                let v = cx.new_typed_array(Local::null(), 1, fixed, 0, None)?;
                let empty = cx.allocate_array_buffer(Local::null(), 0, None)?;
                let rab = cx.allocate_array_buffer(Local::null(), 8, Some(64))?;
                let sab = cx.allocate_shared_array_buffer(Local::null(), 8, Some(64))?;
                for b in [fixed, rab, sab] {
                    for i in 0..8 {
                        cx.set_value_in_buffer(b, i, 1, Local::int32(0x20 + i as i32), true)?;
                    }
                }
                let dv = cx.new_data_view(Local::null(), fixed, 0, Some(8))?;
                let bufs = [
                    (fixed, ObjectClass::ArrayBuffer),
                    (empty, ObjectClass::ArrayBuffer),
                    (rab, ObjectClass::ArrayBuffer),
                    (sab, ObjectClass::SharedArrayBuffer),
                ];
                let before = snapshot(cx, bufs, dv);
                let o = [fixed, empty, rab, sab, dv][which];
                let x = Local::number(1e6);
                let t = panic_text(|| match op {
                    0 => cx.fulfill_promise(o, x),
                    1 => cx.reject_promise(o, x),
                    _ => {
                        cx.perform_promise_then(o, Local::undefined(), Local::undefined(), None);
                    }
                });
                assert_panic_any(t, want, &format!("object {which}, op {op}"));
                assert_eq!(snapshot(cx, bufs, dv), before, "object {which}, op {op}");
                let g = cx.global();
                cx.set(g, "v", v, true)?;
                // A minor GC (the default heap is generational): the young buffers, view and
                // DataView become old and keep their slots and bytes.
                cx.collect_garbage_minor();
                assert_eq!(
                    snapshot(cx, bufs, dv),
                    before,
                    "object {which}, op {op}, minor"
                );
                Ok(())
            })
            .unwrap();
            assert!(
                iso.gc_stats().minor_collections > minors,
                "the minor GC ran"
            );
            iso.collect_garbage();
            iso.with_cx(|cx| {
                let g = cx.global();
                let v = cx.get(g, "v")?.as_object().expect("the view");
                assert_eq!(cx.typed_array_length(v)?, Some(8));
                for i in 0..8 {
                    let x = cx.get(v, i.to_string().as_str())?;
                    assert_eq!(x.as_number(), Some(f64::from(0x20 + i)), "element {i}");
                }
                Ok(())
            })
            .unwrap();
            iso.verify_heap().unwrap();
        }
    }
}
