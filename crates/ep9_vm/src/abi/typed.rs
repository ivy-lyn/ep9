#![allow(dead_code)] // m8-stub
//! ArrayBuffer, SharedArrayBuffer, TypedArrays, DataView and Atomics on the builtin ABI (H20
//! part 2; TypedArray workflow, design `docs/plan/notes/typedarray-design.md` §8.1). TA-0
//! TA-DECL declares the `Cx` method list with stub bodies (plan rule 4 form, `m8-stub`): each
//! method that can fail throws a TypeError "not built"; the two that never throw give the
//! answer for a value that is no buffer and no view ([`Cx::is_detached_buffer`] false,
//! [`Cx::typed_array_element`] `None`), which is every value before TA-1. TA-1 TA-VM
//! fills the bodies over `array_buffer.rs` and `typed_array.rs`; the builtins of TA-2 to TA-4
//! (`ep9_builtins` array_buffer.rs, data_view.rs, typed_array/, atomics.rs) call them.
//!
//! ```text
//! ArrayBuffer / SharedArrayBuffer: SLOT_OBJECT, ObjectClass::ArrayBuffer (5) / SharedArrayBuffer
//!   allocate_*  byte_length  detach  copy_and_detach  resize  grow  copy_buffer_bytes
//!   get_value_from_buffer / set_value_in_buffer           (GetValueFromBuffer, SetValueInBuffer)
//! TypedArray: a view kind 29-40 (kind - 29 = its element type, below)
//!   new_typed_array  typed_array_element  _buffer  _byte_offset  _fixed_length  _length
//! DataView: SLOT_OBJECT, ObjectClass::DataView         new_data_view (+ the buffer methods)
//! Atomics: atomic_load  atomic_store  atomic_read_modify_write  atomic_compare_exchange
//!          atomics_wait  atomics_wait_async  atomics_notify
//! ```
//!
//! **Element types.** An `element: u8` is the row of spec Table 71 ("The TypedArray
//! Constructors"), counted from 0, and the order of the view kind ids (`kinds::INT8_VIEW` + the
//! row; `layout::VIEW_ELEMENT_SIZES` gives each size):
//!
//! | Row | Element type | Size | Row | Element type | Size |
//! |---|---|---|---|---|---|
//! | 0 | Int8 | 1 | 6 | Uint32 | 4 |
//! | 1 | Uint8 | 1 | 7 | BigInt64 | 8 |
//! | 2 | Uint8Clamped | 1 | 8 | BigUint64 | 8 |
//! | 3 | Int16 | 2 | 9 | Float16 | 2 |
//! | 4 | Uint16 | 2 | 10 | Float32 | 4 |
//! | 5 | Int32 | 4 | 11 | Float64 | 8 |
//!
//! Rows 7 and 8 hold BigInts ([[ContentType]] bigint); the others hold Numbers. A row above 11
//! is a bug in the caller: the filled methods panic on it.
//!
//! **Read-modify-write ops** (`op: u8` of [`Cx::atomic_read_modify_write`]): 0 add, 1 and,
//! 2 exchange, 3 or, 4 sub, 5 xor (the Atomics functions of the same names).
//!
//! | Rule | Statement |
//! |---|---|
//! | Values are converted by the caller | No method here runs JS. The builtin does the spec's ToIndex, ToNumber, ToBigInt and species steps first, in spec order; a value argument is then a Number, or a BigInt for rows 7 and 8, else a TypeError |
//! | Memory safety does not trust the caller | A method checks its byte range against the buffer's current length before it reads or writes: a range past it (a builtin bug, or a buffer that a conversion detached or shrank) is a TypeError or a RangeError, never an access |
//! | GC (plan rule 8) | No Value lives in Rust memory: a buffer's store and view list are held through its slots and the isolate's buffer table (one weak source; design §5.1). R3 of the ABI holds: every `&mut` method may allocate |
//! | Shared memory | A SharedArrayBuffer's bytes are read and written with atomic operations only: unordered access is Relaxed, the Atomics methods are SeqCst (TA-D11) |

