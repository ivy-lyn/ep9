//! ArrayBuffer, SharedArrayBuffer, TypedArrays, DataView and Atomics on the builtin ABI (H20
//! part 2; TypedArray workflow, design `docs/plan/notes/typedarray-design.md` §8.1). TA-0
//! TA-DECL declared the `Cx` method list with stub bodies (plan rule 4 form, `m8-stub`); TA-1
//! TA-VM filled the bodies over `array_buffer.rs` and `typed_array.rs`, all but the three
//! waiter methods ([`Cx::atomics_wait`], [`Cx::atomics_wait_async`], [`Cx::atomics_notify`]),
//! which keep their `m8-stub` TypeError until TA-4 TA-ATOMICS builds the waiter list
//! (design §5.6, §8.2). The builtins of TA-2 to TA-4 (`ep9_builtins` array_buffer.rs,
//! data_view.rs, typed_array/, atomics.rs) call them.
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
//! Rows 7 and 8 hold BigInts (`[[ContentType]]` bigint); the others hold Numbers. A row above 11
//! is a bug in the caller: the filled methods panic on it.
//!
//! **Read-modify-write ops** (`op: u8` of [`Cx::atomic_read_modify_write`]): 0 add, 1 and,
//! 2 exchange, 3 or, 4 sub, 5 xor (the Atomics functions of the same names).
//!
//! | Rule | Statement |
//! |---|---|
//! | Values are converted by the caller | No method here runs JS. The builtin does the spec's ToIndex, ToNumber, ToBigInt and species steps first, in spec order; a value argument is then a Number, or a BigInt for rows 7 and 8, else a TypeError |
//! | Memory safety does not trust the caller | A method checks its byte range against the buffer's current length before it reads or writes: a range past it (a builtin bug, or a buffer that a conversion detached or shrank) is a TypeError or a RangeError, never an access. The slots that bound an access (a buffer's store, byte length, flags and table index; a DataView's data address) are written here only. A buffer or a DataView made here owns a table entry from its birth (the brand; `array_buffer` module doc): each method takes a buffer only if it owns the entry that its slot 5 names, so an object of a buffer class that the generic slot API makes is a TypeError here; the generic slot API refuses every slot of a buffer or a DataView made here ([`Cx::set_slot`]) |
//! | GC (plan rule 8) | No Value lives in Rust memory: a buffer's store and view list are held through its slots and the isolate's buffer table (one weak source; design §5.1). R3 of the ABI holds: every `&mut` method may allocate |
//! | Shared memory | A SharedArrayBuffer's bytes are read and written with atomic operations only: unordered access is Relaxed, the Atomics methods are SeqCst (TA-D11) |

use ep9_value::{JsResult, Throw};

use super::{Cx, JsObject, Local};
use crate::array_buffer::{self as ab, Fail};
use crate::frame::NULL;
use crate::kinds;
use crate::typed_array as ta;

/// The TypeError of a method whose body TA-4 TA-ATOMICS writes (the waiter list).
fn not_built(cx: &mut Cx<'_>, what: &str) -> Throw {
    cx.throw_type_error(&format!("{what}: not built yet (m8-stub, TypedArray TA-4)"))
}

/// The prototype argument of a maker: an object or null (a builtin bug otherwise).
fn proto_bits(p: Local<'_>) -> u64 {
    let b = p.bits();
    assert!(
        b == NULL || kinds::is_object(b),
        "a prototype is an object or null"
    );
    b
}

impl<'s> Cx<'s> {
    /// The pending error of `f` (ABI rule R4).
    fn buffer_fail(&mut self, f: Fail) -> Throw {
        match f {
            Fail::Type(m) => self.throw_type_error(m),
            Fail::Range(m) => self.throw_range_error(m),
        }
    }

    /// `r` with its failure thrown.
    fn buffer_result<T>(&mut self, r: Result<T, Fail>) -> JsResult<T> {
        r.map_err(|f| self.buffer_fail(f))
    }

    /// The TypedArray `v` (its bits), or a TypeError.
    fn typed_array_bits(&mut self, v: Local<'s>) -> JsResult<u64> {
        let b = v.bits();
        if kinds::is_view(b) {
            return Ok(b);
        }
        Err(self.throw_type_error("this is not a typed array."))
    }

    /// The row of `element`, or a panic (a builtin bug, the module docs).
    fn element_row(element: u8) -> u8 {
        assert!(
            element < kinds::VIEW_KINDS,
            "element type {element} is not a Table 71 row"
        );
        element
    }

    // ---- ArrayBuffer and SharedArrayBuffer (§25.1, §25.2) ----

    /// AllocateArrayBuffer (sec-allocatearraybuffer) after OrdinaryCreateFromConstructor: a new
    /// ArrayBuffer with prototype `proto` (an object: the caller ran
    /// GetPrototypeFromConstructor) and `byte_length` zero bytes. With `Some(max)` it is
    /// resizable up to `max` bytes. A RangeError if `byte_length` is above `max`, `max` is above
    /// the reservation cap (2^32, design §2.3), or the bytes cannot be allocated
    /// (CreateByteDataBlock).
    pub fn allocate_array_buffer(
        &mut self,
        proto: impl Into<Local<'s>>,
        byte_length: u64,
        max_byte_length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        let p = proto_bits(proto.into());
        let r = ab::allocate(self.rt(), p, byte_length, max_byte_length);
        self.buffer_result(r).map(Self::local)
    }

