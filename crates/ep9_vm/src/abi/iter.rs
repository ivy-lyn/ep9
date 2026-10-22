//! The iterator protocol on the builtin ABI (H20 ABI-3): `get_iterator`,
//! `get_iterator_from_method`, `iterator_step_value` and `iterator_close` over an
//! [`IteratorRecord`].
//!
//! ```text
//! let mut rec = cx.get_iterator(iterable)?;          GetIterator(obj, sync)
//! while let Some(v) = cx.iterator_step_value(&mut rec)? {   IteratorStepValue: done -> None
//!     if let Err(t) = work(cx, v) {                  an abrupt completion of the loop body
//!         return cx.iterator_close(&rec, Err(t));    IteratorClose: calls `return`, keeps `t`
//!     }
//! }
//! ```
//!
//! The record holds `Local`s, so it is stack-only (H20 R2): `cargo xtask gc-lint` derives
//! that from its fields, and a `Vec<IteratorRecord>` is a finding. These are the spec steps
//! over ObjectOps; the interpreter's own GetIterator (`interp/slow/iter.rs`) adds fast paths
//! for the built-in array and string iterators, which give the same values.

use ep9_value::{JsResult, Throw};

use super::{Cx, JsObject, Local};
use crate::conv::describe;
use crate::runtime::Atom;
use crate::symbol::{self, WellKnownSymbol};
use crate::{interp, property};

/// An Iterator Record (§7.4.1): the iterator object, its `next` method and whether it is done.
/// Stack-only (H20 R2): keep it in a Rust local, never in Rust heap memory.
#[derive(Debug)]
pub struct IteratorRecord<'s> {
    iterator: Local<'s, JsObject>,
    next_method: Local<'s>,
    done: bool,
}

impl<'s> IteratorRecord<'s> {
    /// `[[Iterator]]`.
    pub fn iterator(&self) -> Local<'s, JsObject> {
        self.iterator
    }

    /// `[[NextMethod]]`: the value `next` had when the iterator was made (any value).
    pub fn next_method(&self) -> Local<'s> {
        self.next_method
    }

    /// `[[Done]]`: true once a step reported done or threw.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

impl<'s> Cx<'s> {
    /// GetIterator(obj, sync) (§7.4.4): GetMethod(obj, @@iterator), then
    /// [`Cx::get_iterator_from_method`]. TypeError "x is not iterable" for undefined, null and
    /// a value whose `@@iterator` is not callable.
    pub fn get_iterator(&mut self, obj: impl Into<Local<'s>>) -> JsResult<IteratorRecord<'s>> {
        let obj = obj.into();
        // 2.a. Let method be ? GetMethod(obj, %Symbol.iterator%) (GetV throws for undefined
        // and null; the message is the one of the interpreter's GetIterator).
        if obj.is_undefined_or_null() {
            return Err(self.not_iterable(obj));
        }
        let key = symbol::well_known(self.rt(), WellKnownSymbol::Iterator);
        let method: Local<'s> = Self::local(property::get(self.cx, obj.bits(), key)?);
        // 3. If method is undefined, throw a TypeError exception. (GetMethod step 3: a
        // non-callable method is a TypeError too.)
        if !method.is_callable() {
            return Err(self.not_iterable(obj));
        }
        // 4. Return ? GetIteratorFromMethod(obj, method).
        self.get_iterator_from_method(obj, method)
    }

    /// GetIteratorFromMethod(obj, method) (§7.4.3): calls `method` with `obj` as `this`; the
    /// result must be an object, whose `next` the record keeps.
    pub fn get_iterator_from_method(
        &mut self,
        obj: impl Into<Local<'s>>,
        method: impl Into<Local<'s>>,
    ) -> JsResult<IteratorRecord<'s>> {
        // 1. Let iterator be ? Call(method, obj).
        let iterator = self.call(method, obj, &[])?;
        // 2. If iterator is not an Object, throw a TypeError exception.
        let Some(iterator) = iterator.as_object() else {
            return Err(
                self.throw_type_error("Result of the Symbol.iterator method is not an object")
            );
        };
        // 3. Let nextMethod be ? Get(iterator, "next").
        let next = self.atom(Atom::Next);
        let next_method = self.get(iterator, next)?;
        // 4-5. The record, [[Done]] false.
        Ok(IteratorRecord {
            iterator,
            next_method,
            done: false,
        })
    }

