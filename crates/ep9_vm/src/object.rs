//! v0 object model (M0 week 3). Week 4 replaces the property dictionary with shapes (H8, H17)
//! and the strings with H19 strings. Every shortcut is marked `// v0:`.
//!
//! All cells are `ep9_heap` cells. Every word after word0 of an object, table or context is a
//! valid `Value` (raw pointers use the native-word encoding of `frame.rs`), so one trace
//! function visits them all. Strings hold raw code units after a 3-word header and have no
//! references.
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | STRING | 1 | +8 length (int32), +16 encoding (int32: 0 Latin-1, 1 UTF-16), +24 code units | 24 + len or 24 + 2 len |
//! | OBJECT | 2 | +8 prototype, +16 property table or undefined, +24 flags (int32) | 32 |
//! | FUNCTION | 3 | object + 32 FunctionInfo (native word), +40 context, +48 `[[HomeObject]]` or undefined | 56 |
//! | ARRAY | 4 | object + 32 elements or undefined, +40 length (int32) | 48 |
//! | ERROR | 5 | object + 32 unit index (int32 or undefined), +40 source offset (int32) | 48 |
//! | PROPS | 6 | +8 capacity, +16 count (int32s), +24 entries: key, value, attributes | 24 + 24 cap |
//! | ELEMENTS | 7 | +8 capacity (int32), +16 values (hole = empty) | 16 + 8 cap |
//! | CONTEXT | 8 | +8 previous, +16 scope id (int32), +24 length (int32), +32 slots | 32 + 8 len |
//! | ITERATOR | 9 | object + 32 iterated array or string (undefined when done), +40 next index (int32) | 48 |

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, KindId, Slot, Tracer};

use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of, native_of, native_ptr};
use crate::runtime::{Root, Runtime, val};
use crate::unit::FunctionInfo;

/// The cell test on raw bits (JSC `isCell`, H6): no number tag and no immediate tag bit.
#[inline(always)]
pub(crate) const fn is_cell_bits(bits: u64) -> bool {
    bits & ep9_value::encoding::NOT_CELL_MASK == 0
}

/// Cell kind ids (word0 byte 5).
pub(crate) mod kind {
    use ep9_heap::KindId;

    const fn k(n: u8) -> KindId {
        match KindId::new(n) {
            Some(k) => k,
            None => panic!("kind 0 is reserved"),
        }
    }

    pub const STRING: KindId = k(1);
    pub const OBJECT: KindId = k(2);
    pub const FUNCTION: KindId = k(3);
    pub const ARRAY: KindId = k(4);
    pub const ERROR: KindId = k(5);
    pub const PROPS: KindId = k(6);
    pub const ELEMENTS: KindId = k(7);
    pub const CONTEXT: KindId = k(8);
    /// v0: the array and string iterator (%ArrayIteratorPrototype%, %StringIteratorPrototype%).
    pub const ITERATOR: KindId = k(9);
}

/// Object fields shared by OBJECT, FUNCTION, ARRAY and ERROR.
pub(crate) mod obj {
    pub const PROTO: usize = 8;
    pub const PROPS: usize = 16;
    pub const FLAGS: usize = 24;
    pub const SIZE: usize = 32;
    /// Flag bits.
    pub const EXTENSIBLE: i32 = 1;
    /// A function with `[[Construct]]`.
    pub const CONSTRUCTOR: i32 = 2;
}

pub(crate) mod fun {
    pub const INFO: usize = 32;
    pub const CONTEXT: usize = 40;
    /// `[[HomeObject]]` (§10.2): set for methods; undefined otherwise.
    pub const HOME: usize = 48;
    pub const SIZE: usize = 56;
}

pub(crate) mod iter {
    pub const ITERATED: usize = 32;
    pub const NEXT: usize = 40;
    pub const SIZE: usize = 48;
}

pub(crate) mod arr {
    pub const ELEMENTS: usize = 32;
    pub const LENGTH: usize = 40;
    pub const SIZE: usize = 48;
    /// v0: dense arrays only, up to this length. Week 4 adds elements kinds (H18).
    pub const MAX_LENGTH: u32 = 1 << 26;
}