    /// AllocateSharedArrayBuffer (sec-allocatesharedarraybuffer): a new SharedArrayBuffer with
    /// prototype `proto` and `byte_length` zero bytes, growable up to `max` bytes with
    /// `Some(max)`. A RangeError as [`Cx::allocate_array_buffer`].
    pub fn allocate_shared_array_buffer(
        &mut self,
        proto: impl Into<Local<'s>>,
        byte_length: u64,
        max_byte_length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        let p = proto_bits(proto.into());
        let r = ab::allocate_shared(self.rt(), p, byte_length, max_byte_length);
        self.buffer_result(r).map(Self::local)
    }

    /// ArrayBufferByteLength (sec-arraybufferbytelength) of an ArrayBuffer or a
    /// SharedArrayBuffer (whose length is read with order seq-cst): 0 when detached. A TypeError
    /// if `buffer` is neither.
    pub fn array_buffer_byte_length(&mut self, buffer: impl Into<Local<'s>>) -> JsResult<u64> {
        let b = buffer.into().bits();
        if ab::buffer_class(self.rt(), b).is_none() {
            return Err(self.throw_type_error("Receiver is not an ArrayBuffer"));
        }
        Ok(ab::byte_length(self.rt(), b) as u64)
    }

    /// IsDetachedBuffer (sec-isdetachedbuffer): true if `buffer` is a detached ArrayBuffer;
    /// false for any other value (a SharedArrayBuffer is never detached). Never throws.
    pub fn is_detached_buffer(&self, buffer: impl Into<Local<'s>>) -> bool {
        let b = buffer.into().bits();
        ab::buffer_class(self.rt(), b).is_some() && ab::is_detached(b)
    }

    /// DetachArrayBuffer (sec-detacharraybuffer): every view of `buffer` gets length 0 and no
    /// data (TA-D5), then the buffer drops its bytes and its byte length is 0. A TypeError if
    /// `buffer` is not an ArrayBuffer (a SharedArrayBuffer included) or if its
    /// `[[ArrayBufferDetachKey]]` is not `key` (SameValue). Detaching a detached buffer does
    /// nothing.
    pub fn detach_array_buffer(
        &mut self,
        buffer: impl Into<Local<'s>>,
        key: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let r = ab::detach(self.rt(), buffer.into().bits(), key.into().bits());
        self.buffer_result(r)
    }

