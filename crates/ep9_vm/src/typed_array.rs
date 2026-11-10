//! TypedArray view cells and the TypedArray exotic object (ECMA-262 §10.4.5, §23.2; TypedArray
//! TA-1 TA-VM, design `docs/plan/notes/typedarray-design.md` §2.2, §3.2, §4): making a view,
//! IsTypedArrayOutOfBounds and TypedArrayLength (with length-tracking views),
//! IsValidIntegerIndex, TypedArrayGetElement and TypedArraySetElement for the 12 element
//! types, CanonicalNumericIndexString, and the internal methods of the view kinds that
//! `exotic.rs` and `property.rs` route here.
//!
//! ```text
//! view cell (kinds 29-40; element type = kind - 29, spec Table 71 order)
//!   +0 word0 (shape: elements kind 7 TYPED_FIXED or 8 TYPED_VARIABLE, set at birth, kept)
//!   +8 SLOTS / PROPS   +16 ELEMENTS: always undefined
//!   +24 buffer (strong)  +32 data: ExternalRef<u8> = store base + byteOffset (null: detached)
//!   +40 length: int32 / double, the int32 -1 for auto; 0 once a TYPED_FIXED view is detached
//!   +48 byteOffset  +56 in-object property slots (none yet: TA-1 makes views with 0)
//!
//! key on a view ── CanonicalNumericIndexString(key) ── none ──> ordinary (props, then the chain)
//!                                                    └ n ────> IsValidIntegerIndex(view, n)
//!   [[Get]] element or undefined   [[Set]] TypedArraySetElement (receiver = view) / true
//!   [[HasProperty]] valid?         [[GetOwnProperty]] {v, w, e, c} or none   [[Delete]] !valid
//!   [[DefineOwnProperty]] valid and {w, e, c}-compatible   no prototype walk on any of them
//! ```
//!
//! | Rule | Statement |
//! |---|---|
//! | Elements kind | 7 when IsTypedArrayFixedLength (a fixed length on a fixed-length ArrayBuffer or on any SharedArrayBuffer), else 8. A view keeps it: no store, delete, define or freeze reaches `elements::transition` for a view (the view keys never reach the element paths: needs N4) |
//! | Bound | TYPED_FIXED: the +40 length (0 once detached). TYPED_VARIABLE: from the buffer's byte length at each access, so a resize needs no view write |
//! | Conversion | `[[Set]]` converts (ToNumber, or ToBigInt for BigInt64 and BigUint64) before the bound check; the conversion may detach or resize, and then nothing is stored |
//! | NaN | a store writes one NaN pattern per float type (`array_buffer::number_to_raw`) |
//! | GC | the common object trace (`object::register_kinds`, 4 fixed fields); `data` is a native word that the GC skips; the buffer and its view list are in `array_buffer.rs` |
//! | Policy (M-R9) | nothing here reads `ExecPolicy`, `ExecOptions` or the code set |

use ep9_heap::KindId;

use crate::array_buffer::{self as ab, BufResult, Fail, elem_size, is_bigint_elem};
use crate::conv::{self, Res};
use crate::descriptor::Desc;
use crate::elements::kind as ek;
use crate::external_ref::ExternalRef;
use crate::frame::{UNDEFINED, int32_bits};
use crate::interp::Cx;
use crate::kinds;
use crate::layout;
use crate::object::{self, attr, cell, init, obj, rd, wr};
use crate::runtime::{Runtime, val};
use crate::{shape, string};

/// The fixed fields of a view cell: buffer, data, length, byteOffset (`layout::VIEW_*`).
pub(crate) const VIEW_FIXED: usize = 4;
const _: () = assert!(layout::VIEW_FIELDS as usize == obj::FIXED + 8 * VIEW_FIXED);

/// The in-object slot count of a new view (TA-1: 0, so a named property goes out of line and
/// no in-object field offset depends on `object::fixed_count` of a view kind).
const VIEW_CAPACITY: u8 = 0;

/// The element type (spec Table 71 row) of the view `o`.
#[inline(always)]
pub(crate) fn element_type(o: u64) -> u8 {
    kinds::cell_kind(o) - kinds::INT8_VIEW.get()
}

/// `[[ViewedArrayBuffer]]` of the view `o`.
#[inline(always)]
pub(crate) fn buffer_of(o: u64) -> u64 {
    rd(cell(o), layout::VIEW_BUFFER as usize)
}