use ep9_value::{JsResult, Throw};

use super::{Cx, JsObject, Local};

/// The TypeError of a method whose body TA-1 TA-VM writes.
fn not_built(cx: &mut Cx<'_>, what: &str) -> Throw {
    cx.throw_type_error(&format!("{what}: not built yet (m8-stub, TypedArray TA-1)"))
}

impl<'s> Cx<'s> {
    // ---- ArrayBuffer and SharedArrayBuffer (§25.1, §25.2) ----

    /// AllocateArrayBuffer (sec-allocatearraybuffer) after OrdinaryCreateFromConstructor: a new
    /// ArrayBuffer with prototype `proto` (an object: the caller ran
    /// GetPrototypeFromConstructor) and `byte_length` zero bytes. With `Some(max)` it is
    /// resizable up to `max` bytes. A RangeError if `byte_length` is above `max` or the bytes
    /// cannot be allocated (CreateByteDataBlock).
    pub fn allocate_array_buffer(
        &mut self,
        _proto: impl Into<Local<'s>>,
        _byte_length: u64,
        _max_byte_length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "allocate_array_buffer"))
    }

    /// AllocateSharedArrayBuffer (sec-allocatesharedarraybuffer): a new SharedArrayBuffer with
    /// prototype `proto` and `byte_length` zero bytes, growable up to `max` bytes with
    /// `Some(max)`. A RangeError as [`Cx::allocate_array_buffer`].
    pub fn allocate_shared_array_buffer(
        &mut self,
        _proto: impl Into<Local<'s>>,
        _byte_length: u64,
        _max_byte_length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "allocate_shared_array_buffer"))
    }

    /// ArrayBufferByteLength (sec-arraybufferbytelength) of an ArrayBuffer or a
    /// SharedArrayBuffer (whose length is read with order seq-cst): 0 when detached. A TypeError
    /// if `buffer` is neither.
    pub fn array_buffer_byte_length(&mut self, _buffer: impl Into<Local<'s>>) -> JsResult<u64> {
        Err(not_built(self, "array_buffer_byte_length"))
    }

    /// IsDetachedBuffer (sec-isdetachedbuffer): true if `buffer` is a detached ArrayBuffer;
    /// false for any other value (a SharedArrayBuffer is never detached). Never throws.
    pub fn is_detached_buffer(&self, _buffer: impl Into<Local<'s>>) -> bool {
        false
    }

    /// DetachArrayBuffer (sec-detacharraybuffer): every view of `buffer` gets length 0 and no
    /// data (TA-D5), then the buffer drops its bytes and its byte length is 0. A TypeError if
    /// `buffer` is not an ArrayBuffer (a SharedArrayBuffer included) or if its
    /// `[[ArrayBufferDetachKey]]` is not `key` (SameValue). Detaching a detached buffer does
    /// nothing.
    pub fn detach_array_buffer(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _key: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        Err(not_built(self, "detach_array_buffer"))
    }

    /// ArrayBufferCopyAndDetach (sec-arraybuffercopyanddetach) after its ToIndex step, for
    /// `transfer` and `transferToFixedLength`: a new ArrayBuffer with prototype `proto` (the
    /// current realm's %ArrayBuffer.prototype%) and `new_byte_length` bytes, resizable with the
    /// old maximum if `preserve_resizability` and `buffer` is resizable; it takes the first
    /// min(old, new) bytes of `buffer` (the store itself when the length is the same), and
    /// `buffer` is detached. A TypeError if `buffer` is not an ArrayBuffer, is detached, or has
    /// a detach key that is not undefined; a RangeError if `new_byte_length` is above the new
    /// maximum.
    pub fn array_buffer_copy_and_detach(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _proto: impl Into<Local<'s>>,
        _new_byte_length: u64,
        _preserve_resizability: bool,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "array_buffer_copy_and_detach"))
    }

    /// The resize of `ArrayBuffer.prototype.resize` (sec-arraybuffer.prototype.resize) after
    /// its ToIndex step: the byte length of the resizable ArrayBuffer `buffer` becomes
    /// `new_byte_length`; new bytes are 0. The store keeps its base address, so the views keep
    /// their data address (design §5.2). A TypeError if `buffer` is not a resizable
    /// ArrayBuffer or is detached; a RangeError if `new_byte_length` is above its maximum.
    pub fn array_buffer_resize(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _new_byte_length: u64,
    ) -> JsResult<()> {
        Err(not_built(self, "array_buffer_resize"))
    }

    /// The grow of `SharedArrayBuffer.prototype.grow` (sec-sharedarraybuffer.prototype.grow)
    /// after its ToIndex step: the byte length of the growable SharedArrayBuffer `buffer`
    /// becomes `new_byte_length` (a SeqCst store in the shared block; new bytes are 0). A
    /// TypeError if `buffer` is not a growable SharedArrayBuffer; a RangeError if
    /// `new_byte_length` is below its byte length or above its maximum.
    pub fn shared_array_buffer_grow(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _new_byte_length: u64,
    ) -> JsResult<()> {
        Err(not_built(self, "shared_array_buffer_grow"))
    }

    /// CopyDataBlockBytes (sec-copydatablockbytes) on the stores of two buffers: `count` bytes
    /// of `from` at `from_index` to `to` at `to_index`. The buffers may be the same and the
    /// ranges may overlap (`copyWithin`): the bytes are copied as if through a temporary
    /// copy. A TypeError if either value is not a buffer or is detached, or a range is past
    /// its buffer's byte length (nothing is copied then).
    pub fn copy_buffer_bytes(
        &mut self,
        _to: impl Into<Local<'s>>,
        _to_index: u64,
        _from: impl Into<Local<'s>>,
        _from_index: u64,
        _count: u64,
    ) -> JsResult<()> {
        Err(not_built(self, "copy_buffer_bytes"))
    }

    /// GetValueFromBuffer (sec-getvaluefrombuffer) with order unordered: the element of type
    /// `element` (a Table 71 row, the module docs) at byte `byte_index` of `buffer`, little
    /// endian if `little_endian`: a Number, or a BigInt for rows 7 and 8. A TypeError if
    /// `buffer` is not a buffer, is detached, or the element is past its byte length.
    pub fn get_value_from_buffer(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _little_endian: bool,
    ) -> JsResult<Local<'s>> {
        Err(not_built(self, "get_value_from_buffer"))
    }

    /// SetValueInBuffer (sec-setvalueinbuffer) with order unordered: stores `value` (a Number,
    /// or a BigInt for rows 7 and 8: the caller converted it) as the element of type `element`
    /// at byte `byte_index` of `buffer` (NumericToRawBytes: Uint8Clamped rounds half to even
    /// and clamps; Float16 and Float32 round to nearest, ties to even; a NaN is one pattern). A
    /// TypeError as [`Cx::get_value_from_buffer`], or if `value` has the wrong type.
    pub fn set_value_in_buffer(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _value: impl Into<Local<'s>>,
        _little_endian: bool,
    ) -> JsResult<()> {
        Err(not_built(self, "set_value_in_buffer"))
    }

    // ---- TypedArrays (§23.2, §10.4.5) ----

    /// A new TypedArray of element type `element` with prototype `proto`, on `buffer` (an
    /// ArrayBuffer or a SharedArrayBuffer) from `byte_offset`: `Some(length)` elements, or a
    /// length-tracking view with `None` (`[[ArrayLength]]` auto). The caller ran the checks of
    /// InitializeTypedArrayFromArrayBuffer (sec-initializetypedarrayfromarraybuffer; the
    /// alignment of `byte_offset`, detach and bounds) in spec order; the view's elements kind
    /// is TYPED_FIXED or TYPED_VARIABLE by IsTypedArrayFixedLength (design §3.2). A TypeError if
    /// `buffer` is not a buffer or is detached; a RangeError if the range is not inside it.
    pub fn new_typed_array(
        &mut self,
        _proto: impl Into<Local<'s>>,
        _element: u8,
        _buffer: impl Into<Local<'s>>,
        _byte_offset: u64,
        _length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "new_typed_array"))
    }

    /// The element type (a Table 71 row) of `v` if it is a TypedArray (its
    /// `[[TypedArrayName]]`; rows 7 and 8 have `[[ContentType]]` bigint), else `None`. Never
    /// throws.
    pub fn typed_array_element(&self, _v: impl Into<Local<'s>>) -> Option<u8> {
        None
    }

    /// `[[ViewedArrayBuffer]]` of the TypedArray `ta` (also when it is detached). A TypeError if
    /// `ta` is not a TypedArray.
    pub fn typed_array_buffer(
        &mut self,
        _ta: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "typed_array_buffer"))
    }

    /// `[[ByteOffset]]` of the TypedArray `ta` as stored (the `byteOffset` getter gives 0 when
    /// `ta` is out of bounds: the caller tests [`Cx::typed_array_length`] first). A TypeError
    /// if `ta` is not a TypedArray.
    pub fn typed_array_byte_offset(&mut self, _ta: impl Into<Local<'s>>) -> JsResult<u64> {
        Err(not_built(self, "typed_array_byte_offset"))
    }

    /// `[[ArrayLength]]` of the TypedArray `ta`: `Some(n)`, or `None` when it is auto (a
    /// length-tracking view). A TypeError if `ta` is not a TypedArray.
    pub fn typed_array_fixed_length(&mut self, _ta: impl Into<Local<'s>>) -> JsResult<Option<u64>> {
        Err(not_built(self, "typed_array_fixed_length"))
    }

    /// TypedArrayLength (sec-typedarraylength) of MakeTypedArrayWithBufferWitnessRecord(`ta`,
    /// seq-cst): `Some(length)`, or `None` when IsTypedArrayOutOfBounds (a detached buffer
    /// included). A TypeError if `ta` is not a TypedArray.
    pub fn typed_array_length(&mut self, _ta: impl Into<Local<'s>>) -> JsResult<Option<u64>> {
        Err(not_built(self, "typed_array_length"))
    }

    // ---- DataView (§25.3) ----

    /// A new DataView with prototype `proto` on `buffer` from `byte_offset`: `Some(n)` bytes,
    /// or `None` for a length-tracking view (`[[ByteLength]]` auto). The caller ran the checks
    /// of the DataView constructor (sec-dataview-buffer-byteoffset-bytelength) in spec order. A
    /// TypeError if `buffer` is not a buffer or is detached; a RangeError if the range is not
    /// inside it.
    pub fn new_data_view(
        &mut self,
        _proto: impl Into<Local<'s>>,
        _buffer: impl Into<Local<'s>>,
        _byte_offset: u64,
        _byte_length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "new_data_view"))
    }

    // ---- Atomics (§25.4): `buffer` and `byte_index` are the caller's ValidateAtomicAccess
    // result, for an integer element type (rows 0-8 except 2), aligned ----

    /// GetValueFromBuffer with order seq-cst (`Atomics.load`, sec-atomics.load): the element as
    /// a Number, or a BigInt for rows 7 and 8. A TypeError as [`Cx::get_value_from_buffer`].
    pub fn atomic_load(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
    ) -> JsResult<Local<'s>> {
        Err(not_built(self, "atomic_load"))
    }

    /// SetValueInBuffer with order seq-cst (`Atomics.store`, sec-atomics.store) of the
    /// converted `value`. A TypeError as [`Cx::set_value_in_buffer`].
    pub fn atomic_store(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _value: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        Err(not_built(self, "atomic_store"))
    }

    /// GetModifySetValueInBuffer (sec-getmodifysetvalueinbuffer), seq-cst: the element becomes
    /// `old op value` for the read-modify-write op `op` (the module docs: 0 add, 1 and,
    /// 2 exchange, 3 or, 4 sub, 5 xor), in one atomic step; the result is the old element. A
    /// TypeError as [`Cx::set_value_in_buffer`].
    pub fn atomic_read_modify_write(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _op: u8,
        _value: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s>> {
        Err(not_built(self, "atomic_read_modify_write"))
    }

    /// The compare-exchange of `Atomics.compareExchange` (sec-atomics.compareexchange) after its
    /// conversions: in one seq-cst step, if the element's bytes equal the raw bytes of
    /// `expected`, the element becomes `replacement`; the result is the old element. A
    /// TypeError as [`Cx::set_value_in_buffer`].
    pub fn atomic_compare_exchange(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _expected: impl Into<Local<'s>>,
        _replacement: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s>> {
        Err(not_built(self, "atomic_compare_exchange"))
    }

    /// DoWait (sec-dowait) with mode sync after its conversions: if the Int32 or BigInt64
    /// element (row 5 or 7) of the SharedArrayBuffer `buffer` is `value`, the agent waits until
    /// a notify, `timeout_ms` milliseconds (at least 0; +∞ waits for good) or an isolate
    /// interrupt. The result is the String "ok", "not-equal" or "timed-out". A TypeError if the
    /// agent cannot suspend (AgentCanSuspend) or `buffer` is not shared.
    pub fn atomics_wait(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _value: impl Into<Local<'s>>,
        _timeout_ms: f64,
    ) -> JsResult<Local<'s>> {
        Err(not_built(self, "atomics_wait"))
    }

    /// DoWait with mode async (`Atomics.waitAsync`, sec-dowait) after its conversions: the
    /// result object `{ async, value }`, whose value is "not-equal", "timed-out" or a promise
    /// that a notify or the timeout resolves (a host task in this isolate's job queue). A
    /// TypeError if `buffer` is not shared.
    pub fn atomics_wait_async(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _element: u8,
        _value: impl Into<Local<'s>>,
        _timeout_ms: f64,
    ) -> JsResult<Local<'s, JsObject>> {
        Err(not_built(self, "atomics_wait_async"))
    }

    /// `Atomics.notify` (sec-atomics.notify) after its conversions: wakes at most `count`
    /// waiters (`u64::MAX`: all) on byte `byte_index` of `buffer`, first come first woken, and
    /// gives how many it woke (0 for a buffer that is not shared). A TypeError if `buffer` is
    /// not a buffer.
    pub fn atomics_notify(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _count: u64,
    ) -> JsResult<u64> {
        Err(not_built(self, "atomics_notify"))
    }
}