pub(crate) mod err {
    pub const UNIT: usize = 32;
    pub const OFFSET: usize = 40;
    pub const SIZE: usize = 48;
}

mod strf {
    pub const LENGTH: usize = 8;
    pub const ENCODING: usize = 16;
    pub const DATA: usize = 24;
}

/// Largest string length in code units (V8: 2^29 - 24).
pub(crate) const MAX_STRING_LENGTH: usize = (1 << 29) - 24;

mod props {
    pub const CAPACITY: usize = 8;
    pub const COUNT: usize = 16;
    pub const ENTRIES: usize = 24;
    pub const ENTRY: usize = 24;
}

mod elems {
    pub const CAPACITY: usize = 8;
    pub const DATA: usize = 16;
}

mod ctxf {
    pub const PREVIOUS: usize = 8;
    pub const SCOPE: usize = 16;
    pub const LENGTH: usize = 24;
    pub const SLOTS: usize = 32;
}

/// Property attribute bits (int32 in the table).
pub(crate) mod attr {
    pub const WRITABLE: i32 = 1;
    pub const ENUMERABLE: i32 = 2;
    pub const CONFIGURABLE: i32 = 4;
    /// Ordinary assignment: all three.
    pub const DEFAULT: i32 = WRITABLE | ENUMERABLE | CONFIGURABLE;
    /// Built-in methods and prototype links: writable, configurable, not enumerable.
    pub const HIDDEN: i32 = WRITABLE | CONFIGURABLE;
}

// ---- raw cell access ----------------------------------------------------------------------
//
// Crate invariant (H10): a cell that VM code holds (in a Rust local, on the VM stack or in a
// rooted slot) is live, and the heap never moves cells (S0-S2). The helpers below read and
// write the Value slots of such cells at offsets that the kind's layout table defines.

#[inline(always)]
pub(crate) fn cell(b: u64) -> CellPtr {
    debug_assert!(is_cell_bits(b) && b != 0);
    CellPtr::from_value(val(b)).expect("a cell value")
}

#[inline(always)]
fn slot_of<'a>(c: CellPtr, off: usize) -> &'a Slot {
    // SAFETY: `c` is live (crate invariant) and every caller passes an offset of a Value slot
    // inside the cell's layout (checked against the cell's own capacity where it varies).
    unsafe { c.slot(off) }
}

#[inline(always)]
pub(crate) fn rd(c: CellPtr, off: usize) -> u64 {
    slot_of(c, off).bits()
}

/// Initializing store into a cell allocated since the last GC point (E2: no barrier).
#[inline(always)]
pub(crate) fn init(c: CellPtr, off: usize, v: u64) {
    slot_of(c, off).init(val(v));
}

/// Barriered store.
#[inline(always)]
pub(crate) fn wr(heap: &Heap, c: CellPtr, off: usize, v: u64) {
    // SAFETY: `c` is live (crate invariant).
    let h = unsafe { c.header() };
    slot_of(c, off).set(h, val(v), heap.barrier());
}

#[inline(always)]
fn rd_i32(c: CellPtr, off: usize) -> i32 {
    let b = rd(c, off);
    debug_assert!(val(b).is_int32());
    int32_of(b)
}

/// The kind byte of a cell value, or 0 for a non-cell.
#[inline(always)]
pub(crate) fn cell_kind(b: u64) -> u8 {
    if is_cell_bits(b) {
        // SAFETY: a cell Value held by the VM is live (crate invariant).
        unsafe { cell(b).header() }.kind()
    } else {
        0
    }
}

#[inline(always)]
pub(crate) fn is_object(b: u64) -> bool {
    matches!(cell_kind(b), 2..=5 | 9)
}

#[inline(always)]
pub(crate) fn is_iterator(b: u64) -> bool {
    cell_kind(b) == kind::ITERATOR.get()
}

#[inline(always)]
pub(crate) fn is_string(b: u64) -> bool {
    cell_kind(b) == kind::STRING.get()
}

#[inline(always)]
pub(crate) fn is_function(b: u64) -> bool {
    cell_kind(b) == kind::FUNCTION.get()
}

#[inline(always)]
pub(crate) fn is_array(b: u64) -> bool {
    cell_kind(b) == kind::ARRAY.get()
}

