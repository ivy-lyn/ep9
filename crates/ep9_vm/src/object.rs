//! JS objects on shapes: the H17 v0 layout (M0 week 4), plus contexts and the ELEMENTS
//! store. Shapes are in `shape.rs`, own-property storage (fast slots and the dictionary table)
//! in `props.rs`, internal methods in `property.rs`, exotic own properties in `exotic.rs`.
//!
//! ```text
//!  JS object (kinds 16-63)                                   word0 flags
//!  +0  word0: shape_id | gc_state | kind | flags              bits 0-7  in-object slot count
//!  +8  SLOTS (out-of-line fields) | PROPS (dictionary) | undefined   bit 8  IS_PROTOTYPE
//!  +16 ELEMENTS (arrays) | undefined                          bit 9  reserved (hash present)
//!  +24 the kind's fixed fields, then the in-object slots      bits 10-15 reserved
//!  size = 24 + 8 * (fixed fields + in-object slots)   (read from the cell: kind + flags)
//! ```
//!
//! `[[Prototype]]` and `[[Extensible]]` live in the shape. A function's `[[Construct]]` bit
//! comes from its FunctionInfo (OM-11).
//!
//! | Kind | Id | Fixed fields (from +24) | In-object slots |
//! |---|---|---|---|
//! | OBJECT | 16 | none | 4 for `{}`; n for a literal with n named keys; 8 for other prototypes (`new F`) |
//! | FUNCTION | 17 | +24 FunctionInfo (native word), +32 context, +40 `[[HomeObject]]` or undefined, +48 `[[Realm]]` (a REALM cell) | 2 (`length`, `name`) |
//! | ARRAY | 18 | +24 length (int32); elements at +16 | 0 |
//! | ERROR | 19 | +24 unit index (int32 or undefined), +32 source offset (int32) | 2 |
//! | ITERATOR | 20 | +24 iterated value: an object (undefined when done, null while an array iterator's `next` runs user code) or a string ("" when done), +32 next index (int32), +40 kind (int32, [`iter_kind`]: values, keys, entries) | 0 |
//! | PRIMITIVE_WRAPPER | 21 | +24 the primitive | 0 |
//! | BOUND_FUNCTION | 22 | +24 target, +32 bound `this`, +40 bound arguments | 2 |
//! | GENERATOR | 23 | +24 state, +32 mode (int32s), +40 function, +48 receiver, +56 context, +64 registers (an ELEMENTS cell), +72 resume mode (int32), +80 async slot ([`genf`]) | 0 |
//! | SLOT_OBJECT | 24 | +24 ObjectClass id (int32), +32 internal slots: an ELEMENTS cell of n Values, or undefined when n = 0 ([`slot_obj`]) | 0 |
//! | ARGUMENTS | 25 | +24 parameter map (a CONTEXT, or undefined for an unmapped object), +32 mapped count (int32); elements at +16 ([`argsf`]) | 3 (`length`, @@iterator, `callee`) |
//! | MODULE_NAMESPACE | 26 | +24 module (a MODULE cell), +32 export names (an ELEMENTS cell of sorted strings) ([`nsf`]) | 1 (@@toStringTag) |
//! | PROXY, PROXY_CALLABLE | 27, 28 | +24 `[[ProxyTarget]]`, +32 `[[ProxyHandler]]` (null once revoked; `proxy.rs`, M4-P4 PROXY1) | 0 |
//!
//! | Kind | Id | Layout (byte offsets) | Size |
//! |---|---|---|---|
//! | ELEMENTS | 65 | +8 capacity (int32), +16 values (hole = empty) | 16 + 8 cap |
//! | CONTEXT | 66 | +8 previous, +16 scope (native word: a [`ContextScope`] record), +24 length (int32), +32 slots | 32 + 8 len |
//! | MODULE | 76 | the Source Text Module Record (§16.2.1.4-6), 16 Values from +8 ([`modf`]) | 136 |
//! | ORDERED_TABLE | 77 | Map and Set storage: +8 capacity, +16 live count, +24 deleted count, +32 bucket count, +40 entry width, +48 flags (int32s; flag bit 0 = weak keys, bit 1 = obsolete: a rehash or `clear` replaced the table and bucket head 0 holds the forward to the new one, bit 2 = cleared: `clear` replaced it, so a cursor restarts at 0), then the bucket heads and the entries (`ordered_table.rs`) | 56 + 8 (buckets + capacity x width) |
//! | REALM | 78 | +8 global object, +16 global lexical record, +24 intrinsics (an ELEMENTS cell, index = `IntrinsicId`), +32 template registry, +40 host-defined ([`realmf`]) | 48 |
//!
//! Every word after word0 is a valid `Value` (raw pointers use the native-word encoding of
//! `frame.rs`), so one trace function per size visits them all; no trace function visits
//! the word0 shape (the heap marks it, H08). The fixed-field API ([`new_object_with_fields`],
//! [`fixed_get`], [`fixed_set`]) reads and writes fixed fields by index, so
//! PRIMITIVE_WRAPPER and BOUND_FUNCTION code never hard-codes an offset.
//!
//! OBJECT_LAYOUT 2 (`versions.rs`, M2-P0): the GENERATOR and SLOT_OBJECT kinds, the ITERATOR
//! kind field (+40; the cell grows from 40 to 48 B) and the CONTEXT scope word, which is now a
//! native word that points at the VM-side [`ContextScope`] of the context's unit scope (C5),
//! so a context names its slots: [`context_scope`] gives the unit and the scope descriptor.
//!
//! OBJECT_LAYOUT 3 (`versions.rs`, M3-P0): FUNCTION gets the `[[Realm]]` field (+48; `fun::FIXED`
//! 3 -> 4, so the in-object slots move from +48 to +56 and a function cell grows by 8 B), and
//! every function stores its isolate's REALM cell there ([`new_function`]). New kinds:
//! ARGUMENTS and MODULE_NAMESPACE (JS objects; their exotic internal methods are routed from
//! `exotic.rs` and `property.rs` to `arguments.rs` and `module.rs`), MODULE, ORDERED_TABLE and
//! REALM (internal cells). Month-3 roles fill the behaviour; the layout is fixed here.

use ep9_heap::{CellKind, CellPtr, CellSize, Heap, KindId, Slot, Tracer};

use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of, native_of, native_ptr};
use crate::kinds as kind;
use crate::runtime::{Atom, Root, Runtime, val};
use crate::shape;
use crate::unit::{ContextScope, FunctionInfo, LoadedUnit};

pub(crate) use crate::key::key_eq;
pub(crate) use crate::kinds::{
    cell_kind, is_array, is_context, is_error, is_function, is_iterator, is_object, is_string,
};
pub(crate) use crate::props::{own_count, own_define, own_delete, own_entry, own_get, own_replace};
pub(crate) use crate::string::{concat, new_string, new_string_utf16, str_len};

