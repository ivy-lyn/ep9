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
#![forbid(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use ep9_vm::{Isolate, IsolateConfig, Local, ObjectClass};

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