    /// ArrayBufferCopyAndDetach (sec-arraybuffercopyanddetach) after its ToIndex step, for
    /// `transfer` and `transferToFixedLength`: a new ArrayBuffer with prototype `proto` (the
    /// current realm's %ArrayBuffer.prototype%) and `new_byte_length` bytes, resizable with the
    /// old maximum if `preserve_resizability` and `buffer` is resizable; it takes the first
    /// min(old, new) bytes of `buffer` (the store itself when the length is the same, or when a
    /// resizable buffer stays resizable), and `buffer` is detached. A TypeError if `buffer` is
    /// not an ArrayBuffer, is detached, or has a detach key that is not undefined; a RangeError
    /// if `new_byte_length` is above the new maximum or cannot be allocated.
    pub fn array_buffer_copy_and_detach(
        &mut self,
        buffer: impl Into<Local<'s>>,
        proto: impl Into<Local<'s>>,
        new_byte_length: u64,
        preserve_resizability: bool,
    ) -> JsResult<Local<'s, JsObject>> {
        let b = buffer.into().bits();
        let p = proto_bits(proto.into());
        let r = ab::copy_and_detach(self.rt(), b, p, new_byte_length, preserve_resizability);
        self.buffer_result(r).map(Self::local)
    }

    /// The resize of `ArrayBuffer.prototype.resize` (sec-arraybuffer.prototype.resize) after
    /// its ToIndex step: the byte length of the resizable ArrayBuffer `buffer` becomes
    /// `new_byte_length`; new bytes are 0. The store keeps its base address, so the views keep
    /// their data address (design §5.2). A TypeError if `buffer` is not a resizable
    /// ArrayBuffer or is detached; a RangeError if `new_byte_length` is above its maximum.
    pub fn array_buffer_resize(
        &mut self,
        buffer: impl Into<Local<'s>>,
        new_byte_length: u64,
    ) -> JsResult<()> {
        let r = ab::resize(self.rt(), buffer.into().bits(), new_byte_length);
        self.buffer_result(r)
    }

    /// The grow of `SharedArrayBuffer.prototype.grow` (sec-sharedarraybuffer.prototype.grow)
    /// after its ToIndex step: the byte length of the growable SharedArrayBuffer `buffer`
    /// becomes `new_byte_length` (a SeqCst store in the shared block; new bytes are 0). A
    /// TypeError if `buffer` is not a growable SharedArrayBuffer; a RangeError if
    /// `new_byte_length` is below its byte length or above its maximum.
    pub fn shared_array_buffer_grow(
        &mut self,
        buffer: impl Into<Local<'s>>,
        new_byte_length: u64,
    ) -> JsResult<()> {
        let r = ab::grow_shared(self.rt(), buffer.into().bits(), new_byte_length);
        self.buffer_result(r)
    }

    /// CopyDataBlockBytes (sec-copydatablockbytes) on the stores of two buffers: `count` bytes
    /// of `from` at `from_index` to `to` at `to_index`. The buffers may be the same and the
    /// ranges may overlap (`copyWithin`): the bytes are copied as if through a temporary
    /// copy. A TypeError if either value is not a buffer or is detached; a RangeError if a
    /// range is past its buffer's byte length (nothing is copied then).
    pub fn copy_buffer_bytes(
        &mut self,
        to: impl Into<Local<'s>>,
        to_index: u64,
        from: impl Into<Local<'s>>,
        from_index: u64,
        count: u64,
    ) -> JsResult<()> {
        let (t, f) = (to.into().bits(), from.into().bits());
        let r = ab::copy_bytes(self.rt(), t, to_index, f, from_index, count);
        self.buffer_result(r)
    }

    /// GetValueFromBuffer (sec-getvaluefrombuffer) with order unordered: the element of type
    /// `element` (a Table 71 row, the module docs) at byte `byte_index` of `buffer`, little
    /// endian if `little_endian`: a Number, or a BigInt for rows 7 and 8. A TypeError if
    /// `buffer` is not a buffer or is detached; a RangeError if the element is past its byte
    /// length.
    pub fn get_value_from_buffer(
        &mut self,
        buffer: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        little_endian: bool,
    ) -> JsResult<Local<'s>> {
        let t = Self::element_row(element);
        let r = ab::get_value(
            self.rt(),
            buffer.into().bits(),
            byte_index,
            t,
            little_endian,
        );
        self.buffer_result(r).map(Self::local)
    }

    /// SetValueInBuffer (sec-setvalueinbuffer) with order unordered: stores `value` (a Number,
    /// or a BigInt for rows 7 and 8: the caller converted it) as the element of type `element`
    /// at byte `byte_index` of `buffer` (NumericToRawBytes: Uint8Clamped rounds half to even
    /// and clamps; Float16 and Float32 round to nearest, ties to even; a NaN is one pattern).
    /// An error as [`Cx::get_value_from_buffer`], or a TypeError if `value` has the wrong type.
    pub fn set_value_in_buffer(
        &mut self,
        buffer: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        value: impl Into<Local<'s>>,
        little_endian: bool,
    ) -> JsResult<()> {
        let t = Self::element_row(element);
        let (b, v) = (buffer.into().bits(), value.into().bits());
        let r = ab::set_value(self.rt(), b, byte_index, t, v, little_endian);
        self.buffer_result(r)
    }

    // ---- TypedArrays (§23.2, §10.4.5) ----

    /// A new TypedArray of element type `element` with prototype `proto`, on `buffer` (an
    /// ArrayBuffer or a SharedArrayBuffer) from `byte_offset`: `Some(length)` elements, or
    /// `None` for a length that is undefined: on a resizable ArrayBuffer or a growable
    /// SharedArrayBuffer a length-tracking view (`[[ArrayLength]]` auto), on a fixed-length
    /// buffer (byte length - `byte_offset`) / element size elements (step 9.a). The caller ran
    /// the checks of InitializeTypedArrayFromArrayBuffer
    /// (sec-initializetypedarrayfromarraybuffer; the alignment of `byte_offset`, detach and
    /// bounds) in spec order; the view's elements kind is TYPED_FIXED or TYPED_VARIABLE by
    /// IsTypedArrayFixedLength (design §3.2). A TypeError if `buffer` is not a buffer or is
    /// detached; a RangeError if `byte_offset` is not a multiple of the element size, the range
    /// is not inside the buffer, or (`None` on a fixed-length buffer) the byte length is not a
    /// multiple of the element size.
    pub fn new_typed_array(
        &mut self,
        proto: impl Into<Local<'s>>,
        element: u8,
        buffer: impl Into<Local<'s>>,
        byte_offset: u64,
        length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        let t = Self::element_row(element);
        let p = proto_bits(proto.into());
        let r = ta::new_view(self.rt(), p, t, buffer.into().bits(), byte_offset, length);
        self.buffer_result(r).map(Self::local)
    }

    /// The element type (a Table 71 row) of `v` if it is a TypedArray (its
    /// `[[TypedArrayName]]`; rows 7 and 8 have `[[ContentType]]` bigint), else `None`. Never
    /// throws.
    pub fn typed_array_element(&self, v: impl Into<Local<'s>>) -> Option<u8> {
        let b = v.into().bits();
        kinds::is_view(b).then(|| ta::element_type(b))
    }

    /// `[[ViewedArrayBuffer]]` of the TypedArray `ta` (also when it is detached). A TypeError if
    /// `ta` is not a TypedArray.
    pub fn typed_array_buffer(
        &mut self,
        ta: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let o = self.typed_array_bits(ta.into())?;
        Ok(Self::local(ta::buffer_of(o)))
    }

    /// `[[ByteOffset]]` of the TypedArray `ta` as stored (the `byteOffset` getter gives 0 when
    /// `ta` is out of bounds: the caller tests [`Cx::typed_array_length`] first). A TypeError
    /// if `ta` is not a TypedArray.
    pub fn typed_array_byte_offset(&mut self, ta: impl Into<Local<'s>>) -> JsResult<u64> {
        let o = self.typed_array_bits(ta.into())?;
        Ok(ta::byte_offset(o))
    }

    /// `[[ArrayLength]]` of the TypedArray `ta`: `Some(n)`, or `None` when it is auto (a
    /// length-tracking view). A detached fixed-length view gives `Some(0)` (design §5.2: its
    /// length field is its bound); IsTypedArrayOutOfBounds is [`Cx::typed_array_length`]'s. A
    /// TypeError if `ta` is not a TypedArray.
    pub fn typed_array_fixed_length(&mut self, ta: impl Into<Local<'s>>) -> JsResult<Option<u64>> {
        let o = self.typed_array_bits(ta.into())?;
        Ok(ta::fixed_length(o))
    }

    /// TypedArrayLength (sec-typedarraylength) of MakeTypedArrayWithBufferWitnessRecord(`ta`,
    /// seq-cst): `Some(length)`, or `None` when IsTypedArrayOutOfBounds (a detached buffer
    /// included). A TypeError if `ta` is not a TypedArray.
    pub fn typed_array_length(&mut self, ta: impl Into<Local<'s>>) -> JsResult<Option<u64>> {
        let o = self.typed_array_bits(ta.into())?;
        Ok(ta::length(self.rt(), o))
    }

    // ---- DataView (§25.3) ----

    /// A new DataView with prototype `proto` on `buffer` from `byte_offset`: `Some(n)` bytes,
    /// or `None` for a byteLength that is undefined: on a resizable ArrayBuffer or a growable
    /// SharedArrayBuffer a length-tracking view (`[[ByteLength]]` auto), on a fixed-length
    /// buffer the byte length - `byte_offset` bytes (step 8.a). The caller ran the checks
    /// of the DataView constructor (sec-dataview-buffer-byteoffset-bytelength) in spec order. A
    /// TypeError if `buffer` is not a buffer or is detached; a RangeError if the range is not
    /// inside it. Its internal slots (`ObjectClass::DataView`): 0 `[[ViewedArrayBuffer]]`,
    /// 1 `[[ByteOffset]]`, 2 `[[ByteLength]]` (the int32 -1 for auto; 0 once a fixed one is
    /// detached), 3 the data address (an internal native word), 4 the index of its own buffer
    /// table entry (internal).
    pub fn new_data_view(
        &mut self,
        proto: impl Into<Local<'s>>,
        buffer: impl Into<Local<'s>>,
        byte_offset: u64,
        byte_length: Option<u64>,
    ) -> JsResult<Local<'s, JsObject>> {
        let p = proto_bits(proto.into());
        let b = buffer.into().bits();
        let r = ab::new_data_view(self.rt(), p, b, byte_offset, byte_length);
        self.buffer_result(r).map(Self::local)
    }

    // ---- Atomics (§25.4): `buffer` and `byte_index` are the caller's ValidateAtomicAccess
    // result, for an integer element type (rows 0-8 except 2), aligned ----

    /// GetValueFromBuffer with order seq-cst (`Atomics.load`, sec-atomics.load): the element as
    /// a Number, or a BigInt for rows 7 and 8. A TypeError if `buffer` is not a buffer, is
    /// detached or `element` is not an integer type; a RangeError for an index that is not
    /// aligned or is past the byte length.
    pub fn atomic_load(
        &mut self,
        buffer: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
    ) -> JsResult<Local<'s>> {
        let t = Self::element_row(element);
        let r = ab::atomic_load(self.rt(), buffer.into().bits(), byte_index, t);
        self.buffer_result(r).map(Self::local)
    }

    /// SetValueInBuffer with order seq-cst (`Atomics.store`, sec-atomics.store) of the
    /// converted `value`. An error as [`Cx::atomic_load`], or a TypeError if `value` has the
    /// wrong type.
    pub fn atomic_store(
        &mut self,
        buffer: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        value: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let t = Self::element_row(element);
        let (b, v) = (buffer.into().bits(), value.into().bits());
        let r = ab::atomic_store(self.rt(), b, byte_index, t, v);
        self.buffer_result(r)
    }

    /// GetModifySetValueInBuffer (sec-getmodifysetvalueinbuffer), seq-cst: the element becomes
    /// `old op value` for the read-modify-write op `op` (the module docs: 0 add, 1 and,
    /// 2 exchange, 3 or, 4 sub, 5 xor), in one atomic step; the result is the old element. An
    /// error as [`Cx::atomic_store`].
    pub fn atomic_read_modify_write(
        &mut self,
        buffer: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        op: u8,
        value: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s>> {
        let t = Self::element_row(element);
        let (b, v) = (buffer.into().bits(), value.into().bits());
        let r = ab::atomic_rmw(self.rt(), b, byte_index, t, op, v);
        self.buffer_result(r).map(Self::local)
    }

    /// The compare-exchange of `Atomics.compareExchange` (sec-atomics.compareexchange) after its
    /// conversions: in one seq-cst step, if the element's bytes equal the raw bytes of
    /// `expected`, the element becomes `replacement`; the result is the old element. An error
    /// as [`Cx::atomic_store`].
    pub fn atomic_compare_exchange(
        &mut self,
        buffer: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        expected: impl Into<Local<'s>>,
        replacement: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s>> {
        let t = Self::element_row(element);
        let b = buffer.into().bits();
        let (e, n) = (expected.into().bits(), replacement.into().bits());
        let r = ab::atomic_compare_exchange(self.rt(), b, byte_index, t, e, n);
        self.buffer_result(r).map(Self::local)
    }

    /// DoWait (sec-dowait) with mode sync after its conversions: if the Int32 or BigInt64
    /// element (row 5 or 7) of the SharedArrayBuffer `buffer` is `value`, the agent waits until
    /// a notify, `timeout_ms` milliseconds (at least 0; +∞ waits for good) or an isolate
    /// interrupt. The result is the String "ok", "not-equal" or "timed-out". A TypeError if the
    /// agent cannot suspend (AgentCanSuspend) or `buffer` is not shared. Still an `m8-stub`
    /// (TypeError): TA-4 TA-ATOMICS builds the waiter list.
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
    /// TypeError if `buffer` is not shared. Still an `m8-stub` (TypeError), as
    /// [`Cx::atomics_wait`].
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
    /// not a buffer. Still an `m8-stub` (TypeError), as [`Cx::atomics_wait`].
    pub fn atomics_notify(
        &mut self,
        _buffer: impl Into<Local<'s>>,
        _byte_index: u64,
        _count: u64,
    ) -> JsResult<u64> {
        Err(not_built(self, "atomics_notify"))
    }
}

/// TA-2 TA-BUFFERS (TA-ORCH-10; design study judge.md 4.7): the brand-checked reads that the
/// getters of `ArrayBuffer.prototype`, `SharedArrayBuffer.prototype` and `DataView.prototype`
/// need, and the DataView get and set. Each method tests the brand first (`buffer_class`,
/// `is_data_view`: an object of these classes that the generic slot API made is not a buffer
/// or a DataView here) and reads the slots only after it. None runs JS.
impl<'s> Cx<'s> {
    /// True if `v` is an ArrayBuffer (not a SharedArrayBuffer) that `abi/typed.rs` made: the
    /// `[[ArrayBufferData]]` test of RequireInternalSlot with IsSharedArrayBuffer false. Never
    /// throws.
    pub fn is_array_buffer(&self, v: impl Into<Local<'s>>) -> bool {
        ab::buffer_class(self.rt(), v.into().bits()) == Some(crate::abi::ObjectClass::ArrayBuffer)
    }

    /// True if `v` is a SharedArrayBuffer that `abi/typed.rs` made (IsSharedArrayBuffer of a
    /// value with `[[ArrayBufferData]]`). Never throws.
    pub fn is_shared_array_buffer(&self, v: impl Into<Local<'s>>) -> bool {
        ab::buffer_class(self.rt(), v.into().bits())
            == Some(crate::abi::ObjectClass::SharedArrayBuffer)
    }

    /// `[[ArrayBufferMaxByteLength]]` of the ArrayBuffer or SharedArrayBuffer `buffer`:
    /// `Some(max)` for a resizable ArrayBuffer or a growable SharedArrayBuffer, `None` when
    /// IsFixedLengthArrayBuffer is true (also when it is detached: the slot stays). A TypeError
    /// if `buffer` is neither.
    pub fn array_buffer_max_byte_length(
        &mut self,
        buffer: impl Into<Local<'s>>,
    ) -> JsResult<Option<u64>> {
        let b = buffer.into().bits();
        if ab::buffer_class(self.rt(), b).is_none() {
            return Err(self.throw_type_error("Receiver is not an ArrayBuffer"));
        }
        Ok(ab::max_byte_length(b))
    }

    /// True if `v` is a DataView that `abi/typed.rs` made (the `[[DataView]]` test of
    /// RequireInternalSlot). Never throws.
    pub fn is_data_view(&self, v: impl Into<Local<'s>>) -> bool {
        ab::is_data_view(self.rt(), v.into().bits())
    }

    /// The DataView `v` (its bits), or a TypeError.
    fn data_view_bits(&mut self, v: Local<'s>) -> JsResult<u64> {
        let b = v.bits();
        if ab::is_data_view(self.rt(), b) {
            return Ok(b);
        }
        Err(self.throw_type_error("Receiver is not a DataView"))
    }

    /// `[[ViewedArrayBuffer]]` of the DataView `view` (also when it is detached). A TypeError
    /// if `view` is not a DataView.
    pub fn data_view_buffer(
        &mut self,
        view: impl Into<Local<'s>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let v = self.data_view_bits(view.into())?;
        Ok(Self::local(ab::data_view_buffer(v)))
    }

    /// `[[ByteOffset]]` of the DataView `view`, or `None` when IsViewOutOfBounds (a detached
    /// buffer included) of MakeDataViewWithBufferWitnessRecord(`view`, seq-cst). A TypeError if
    /// `view` is not a DataView.
    pub fn data_view_byte_offset(&mut self, view: impl Into<Local<'s>>) -> JsResult<Option<u64>> {
        let v = self.data_view_bits(view.into())?;
        let in_bounds = ab::data_view_byte_length(self.rt(), v).is_some();
        Ok(in_bounds.then(|| ab::data_view_byte_offset(v)))
    }

    /// GetViewByteLength (sec-getviewbytelength) of MakeDataViewWithBufferWitnessRecord(`view`,
    /// seq-cst): `Some(n)` (for a length-tracking view the buffer's byte length now, less the
    /// byte offset), or `None` when IsViewOutOfBounds (a detached buffer included). A TypeError
    /// if `view` is not a DataView.
    pub fn data_view_byte_length(&mut self, view: impl Into<Local<'s>>) -> JsResult<Option<u64>> {
        let v = self.data_view_bits(view.into())?;
        Ok(ab::data_view_byte_length(self.rt(), v))
    }

    /// GetViewValue (sec-getviewvalue) after its ToIndex and ToBoolean steps: the element of
    /// type `element` (a Table 71 row, the module docs) at byte `byte_index` of the DataView
    /// `view`, little endian if `little_endian`: a Number, or a BigInt for rows 7 and 8. A
    /// TypeError if `view` is not a DataView or is out of bounds (IsViewOutOfBounds, a detached
    /// buffer included); a RangeError if the element is past the view's byte length.
    pub fn data_view_get_value(
        &mut self,
        view: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        little_endian: bool,
    ) -> JsResult<Local<'s>> {
        let t = Self::element_row(element);
        let r = ab::data_view_get(self.rt(), view.into().bits(), byte_index, t, little_endian);
        self.buffer_result(r).map(Self::local)
    }

    /// SetViewValue (sec-setviewvalue) after its ToIndex, ToNumber or ToBigInt, and ToBoolean
    /// steps: stores `value` (a Number, or a BigInt for rows 7 and 8: the caller converted it)
    /// as the element of type `element` at byte `byte_index` of the DataView `view`
    /// (NumericToRawBytes, as [`Cx::set_value_in_buffer`]). An error as
    /// [`Cx::data_view_get_value`], or a TypeError if `value` has the wrong type.
    pub fn data_view_set_value(
        &mut self,
        view: impl Into<Local<'s>>,
        byte_index: u64,
        element: u8,
        value: impl Into<Local<'s>>,
        little_endian: bool,
    ) -> JsResult<()> {
        let t = Self::element_row(element);
        let (v, x) = (view.into().bits(), value.into().bits());
        let r = ab::data_view_set(self.rt(), v, byte_index, t, x, little_endian);
        self.buffer_result(r)
    }
}

