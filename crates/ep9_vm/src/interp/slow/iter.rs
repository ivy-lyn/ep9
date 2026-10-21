//! Iteration, spread, the arguments object and for-in.

use ep9_value::Throw;

use crate::conv::describe;
use crate::error::{FrameRef, throw_error};
use crate::frame::UNDEFINED;
use crate::interp::Cx;
use crate::object::{self, is_array, is_object, is_string};
use crate::property;
use crate::runtime::{ErrorKind, Root, Runtime, val};

use super::{enter, info, ret, throw_msg_now};

/// `CreateArrayFromIterable` (v0: arrays and strings, the only iterables without symbols).
#[cold]
#[inline(never)]
pub(crate) fn array_from_iterable(cx: &mut Cx, fp: *mut u64, pc: *const u8, v: u64) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if is_string(v) {
        let a = object::new_array(rt, 0);
        push_code_points(rt, a, 0, v);
        return a;
    }
    if !is_array(v) {
        return ret(Err(not_iterable(cx, v)));
    }
    let n = object::array_len(v);
    let a = object::new_array(rt, n as usize);
    for i in 0..n {
        object::array_set(rt, a, i, object::array_get(v, i).unwrap_or(UNDEFINED));
    }
    object::array_set_length(rt, a, n);
    a
}

/// The TypeError of GetIterator for a value without `@@iterator` (v0: not an array or string).
pub(super) fn not_iterable(cx: &mut Cx, v: u64) -> Throw {
    let msg = format!("{} is not iterable", describe(v));
    throw_error(cx, ErrorKind::TypeError, &msg)
}

/// Appends the code points of the string `s` (one string each, as the string iterator yields
/// them, §22.1.5.2.1) to the array `out` from index `start`. `out` must be rooted by the
/// caller (a Rust local). Returns the new length.
pub(super) fn push_code_points(rt: &Runtime, out: u64, start: u32, s: u64) -> u32 {
    let units: Vec<u16> = object::str_view(s).to_utf16();
    let mut n = start;
    let mut i = 0;
    while i < units.len() {
        let pair = (0xD800..0xDC00).contains(&units[i])
            && i + 1 < units.len()
            && (0xDC00..0xE000).contains(&units[i + 1]);
        let len = if pair { 2 } else { 1 };
        let cp = object::new_string_utf16(rt, &units[i..i + len]);
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

/// `GetIterator(obj)` (§7.4.3) for v0: the built-in iterator of an array or a string.
#[cold]
#[inline(never)]
pub(crate) fn get_iterator(cx: &mut Cx, fp: *mut u64, pc: *const u8, o: u64) -> u64 {
    enter(cx, fp, pc);
    let rt = cx.rt();
    if is_array(o) || is_string(o) {
        return object::new_iterator(rt, rt.root(Root::ArrayIteratorPrototype), o);
    }
    let msg = format!("{} is not iterable", describe(o));
    ret(Err(throw_error(cx, ErrorKind::TypeError, &msg)))
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