/// Object fields shared by every JS object kind (H17 v0).
pub(crate) mod obj {
    /// SLOTS (fast mode, out-of-line fields), PROPS (dictionary mode) or undefined.
    pub const SLOTS: usize = 8;
    /// ELEMENTS or undefined (v0: arrays only).
    pub const ELEMENTS: usize = 16;
    /// The first fixed field; the in-object slots follow the fixed fields.
    pub const FIXED: usize = 24;
    /// Most in-object slots (word0 flag bits 0-7).
    pub const MAX_INOBJECT: usize = 255;
    /// Flag bits of [`super::flags_of`] (the week-3 FLAGS word, now computed).
    pub const EXTENSIBLE: i32 = 1;
    /// A function with `[[Construct]]`.
    pub const CONSTRUCTOR: i32 = 2;
}

pub(crate) mod fun {
    pub const INFO: usize = 24;
    pub const CONTEXT: usize = 32;
    /// `[[HomeObject]]` (§10.2): set for methods; undefined otherwise.
    pub const HOME: usize = 40;
    /// `[[Realm]]` (§10.2): the REALM cell of the realm the function was made in
    /// (OBJECT_LAYOUT 3). Host functions have one too.
    pub const REALM: usize = 48;
    pub const FIXED: usize = 4;
    /// In-object slots: `length` and `name`.
    pub const CAPACITY: u8 = 2;
}

pub(crate) mod iter {
    pub const ITERATED: usize = 24;
    pub const NEXT: usize = 32;
    /// The iteration kind (int32, [`super::iter_kind`]).
    pub const KIND: usize = 40;
    pub const FIXED: usize = 3;
}

/// The kind field of an ITERATOR cell (CreateArrayIterator's `kind`, §23.1.5.1). String
/// iterators use [`VALUES`](iter_kind::VALUES).
pub(crate) mod iter_kind {
    pub const VALUES: i32 = 0;
    pub const KEYS: i32 = 1;
    pub const ENTRIES: i32 = 2;
}

/// GENERATOR fixed fields (OBJECT_LAYOUT 2). Every field holds a Value; state, mode and
/// resume mode are int32s. One kind serves generators, async functions and async generators
/// (the mode). Accessors: [`new_generator`], [`gen_get`], [`gen_set`], [`gen_regs`].
pub(crate) mod genf {
    /// The suspend point id, or a closed/executing marker (int32).
    pub const STATE: usize = 24;
    /// [`MODE_SYNC`], [`MODE_ASYNC_FUNCTION`] or [`MODE_ASYNC_GENERATOR`] (int32).
    pub const MODE: usize = 32;
    /// The generator function.
    pub const FUNCTION: usize = 40;
    /// The receiver (`this`) of the body.
    pub const RECEIVER: usize = 48;
    /// The context of the suspended frame.
    pub const CONTEXT: usize = 56;
    /// The saved parameters and registers: an ELEMENTS cell (never a Rust buffer, plan rule 8).
    pub const REGS: usize = 64;
    /// How the frame resumes: next, return or throw (int32).
    pub const RESUME_MODE: usize = 72;
    /// Async functions and async generators: the promise or the request queue.
    pub const ASYNC: usize = 80;
    pub const FIXED: usize = 8;
    pub const MODE_SYNC: i32 = 0;
    pub const MODE_ASYNC_FUNCTION: i32 = 1;
    #[allow(dead_code)] // month 3 (async)
    pub const MODE_ASYNC_GENERATOR: i32 = 2;
    /// [`STATE`] of a completed generator (M2-P1-GEN-NUMBERS; suspend ids are >= 0).
    pub const STATE_CLOSED: i32 = -1;
    /// [`STATE`] while the body runs.
    pub const STATE_EXECUTING: i32 = -2;
    /// [`RESUME_MODE`] values (M2-P1-GEN-NUMBERS, V8's `JSGeneratorObject::ResumeMode`).
    pub const RESUME_NEXT: i32 = 0;
    pub const RESUME_RETURN: i32 = 1;
    pub const RESUME_THROW: i32 = 2;
    /// [`REGS`] index of the value that `next`/`return`/`throw` passed (`ResumeGenerator`
    /// puts it in the accumulator); the saved parameters, then the registers, follow it.
    pub const REGS_INPUT: usize = 0;
}

/// SLOT_OBJECT fixed fields (OBJECT_LAYOUT 2): the ObjectClass id and a separate cell for the
/// internal slots. The internal slots are never in-object slots: property code owns those
/// (`props::field_ref` stores named fields there, `props::to_dictionary` clears them), and it
/// never touches fixed fields. Accessors: [`new_slot_object`] and its neighbours (file end).
pub(crate) mod slot_obj {
    /// The ObjectClass id (int32).
    pub const CLASS: usize = 24;
    /// The internal slots: an ELEMENTS cell of n Values, or undefined when the class has none.
    pub const SLOTS: usize = 32;
    pub const FIXED: usize = 2;
    pub const MAX_SLOTS: usize = 255; // most internal slots of one object (more is a caller bug)
}

/// ARGUMENTS fixed fields (OBJECT_LAYOUT 3, §10.4.4). The elements (the argument values)
/// sit at `obj::ELEMENTS` (+16). Filled by M3-P2 VM-ARGS (`arguments.rs`).
pub(crate) mod argsf {
    /// The parameter map: the function's CONTEXT whose slots alias the mapped parameters, or
    /// undefined for an unmapped arguments object.
    pub const MAP: usize = 24;
    /// The number of mapped parameters (int32; 0 for an unmapped object).
    pub const MAPPED_COUNT: usize = 32;
    pub const FIXED: usize = 2;
    /// In-object slots: `length`, @@iterator and `callee`.
    pub const CAPACITY: u8 = 3;
}

/// MODULE_NAMESPACE fixed fields (OBJECT_LAYOUT 3, §10.4.6). Filled by M3-P2 MODULE
/// (`module.rs`).
pub(crate) mod nsf {
    /// The module: a MODULE cell.
    pub const MODULE: usize = 24;
    /// The exports: an ELEMENTS cell of 2n Values, the n export names (strings in code-unit
    /// order), then the n bindings (a box: an ELEMENTS of length 1, the hole while the
    /// binding is in its TDZ; or a MODULE cell for `export * as ns`). `module.rs` makes it.
    pub const EXPORTS: usize = 32;
    pub const FIXED: usize = 2;
    /// In-object slots: @@toStringTag.
    pub const CAPACITY: u8 = 1;
}