/// The stub bodies (TA-0): each method that can fail returns `Err` with a pending TypeError
/// (ABI rule R4), and the two that never throw give the answer for a value that is no buffer and
/// no view. TA-1 TA-VM replaces these tests with the tests of the filled bodies.
#[cfg(test)]
mod tests {
    use crate::abi::{Cx, Local};
    use crate::frame::UNDEFINED;
    use crate::tests::{config, isolate};
    use crate::{Dispatch, Isolate};

    #[test]
    fn each_stub_throws_a_type_error_and_the_tests_say_no() {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        let iso: &mut Isolate = &mut iso;
        let rt: &crate::runtime::Runtime = &iso.rt;
        let mut cx = Cx::new(&mut iso.cx);
        let u: Local<'_> = Local::from_bits(UNDEFINED);
        let o = cx.new_object();
        let mut fails = 0;
        let mut check = |cx: &mut Cx<'_>, r: Result<(), ep9_value::Throw>, what: &str| {
            assert!(r.is_err(), "{what}");
            let e: Local<'_> = Local::from_bits(rt.take_pending());
            let e = e.as_object().expect("an Error object");
            let msg = cx.get(e, "message").unwrap();
            let msg = cx.to_rust_string(msg).unwrap();
            assert!(
                msg.starts_with(what) && msg.contains("m8-stub"),
                "{what}: {msg}"
            );
            let ctor = cx.get(e, "constructor").unwrap();
            let name = cx.get(ctor.as_object().unwrap(), "name").unwrap();
            assert_eq!(cx.to_rust_string(name).unwrap(), "TypeError", "{what}");
            fails += 1;
        };
        let r = cx.allocate_array_buffer(o, 8, None).map(drop);
        check(&mut cx, r, "allocate_array_buffer");
        let r = cx.allocate_shared_array_buffer(o, 8, Some(16)).map(drop);
        check(&mut cx, r, "allocate_shared_array_buffer");
        let r = cx.array_buffer_byte_length(o).map(drop);
        check(&mut cx, r, "array_buffer_byte_length");
        let r = cx.detach_array_buffer(o, u);
        check(&mut cx, r, "detach_array_buffer");
        let r = cx.array_buffer_copy_and_detach(o, o, 0, true).map(drop);
        check(&mut cx, r, "array_buffer_copy_and_detach");
        let r = cx.array_buffer_resize(o, 4);
        check(&mut cx, r, "array_buffer_resize");
        let r = cx.shared_array_buffer_grow(o, 4);
        check(&mut cx, r, "shared_array_buffer_grow");
        let r = cx.copy_buffer_bytes(o, 0, o, 1, 1);
        check(&mut cx, r, "copy_buffer_bytes");
        let r = cx.get_value_from_buffer(o, 0, 0, true).map(drop);
        check(&mut cx, r, "get_value_from_buffer");
        let r = cx.set_value_in_buffer(o, 0, 0, u, true);
        check(&mut cx, r, "set_value_in_buffer");
        let r = cx.new_typed_array(o, 0, o, 0, None).map(drop);
        check(&mut cx, r, "new_typed_array");
        let r = cx.typed_array_buffer(o).map(drop);
        check(&mut cx, r, "typed_array_buffer");
        let r = cx.typed_array_byte_offset(o).map(drop);
        check(&mut cx, r, "typed_array_byte_offset");
        let r = cx.typed_array_fixed_length(o).map(drop);
        check(&mut cx, r, "typed_array_fixed_length");
        let r = cx.typed_array_length(o).map(drop);
        check(&mut cx, r, "typed_array_length");
        let r = cx.new_data_view(o, o, 0, Some(1)).map(drop);
        check(&mut cx, r, "new_data_view");
        let r = cx.atomic_load(o, 0, 5).map(drop);
        check(&mut cx, r, "atomic_load");
        let r = cx.atomic_store(o, 0, 5, u);
        check(&mut cx, r, "atomic_store");
        let r = cx.atomic_read_modify_write(o, 0, 5, 0, u).map(drop);
        check(&mut cx, r, "atomic_read_modify_write");
        let r = cx.atomic_compare_exchange(o, 0, 5, u, u).map(drop);
        check(&mut cx, r, "atomic_compare_exchange");
        let r = cx.atomics_wait(o, 0, 5, u, f64::INFINITY).map(drop);
        check(&mut cx, r, "atomics_wait");
        let r = cx.atomics_wait_async(o, 0, 5, u, 0.0).map(drop);
        check(&mut cx, r, "atomics_wait_async");
        let r = cx.atomics_notify(o, 0, u64::MAX).map(drop);
        check(&mut cx, r, "atomics_notify");
        assert_eq!(fails, 23, "every method that can fail");
        assert!(!cx.is_detached_buffer(o));
        assert!(!cx.is_detached_buffer(u));
        assert_eq!(cx.typed_array_element(o), None);
        assert_eq!(cx.typed_array_element(u), None);
    }
}