/// The data address of the view `o` (+32): its buffer's store + its byte offset; null once
/// detached or for an empty store.
#[inline(always)]
fn data_of(o: u64) -> *mut u8 {
    ExternalRef::<u8>::from_word(rd(cell(o), layout::VIEW_DATA as usize))
        .get()
        .cast_mut()
}

/// `[[ByteOffset]]` of the view `o` (as stored; also when it is out of bounds).
#[inline]
pub(crate) fn byte_offset(o: u64) -> u64 {
    val(rd(cell(o), layout::VIEW_BYTE_OFFSET as usize))
        .as_number()
        .expect("a view's byte offset") as u64
}

/// `[[ArrayLength]]` of the view `o`: `Some(n)`, or `None` for auto. A detached TYPED_FIXED
/// view reads 0 (its bound, design §5.2).
#[inline]
pub(crate) fn fixed_length(o: u64) -> Option<u64> {
    let v = val(rd(cell(o), layout::VIEW_LENGTH as usize));
    if v.as_int32() == Some(layout::VIEW_LENGTH_AUTO) {
        return None;
    }
    Some(v.as_number().expect("a view's length") as u64)
}

/// The Value of a length or offset field.
fn length_value(n: u64) -> u64 {
    match i32::try_from(n) {
        Ok(i) => int32_bits(i),
        Err(_) => conv::num(n as f64),
    }
}

/// True if the view `o` has elements kind TYPED_FIXED (IsTypedArrayFixedLength).
#[inline]
pub(crate) fn is_fixed_length(o: u64) -> bool {
    crate::elements::kind_of(o) == ek::TYPED_FIXED
}

/// A new TypedArray of element type `t` with prototype `proto` (an object or null) on the
/// buffer `b` from `offset`: `Some(length)` elements; with `None`, a length-tracking view on a
/// resizable ArrayBuffer or a growable SharedArrayBuffer, and on a fixed-length buffer a
/// fixed-length view of (byte length - `offset`) / element size elements
/// (InitializeTypedArrayFromArrayBuffer steps 8 and 9.a). The checks of that operation that
/// keep memory safe or pick the length run here again: a TypeError if `b` is no buffer or is
/// detached; a RangeError for an offset that is not a multiple of the element size, a range
/// that is not inside the buffer, or (`None` on a fixed-length buffer) a byte length that is
/// not a multiple of the element size. GC point.
pub(crate) fn new_view(
    rt: &Runtime,
    proto: u64,
    t: u8,
    b: u64,
    offset: u64,
    length: Option<u64>,
) -> BufResult<u64> {
    assert!(
        usize::from(t) < usize::from(kinds::VIEW_KINDS),
        "element type {t}"
    );
    if ab::buffer_class(b).is_none() {
        return Err(Fail::Type(
            "The buffer is not an ArrayBuffer or a SharedArrayBuffer",
        ));
    }
    if ab::is_detached(b) {
        return Err(Fail::Type(
            "Cannot perform Construct on a detached ArrayBuffer",
        ));
    }
    let size = elem_size(t) as u64;
    if !offset.is_multiple_of(size) {
        return Err(Fail::Range(
            "The start offset is not a multiple of the element size",
        ));
    }
    let buf_len = ab::byte_length(rt, b) as u64;
    // Step 9.a: no length on a fixed-length buffer (IsFixedLengthArrayBuffer) computes it, so
    // the view is fixed-length; only a resizable or growable buffer gets an auto view (step 8).
    let length = match length {
        None if !ab::is_resizable(b) => {
            if !buf_len.is_multiple_of(size) {
                return Err(Fail::Range(
                    "The byte length of the buffer is not a multiple of the element size",
                ));
            }
            if offset > buf_len {
                return Err(Fail::Range(
                    "Start offset is outside the bounds of the buffer",
                ));
            }
            Some((buf_len - offset) / size)
        }
        l => l,
    };
    match length {
        Some(n)
            if n.checked_mul(size)
                .and_then(|x| x.checked_add(offset))
                .is_none_or(|e| e > buf_len) =>
        {
            return Err(Fail::Range("Invalid typed array length"));
        }
        None if offset > buf_len => {
            return Err(Fail::Range(
                "Start offset is outside the bounds of the buffer",
            ));
        }
        _ => {}
    }
    let fixed = length.is_some() && (ab::is_shared(b) || !ab::is_resizable(b));
    let ekind = if fixed {
        ek::TYPED_FIXED
    } else {
        ek::TYPED_VARIABLE
    };
    let k = KindId::new(kinds::INT8_VIEW.get() + t).expect("a view kind");
    // GC points: the shapes, then the cell; `proto` and `b` live in Rust locals (CSS).
    let s = shape::initial_shape(rt, proto, k, VIEW_CAPACITY);
    let s = shape::with_elements_kind(rt, s, ekind).expect("a root shape has no property");
    let c = rt.alloc(
        k,
        obj::FIXED + 8 * (VIEW_FIXED + usize::from(VIEW_CAPACITY)),
    );
    // The size function reads the in-object count: set it before the next GC point.
    // SAFETY: `c` is the new live cell.
    unsafe { c.header() }.set_flags(u16::from(VIEW_CAPACITY));
    let o = c.to_value().to_bits();
    shape::install(rt, o, s);
    // Not GC points: the stores below and `add_view`.
    let data = ab::view_data(rt, b, offset as usize);
    init(c, layout::VIEW_BUFFER as usize, b);
    init(
        c,
        layout::VIEW_DATA as usize,
        ExternalRef::new(data).to_word(),
    );
    let len = length.map_or(int32_bits(layout::VIEW_LENGTH_AUTO), length_value);
    init(c, layout::VIEW_LENGTH as usize, len);
    init(c, layout::VIEW_BYTE_OFFSET as usize, length_value(offset));
    ab::add_view(rt, b, o);
    Ok(o)
}