/// MODULE fields (OBJECT_LAYOUT 3): a Source Text Module Record (§16.2.1.4 Cyclic Module
/// Record, §16.2.1.6). Every field is a Value; the unit is a native word. Filled by M3-P2
/// MODULE and M3-P3 IMPORT-TLA (`module.rs`).
#[allow(dead_code)] // filled by M3-P2 MODULE
pub(crate) mod modf {
    /// `[[Status]]` (int32; undefined until MODULE fills it).
    pub const STATUS: usize = 8;
    /// `[[Environment]]`: a CONTEXT made with the record (`module.rs` `env`): the module's
    /// name, the generator object of its function 0 (made at InitializeEnvironment, resumed
    /// at Evaluate; MODULE step 2), `import.meta`, then one box per module cell.
    pub const ENVIRONMENT: usize = 16;
    /// `[[Namespace]]`: a MODULE_NAMESPACE object, or undefined until made.
    pub const NAMESPACE: usize = 24;
    /// The module's unit: a native word to its `LoadedUnit`.
    pub const UNIT: usize = 32;
    /// `[[RequestedModules]]`: an ELEMENTS cell of specifier strings.
    pub const REQUESTED: usize = 40;
    /// `[[LoadedModules]]`: an ELEMENTS cell of MODULE cells (or undefined entries).
    pub const LOADED: usize = 48;
    /// `[[EvaluationError]]`: the thrown value, or the hole for none.
    pub const EVALUATION_ERROR: usize = 56;
    /// `[[DFSIndex]]` (int32, or undefined for empty).
    pub const DFS_INDEX: usize = 64;
    /// `[[DFSAncestorIndex]]` (int32, or undefined for empty).
    pub const DFS_ANCESTOR_INDEX: usize = 72;
    /// `[[CycleRoot]]`: a MODULE cell, or undefined.
    pub const CYCLE_ROOT: usize = 80;
    /// `[[HasTLA]]` (a boolean; undefined until MODULE fills it).
    pub const HAS_TLA: usize = 88;
    /// `[[AsyncEvaluationOrder]]` (int32, or undefined for unset, or done).
    pub const ASYNC_EVALUATION_ORDER: usize = 96;
    /// `[[TopLevelCapability]]`: a promise capability record, or undefined.
    pub const TOP_LEVEL_CAPABILITY: usize = 104;
    /// `[[AsyncParentModules]]`: an ELEMENTS cell of MODULE cells, or undefined.
    pub const ASYNC_PARENT_MODULES: usize = 112;
    /// `[[PendingAsyncDependencies]]` (int32, or undefined for empty).
    pub const PENDING_ASYNC_DEPENDENCIES: usize = 120;
    /// `[[HostDefined]]`: any Value.
    pub const HOST_DEFINED: usize = 128;
    /// The cell size: word0 and 16 Values.
    pub const SIZE: usize = 136;
}

/// REALM fields (OBJECT_LAYOUT 3, §9.3 Realm Record). N per isolate: `realm::init` makes the
/// first, `realm::create` (`Cx::create_realm`, `$262.createRealm`) makes more, and a realm
/// that nothing reachable holds is collected. Every function's `[[Realm]]` ([`fun::REALM`]).
pub(crate) mod realmf {
    /// `[[GlobalObject]]`.
    pub const GLOBAL_OBJECT: usize = 8;
    /// The global declarative record (script `let`/`const`/`class`; today a null-prototype
    /// object, as the `Root::Lexical` root).
    pub const GLOBAL_LEXICAL: usize = 16;
    /// `[[Intrinsics]]`: an ELEMENTS cell, index = `IntrinsicId` (the H13 stable index);
    /// undefined for an intrinsic nothing made.
    pub const INTRINSICS: usize = 24;
    /// `[[TemplateMap]]`: an ELEMENTS list (entry 0 = the used count) that owns the template
    /// objects this realm made (`realm::add_template`); undefined until the first one.
    pub const TEMPLATES: usize = 32;
    /// `[[HostDefined]]`: the realm's module map (`module.rs` `map_get`, `map_add`): an
    /// ELEMENTS list, entry 0 the count n, then n pairs (name atom, MODULE record).
    pub const HOST_DEFINED: usize = 40;
    /// The cell size: word0 and 5 Values.
    pub const SIZE: usize = 48;
}

pub(crate) mod arr {
    pub const LENGTH: usize = 24;
    pub const FIXED: usize = 1;
    /// v0: dense arrays only, up to this length. Elements kinds come with H18 (M1a).
    pub const MAX_LENGTH: u32 = 1 << 26;
}

pub(crate) mod err {
    pub const UNIT: usize = 24;
    pub const OFFSET: usize = 32;
    pub const FIXED: usize = 2;
}

/// Fixed-field counts of the kinds that only use the fixed-field API.
pub(crate) mod fixed {
    /// PRIMITIVE_WRAPPER: the primitive value.
    pub const WRAPPER: usize = 1;
    /// BOUND_FUNCTION: target, bound `this`, bound arguments.
    pub const BOUND: usize = 3;
    /// PROXY and PROXY_CALLABLE: `[[ProxyTarget]]`, `[[ProxyHandler]]` (`proxy.rs`).
    pub const PROXY: usize = 2;
}

/// ELEMENTS cell fields (+8 capacity (int32), +16 values). Also used by the literal-site hit
/// (`literal_site.rs`).
pub(crate) mod elems {
    pub const CAPACITY: usize = 8;
    pub const DATA: usize = 16;
}

mod ctxf {
    pub const PREVIOUS: usize = 8;
    pub const SCOPE: usize = 16;
    pub const LENGTH: usize = 24;
    pub const SLOTS: usize = 32;
}

