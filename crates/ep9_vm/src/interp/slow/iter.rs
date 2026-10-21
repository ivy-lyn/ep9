//! Iteration, spread, the arguments object and for-in.
//!
//! ```text
//! GetIterator(obj) (0x8f: for-of, array destructuring, spread elements after the first)
//!   method = GetMethod(obj, @@iterator)       undefined, null or not callable -> TypeError
//!                                             "x is not iterable"
//!   method is %Array.prototype.values%, obj an object        -> a new array ITERATOR cell
//!   method is %String.prototype[@@iterator]%, obj a string   -> a new string ITERATOR cell
//!   else iterator = Call(method, obj)         not an object -> TypeError
//!   (BytecodeGen then loads `next` and calls it each step)
//! CreateArrayFromIterable(obj) (0x6f: `[...obj]`)
//!   the same method lookup; while `next` of the iterator's prototype is also the intrinsic
//!   (an own data property): an Array is copied element by element (holes read through
//!   [[Get]]), a string is split into code points; else the generic IteratorStep loop
//! ```
//!
//! The fast paths give the same values as the protocol: the cells are what `values` and
//! `[@@iterator]` return, and the copy loops are the steps of their `next` (natives.rs). The
//! intrinsic test (`builtins::existing_function`) runs once per GetIterator, not per step.

use ep9_value::Throw;

use crate::builtins::{self, BuiltinId, IntrinsicId};
use crate::call;
use crate::conv::{Res, describe, num, to_boolean};
use crate::error::{FrameRef, throw_error};
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::kinds::is_callable;
use crate::natives;
use crate::object::{self, is_array, is_object, is_string};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, Runtime, val};
use crate::symbol::{self, WellKnownSymbol};

use super::{enter, info, interrupt, ret, throw_msg_now};

/// `CreateArrayFromIterable` (0x6f): `[...v]`.
#[cold]
#[inline(never)]
pub(crate) fn array_from_iterable(cx: &mut Cx, fp: *mut u64, pc: *const u8, v: u64) -> u64 {
    enter(cx, fp, pc);
    ret(iterable_to_array(cx, v))
}

/// The TypeError of GetIterator for a value without a callable `@@iterator`.
fn not_iterable(cx: &mut Cx, v: u64) -> Throw {
    let msg = format!("{} is not iterable", describe(v));
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// GetMethod(v, @@iterator) (§7.3.10) for GetIterator (§7.4.3 steps 1-3): the method, or the
/// TypeError "v is not iterable" for undefined and null (GetV) and when there is no callable
/// method. GC point (a getter).
fn iterator_method(cx: &mut Cx, v: u64) -> Res<u64> {
    if v == UNDEFINED || v == NULL {
        return Err(not_iterable(cx, v));
    }
    let key = symbol::well_known(cx.rt(), WellKnownSymbol::Iterator);
    let method = property::get(cx, v, key)?;
    if !is_callable(method) {
        return Err(not_iterable(cx, v));
    }
    Ok(method)
}

/// The built-in iterators the fast paths make without a call.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Builtin {
    /// `method` is %Array.prototype.values% and `v` is an object.
    Array,
    /// `method` is %String.prototype[@@iterator]% and `v` is a string primitive.
    String,
}

/// True if `f` is the function object of VM builtin `index` in this realm.
fn is_vm_builtin(rt: &Runtime, f: u64, index: u16) -> bool {
    builtins::existing_function(rt, BuiltinId::new(natives::VM_BLOCK, index)) == Some(f)
}

/// Which built-in iterator calling `method` on `v` makes, if the call can be skipped. Not a
/// GC point.
fn builtin_iterator(rt: &Runtime, v: u64, method: u64) -> Option<Builtin> {
    if is_object(v) && is_vm_builtin(rt, method, natives::vm::ARRAY_VALUES) {
        Some(Builtin::Array)
    } else if is_string(v) && is_vm_builtin(rt, method, natives::vm::STRING_ITERATOR) {
        Some(Builtin::String)
    } else {
        None
    }
}

/// GetIterator steps 4-5 with the method found: the iterator object. GC point.
fn iterator_from_method(cx: &mut Cx, v: u64, method: u64) -> Res<u64> {
    let rt = cx.rt();
    match builtin_iterator(rt, v, method) {
        Some(Builtin::Array) => return Ok(natives::new_array_iterator(rt, v)),
        Some(Builtin::String) => return Ok(natives::new_string_iterator(rt, v)),
        None => {}
    }
    let it = call::call_function(cx, method, v, &[])?;
    if !is_object(it) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Result of the Symbol.iterator method is not an object",
        ));
    }
    Ok(it)
}

/// `GetIterator(obj, sync)` (§7.4.3, 0x8f): the iterator object; BytecodeGen loads `next`
/// next.
#[cold]
#[inline(never)]
pub(crate) fn get_iterator(cx: &mut Cx, fp: *mut u64, pc: *const u8, o: u64) -> u64 {
    enter(cx, fp, pc);
    ret(iterator_method(cx, o).and_then(|m| iterator_from_method(cx, o, m)))
}

/// True if the own `next` of the iterator prototype `proto` is the data property that the
/// realm made: VM builtin `index`. Then the steps of that native can run without the calls.
fn next_is_intrinsic(rt: &Runtime, proto: u64, index: u16) -> bool {
    object::own_get(proto, rt.atom(Atom::Next)).is_some_and(|(f, _)| is_vm_builtin(rt, f, index))
}