    /// IteratorStepValue(iteratorRecord) (§7.4.10): the next value, or `None` once the
    /// iterator is done. Any throw (from `next`, a result that is not an object, its `done` or
    /// `value` getter) sets `[[Done]]`, so the caller must not close the iterator then. A
    /// record that is already done gives `None` without a call. Each step takes one interrupt
    /// budget unit: a host `next` never polls.
    pub fn iterator_step_value(
        &mut self,
        rec: &mut IteratorRecord<'s>,
    ) -> JsResult<Option<Local<'s>>> {
        if rec.done {
            return Ok(None);
        }
        self.cx.budget -= 1;
        if self.cx.budget <= 0 && interp::slow::interrupt(self.cx, core::ptr::null_mut()) == 0 {
            rec.done = true;
            return Err(Throw);
        }
        let r = self.step(rec);
        if !matches!(r, Ok(Some(_))) {
            rec.done = true;
        }
        r
    }

    /// IteratorStepValue steps 1-6 (IteratorNext, IteratorComplete, IteratorValue); the
    /// caller sets `[[Done]]` on a throw or done.
    fn step(&mut self, rec: &IteratorRecord<'s>) -> JsResult<Option<Local<'s>>> {
        // IteratorNext: Call(nextMethod, iterator); the result must be an object.
        let result = self.call(rec.next_method, rec.iterator, &[])?;
        let Some(result) = result.as_object() else {
            let msg = format!(
                "Iterator result {} is not an object",
                describe(result.bits())
            );
            return Err(self.throw_type_error(&msg));
        };
        // IteratorComplete: ToBoolean(? Get(iterResult, "done")).
        let done = self.atom(Atom::Done);
        let done = self.get(result, done)?;
        if self.to_boolean(done) {
            return Ok(None);
        }
        // IteratorValue: ? Get(iterResult, "value").
        let value = self.atom(Atom::ValueKey);
        self.get(result, value).map(Some)
    }

    /// IteratorClose(iteratorRecord, completion) (§7.4.11): calls the iterator's `return`
    /// method, if any, and gives `completion` back. A throw completion (`Err`, with its
    /// exception pending) wins over anything `return` does; for a normal one, a throw from
    /// `return` or a result that is not an object is the result. While the isolate terminates,
    /// a throw completion passes on without running `return`.
    pub fn iterator_close<T>(
        &mut self,
        rec: &IteratorRecord<'s>,
        completion: JsResult<T>,
    ) -> JsResult<T> {
        let rt = self.rt();
        if completion.is_err() && rt.terminating.get() {
            return completion;
        }
        // The pending exception of a throw completion waits in a Rust local (a root by the
        // stack scan) while `return` runs, which needs no pending exception (R4).
        let saved = if completion.is_err() {
            rt.take_pending()
        } else {
            0
        };
        // 3-4. innerResult = Completion(GetMethod(iterator, "return")), then its call.
        let inner = self.call_return(rec.iterator);
        // 5. If completion is a throw completion, return ? completion.
        if completion.is_err() {
            if inner.is_err() {
                rt.take_pending();
            }
            rt.throw_value(saved);
            return completion;
        }
        // 6. If innerResult is a throw completion, return ? innerResult.
        // 7. If innerResult.[[Value]] is not an Object, throw a TypeError exception.
        match inner? {
            None => {}
            Some(r) if r.is_object() => {}
            Some(r) => {
                let msg = format!("Iterator result {} is not an object", describe(r.bits()));
                return Err(self.throw_type_error(&msg));
            }
        }
        // 8. Return ? completion.
        completion
    }

    /// GetMethod(iterator, "return") (§7.3.10, as `interp/slow/iter.rs` cites it) and, if there is one, its call: `None` if the
    /// method is undefined or null (IteratorClose step 4.b).
    fn call_return(&mut self, iterator: Local<'s, JsObject>) -> JsResult<Option<Local<'s>>> {
        let key = self.atom(Atom::Return);
        let method = self.get(iterator, key)?;
        if method.is_undefined_or_null() {
            return Ok(None);
        }
        if !method.is_callable() {
            let msg = format!("{} is not a function", describe(method.bits()));
            return Err(self.throw_type_error(&msg));
        }
        self.call(method, iterator, &[]).map(Some)
    }

    /// The TypeError of GetIterator for a value without a callable `@@iterator`.
    fn not_iterable(&mut self, v: Local<'s>) -> Throw {
        let msg = format!("{} is not iterable", describe(v.bits()));
        self.throw_type_error(&msg)
    }
}