/// Property attribute bits (int32 in the dictionary table; bits 0-3 of shape details).
pub(crate) mod attr {
    pub const WRITABLE: i32 = 1;
    pub const ENUMERABLE: i32 = 2;
    pub const CONFIGURABLE: i32 = 4;
    /// The slot holds an ACCESSOR_PAIR (H17, OM-9; P3-ACC stores the pairs). Equals
    /// `shape::ACCESSOR`.
    pub const ACCESSOR: i32 = 8;
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
    debug_assert!(kind::is_cell_bits(b) && b != 0);
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

/// Initializing store into a cell allocated since the last GC point (E2: no barrier). Once a
/// GC-stress heap exists, the heap's E2 check tests the claim (`ep9_heap::e2_check_init`: the
/// host must not have survived a GC) and names this call's caller.
#[inline(always)]
#[track_caller]
pub(crate) fn init(c: CellPtr, off: usize, v: u64) {
    if ep9_heap::e2_check_on() {
        ep9_heap::e2_check_init(c, v, core::panic::Location::caller());
    }
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
pub(crate) fn rd_i32(c: CellPtr, off: usize) -> i32 {
    let b = rd(c, off);
    debug_assert!(val(b).is_int32());
    int32_of(b)
}

// ---- kind registration ---------------------------------------------------------------------

/// Visits every word in [8, size) of `c` as a Value slot.
pub(crate) fn trace_from_8(c: CellPtr, t: &mut Tracer<'_>, size: usize) {
    // SAFETY: the heap calls trace functions with live cells of the registered kind; every
    // word in [8, size) of these kinds is a Value slot (layout tables).
    t.visit_slots(unsafe { c.slots(8, (size - 8) / 8) });
}

/// The in-object slot count of a JS object cell (word0 flag bits 0-7).
#[inline(always)]
pub(crate) fn inobject_count(c: CellPtr) -> usize {
    // SAFETY: `c` is live (crate invariant; the heap passes live cells to size functions).
    usize::from(unsafe { c.header() }.flags() & kind::FLAG_INOBJECT_MASK)
}

/// The size of a JS object cell with `F` fixed fields.
fn object_size<const F: usize>(c: CellPtr) -> usize {
    obj::FIXED + 8 * (F + inobject_count(c))
}

fn trace_object<const F: usize>(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, object_size::<F>(c));
}

/// FUNCTION: the slots (the INFO word is a native word, not a reference), then the referrer
/// realm of its eval unit (`module::trace_referrer_realm`, C3-M1): an edge through the
/// unit's side table, not a slot of the cell (OBJECT_LAYOUT 3 does not change).
fn trace_function(c: CellPtr, t: &mut Tracer<'_>) {
    trace_object::<{ fun::FIXED }>(c, t);
    // SAFETY: the slot +24 of a FUNCTION cell holds the native word of a FunctionInfo that
    // lives as long as the isolate (`new_function` writes it before the next GC point).
    let fi = unsafe { &*native_ptr::<FunctionInfo>(rd(c, fun::INFO)) };
    crate::module::trace_referrer_realm(fi, t);
}

/// Fixed-field counts by kind id (0 for OBJECT and for ids that are not JS object kinds).
const FIXED_COUNTS: [u8; 256] = {
    let mut t = [0u8; 256];
    t[kind::FUNCTION.get() as usize] = fun::FIXED as u8;
    t[kind::ARRAY.get() as usize] = arr::FIXED as u8;
    t[kind::ERROR.get() as usize] = err::FIXED as u8;
    t[kind::ITERATOR.get() as usize] = iter::FIXED as u8;
    t[kind::PRIMITIVE_WRAPPER.get() as usize] = fixed::WRAPPER as u8;
    t[kind::BOUND_FUNCTION.get() as usize] = fixed::BOUND as u8;
    t[kind::GENERATOR.get() as usize] = genf::FIXED as u8;
    t[kind::SLOT_OBJECT.get() as usize] = slot_obj::FIXED as u8;
    t[kind::ARGUMENTS.get() as usize] = argsf::FIXED as u8;
    t[kind::MODULE_NAMESPACE.get() as usize] = nsf::FIXED as u8;
    t[kind::PROXY.get() as usize] = fixed::PROXY as u8;
    t[kind::PROXY_CALLABLE.get() as usize] = fixed::PROXY as u8;
    t
};

/// The byte offset of in-object field 0 of an OBJECT cell: `obj::FIXED` plus OBJECT's fixed
/// fields (none today), the offset `ic.rs` `field_location` gives field 0. The literal and
/// clone hits write fields from here (`literal_site.rs`), so a fixed field added to OBJECT
/// moves them too (M4-P1 review R4).
pub(crate) const OBJECT_FIELD0: usize =
    obj::FIXED + 8 * FIXED_COUNTS[kind::OBJECT.get() as usize] as usize;

/// The fixed-field count of the JS object kind `k` (16-63): one table load.
#[inline(always)]
pub(crate) fn fixed_count(k: u8) -> usize {
    debug_assert!(
        [
            kind::OBJECT,
            kind::FUNCTION,
            kind::ARRAY,
            kind::ERROR,
            kind::ITERATOR,
            kind::PRIMITIVE_WRAPPER,
            kind::BOUND_FUNCTION,
            kind::GENERATOR,
            kind::SLOT_OBJECT,
            kind::ARGUMENTS,
            kind::MODULE_NAMESPACE,
            kind::PROXY,
            kind::PROXY_CALLABLE
        ]
        .iter()
        .any(|x| x.get() == k),
        "kind {k} is not a JS object kind"
    );
    usize::from(FIXED_COUNTS[usize::from(k)])
}

/// The in-object slot count new objects of kind `k` get (the OBJECT count is chosen per
/// prototype in [`new_object`]).
fn default_capacity(k: KindId) -> u8 {
    if k == kind::FUNCTION || k == kind::ERROR || k == kind::BOUND_FUNCTION {
        2
    } else if k == kind::OBJECT {
        4
    } else {
        0
    }
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

fn object_kind(
    name: &'static str,
    size: fn(CellPtr) -> usize,
    trace: fn(CellPtr, &mut Tracer<'_>),
) -> CellKind {
    CellKind {
        name,
        size: CellSize::Dynamic(size),
        trace,
    }
}

fn trace_module(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, modf::SIZE);
}

fn trace_realm(c: CellPtr, t: &mut Tracer<'_>) {
    trace_from_8(c, t, realmf::SIZE);
}

/// Registers every VM cell kind with a new heap: the kinds of this file, then each module
/// that owns a layout (strings, the property dictionary, symbols, shapes, feedback,
/// accessors, ICs, the ordered tables).
pub(crate) fn register_kinds(heap: &Heap) {
    let kinds: [(KindId, CellKind); 17] = [
        (
            kind::OBJECT,
            object_kind("object", object_size::<0>, trace_object::<0>),
        ),
        (
            kind::FUNCTION,
            object_kind("function", object_size::<{ fun::FIXED }>, trace_function),
        ),
        (
            kind::ARRAY,
            object_kind(
                "array",
                object_size::<{ arr::FIXED }>,
                trace_object::<{ arr::FIXED }>,
            ),
        ),
        (
            kind::ERROR,
            object_kind(
                "error",
                object_size::<{ err::FIXED }>,
                trace_object::<{ err::FIXED }>,
            ),
        ),
        (
            kind::ITERATOR,
            object_kind(
                "iterator",
                object_size::<{ iter::FIXED }>,
                trace_object::<{ iter::FIXED }>,
            ),
        ),
        (
            kind::PRIMITIVE_WRAPPER,
            object_kind(
                "primitive-wrapper",
                object_size::<{ fixed::WRAPPER }>,
                trace_object::<{ fixed::WRAPPER }>,
            ),
        ),
        (
            kind::BOUND_FUNCTION,
            object_kind(
                "bound-function",
                object_size::<{ fixed::BOUND }>,
                trace_object::<{ fixed::BOUND }>,
            ),
        ),
        (
            kind::GENERATOR,
            object_kind(
                "generator",
                object_size::<{ genf::FIXED }>,
                trace_object::<{ genf::FIXED }>,
            ),
        ),
        (
            kind::SLOT_OBJECT,
            object_kind(
                "slot-object",
                object_size::<{ slot_obj::FIXED }>,
                trace_object::<{ slot_obj::FIXED }>,
            ),
        ),
        (
            kind::ARGUMENTS,
            object_kind(
                "arguments",
                object_size::<{ argsf::FIXED }>,
                trace_object::<{ argsf::FIXED }>,
            ),
        ),
        (
            kind::MODULE_NAMESPACE,
            object_kind(
                "module-namespace",
                object_size::<{ nsf::FIXED }>,
                trace_object::<{ nsf::FIXED }>,
            ),
        ),
        (
            kind::PROXY,
            object_kind(
                "proxy",
                object_size::<{ fixed::PROXY }>,
                trace_object::<{ fixed::PROXY }>,
            ),
        ),
        (
            kind::PROXY_CALLABLE,
            object_kind(
                "proxy-callable",
                object_size::<{ fixed::PROXY }>,
                trace_object::<{ fixed::PROXY }>,
            ),
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
            kind::MODULE,
            CellKind {
                name: "module",
                size: CellSize::Fixed(modf::SIZE),
                trace: trace_module,
            },
        ),
        (
            kind::REALM,
            CellKind {
                name: "realm",
                size: CellSize::Fixed(realmf::SIZE),
                trace: trace_realm,
            },
        ),
    ];
    crate::string::register_kinds(heap);
    for (id, k) in kinds {
        heap.register_kind(id, k).expect("register a VM cell kind");
    }
    crate::props::register_kinds(heap);
    crate::symbol::register_kinds(heap);
    crate::shape::register_kinds(heap);
    crate::feedback::register_kinds(heap);
    crate::accessor::register_kinds(heap);
    crate::ic::register_kinds(heap);
    crate::ordered_table::register_kinds(heap);
}

// ---- objects --------------------------------------------------------------------------------

/// A new JS object of kind `k` with prototype `proto`, `capacity` in-object slots and no
/// properties; its fixed fields and slots hold undefined. GC point.
pub(crate) fn new_js_object(rt: &Runtime, k: KindId, proto: u64, capacity: u8) -> CellPtr {
    // GC points: the shape first (it may make `proto` a prototype), then the cell. Both
    // live in Rust locals (CSS).
    let s = shape::initial_shape(rt, proto, k, capacity);
    alloc_with_shape(rt, k, s, capacity)
}

/// Allocates a JS object of kind `k` with `capacity` in-object slots and installs `shape`
/// (a live shape of that kind) before the next GC point. GC point (the allocation).
fn alloc_with_shape(rt: &Runtime, k: KindId, s: u64, capacity: u8) -> CellPtr {
    let size = obj::FIXED + 8 * (fixed_count(k.get()) + usize::from(capacity));
    let c = rt.alloc(k, size);
    // The size function reads the in-object count: set it before the next GC point.
    // SAFETY: `c` is the new live cell.
    unsafe { c.header() }.set_flags(u16::from(capacity));
    shape::install(rt, c.to_value().to_bits(), s);
    c
}

/// A new JS object of kind `k` with `shape` (a live fast shape of that kind) and the shape's
/// in-object capacity (`shape::field_layout`); every field holds undefined. GC point (the
/// allocation): the caller has published its frame and keeps every Value it needs in a frame
/// or a Rust local. The literal-site and construct-site hits use it (`literal_site.rs`,
/// `construct_ic.rs`; M4-P1 OBJ-ALLOC).
///
/// The body is [`alloc_with_shape`]'s, kept as a second copy on purpose: `inline(always)`
/// keeps the allocation inside the T0 hit handlers, and leaving `alloc_with_shape` without
/// the attribute keeps the code of its other callers as it was (one shared inline body
/// changed the handler call counts of two targets by one, disasm-check, M4-P1 fixer).
#[inline(always)]
pub(crate) fn alloc_from_shape(rt: &Runtime, k: KindId, shape: u64) -> CellPtr {
    let (cap, sk) = shape::field_layout(shape);
    debug_assert_eq!(sk, k.get(), "a shape of another kind");
    let size = obj::FIXED + 8 * (fixed_count(k.get()) + cap);
    let c = rt.alloc(k, size);
    // The size function reads the in-object count: set it before the next GC point.
    // SAFETY: `c` is the new live cell.
    unsafe { c.header() }.set_flags(cap as u16);
    shape::install(rt, c.to_value().to_bits(), shape);
    c
}

/// `OrdinaryObjectCreate(proto)` (`proto` an object or null). In-object slots: 4 when the
/// prototype is %Object.prototype% (`{}`), 8 otherwise (`new F`, which reaches here
/// through OrdinaryCreateFromConstructor, and Object.create). GC point.
pub(crate) fn new_object(rt: &Runtime, proto: u64) -> u64 {
    let cap = if proto == rt.root(Root::ObjectPrototype) {
        4
    } else {
        8
    };
    new_object_with_capacity(rt, proto, cap)
}

/// An ordinary object with `capacity` in-object slots (literals: one per named key, at most
/// [`obj::MAX_INOBJECT`]). GC point.
pub(crate) fn new_object_with_capacity(rt: &Runtime, proto: u64, capacity: usize) -> u64 {
    let cap = capacity.min(obj::MAX_INOBJECT) as u8;
    new_js_object(rt, kind::OBJECT, proto, cap)
        .to_value()
        .to_bits()
}

pub(crate) fn new_plain_object(rt: &Runtime) -> u64 {
    new_object(rt, rt.root(Root::ObjectPrototype))
}

/// A new extensible object of kind `k` with prototype `proto` and `n` fixed fields, all
/// undefined. `n` must be the kind's fixed-field count. GC point.
#[allow(dead_code)] // callers: wrapper.rs (P2-SYM), bound.rs (P3-BLTA)
pub(crate) fn new_object_with_fields(rt: &Runtime, k: KindId, proto: u64, n: usize) -> u64 {
    assert_eq!(
        n,
        fixed_count(k.get()),
        "kind {} has another fixed-field count",
        k.get()
    );
    new_js_object(rt, k, proto, default_capacity(k))
        .to_value()
        .to_bits()
}

/// Fixed field `i` of the object `o`.
#[allow(dead_code)] // callers: wrapper.rs (P2-SYM), bound.rs (P3-BLTA)
pub(crate) fn fixed_get(o: u64, i: usize) -> u64 {
    debug_assert!(i < fixed_count(cell_kind(o)));
    rd(cell(o), obj::FIXED + 8 * i)
}

/// Stores fixed field `i` of the object `o` (barriered).
#[allow(dead_code)] // callers: wrapper.rs (P2-SYM), bound.rs (P3-BLTA)
pub(crate) fn fixed_set(rt: &Runtime, o: u64, i: usize, v: u64) {
    debug_assert!(i < fixed_count(cell_kind(o)));
    wr(&rt.heap, cell(o), obj::FIXED + 8 * i, v);
}

/// `[[Prototype]]` of the object `o` (an object or null): its shape's.
#[inline]
pub(crate) fn proto_of(o: u64) -> u64 {
    shape::proto(shape::shape_of(o))
}

/// Sets the prototype of `o` to `p` (an object or null), unless that makes a cycle
/// (§10.1.2 OrdinarySetPrototypeOf step 8). Returns false for a cycle. A raw store: it
/// does not check `[[Extensible]]` (`property::set_prototype_of` does). GC point.
pub(crate) fn set_proto(rt: &Runtime, o: u64, p: u64) -> bool {
    let mut q = p;
    while q != NULL {
        if q == o {
            return false;
        }
        // Step 8.c.i: a proxy's [[GetPrototypeOf]] is not the ordinary one, so the walk stops
        // there (no cycle). Its cell's own prototype is null, so this only says it.
        if crate::proxy::is_proxy_kind(cell_kind(q)) {
            break;
        }
        q = proto_of(q);
    }
    if proto_of(o) == p {
        return true;
    }
    let s = shape::shape_of(o);
    if !shape::is_dictionary(s) && shape::flags(s) & shape::flag::UNIQUE == 0 && shape::count(s) > 0
    {
        // OM-10: an object with properties in a shared fast shape goes to dictionary mode.
        crate::props::to_dictionary(rt, o);
    }
    // GC point below: `o` and `p` live in Rust locals.
    let cap = inobject_count(cell(o)) as u8;
    let ns = shape::with_proto(rt, shape::shape_of(o), p, cap);
    shape::install(rt, o, ns);
    shape::note_change(rt, o);
    true
}

/// The week-3 FLAGS word, computed: [`obj::EXTENSIBLE`] from the shape, [`obj::CONSTRUCTOR`]
/// for a FUNCTION with `[[Construct]]`.
#[allow(dead_code)] // the P0 compatibility API (no caller left)
pub(crate) fn flags_of(o: u64) -> i32 {
    let mut f = 0;
    if shape::is_extensible(shape::shape_of(o)) {
        f |= obj::EXTENSIBLE;
    }
    if is_constructor(o) {
        f |= obj::CONSTRUCTOR;
    }
    f
}

// ---- arrays --------------------------------------------------------------------------------

pub(crate) fn new_array(rt: &Runtime, capacity: usize) -> u64 {
    let a = new_js_object(rt, kind::ARRAY, rt.root(Root::ArrayPrototype), 0);
    init(a, arr::LENGTH, int32_bits(0));
    if capacity > 0 {
        let e = new_elements(rt, capacity);
        wr(&rt.heap, a, obj::ELEMENTS, e.to_value().to_bits());
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

/// A new ELEMENTS cell of `n` Values, all `fill` (a list that a record holds: the realm's
/// intrinsics, a module's requested modules). GC point: `fill` lives in a Rust local.
pub(crate) fn new_list(rt: &Runtime, n: usize, fill: u64) -> u64 {
    let c = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * n);
    init(c, elems::CAPACITY, int32_bits(n as i32));
    for i in 0..n {
        init(c, elems::DATA + 8 * i, fill);
    }
    c.to_value().to_bits()
}

/// The length of the ELEMENTS cell `l`.
pub(crate) fn list_len(l: u64) -> usize {
    debug_assert!(cell_kind(l) == kind::ELEMENTS.get());
    rd_i32(cell(l), elems::CAPACITY) as usize
}

/// Entry `i` of the ELEMENTS cell `l`. Panics if `i` ≥ [`list_len`].
#[allow(dead_code)] // callers: tests/layout3.rs; month-3 records (realms, modules)
pub(crate) fn list_get(l: u64, i: usize) -> u64 {
    assert!(i < list_len(l), "list index {i} out of range");
    rd(cell(l), elems::DATA + 8 * i)
}

/// Stores entry `i` of the ELEMENTS cell `l` (barriered). Panics if `i` ≥ [`list_len`].
pub(crate) fn list_set(rt: &Runtime, l: u64, i: usize, v: u64) {
    assert!(i < list_len(l), "list index {i} out of range");
    wr(&rt.heap, cell(l), elems::DATA + 8 * i, v);
}

/// Field `off` of an internal record cell (MODULE [`modf`], REALM [`realmf`]): a Value slot
/// from +8 below the kind's size.
pub(crate) fn record_get(r: u64, off: usize) -> u64 {
    debug_assert!(record_field_ok(r, off), "bad record field {off}");
    rd(cell(r), off)
}

/// Stores field `off` of an internal record cell (barriered).
pub(crate) fn record_set(rt: &Runtime, r: u64, off: usize, v: u64) {
    debug_assert!(record_field_ok(r, off), "bad record field {off}");
    wr(&rt.heap, cell(r), off, v);
}

fn record_field_ok(r: u64, off: usize) -> bool {
    let size = match cell_kind(r) {
        k if k == kind::MODULE.get() => modf::SIZE,
        k if k == kind::REALM.get() => realmf::SIZE,
        _ => return false,
    };
    (8..size).contains(&off) && off.is_multiple_of(8)
}

/// A new internal record cell of kind `k` (MODULE or REALM), every field undefined. GC point.
pub(crate) fn new_record(rt: &Runtime, k: KindId) -> u64 {
    let size = if k == kind::MODULE {
        modf::SIZE
    } else {
        assert!(k == kind::REALM, "kind {} is not a record kind", k.get());
        realmf::SIZE
    };
    let c = rt.alloc(k, size);
    for off in (8..size).step_by(8) {
        init(c, off, UNDEFINED);
    }
    c.to_value().to_bits()
}

pub(crate) fn array_len(a: u64) -> u32 {
    rd_i32(cell(a), arr::LENGTH) as u32
}

fn elements_of(a: u64) -> Option<CellPtr> {
    let e = rd(cell(a), obj::ELEMENTS);
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

/// Stores element `i`, growing the array; [`HOLE`] removes it. False if `i` is past the v0
/// dense limit. A raw store: callers check `[[Extensible]]` for a new element, and a
/// `[[Set]]` caller the prototype chain (`shape::proto_index_keys`). Every element write goes
/// through here and [`array_set_length`]: an element that appears or goes away is a
/// structural change of a prototype (OM-8, `shape::note_change`); a value store is not. An
/// element that appears on a prototype also sets `shape::proto_index_keys`.
pub(crate) fn array_set(rt: &Runtime, a: u64, i: u32, v: u64) -> bool {
    if i >= arr::MAX_LENGTH {
        return false;
    }
    // Only a prototype pays for more than the flag test.
    let structural = shape::is_prototype_object(a) && array_get(a, i).is_some() != (v != HOLE);
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
        wr(&rt.heap, cell(a), obj::ELEMENTS, n.to_value().to_bits());
        n
    };
    wr(&rt.heap, e, elems::DATA + 8 * i as usize, v);
    if i >= array_len(a) {
        wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(i as i32 + 1));
    }
    if structural {
        if v != HOLE {
            shape::note_proto_index_key(rt, a);
        }
        shape::note_change(rt, a);
    }
    true
}

/// Sets `length` (§10.4.2.4 ArraySetLength, v0: dense, no non-configurable elements). False
/// if `len` is past the v0 dense limit. A cut that removes an element is a structural change
/// of a prototype (OM-8); growth and a cut over holes only change the value of `length`.
pub(crate) fn array_set_length(rt: &Runtime, a: u64, len: u32) -> bool {
    if len > arr::MAX_LENGTH {
        return false;
    }
    let old = array_len(a);
    if len < old
        && let Some(e) = elements_of(a)
    {
        let end = old.min(rd_i32(e, elems::CAPACITY) as u32);
        let at = |j: u32| elems::DATA + 8 * j as usize;
        // Only a prototype pays for more than the flag test.
        let structural = shape::is_prototype_object(a) && (len..end).any(|j| rd(e, at(j)) != HOLE);
        for j in len..end {
            wr(&rt.heap, e, at(j), HOLE);
        }
        if structural {
            shape::note_change(rt, a);
        }
    }
    wr(&rt.heap, cell(a), arr::LENGTH, int32_bits(len as i32));
    true
}

// ---- functions, contexts, errors -----------------------------------------------------------

/// A new function object for `info` with `context`. It gets the own data properties
/// `length` and `name` from the FunctionInfo, {w: false, e: false, c: true} (§10.2.9
/// SetFunctionName, §10.2.10 SetFunctionLength), in its 2 in-object slots. `prototype`
/// stays lazy (`exotic.rs`). Its `[[Prototype]]` is %Function.prototype%, or
/// %GeneratorFunction.prototype% for a generator function (§15.5.3),
/// %AsyncFunction.prototype% for an async function (§15.8.3) and
/// %AsyncGeneratorFunction.prototype% for an async generator function (§15.6.3). Its
/// `[[Realm]]` is the current realm (the `Root::Realm` root; OBJECT_LAYOUT 3). GC point.
pub(crate) fn new_function(rt: &Runtime, info: *const FunctionInfo, context: u64) -> u64 {
    // SAFETY: function infos outlive the isolate's cells (units and host infos are never
    // freed while the isolate lives).
    let fi = unsafe { &*info };
    // GC points first (the name atom of a host function, the shape); the values live in
    // Rust locals (CSS).
    let name = function_name(rt, fi);
    // One flag test on the common path; the generator and async kinds take their own shape.
    use crate::function_info::fi_flags;
    let s = if fi.flags & (fi_flags::GENERATOR | fi_flags::ASYNC | fi_flags::ASYNC_GENERATOR) == 0 {
        shape::function_shape(rt)
    } else if fi.is_generator() {
        shape::generator_function_shape(rt)
    } else if fi.is_async_function() {
        shape::async_function_shape(rt)
    } else {
        shape::async_generator_function_shape(rt)
    };
    let f = alloc_with_shape(rt, kind::FUNCTION, s, fun::CAPACITY);
    init(f, fun::INFO, native_of(info));
    init(f, fun::CONTEXT, context);
    init(f, fun::REALM, rt.root(Root::Realm));
    let slots = obj::FIXED + 8 * fun::FIXED;
    init(f, slots, crate::conv::num(f64::from(fi.length)));
    init(f, slots + 8, name);
    f.to_value().to_bits()
}

/// The `name` value of a new function for `fi`: the unit's atom for a bytecode function (no
/// allocation), the interned text for a host function. GC point.
fn function_name(rt: &Runtime, fi: &FunctionInfo) -> u64 {
    if let Some(u) = fi.loaded_unit() {
        return match u.unit.functions[fi.function_id as usize].name {
            Some(id) => u.string(id.index()),
            None => rt.atom(Atom::Empty),
        };
    }
    if fi.name.is_empty() {
        rt.atom(Atom::Empty)
    } else {
        crate::atoms::intern_str(rt, &fi.name)
    }
}

/// The FunctionInfo of a function cell.
pub(crate) fn function_info<'a>(f: u64) -> &'a FunctionInfo {
    // SAFETY: a FUNCTION cell's slot +24 holds the native word of a FunctionInfo that lives as
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

/// `[[Realm]]` of a function cell: a REALM cell (OBJECT_LAYOUT 3; M3-P1 REALMS reads it on a
/// call).
#[allow(dead_code)] // first reader: M3-P1 REALMS
pub(crate) fn function_realm(f: u64) -> u64 {
    rd(cell(f), fun::REALM)
}

/// A new ITERATOR cell with prototype `proto` over `iterated`: an object for an array iterator
/// (CreateArrayIterator, §23.1.5.1, kind "value"; the object is read as an array-like), or a
/// string for a string iterator (CreateStringIterator, §22.1.5.1). The other states are listed
/// at [`iterator_state`].
pub(crate) fn new_iterator(rt: &Runtime, proto: u64, iterated: u64) -> u64 {
    new_iterator_of_kind(rt, proto, iterated, iter_kind::VALUES)
}

/// [`new_iterator`] with the iteration kind `k` ([`iter_kind`]). GC point.
pub(crate) fn new_iterator_of_kind(rt: &Runtime, proto: u64, iterated: u64, k: i32) -> u64 {
    let c = new_js_object(rt, kind::ITERATOR, proto, 0);
    init(c, iter::ITERATED, iterated);
    init(c, iter::NEXT, int32_bits(0));
    init(c, iter::KIND, int32_bits(k));
    c.to_value().to_bits()
}

/// The iteration kind of an ITERATOR cell ([`iter_kind`]).
pub(crate) fn iterator_kind(it: u64) -> i32 {
    rd_i32(cell(it), iter::KIND)
}

/// (iterated value, next index) of an ITERATOR cell. Array iterator: an object (suspended),
/// null (executing: `next` runs user code, W4-P3FIX-ITER-EXEC-NULL) or undefined (done).
/// String iterator: a string, or "" (done).
pub(crate) fn iterator_state(it: u64) -> (u64, u32) {
    let c = cell(it);
    (rd(c, iter::ITERATED), rd_i32(c, iter::NEXT) as u32)
}

/// Updates an ITERATOR cell; `iterated` = undefined (array iterator) or "" (string iterator)
/// marks it done, and null marks an array iterator executing.
pub(crate) fn set_iterator_state(rt: &Runtime, it: u64, iterated: u64, next: u32) {
    let c = cell(it);
    wr(&rt.heap, c, iter::ITERATED, iterated);
    wr(&rt.heap, c, iter::NEXT, int32_bits(next as i32));
}

// ---- generators (GENERATOR cells, [`genf`]) ---------------------------------------------------

/// A new GENERATOR object with prototype `proto` for the generator function `function`, in
/// state executing with resume mode next: `receiver` is the body's `this`, `context` its
/// context, and REGS a new ELEMENTS cell of `regs` Values (holes; the input slot undefined).
/// GC point.
pub(crate) fn new_generator(
    rt: &Runtime,
    proto: u64,
    function: u64,
    receiver: u64,
    context: u64,
    mode: i32,
    regs: usize,
) -> u64 {
    // GC points first (the ELEMENTS cell, then the object); the values live in Rust locals
    // (CSS) until the object holds them.
    let r = new_elements(rt, regs.max(1));
    init(r, elems::DATA + 8 * genf::REGS_INPUT, UNDEFINED);
    let r = r.to_value().to_bits();
    let g = new_js_object(rt, kind::GENERATOR, proto, 0);
    init(g, genf::STATE, int32_bits(genf::STATE_EXECUTING));
    init(g, genf::MODE, int32_bits(mode));
    init(g, genf::FUNCTION, function);
    init(g, genf::RECEIVER, receiver);
    init(g, genf::CONTEXT, context);
    init(g, genf::REGS, r);
    init(g, genf::RESUME_MODE, int32_bits(genf::RESUME_NEXT));
    init(g, genf::ASYNC, UNDEFINED);
    g.to_value().to_bits()
}

/// GENERATOR field `off` ([`genf`]) of `g`.
#[inline]
pub(crate) fn gen_get(g: u64, off: usize) -> u64 {
    debug_assert!(cell_kind(g) == kind::GENERATOR.get());
    debug_assert!(
        (genf::STATE..genf::STATE + 8 * genf::FIXED).contains(&off) && off.is_multiple_of(8)
    );
    rd(cell(g), off)
}

/// An int32 GENERATOR field (state, mode, resume mode) of `g`.
#[inline]
pub(crate) fn gen_get_i32(g: u64, off: usize) -> i32 {
    int32_of(gen_get(g, off))
}

/// Stores GENERATOR field `off` of `g` (barriered).
#[inline]
pub(crate) fn gen_set(rt: &Runtime, g: u64, off: usize, v: u64) {
    debug_assert!(cell_kind(g) == kind::GENERATOR.get());
    debug_assert!(
        (genf::STATE..genf::STATE + 8 * genf::FIXED).contains(&off) && off.is_multiple_of(8)
    );
    wr(&rt.heap, cell(g), off, v);
}

/// The saved values of a generator (its REGS cell): `[input, params.., registers..]`
/// ([`genf::REGS_INPUT`]). A view of a live cell: hold it only while the generator is held,
/// and do not keep it across a GC point after [`gen_release`].
#[derive(Clone, Copy)]
pub(crate) struct GenRegs {
    cell: CellPtr,
    len: usize,
}

impl GenRegs {
    /// The number of saved slots.
    #[inline]
    pub fn len(self) -> usize {
        self.len
    }

    /// Slot `i` (`i < len`).
    #[inline]
    pub fn get(self, i: usize) -> u64 {
        assert!(i < self.len, "generator slot {i} out of range");
        rd(self.cell, elems::DATA + 8 * i)
    }

    /// Stores slot `i` (`i < len`, barriered).
    #[inline]
    pub fn set(self, rt: &Runtime, i: usize, v: u64) {
        assert!(i < self.len, "generator slot {i} out of range");
        wr(&rt.heap, self.cell, elems::DATA + 8 * i, v);
    }
}

/// The saved values of `g`, or `None` once they were released ([`gen_release`]).
pub(crate) fn gen_regs(g: u64) -> Option<GenRegs> {
    let r = gen_get(g, genf::REGS);
    if r == UNDEFINED {
        return None;
    }
    debug_assert!(cell_kind(r) == kind::ELEMENTS.get());
    let c = cell(r);
    Some(GenRegs {
        cell: c,
        len: rd_i32(c, elems::CAPACITY) as usize,
    })
}

/// Drops the saved values of the completed generator `g` (REGS = undefined), so it keeps
/// nothing alive that only its frame used.
pub(crate) fn gen_release(rt: &Runtime, g: u64) {
    gen_set(rt, g, genf::REGS, UNDEFINED);
}

/// IsConstructor (§7.2.4): a FUNCTION whose FunctionInfo has `[[Construct]]`, or a bound
/// function whose target is a constructor (`bound::is_constructor`).
pub(crate) fn is_constructor(f: u64) -> bool {
    let k = cell_kind(f);
    if k == kind::FUNCTION.get() {
        function_info(f).is_constructor()
    } else {
        k == kind::BOUND_FUNCTION.get() && crate::bound::is_constructor(f)
    }
}

/// A new context with `len` slots (undefined) for the unit scope `scope`, whose previous
/// context is `previous`. `scope` is an entry of its unit's [`LoadedUnit::context_scopes`],
/// which lives as long as the isolate. GC point.
pub(crate) fn new_context(
    rt: &Runtime,
    previous: u64,
    scope: *const ContextScope,
    len: usize,
) -> u64 {
    let c = rt.alloc(kind::CONTEXT, ctxf::SLOTS + 8 * len);
    init(c, ctxf::PREVIOUS, previous);
    init(c, ctxf::SCOPE, native_of(scope));
    init(c, ctxf::LENGTH, int32_bits(len as i32));
    c.to_value().to_bits()
}

/// The unit and the scope descriptor of the context `c` (C5): a slot name resolves with
/// `unit.string(scope.slots[i].name.index())`.
pub(crate) fn context_scope<'a>(c: u64) -> (&'a LoadedUnit, &'a ep9_bytecode::Scope) {
    // SAFETY: the SCOPE word of a CONTEXT cell is the native word of a `ContextScope` in a
    // boxed LoadedUnit (`new_context`); units are never freed while the isolate lives
    // (`Runtime::units` only grows), and the record's pointers point into that unit.
    let cs = unsafe { &*native_ptr::<ContextScope>(rd(cell(c), ctxf::SCOPE)) };
    // SAFETY: as above; `scope` points into the unit's verified scope table, which never
    // changes after verification.
    unsafe { (&*cs.unit, &*cs.scope) }
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
    let c = new_js_object(rt, kind::ERROR, proto, default_capacity(kind::ERROR));
    c.to_value().to_bits()
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

// ---- SLOT_OBJECT: objects with internal slots (abi/slots.rs is the builtin ABI) ----------

/// A new SLOT_OBJECT with prototype `proto` (an object or null), ObjectClass id `class` and
/// `n` internal slots, all undefined; `n` = 0 makes no slots cell. It gets no in-object
/// slots, so a named property goes out of line. Panics if `n` > [`slot_obj::MAX_SLOTS`].
/// GC point.
pub(crate) fn new_slot_object(rt: &Runtime, proto: u64, class: u16, n: usize) -> u64 {
    assert!(
        n <= slot_obj::MAX_SLOTS,
        "{n} internal slots (at most {})",
        slot_obj::MAX_SLOTS
    );
    // GC points first (the slots cell, then the object); the cell lives in a Rust local (CSS)
    // until the object holds it, as in `new_generator`.
    let slots = if n == 0 {
        UNDEFINED
    } else {
        let c = rt.alloc(kind::ELEMENTS, elems::DATA + 8 * n);
        init(c, elems::CAPACITY, int32_bits(n as i32));
        for i in 0..n {
            init(c, elems::DATA + 8 * i, UNDEFINED);
        }
        c.to_value().to_bits()
    };
    let o = new_js_object(rt, kind::SLOT_OBJECT, proto, 0);
    init(o, slot_obj::CLASS, int32_bits(i32::from(class)));
    init(o, slot_obj::SLOTS, slots);
    o.to_value().to_bits()
}

/// The ObjectClass id of `v`, or `None` when `v` is not a SLOT_OBJECT (any Value).
#[inline]
pub(crate) fn slot_class(v: u64) -> Option<u16> {
    if cell_kind(v) != kind::SLOT_OBJECT.get() {
        return None;
    }
    Some(rd_i32(cell(v), slot_obj::CLASS) as u16)
}

/// The internal slot count of the SLOT_OBJECT `o`.
#[inline]
pub(crate) fn slot_count(o: u64) -> usize {
    debug_assert!(cell_kind(o) == kind::SLOT_OBJECT.get());
    let s = rd(cell(o), slot_obj::SLOTS);
    if s == UNDEFINED {
        return 0;
    }
    debug_assert!(cell_kind(s) == kind::ELEMENTS.get());
    rd_i32(cell(s), elems::CAPACITY) as usize
}

/// Internal slot `i` of the SLOT_OBJECT `o`. Panics if `i` ≥ [`slot_count`].
#[inline]
pub(crate) fn slot_get(o: u64, i: usize) -> u64 {
    let n = slot_count(o);
    assert!(i < n, "internal slot {i} out of range (the object has {n})");
    rd(cell(rd(cell(o), slot_obj::SLOTS)), elems::DATA + 8 * i)
}

/// Stores internal slot `i` of the SLOT_OBJECT `o` (barriered). Panics if `i` ≥
/// [`slot_count`].
#[inline]
pub(crate) fn slot_set(rt: &Runtime, o: u64, i: usize, v: u64) {
    let n = slot_count(o);
    assert!(i < n, "internal slot {i} out of range (the object has {n})");
    wr(
        &rt.heap,
        cell(rd(cell(o), slot_obj::SLOTS)),
        elems::DATA + 8 * i,
        v,
    );
}