/// Step 2 of design §5.2 for the view `v`: a null data word, and length 0 for a TYPED_FIXED
/// view (a TYPED_VARIABLE view keeps its length field: its bound reads the buffer, whose byte
/// length is 0 once detached). Not a GC point.
pub(crate) fn detach_view(rt: &Runtime, v: u64) {
    let c = cell(v);
    let null = ExternalRef::<u8>::new(core::ptr::null());
    wr(
        &rt.heap,
        c,
        layout::VIEW_DATA as usize,
        ExternalRef::to_word(null),
    );
    if is_fixed_length(v) {
        wr(&rt.heap, c, layout::VIEW_LENGTH as usize, int32_bits(0));
    }
}

/// TypedArrayLength (§10.4.5.12) of MakeTypedArrayWithBufferWitnessRecord(`o`, seq-cst), or
/// `None` when IsTypedArrayOutOfBounds (§10.4.5.11; a detached buffer included). Not a GC
/// point.
pub(crate) fn length(rt: &Runtime, o: u64) -> Option<u64> {
    let b = buffer_of(o);
    if ab::is_detached(b) {
        return None;
    }
    let n = fixed_length(o);
    if is_fixed_length(o) {
        // A fixed length on a fixed-length ArrayBuffer (out of bounds only when detached) or on
        // a SharedArrayBuffer (it only grows).
        return n;
    }
    let size = elem_size(element_type(o)) as u64;
    let buf_len = ab::byte_length(rt, b) as u64;
    let start = byte_offset(o);
    match n {
        None if start > buf_len => None,
        None => Some((buf_len - start) / size),
        Some(n) if start + n * size > buf_len => None,
        Some(n) => Some(n),
    }
}

/// IsValidIntegerIndex (§10.4.5.14) of `index` on the view `o`: the element index, or `None`.
/// Not a GC point.
#[inline]
pub(crate) fn valid_index(rt: &Runtime, o: u64, index: f64) -> Option<usize> {
    // Not integral, negative or -0 (NaN is no `Ge`).
    let not_ge_0 = !matches!(
        index.partial_cmp(&0.0),
        Some(core::cmp::Ordering::Greater | core::cmp::Ordering::Equal)
    );
    if not_ge_0 || index.fract() != 0.0 || (index == 0.0 && index.is_sign_negative()) {
        return None;
    }
    let len = length(rt, o)?;
    (index < len as f64).then_some(index as usize)
}

/// TypedArrayGetElement (§10.4.5.15): the element at `index`, or undefined. GC point (a
/// BigInt).
pub(crate) fn get_element(rt: &Runtime, o: u64, index: f64) -> u64 {
    let Some(i) = valid_index(rt, o, index) else {
        return UNDEFINED;
    };
    let t = element_type(o);
    let size = elem_size(t);
    let shared = ab::is_shared(buffer_of(o));
    // SAFETY: `i` is below the view's length, so [data + i * size, + size) is inside the live
    // store (the view holds its buffer, the buffer its store).
    let x = unsafe { ab::read_raw(data_of(o).wrapping_add(i * size), size, shared) };
    ab::raw_to_value(rt, t, x)
}