#[inline(always)]
pub(crate) fn is_error(b: u64) -> bool {
    cell_kind(b) == kind::ERROR.get()
}

#[inline(always)]
pub(crate) fn is_context(b: u64) -> bool {
    cell_kind(b) == kind::CONTEXT.get()
}

// ---- kind registration ---------------------------------------------------------------------

fn trace_none(_: CellPtr, _: &mut Tracer<'_>) {}

fn trace_from_8(c: CellPtr, t: &mut Tracer<'_>, size: usize) {
    // SAFETY: the heap calls trace functions with live cells of the registered kind; every
    // word in [8, size) of these kinds is a Value slot (layout table above).
    t.visit_slots(unsafe { c.slots(8, (size - 8) / 8) });
}

fn trace_object(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, obj::SIZE);
}

fn trace_object48(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, 48);
}

fn trace_function(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, fun::SIZE);
}

fn string_size(c: CellPtr) -> usize {
    let len = rd_i32(c, strf::LENGTH) as usize;
    let enc = rd_i32(c, strf::ENCODING) as usize;
    strf::DATA + (len << enc)
}

fn props_size(c: CellPtr) -> usize {
    props::ENTRIES + props::ENTRY * rd_i32(c, props::CAPACITY) as usize
}

fn trace_props(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, props_size(c));
}

fn elements_size(c: CellPtr) -> usize {
    elems::DATA + 8 * rd_i32(c, elems::CAPACITY) as usize
}

fn trace_elements(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, elements_size(c));
}

fn context_size(c: CellPtr) -> usize {
    ctxf::SLOTS + 8 * rd_i32(c, ctxf::LENGTH) as usize
}

fn trace_context(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, context_size(c));
}

/// Registers the v0 cell kinds with a new heap.
pub(crate) fn register_kinds(heap: &Heap) {
    let kinds: [(KindId, CellKind); 9] = [
        (
            kind::STRING,
            CellKind {
                name: "string",
                size: CellSize::Dynamic(string_size),
                trace: trace_none,
            },
        ),
        (
            kind::OBJECT,
            CellKind {
                name: "object",
                size: CellSize::Fixed(obj::SIZE),
                trace: trace_object,
            },
        ),
        (
            kind::FUNCTION,
            CellKind {
                name: "function",
                size: CellSize::Fixed(fun::SIZE),
                trace: trace_function,
            },
        ),
        (
            kind::ARRAY,
            CellKind {
                name: "array",
                size: CellSize::Fixed(arr::SIZE),
                trace: trace_object48,
            },
        ),
        (
            kind::ERROR,
            CellKind {
                name: "error",
                size: CellSize::Fixed(err::SIZE),
                trace: trace_object48,
            },
        ),
        (
            kind::PROPS,
            CellKind {
                name: "property-table",
                size: CellSize::Dynamic(props_size),
                trace: trace_props,
            },
        ),
        (
            kind::ELEMENTS,
            CellKind {
                name: "elements",
                size: CellSize::Dynamic(elements_size),
                trace: trace_elements,
            },
        ),
        (
            kind::CONTEXT,
            CellKind {
                name: "context",
                size: CellSize::Dynamic(context_size),
                trace: trace_context,
            },
        ),
        (
            kind::ITERATOR,
            CellKind {
                name: "iterator",
                size: CellSize::Fixed(iter::SIZE),
                trace: trace_object48,
            },
        ),
    ];
    for (id, k) in kinds {
        heap.register_kind(id, k).expect("register a VM cell kind");
    }
}

// ---- strings -------------------------------------------------------------------------------