/// The filled bodies on the ABI (TA-1): the error kind of each method on a value that is no
/// buffer or no view, the 3 waiter stubs that stay for TA-4, and one round trip. The element
/// semantics, detach, resize, GC and the exotic methods are tested in `tests::typed_array` and
/// `tests::array_buffer`.
#[cfg(test)]
mod tests {
    use crate::abi::{Cx, Local};
    use crate::frame::UNDEFINED;
    use crate::tests::{config, isolate};
    use crate::{Dispatch, Isolate};

    /// The constructor name and the message of the pending error.
    fn pending(rt: &crate::runtime::Runtime, cx: &mut Cx<'_>) -> (String, String) {
        let e: Local<'_> = Local::from_bits(rt.take_pending());
        let e = e.as_object().expect("an Error object");
        let msg = cx.get(e, "message").unwrap();
        let msg = cx.to_rust_string(msg).unwrap();
        let ctor = cx.get(e, "constructor").unwrap();
        let name = cx.get(ctor.as_object().unwrap(), "name").unwrap();
        (cx.to_rust_string(name).unwrap(), msg)
    }

    #[test]
    fn each_method_throws_its_error_kind_and_the_waiters_stay_stubs() {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        let iso: &mut Isolate = &mut iso;
        let rt: &crate::runtime::Runtime = &iso.rt;
        let mut cx = Cx::new(&mut iso.cx);
        let u: Local<'_> = Local::from_bits(UNDEFINED);
        let o = cx.new_object();
        let mut kinds = Vec::new();
        let mut check =
            |cx: &mut Cx<'_>, r: Result<(), ep9_value::Throw>, what: &str, want: &str| {
                assert!(r.is_err(), "{what}");
                let (name, msg) = pending(rt, cx);
                assert_eq!(name, want, "{what}: {msg}");
                kinds.push((what.to_string(), name, msg));
            };
        let r = cx.allocate_array_buffer(o, 9, Some(8)).map(drop);
        check(&mut cx, r, "allocate_array_buffer len > max", "RangeError");
        let r = cx
            .allocate_array_buffer(o, 0, Some((1 << 32) + 1))
            .map(drop);
        check(&mut cx, r, "allocate_array_buffer max > cap", "RangeError");
        let r = cx.allocate_array_buffer(o, 1 << 50, None).map(drop);
        check(&mut cx, r, "allocate_array_buffer 2^50", "RangeError");
        let r = cx.allocate_shared_array_buffer(o, 9, Some(8)).map(drop);
        check(&mut cx, r, "allocate_shared_array_buffer", "RangeError");
        let r = cx.array_buffer_byte_length(o).map(drop);
        check(&mut cx, r, "array_buffer_byte_length", "TypeError");
        let r = cx.detach_array_buffer(o, u);
        check(&mut cx, r, "detach_array_buffer", "TypeError");
        let r = cx.array_buffer_copy_and_detach(o, o, 0, true).map(drop);
        check(&mut cx, r, "array_buffer_copy_and_detach", "TypeError");
        let r = cx.array_buffer_resize(o, 4);
        check(&mut cx, r, "array_buffer_resize", "TypeError");
        let r = cx.shared_array_buffer_grow(o, 4);
        check(&mut cx, r, "shared_array_buffer_grow", "TypeError");
        let r = cx.copy_buffer_bytes(o, 0, o, 1, 1);
        check(&mut cx, r, "copy_buffer_bytes", "TypeError");
        let r = cx.get_value_from_buffer(o, 0, 0, true).map(drop);
        check(&mut cx, r, "get_value_from_buffer", "TypeError");
        let r = cx.set_value_in_buffer(o, 0, 0, u, true);
        check(&mut cx, r, "set_value_in_buffer", "TypeError");
        let r = cx.new_typed_array(o, 0, o, 0, None).map(drop);
        check(&mut cx, r, "new_typed_array", "TypeError");
        let r = cx.typed_array_buffer(o).map(drop);
        check(&mut cx, r, "typed_array_buffer", "TypeError");
        let r = cx.typed_array_byte_offset(o).map(drop);
        check(&mut cx, r, "typed_array_byte_offset", "TypeError");
        let r = cx.typed_array_fixed_length(o).map(drop);
        check(&mut cx, r, "typed_array_fixed_length", "TypeError");
        let r = cx.typed_array_length(o).map(drop);
        check(&mut cx, r, "typed_array_length", "TypeError");
        let r = cx.new_data_view(o, o, 0, Some(1)).map(drop);
        check(&mut cx, r, "new_data_view", "TypeError");
        let r = cx.atomic_load(o, 0, 5).map(drop);
        check(&mut cx, r, "atomic_load", "TypeError");
        let r = cx.atomic_store(o, 0, 5, Local::number(1.0));
        check(&mut cx, r, "atomic_store", "TypeError");
        let r = cx
            .atomic_read_modify_write(o, 0, 5, 0, Local::number(1.0))
            .map(drop);
        check(&mut cx, r, "atomic_read_modify_write", "TypeError");
        let one = Local::number(1.0);
        let r = cx.atomic_compare_exchange(o, 0, 5, one, one).map(drop);
        check(&mut cx, r, "atomic_compare_exchange", "TypeError");
        // The waiter list is TA-4's (design §8.2): these three keep the m8-stub TypeError.
        let r = cx.atomics_wait(o, 0, 5, u, f64::INFINITY).map(drop);
        check(&mut cx, r, "atomics_wait", "TypeError");
        let r = cx.atomics_wait_async(o, 0, 5, u, 0.0).map(drop);
        check(&mut cx, r, "atomics_wait_async", "TypeError");
        let r = cx.atomics_notify(o, 0, u64::MAX).map(drop);
        check(&mut cx, r, "atomics_notify", "TypeError");
        assert_eq!(kinds.len(), 25);
        let stubs: Vec<&str> = kinds
            .iter()
            .filter(|(_, _, m)| m.contains("m8-stub"))
            .map(|(w, _, _)| w.as_str())
            .collect();
        assert_eq!(
            stubs,
            ["atomics_wait", "atomics_wait_async", "atomics_notify"]
        );
        assert!(!cx.is_detached_buffer(o));
        assert!(!cx.is_detached_buffer(u));
        assert_eq!(cx.typed_array_element(o), None);
        assert_eq!(cx.typed_array_element(u), None);
    }

    /// One trip through the ABI: a buffer, a Float64Array view on it, a DataView store read back
    /// through the view, the element type, the buffer, offset and lengths, then a detach.
    #[test]
    fn a_round_trip_through_the_abi() {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let iso: &mut Isolate = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        let proto = cx.new_object();
        let b = cx.allocate_array_buffer(proto, 24, None).unwrap();
        assert_eq!(cx.array_buffer_byte_length(b).unwrap(), 24);
        let t = cx.new_typed_array(proto, 11, b, 8, Some(2)).unwrap();
        let dv = cx.new_data_view(proto, b, 0, None).unwrap();
        assert_eq!(cx.typed_array_element(t), Some(11));
        assert_eq!(cx.typed_array_buffer(t).unwrap().bits(), b.bits());
        assert_eq!(cx.typed_array_byte_offset(t).unwrap(), 8);
        assert_eq!(cx.typed_array_fixed_length(t).unwrap(), Some(2));
        assert_eq!(cx.typed_array_length(t).unwrap(), Some(2));
        // Big endian through the buffer (the DataView's store), read as a Float64Array element.
        cx.set_value_in_buffer(b, 16, 11, Local::number(-2.5), false)
            .unwrap();
        let v = cx.get_value_from_buffer(b, 16, 11, false).unwrap();
        assert_eq!(v.as_number(), Some(-2.5));
        cx.set_value_in_buffer(b, 16, 11, Local::number(-2.5), true)
            .unwrap();
        let e = cx.get(t, "1").unwrap();
        assert_eq!(e.as_number(), Some(-2.5));
        cx.detach_array_buffer(b, Local::undefined()).unwrap();
        assert!(cx.is_detached_buffer(b));
        assert_eq!(cx.array_buffer_byte_length(b).unwrap(), 0);
        assert_eq!(cx.typed_array_length(t).unwrap(), None);
        assert_eq!(cx.get(t, "1").unwrap().bits(), UNDEFINED);
        std::hint::black_box(dv);
    }

    /// TA-2 TA-BUFFERS: the brand-checked reads and the DataView get and set. Each takes only
    /// an engine buffer or DataView (a TypeError, or false, for a plain object and for an
    /// object of the class that the generic slot API made); a DataView with no byteLength on a
    /// fixed-length buffer is a fixed view of the rest (route TA2-DV-FIXED: slot 2 holds the
    /// byte length, not the auto -1), one on a resizable buffer tracks its length; get and
    /// set check IsViewOutOfBounds (a TypeError) and the view range (a RangeError) against the
    /// byte length at the access; a detach makes every read out of bounds. Plain and under
    /// GC stress.
    #[test]
    fn the_ta2_reads_check_the_brand_and_the_view_bounds() {
        use crate::abi::ObjectClass;
        for stress in [false, true] {
            crate::tests::on_big_stack(move || {
                let mut iso = isolate(config(Dispatch::DEFAULT, stress));
                let iso: &mut Isolate = &mut iso;
                let rt: &crate::runtime::Runtime = &iso.rt;
                let mut cx = Cx::new(&mut iso.cx);
                let p = cx.new_object();
                let o = cx.new_object();
                let forged = cx.new_object_with_slots(p, ObjectClass::ArrayBuffer, 6);
                let forged_dv = cx.new_object_with_slots(p, ObjectClass::DataView, 5);
                let fixed = cx.allocate_array_buffer(p, 8, None).unwrap();
                let rab = cx.allocate_array_buffer(p, 8, Some(16)).unwrap();
                let sab = cx.allocate_shared_array_buffer(p, 8, None).unwrap();
                let gsab = cx.allocate_shared_array_buffer(p, 8, Some(16)).unwrap();
                // The buffer tests.
                for (v, ab, shared) in [
                    (o, false, false),
                    (forged, false, false),
                    (fixed, true, false),
                    (rab, true, false),
                    (sab, false, true),
                    (gsab, false, true),
                ] {
                    assert_eq!(cx.is_array_buffer(v), ab);
                    assert_eq!(cx.is_shared_array_buffer(v), shared);
                }
                for v in [o, forged] {
                    assert!(cx.array_buffer_max_byte_length(v).is_err());
                    assert_eq!(pending(rt, &mut cx).0, "TypeError");
                }
                assert_eq!(cx.array_buffer_max_byte_length(fixed).unwrap(), None);
                assert_eq!(cx.array_buffer_max_byte_length(rab).unwrap(), Some(16));
                assert_eq!(cx.array_buffer_max_byte_length(sab).unwrap(), None);
                assert_eq!(cx.array_buffer_max_byte_length(gsab).unwrap(), Some(16));
                // TA2-DV-FIXED: no byteLength on a fixed buffer gives a fixed view (slot 2 is 6).
                let dv = cx.new_data_view(p, fixed, 2, None).unwrap();
                let auto = cx.new_data_view(p, rab, 2, None).unwrap();
                let shared = cx.new_data_view(p, gsab, 0, None).unwrap();
                let len = cx.slot(dv, ObjectClass::DataView, 2).unwrap();
                assert_eq!(len.as_number(), Some(6.0), "a fixed view of the rest");
                let len = cx.slot(auto, ObjectClass::DataView, 2).unwrap();
                assert_eq!(len.as_int32(), Some(crate::layout::VIEW_LENGTH_AUTO));
                // The DataView reads.
                for v in [o, forged_dv, fixed] {
                    assert!(!cx.is_data_view(v));
                    assert!(cx.data_view_buffer(v).is_err());
                    assert_eq!(pending(rt, &mut cx).0, "TypeError");
                    assert!(cx.data_view_byte_offset(v).is_err());
                    assert_eq!(pending(rt, &mut cx).0, "TypeError");
                    assert!(cx.data_view_byte_length(v).is_err());
                    assert_eq!(pending(rt, &mut cx).0, "TypeError");
                    assert!(cx.data_view_get_value(v, 0, 0, true).is_err());
                    assert_eq!(pending(rt, &mut cx).0, "TypeError");
                    assert!(
                        cx.data_view_set_value(v, 0, 0, Local::number(1.0), true)
                            .is_err()
                    );
                    assert_eq!(pending(rt, &mut cx).0, "TypeError");
                }
                assert!(cx.is_data_view(dv) && cx.is_data_view(auto));
                assert!(cx.data_view_buffer(dv).unwrap().is_same_bits(&fixed));
                assert_eq!(cx.data_view_byte_offset(dv).unwrap(), Some(2));
                assert_eq!(cx.data_view_byte_length(dv).unwrap(), Some(6));
                assert_eq!(cx.data_view_byte_length(auto).unwrap(), Some(6));
                // Big endian through the view, read back through the buffer.
                cx.data_view_set_value(dv, 1, 4, Local::number(4660.0), false)
                    .unwrap();
                let b = cx.get_value_from_buffer(fixed, 3, 1, true).unwrap();
                assert_eq!(b.as_number(), Some(18.0), "byte 3 = 0x12");
                let x = cx.data_view_get_value(dv, 1, 4, true).unwrap();
                assert_eq!(x.as_number(), Some(13330.0), "0x3412 little endian");
                // The view range: 4 bytes from 3 of a 6-byte view.
                assert!(cx.data_view_get_value(dv, 3, 5, true).is_err());
                assert_eq!(pending(rt, &mut cx).0, "RangeError");
                assert!(cx.data_view_get_value(dv, u64::MAX, 0, true).is_err());
                assert_eq!(pending(rt, &mut cx).0, "RangeError");
                // A value of the wrong type (the caller converts): a TypeError.
                assert!(
                    cx.data_view_set_value(dv, 0, 7, Local::number(1.0), true)
                        .is_err()
                );
                assert_eq!(pending(rt, &mut cx).0, "TypeError");
                // A length-tracking view follows a resize; a shrink below its offset puts it
                // out of bounds.
                cx.array_buffer_resize(rab, 16).unwrap();
                assert_eq!(cx.data_view_byte_length(auto).unwrap(), Some(14));
                cx.data_view_set_value(auto, 13, 1, Local::number(7.0), true)
                    .unwrap();
                cx.array_buffer_resize(rab, 1).unwrap();
                assert_eq!(cx.data_view_byte_length(auto).unwrap(), None);
                assert_eq!(cx.data_view_byte_offset(auto).unwrap(), None);
                assert!(cx.data_view_get_value(auto, 0, 1, true).is_err());
                assert_eq!(pending(rt, &mut cx).0, "TypeError");
                cx.array_buffer_resize(rab, 16).unwrap();
                let x = cx.data_view_get_value(auto, 13, 1, true).unwrap();
                assert_eq!(x.as_number(), Some(0.0), "a shrink zeroed the byte");
                // Shared bytes: a growable SAB view grows with it.
                cx.data_view_set_value(shared, 4, 5, Local::number(-2.0), true)
                    .unwrap();
                cx.shared_array_buffer_grow(gsab, 12).unwrap();
                assert_eq!(cx.data_view_byte_length(shared).unwrap(), Some(12));
                let x = cx.data_view_get_value(shared, 4, 5, true).unwrap();
                assert_eq!(x.as_number(), Some(-2.0));
                // A detach: every read of the fixed view is out of bounds; the buffer stays.
                cx.detach_array_buffer(fixed, Local::undefined()).unwrap();
                assert_eq!(cx.data_view_byte_length(dv).unwrap(), None);
                assert_eq!(cx.data_view_byte_offset(dv).unwrap(), None);
                assert!(cx.data_view_get_value(dv, 0, 0, true).is_err());
                assert_eq!(pending(rt, &mut cx).0, "TypeError");
                assert!(cx.data_view_buffer(dv).unwrap().is_same_bits(&fixed));
                std::hint::black_box((o, forged, forged_dv, sab, auto, shared));
            });
        }
    }
}