/// TypedArraySetElement (§10.4.5.16): ToNumber (ToBigInt for BigInt64 and BigUint64) of `v`,
/// then the store if `index` is still valid (the conversion can detach or shrink the buffer).
/// GC point (the conversion may run JS).
pub(crate) fn set_element(cx: &mut Cx, o: u64, index: f64, v: u64) -> Res<()> {
    let t = element_type(o);
    let x = if is_bigint_elem(t) {
        ab::bigint_to_raw(crate::bigint::to_bigint(cx, v)?)
    } else {
        ab::number_to_raw(t, conv::to_number(cx, v)?)
    };
    // GC point above: `o` lives in a Rust local (CSS).
    if let Some(i) = valid_index(cx.rt(), o, index) {
        let size = elem_size(t);
        let shared = ab::is_shared(buffer_of(o));
        // SAFETY: as `get_element`, checked after the conversion.
        unsafe { ab::write_raw(data_of(o).wrapping_add(i * size), size, x, shared) };
    }
    Ok(())
}

// ---- CanonicalNumericIndexString (§7.1.21) ------------------------------------------------

/// CanonicalNumericIndexString of the property key `key`: the Number `n` when `key` is a
/// String that is "-0" or ToString(ToNumber(`key`)), else `None` (a symbol, or another
/// string: "01", "1e3" and " 1" are ordinary keys; "-0", "1.5", "Infinity", "NaN" and "-1"
/// are numeric). An array index answers from the key's hash field. A key that cannot be
/// numeric by its first unit or its length (such as "length" or "foo") is refused with no
/// copy. Not a GC point.
pub(crate) fn canonical_numeric(key: u64) -> Option<f64> {
    if !kinds::is_string(key) {
        return None;
    }
    if let Some(i) = string::array_index(key) {
        return Some(f64::from(i));
    }
    string::with_text(key, |t| {
        // A canonical numeric string is "-0" or a Number::toString result: ASCII, at most
        // MAX_NUMBER_STRING units, and its first unit is a digit, '-', 'I' (Infinity) or 'N'
        // (NaN).
        let first = match t {
            string::JsStr::Latin1(b) => b.first().map(|&c| u16::from(c)),
            string::JsStr::Utf16(u) => u.first().copied(),
        };
        match first {
            Some(c) if c == u16::from(b'-') || c == u16::from(b'I') || c == u16::from(b'N') => {}
            Some(c) if (u16::from(b'0')..=u16::from(b'9')).contains(&c) => {}
            _ => return None,
        }
        let len = t.len();
        if len > MAX_NUMBER_STRING {
            return None;
        }
        let mut buf = [0u16; MAX_NUMBER_STRING];
        let units = &mut buf[..len];
        match t {
            string::JsStr::Latin1(b) => {
                for (u, &c) in units.iter_mut().zip(b) {
                    *u = u16::from(c);
                }
            }
            string::JsStr::Utf16(u) => units.copy_from_slice(u),
        }
        if units.iter().any(|&c| c > 0x7F) {
            return None;
        }
        if *units == [u16::from(b'-'), u16::from(b'0')] {
            return Some(-0.0);
        }
        let n = crate::numconv::string_to_number(units);
        let back = crate::numconv::number_to_string(n);
        (back.len() == units.len() && back.bytes().zip(&*units).all(|(a, &b)| u16::from(a) == b))
            .then_some(n)
    })
}

/// The longest Number::toString result (§6.1.6.1.20), in units: 25, as
/// "-0.0000012345678901234567" (a sign, "0.", 5 zeros and 17 digits). The exponent form is at
/// most 24 ("-1.2345678901234567e-308") and the integer form 22. A longer key is not
/// canonical numeric.
const MAX_NUMBER_STRING: usize = 25;

// ---- the internal methods of the view kinds (§10.4.5.1-7), routed from exotic.rs ---------

/// `[[Get]]` (§10.4.5.4) for a numeric key: `Some(element or undefined)`, with no prototype
/// walk; `None` for another key (OrdinaryGet). GC point (a BigInt).
pub(crate) fn get(rt: &Runtime, o: u64, key: u64) -> Option<u64> {
    let n = canonical_numeric(key)?;
    Some(get_element(rt, o, n))
}