/// The code units of a string (H19 `JsStr`, v0: flat only).
#[derive(Clone, Copy, Debug)]
pub(crate) enum JsStr<'a> {
    Latin1(&'a [u8]),
    Utf16(&'a [u16]),
}

impl<'a> JsStr<'a> {
    pub fn len(&self) -> usize {
        match self {
            JsStr::Latin1(b) => b.len(),
            JsStr::Utf16(u) => u.len(),
        }
    }

    pub fn at(&self, i: usize) -> u16 {
        match self {
            JsStr::Latin1(b) => u16::from(b[i]),
            JsStr::Utf16(u) => u[i],
        }
    }

    pub fn to_utf16(self) -> Vec<u16> {
        match self {
            JsStr::Latin1(b) => b.iter().map(|&c| u16::from(c)).collect(),
            JsStr::Utf16(u) => u.to_vec(),
        }
    }

    /// Rust text; a lone surrogate becomes U+FFFD.
    pub fn to_string_lossy(self) -> String {
        match self {
            JsStr::Latin1(b) => b.iter().map(|&c| char::from(c)).collect(),
            JsStr::Utf16(u) => String::from_utf16_lossy(u),
        }
    }

    pub fn eq(&self, other: &JsStr<'_>) -> bool {
        match (self, other) {
            (JsStr::Latin1(a), JsStr::Latin1(b)) => a == b,
            (JsStr::Utf16(a), JsStr::Utf16(b)) => a == b,
            _ => self.len() == other.len() && (0..self.len()).all(|i| self.at(i) == other.at(i)),
        }
    }

    /// Code-unit order (IsLessThan on strings, §7.2.13).
    pub fn cmp(&self, other: &JsStr<'_>) -> core::cmp::Ordering {
        let n = self.len().min(other.len());
        for i in 0..n {
            match self.at(i).cmp(&other.at(i)) {
                core::cmp::Ordering::Equal => {}
                o => return o,
            }
        }
        self.len().cmp(&other.len())
    }

    /// CanonicalNumericIndexString restricted to array indices: "0", "1", ... "4294967294".
    pub fn array_index(&self) -> Option<u32> {
        let n = self.len();
        if n == 0 || n > 10 {
            return None;
        }
        let first = self.at(0);
        if first == u16::from(b'0') {
            return (n == 1).then_some(0);
        }
        let mut v: u64 = 0;
        for i in 0..n {
            let c = self.at(i);
            if !(u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                return None;
            }
            v = v * 10 + u64::from(c - u16::from(b'0'));
        }
        (v < u64::from(u32::MAX)).then_some(v as u32)
    }
}

/// The code units of a live string cell. The caller keeps the cell live while it uses the
/// view (strings never move or change).
pub(crate) fn str_view<'a>(b: u64) -> JsStr<'a> {
    debug_assert!(is_string(b));
    let c = cell(b);
    let len = rd_i32(c, strf::LENGTH) as usize;
    let utf16 = rd_i32(c, strf::ENCODING) == 1;
    // SAFETY: a live string cell holds `len` code units of its encoding from offset 24
    // (layout table); its size covers them (string_size).
    unsafe {
        if utf16 {
            JsStr::Utf16(core::slice::from_raw_parts(c.field::<u16>(strf::DATA), len))
        } else {
            JsStr::Latin1(core::slice::from_raw_parts(c.field::<u8>(strf::DATA), len))
        }
    }
}

pub(crate) fn str_len(b: u64) -> usize {
    rd_i32(cell(b), strf::LENGTH) as usize
}

fn alloc_string(rt: &Runtime, len: usize, utf16: bool) -> CellPtr {
    assert!(len <= MAX_STRING_LENGTH, "string too long");
    let size = strf::DATA + (len << usize::from(utf16));
    let c = rt.alloc(kind::STRING, size.max(16));
    init(c, strf::LENGTH, int32_bits(len as i32));
    init(c, strf::ENCODING, int32_bits(i32::from(utf16)));
    c
}

pub(crate) fn new_string_latin1(rt: &Runtime, bytes: &[u8]) -> u64 {
    let c = alloc_string(rt, bytes.len(), false);
    // SAFETY: the new cell has room for `len` bytes at offset 24 (alloc_string).
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), c.field::<u8>(strf::DATA), bytes.len())
    };
    c.to_value().to_bits()
}

/// A string from UTF-16 code units. Latin-1 storage when every unit fits.
pub(crate) fn new_string_utf16(rt: &Runtime, units: &[u16]) -> u64 {
    if units.iter().all(|&u| u <= 0xFF) {
        let bytes: Vec<u8> = units.iter().map(|&u| u as u8).collect();
        return new_string_latin1(rt, &bytes);
    }
    let c = alloc_string(rt, units.len(), true);
    // SAFETY: the new cell has room for `len` u16 at offset 24 (8-aligned).
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), c.field::<u16>(strf::DATA), units.len())
    };
    c.to_value().to_bits()
}