/// IterableToList(v) (§7.4.15, with GetIterator) as a new Array: the values of
/// CreateArrayFromIterable (`[...v]`, and the spread argument of `f(...v)`, which BytecodeGen
/// lowers to it) and of a call spread operand that is not an Array (`slow::call`). GC point.
pub(super) fn iterable_to_array(cx: &mut Cx, v: u64) -> Res<u64> {
    let method = iterator_method(cx, v)?;
    let rt = cx.rt();
    match builtin_iterator(rt, v, method) {
        Some(Builtin::Array)
            if is_array(v)
                && next_is_intrinsic(
                    rt,
                    rt.root(Root::ArrayIteratorPrototype),
                    natives::vm::ITERATOR_NEXT,
                ) =>
        {
            return copy_array(cx, v);
        }
        Some(Builtin::String)
            if next_is_intrinsic(
                rt,
                builtins::intrinsic(rt, IntrinsicId::StringIteratorPrototype),
                natives::vm::STRING_ITERATOR_NEXT,
            ) =>
        {
            let a = object::new_array(rt, 0);
            push_string_code_points(cx, a, v);
            return Ok(a);
        }
        _ => {}
    }
    let it = iterator_from_method(cx, v, method)?;
    let next = property::get(cx, it, rt.atom(Atom::Next))?;
    // GC points below: `v`, `it`, `next`, `a`, `r` and `value` live in Rust locals.
    let a = object::new_array(rt, 0);
    let mut n = 0u32;
    loop {
        // A `next` that is a host function never polls: count the step here.
        cx.budget -= 1;
        if cx.budget <= 0 && interrupt(cx, core::ptr::null_mut()) == 0 {
            return Err(Throw);
        }
        // IteratorStepValue (§7.4.10).
        let r = call::call_function(cx, next, it, &[])?;
        if !is_object(r) {
            let msg = format!("Iterator result {} is not an object", describe(r));
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        let done = property::get(cx, r, rt.atom(Atom::Done))?;
        if to_boolean(done) {
            break;
        }
        let value = property::get(cx, r, rt.atom(Atom::ValueKey))?;
        if !object::array_set(rt, a, n, value) {
            return Err(throw_error(
                cx,
                ErrorKind::RangeError,
                "Invalid array length",
            ));
        }
        n += 1;
    }
    Ok(a)
}

/// The steps of %ArrayIteratorPrototype%.next over the Array `v`: element `i` while `i` is
/// below the length (read at each step), a hole through [[Get]]. GC point.
fn copy_array(cx: &mut Cx, v: u64) -> Res<u64> {
    let rt = cx.rt();
    let a = object::new_array(rt, object::array_len(v) as usize);
    let mut i = 0u32;
    while i < object::array_len(v) {
        let x = match object::array_get(v, i) {
            Some(x) => x,
            None => property::get_keyed(cx, v, num(f64::from(i)))?,
        };
        object::array_set(rt, a, i, x);
        i += 1;
    }
    object::array_set_length(rt, a, i);
    Ok(a)
}

/// Appends the code points of the string `s` to the array `out` (rooted by the caller, a Rust
/// local), one string each as %StringIteratorPrototype%.next yields them (single-character
/// atoms for Latin-1 units). Returns the new length.
fn push_string_code_points(cx: &mut Cx, out: u64, s: u64) -> u32 {
    let rt = cx.rt();
    let mut n = object::array_len(out);
    let mut i = 0;
    while let Some((cp, len)) = natives::code_point_string(cx, s, i) {
        object::array_set(rt, out, n, cp);
        n += 1;
        i += len;
    }
    object::array_set_length(rt, out, n);
    n
}

/// Keeps a cell in a stack slot until the end of the scope, so the conservative stack scan
/// sees it (for a cell that only a Rust `Vec` would hold otherwise).
pub(super) struct KeepAlive(pub(super) u64);

impl Drop for KeepAlive {
    fn drop(&mut self) {
        core::hint::black_box(&self.0);
    }
}

/// `CreateMappedArguments`, `CreateUnmappedArguments` (`rest = false`) and
/// `CreateRestParameter` (`rest = true`). v0: the arguments object is an Array, and mapped
/// arguments do not alias the parameters.
#[cold]
#[inline(never)]
pub(crate) fn arguments(cx: &mut Cx, fp: *mut u64, pc: *const u8, rest: bool) -> u64 {
    enter(cx, fp, pc);
    let f = FrameRef(fp);
    let argc = f.argc();
    let from = if rest { info(fp).parameter_count } else { 0 };
    let rt = cx.rt();
    let n = argc.saturating_sub(from);
    let a = object::new_array(rt, n as usize);
    for i in 0..n {
        object::array_set(rt, a, i, f.arg(from + i));
    }
    object::array_set_length(rt, a, n);
    a
}

// ---- for-in --------------------------------------------------------------------------------

/// `ForInEnumerate(receiver)`: the key array (v0: the cache type and the cache array are both
/// this array).
#[cold]
#[inline(never)]
pub(crate) fn for_in_enumerate(cx: &mut Cx, fp: *mut u64, pc: *const u8, receiver: u64) -> u64 {
    enter(cx, fp, pc);
    ret(property::for_in_keys(cx, receiver))
}

/// `ForInNext`: key `index` of the cache array, or undefined if the receiver lost it.
#[cold]
#[inline(never)]
pub(crate) fn for_in_next(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    receiver: u64,
    keys: u64,
    index: u64,
) -> u64 {
    enter(cx, fp, pc);
    let i = val(index).as_int32().unwrap_or(-1);
    if !is_array(keys) || i < 0 {
        return throw_msg_now(cx, "ForInNext state is broken");
    }
    let Some(k) = object::array_get(keys, i as u32) else {
        return UNDEFINED;
    };
    if !is_object(receiver) {
        return k;
    }
    ret(property::has(cx, receiver, k).map(|h| if h { k } else { UNDEFINED }))
}