/// `[[GetOwnProperty]]` (§10.4.5.1) for a numeric key: the element as {w, e, c}, or `None`
/// (no own property; the caller does not look in the storage, which never holds a numeric
/// key of a view). `None` also for another key: the ordinary storage answers. GC point (a
/// BigInt).
pub(crate) fn own_property(rt: &Runtime, o: u64, key: u64) -> Option<(u64, i32)> {
    let n = canonical_numeric(key)?;
    let v = get_element(rt, o, n);
    (v != UNDEFINED).then_some((v, attr::DEFAULT))
}

/// The attributes of the own element `key` of the view `o` ({w, e, c}), if `key` is a valid
/// numeric key. Not a GC point.
pub(crate) fn own_attrs(rt: &Runtime, o: u64, key: u64) -> Option<i32> {
    let n = canonical_numeric(key)?;
    valid_index(rt, o, n).map(|_| attr::DEFAULT)
}

/// `[[HasProperty]]` (§10.4.5.2) for a numeric key: IsValidIntegerIndex, with no prototype
/// walk; `None` for another key. Not a GC point.
pub(crate) fn has(rt: &Runtime, o: u64, key: u64) -> Option<bool> {
    let n = canonical_numeric(key)?;
    Some(valid_index(rt, o, n).is_some())
}

/// `[[Set]]` (§10.4.5.5) for a numeric key: with the view itself as the receiver,
/// TypedArraySetElement and true; with another receiver, true for an invalid index (nothing
/// is stored, §10.4.5.5 step 1.b.ii) and `None` (OrdinarySet from the view) for a valid one.
/// `None` for another key. GC point.
pub(crate) fn set(cx: &mut Cx, o: u64, key: u64, v: u64, receiver: u64) -> Option<Res<bool>> {
    let n = canonical_numeric(key)?;
    if receiver == o {
        return Some(set_element(cx, o, n, v).map(|()| true));
    }
    if valid_index(cx.rt(), o, n).is_none() {
        return Some(Ok(true));
    }
    None
}

/// `[[DefineOwnProperty]]` (§10.4.5.3) for a numeric key: false for an invalid index, a
/// non-configurable, non-enumerable, accessor or non-writable descriptor; else the value is
/// stored (TypedArraySetElement) and true. `None` for another key. GC point.
pub(crate) fn define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Option<Res<bool>> {
    let n = canonical_numeric(key)?;
    if valid_index(cx.rt(), o, n).is_none()
        || desc.configurable == Some(false)
        || desc.enumerable == Some(false)
        || desc.is_accessor()
        || desc.writable == Some(false)
    {
        return Some(Ok(false));
    }
    if let Some(v) = desc.value
        && let Err(e) = set_element(cx, o, n, v)
    {
        return Some(Err(e));
    }
    Some(Ok(true))
}

/// `[[Delete]]` (§10.4.5.6) for a numeric key: true only for an index that is not valid.
/// `None` for another key. Not a GC point.
pub(crate) fn delete(rt: &Runtime, o: u64, key: u64) -> Option<bool> {
    let n = canonical_numeric(key)?;
    Some(valid_index(rt, o, n).is_none())
}

/// `[[OwnPropertyKeys]]` (§10.4.5.7): the indices 0 .. TypedArrayLength (none when out of
/// bounds), then the stored string keys and symbols (OrdinaryOwnPropertyKeys: a view stores
/// no integer key). A new array. GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> u64 {
    let rt = cx.rt();
    let len = length(rt, o).unwrap_or(0);
    // GC: `keys` and `rest` are Rust locals (roots by the stack scan).
    let keys = object::new_array(rt, 0);
    let mut n = 0u32;
    for i in 0..len {
        let k = match u32::try_from(i) {
            Ok(i) if i != u32::MAX => crate::key::index_key(rt, i),
            _ => crate::key::number_key(rt, i as f64),
        };
        object::array_set(rt, keys, n, k);
        n += 1;
    }
    let rest = crate::property::ordinary_own_property_keys(cx, o);
    let rt = cx.rt();
    for j in 0..object::array_len(rest) {
        if let Some(k) = object::array_get(rest, j) {
            object::array_set(rt, keys, n, k);
            n += 1;
        }
    }
    keys
}