pub(crate) fn new_string(rt: &Runtime, s: &str) -> u64 {
    if s.is_ascii() {
        return new_string_latin1(rt, s.as_bytes());
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    new_string_utf16(rt, &units)
}

/// `a + b` for two strings. `None` if the result is too long (RangeError).
pub(crate) fn concat(rt: &Runtime, a: u64, b: u64) -> Option<u64> {
    let (la, lb) = (str_len(a), str_len(b));
    if lb == 0 {
        return Some(a);
    }
    if la == 0 {
        return Some(b);
    }
    let len = la.checked_add(lb).filter(|&n| n <= MAX_STRING_LENGTH)?;
    let (va, vb) = (str_view(a), str_view(b));
    // v0: flat concatenation (H19 adds ropes for length >= 13).
    let r = match (va, vb) {
        (JsStr::Latin1(x), JsStr::Latin1(y)) => {
            let c = alloc_string(rt, len, false);
            // SAFETY: the new cell has room for la + lb bytes; the sources are live strings
            // (the caller holds them) and do not overlap the new cell.
            unsafe {
                let d = c.field::<u8>(strf::DATA);
                core::ptr::copy_nonoverlapping(x.as_ptr(), d, la);
                core::ptr::copy_nonoverlapping(y.as_ptr(), d.add(la), lb);
            }
            c
        }
        _ => {
            let c = alloc_string(rt, len, true);
            // SAFETY: the new cell has room for len u16 code units.
            let d = unsafe { core::slice::from_raw_parts_mut(c.field::<u16>(strf::DATA), len) };
            for (i, x) in d.iter_mut().enumerate() {
                *x = if i < la { va.at(i) } else { vb.at(i - la) };
            }
            c
        }
    };
    Some(r.to_value().to_bits())
}

/// Property-key equality: the same cell, or two strings with equal code units.
#[inline]
pub(crate) fn key_eq(a: u64, b: u64) -> bool {
    a == b || (is_string(a) && is_string(b) && str_view(a).eq(&str_view(b)))
}

// ---- objects and property tables ----------------------------------------------------------

/// A new object of `kind` (OBJECT, FUNCTION, ARRAY or ERROR) with `size` bytes.
pub(crate) fn new_object_of(rt: &Runtime, k: KindId, size: usize, proto: u64) -> CellPtr {
    let c = rt.alloc(k, size);
    init(c, obj::PROTO, proto);
    init(c, obj::FLAGS, int32_bits(obj::EXTENSIBLE));
    c
}

/// `OrdinaryObjectCreate(proto)`.
pub(crate) fn new_object(rt: &Runtime, proto: u64) -> u64 {
    new_object_of(rt, kind::OBJECT, obj::SIZE, proto)
        .to_value()
        .to_bits()
}

pub(crate) fn new_plain_object(rt: &Runtime) -> u64 {
    new_object(rt, rt.root(Root::ObjectPrototype))
}

pub(crate) fn proto_of(o: u64) -> u64 {
    rd(cell(o), obj::PROTO)
}

/// Sets the prototype of `o` to `p` (an object or null), unless that makes a cycle
/// (§10.1.2 OrdinarySetPrototypeOf step 8). Returns false for a cycle.
pub(crate) fn set_proto(rt: &Runtime, o: u64, p: u64) -> bool {
    let mut q = p;
    while q != NULL {
        if q == o {
            return false;
        }
        q = proto_of(q);
    }
    wr(&rt.heap, cell(o), obj::PROTO, p);
    true
}

pub(crate) fn flags_of(o: u64) -> i32 {
    rd_i32(cell(o), obj::FLAGS)
}

fn props_cell(o: u64) -> Option<CellPtr> {
    let p = rd(cell(o), obj::PROPS);
    (p != UNDEFINED).then(|| cell(p))
}

fn props_count(p: CellPtr) -> usize {
    rd_i32(p, props::COUNT) as usize
}

fn props_capacity(p: CellPtr) -> usize {
    rd_i32(p, props::CAPACITY) as usize
}

#[inline]
fn entry(i: usize) -> usize {
    props::ENTRIES + props::ENTRY * i
}

fn new_props(rt: &Runtime, cap: usize) -> CellPtr {
    let c = rt.alloc(kind::PROPS, props::ENTRIES + props::ENTRY * cap);
    init(c, props::CAPACITY, int32_bits(cap as i32));
    init(c, props::COUNT, int32_bits(0));
    c
}

/// Index of `key` in the own property table of `o`.
fn find(o: u64, key: u64) -> Option<(CellPtr, usize)> {
    let p = props_cell(o)?;
    // v0: linear search with content compare. Shapes replace it (week 4).
    (0..props_count(p))
        .find(|&i| key_eq(rd(p, entry(i)), key))
        .map(|i| (p, i))
}

/// An own data property: (value, attributes).
pub(crate) fn own_get(o: u64, key: u64) -> Option<(u64, i32)> {
    let (p, i) = find(o, key)?;
    Some((rd(p, entry(i) + 8), rd_i32(p, entry(i) + 16)))
}

/// Writes an existing own property's value. False if `o` has no such property.
pub(crate) fn own_replace(rt: &Runtime, o: u64, key: u64, v: u64) -> bool {
    match find(o, key) {
        Some((p, i)) => {
            wr(&rt.heap, p, entry(i) + 8, v);
            true
        }
        None => false,
    }
}

/// Defines or overwrites an own data property (insertion order kept). GC point.
pub(crate) fn own_define(rt: &Runtime, o: u64, key: u64, v: u64, attrs: i32) {
    if let Some((p, i)) = find(o, key) {
        wr(&rt.heap, p, entry(i) + 8, v);
        wr(&rt.heap, p, entry(i) + 16, int32_bits(attrs));
        return;
    }
    let old = props_cell(o);
    let (count, cap) = old.map_or((0, 0), |p| (props_count(p), props_capacity(p)));
    let p = if count < cap {
        old.expect("a table with room")
    } else {
        // GC point: `o`, `key`, `v` and `old` live in Rust locals (conservative scan, H10).
        let n = new_props(rt, (cap * 2).max(4));
        if let Some(old) = old {
            for j in 0..count * 3 {
                init(n, props::ENTRIES + 8 * j, rd(old, props::ENTRIES + 8 * j));
            }
            init(n, props::COUNT, int32_bits(count as i32));
        }
        wr(&rt.heap, cell(o), obj::PROPS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, p, entry(count), key);
    wr(&rt.heap, p, entry(count) + 8, v);
    wr(&rt.heap, p, entry(count) + 16, int32_bits(attrs));
    wr(&rt.heap, p, props::COUNT, int32_bits(count as i32 + 1));
}

/// Removes an own property. Returns its attributes, or `None` if it was absent.
pub(crate) fn own_delete(rt: &Runtime, o: u64, key: u64) -> Option<i32> {
    let (p, i) = find(o, key)?;
    let attrs = rd_i32(p, entry(i) + 16);
    let count = props_count(p);
    // v0: shift the tail down (keeps insertion order).
    for j in i * 3..(count - 1) * 3 {
        wr(
            &rt.heap,
            p,
            props::ENTRIES + 8 * j,
            rd(p, props::ENTRIES + 8 * (j + 3)),
        );
    }
    for k in 0..3 {
        wr(&rt.heap, p, entry(count - 1) + 8 * k, UNDEFINED);
    }
    wr(&rt.heap, p, props::COUNT, int32_bits(count as i32 - 1));
    Some(attrs)
}

/// Number of own named properties.
pub(crate) fn own_count(o: u64) -> usize {
    props_cell(o).map_or(0, props_count)
}

/// Own named property `i` in insertion order: (key, value, attributes).
pub(crate) fn own_entry(o: u64, i: usize) -> (u64, u64, i32) {
    let p = props_cell(o).expect("own_entry on an object without properties");
    (
        rd(p, entry(i)),
        rd(p, entry(i) + 8),
        rd_i32(p, entry(i) + 16),
    )
}

// ---- arrays --------------------------------------------------------------------------------

pub(crate) fn new_array(rt: &Runtime, capacity: usize) -> u64 {
    let a = new_object_of(rt, kind::ARRAY, arr::SIZE, rt.root(Root::ArrayPrototype));
    init(a, arr::LENGTH, int32_bits(0));
    if capacity > 0 {
        let e = new_elements(rt, capacity);
        wr(&rt.heap, a, arr::ELEMENTS, e.to_value().to_bits());
    }
    a.to_value().to_bits()
}

fn new_elements(rt: &Runtime, cap: usize) -> CellPtr {
    let c = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * cap);
    init(c, elems::CAPACITY, int32_bits(cap as i32));
    for i in 0..cap {
        init(c, elems::DATA + 8 * i, HOLE);
    }
    c
}

pub(crate) fn array_len(a: u64) -> u32 {
    rd_i32(cell(a), arr::LENGTH) as u32
}

fn elements_of(a: u64) -> Option<CellPtr> {
    let e = rd(cell(a), arr::ELEMENTS);
    (e != UNDEFINED).then(|| cell(e))
}

/// Element `i`: `Some(value)` if `i < length` and not a hole.
pub(crate) fn array_get(a: u64, i: u32) -> Option<u64> {
    if i >= array_len(a) {
        return None;
    }
    let e = elements_of(a)?;
    if i as usize >= rd_i32(e, elems::CAPACITY) as usize {
        return None;
    }
    let v = rd(e, elems::DATA + 8 * i as usize);
    (v != HOLE).then_some(v)
}

/// Stores element `i`, growing the array. False if `i` is past the v0 dense limit.
pub(crate) fn array_set(rt: &Runtime, a: u64, i: u32, v: u64) -> bool {
    if i >= arr::MAX_LENGTH {
        return false;
    }
    let cap = elements_of(a).map_or(0, |e| rd_i32(e, elems::CAPACITY) as usize);
    let e = if (i as usize) < cap {
        elements_of(a).expect("elements")
    } else {
        // GC point: `a` and `v` live in Rust locals.
        let n = new_elements(rt, (i as usize + 1).max(cap * 2).max(4));
        if let Some(old) = elements_of(a) {
            for j in 0..cap {
                init(n, elems::DATA + 8 * j, rd(old, elems::DATA + 8 * j));
            }
        }
        wr(&rt.heap, cell(a), arr::ELEMENTS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, e, elems::DATA + 8 * i as usize, v);
    if i >= array_len(a) {
        wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(i as i32 + 1));
    }
    true
}

/// Sets `length` (§10.4.2.4 ArraySetLength, v0: dense, no non-configurable elements). False
/// if `len` is past the v0 dense limit.
pub(crate) fn array_set_length(rt: &Runtime, a: u64, len: u32) -> bool {
    if len > arr::MAX_LENGTH {
        return false;
    }
    let old = array_len(a);
    if len < old
        && let Some(e) = elements_of(a)
    {
        let cap = rd_i32(e, elems::CAPACITY) as u32;
        for j in len..old.min(cap) {
            wr(&rt.heap, e, elems::DATA + 8 * j as usize, HOLE);
        }
    }
    wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(len as i32));
    true
}

// ---- functions, contexts, errors -----------------------------------------------------------

pub(crate) fn new_function(rt: &Runtime, info: *const FunctionInfo, context: u64) -> u64 {
    // SAFETY: function infos outlive the isolate's cells (units and host infos are never
    // freed while the isolate lives).
    let fi = unsafe { &*info };
    let f = new_object_of(
        rt,
        kind::FUNCTION,
        fun::SIZE,
        rt.root(Root::FunctionPrototype),
    );
    let mut flags = obj::EXTENSIBLE;
    if fi.is_constructor() {
        flags |= obj::CONSTRUCTOR;
    }
    init(f, obj::FLAGS, int32_bits(flags));
    init(f, fun::INFO, native_of(info));
    init(f, fun::CONTEXT, context);
    f.to_value().to_bits()
}

/// The FunctionInfo of a function cell.
pub(crate) fn function_info<'a>(f: u64) -> &'a FunctionInfo {
    // SAFETY: a FUNCTION cell's slot +32 holds the native word of a FunctionInfo that lives as
    // long as the isolate.
    unsafe { &*native_ptr::<FunctionInfo>(rd(cell(f), fun::INFO)) }
}

pub(crate) fn function_context(f: u64) -> u64 {
    rd(cell(f), fun::CONTEXT)
}

/// `[[HomeObject]]` of a function cell (undefined if it has none).
pub(crate) fn home_object(f: u64) -> u64 {
    rd(cell(f), fun::HOME)
}

pub(crate) fn set_home_object(rt: &Runtime, f: u64, home: u64) {
    wr(&rt.heap, cell(f), fun::HOME, home);
}

/// A new v0 iterator over the array or string `iterated` (CreateArrayIterator, §23.1.5.1, kind
/// "value"; CreateStringIterator, §22.1.5.1). `proto` is its prototype.
pub(crate) fn new_iterator(rt: &Runtime, proto: u64, iterated: u64) -> u64 {
    let c = new_object_of(rt, kind::ITERATOR, iter::SIZE, proto);
    init(c, iter::ITERATED, iterated);
    init(c, iter::NEXT, int32_bits(0));
    c.to_value().to_bits()
}

/// (iterated object or undefined, next index) of an ITERATOR cell.
pub(crate) fn iterator_state(it: u64) -> (u64, u32) {
    let c = cell(it);
    (rd(c, iter::ITERATED), rd_i32(c, iter::NEXT) as u32)
}

/// Updates an ITERATOR cell; `iterated = undefined` marks it done.
pub(crate) fn set_iterator_state(rt: &Runtime, it: u64, iterated: u64, next: u32) {
    let c = cell(it);
    wr(&rt.heap, c, iter::ITERATED, iterated);
    wr(&rt.heap, c, iter::NEXT, int32_bits(next as i32));
}

pub(crate) fn is_constructor(f: u64) -> bool {
    is_function(f) && flags_of(f) & obj::CONSTRUCTOR != 0
}

pub(crate) fn new_context(rt: &Runtime, previous: u64, scope: i32, len: usize) -> u64 {
    let c = rt.alloc(kind::CONTEXT, ctxf::SLOTS + 8 * len);
    init(c, ctxf::PREVIOUS, previous);
    init(c, ctxf::SCOPE, int32_bits(scope));
    init(c, ctxf::LENGTH, int32_bits(len as i32));
    c.to_value().to_bits()
}

pub(crate) fn context_previous(c: u64) -> u64 {
    rd(cell(c), ctxf::PREVIOUS)
}

/// Slot `i`, or `None` if out of range (H02-Q2: the VM bounds-checks context slots).
pub(crate) fn context_get(c: u64, i: u32) -> Option<u64> {
    let cc = cell(c);
    ((i as usize) < rd_i32(cc, ctxf::LENGTH) as usize).then(|| rd(cc, ctxf::SLOTS + 8 * i as usize))
}

pub(crate) fn context_set(rt: &Runtime, c: u64, i: u32, v: u64) -> bool {
    let cc = cell(c);
    if (i as usize) < rd_i32(cc, ctxf::LENGTH) as usize {
        wr(&rt.heap, cc, ctxf::SLOTS + 8 * i as usize, v);
        true
    } else {
        false
    }
}

pub(crate) fn new_error_object(rt: &Runtime, proto: u64) -> u64 {
    new_object_of(rt, kind::ERROR, err::SIZE, proto)
        .to_value()
        .to_bits()
}

/// The (unit, source offset) recorded when an Error was created.
pub(crate) fn error_site(e: u64) -> Option<(u32, u32)> {
    let c = cell(e);
    let u = rd(c, err::UNIT);
    (u != UNDEFINED).then(|| (int32_of(u) as u32, int32_of(rd(c, err::OFFSET)) as u32))
}

pub(crate) fn set_error_site(rt: &Runtime, e: u64, unit: u32, offset: u32) {
    wr(&rt.heap, cell(e), err::UNIT, int32_bits(unit as i32));
    wr(&rt.heap, cell(e), err::OFFSET, int32_bits(offset as i32));
}
