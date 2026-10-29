//! Arguments exotic objects (§10.4.4; M3-P2 VM-ARGS): the ARGUMENTS kind (25,
//! `object::argsf`), `CreateMappedArguments` and `CreateUnmappedArguments`, and the internal
//! methods of an arguments object.
//!
//! ```text
//! CreateMappedArguments   (ops.rs cold body) ──> create(.., mapped = true)
//! CreateUnmappedArguments (ops.rs cold body) ──> create(.., mapped = false)
//! exotic.rs hooks (get, own_property, own_attrs) ──────┐
//! property.rs hooks (set, define, delete, own keys) ───┴──> the hook fns below
//! ```
//!
//! Storage of an arguments object made for `n` arguments, `mc` of them mapped:
//!
//! ```text
//!  +16 ELEMENTS: one ELEMENTS cell (a list of 2n + mc Values), or undefined when n = 0
//!      [0, n)        the argument values; HOLE = no such element (deleted, or redefined as
//!                    an accessor: then the property is in the ordinary storage)
//!      [n, 2n)       the attribute bits of each element (int32): any data attributes
//!      [2n, 2n + mc) the parameter map: the context slot (int32) of mapped index j, or
//!                    HOLE once index j is unmapped
//!  +24 MAP           the function's CONTEXT whose slots hold the mapped parameters
//!                    (undefined when mc = 0)
//!  +32 MAPPED_COUNT  mc (int32): min(n, formal parameter count) for a mapped object, else 0
//!  in-object slots   `length`, then @@iterator, then `callee`: ordinary properties
//! ```
//!
//! An element (index i < n, value not HOLE) is always a data property; these hooks own it.
//! Every other key (`length`, `callee`, @@iterator, an index at or past n, an index that was
//! deleted and added again, an accessor index) lives in the ordinary storage, and the hooks
//! answer `None` for it, so the ordinary method runs. A mapped index is always an element
//! (a mapped property is a writable data property: §10.4.4.2 step 6 unmaps an accessor or a
//! non-writable one), so the ordinary storage never holds a mapped index.
//!
//! While index j is mapped, its value is the context slot (the parameter variable): reads go
//! to the slot, and writes go to the slot and to the element (§10.4.4.3-4). The element value
//! may then be old; it is read only after the index is unmapped, and every unmap step writes
//! it first (§10.4.4.2 step 3) or removes it.

use crate::accessor;
use crate::builtins::{self, BuiltinId, IntrinsicId};
use crate::conv::Res;
use crate::descriptor::Desc;
use crate::error::FrameRef;
use crate::frame::{HOLE, UNDEFINED, int32_bits, int32_of, slot};
use crate::interp::Cx;
use crate::key::{Probe, array_index_of};
use crate::kinds as kind;
use crate::natives;
use crate::object::{self, argsf, attr, elems, init, obj, realmf};
use crate::property;
use crate::props;
use crate::runtime::{Atom, Root, Runtime};
use crate::shape;
use crate::symbol::{self, WellKnownSymbol};

/// `CreateMappedArguments` (`mapped`, §10.4.4.7) and `CreateUnmappedArguments` (§10.4.4.6)
/// for the frame `fp`: an arguments object over the frame's arguments. A mapped object maps
/// each formal parameter that the function's ABI 5 mapped-parameter list names (P0-D16) to
/// its slot in the frame's current context. Returns the new accumulator, or 0 if an
/// exception is pending (the `slow` entry convention; none is thrown). GC point.
///
/// The ARGS fast path (V10-D11, M5-P2 VMIC): the first object of each kind in a realm is
/// made the spec way ([`create_slow`]: the root shape, then 3 property adds); its shape goes
/// into the REALM record (`realmf::MAPPED_ARGUMENTS_SHAPE`, `UNMAPPED_ARGUMENTS_SHAPE`).
/// Every later object of that realm and kind takes that shape at allocation and gets its 3
/// slots written in place ([`create_fast`]): no shape search, no transition lookup, no key
/// hashing.
#[cold]
#[inline(never)]
pub(crate) fn create(cx: &mut Cx, fp: *mut u64, pc: *const u8, mapped: bool) -> u64 {
    cx.rt().enter(fp, pc);
    let f = FrameRef(fp);
    // SAFETY: header slots of a live frame.
    let context = unsafe { *fp.offset(slot::CONTEXT) };
    let map = if mapped {
        parameter_map(f, context, f.argc())
    } else {
        &[]
    };
    let rt = cx.rt();
    let realm = rt.root(Root::Realm);
    match cached_shape(rt, realm, mapped) {
        Some(s) => create_fast(rt, f, s, mapped, map),
        None => {
            let o = create_slow(rt, f, mapped, map);
            remember_shape(rt, realm, mapped, o);
            o
        }
    }
}

/// The REALM record field that caches the shape of the realm's mapped or unmapped arguments
/// objects (OBJECT_LAYOUT 5, V10-D11).
#[inline(always)]
fn shape_field(mapped: bool) -> usize {
    if mapped {
        realmf::MAPPED_ARGUMENTS_SHAPE
    } else {
        realmf::UNMAPPED_ARGUMENTS_SHAPE
    }
}

/// The cached arguments shape of `realm` (the current realm, a REALM record) for `mapped`,
/// if [`remember_shape`] stored one and its prototype is still the current
/// %Object.prototype% (a cheap guard: a realm's %Object.prototype% never changes). Not a GC
/// point.
#[inline(always)]
fn cached_shape(rt: &Runtime, realm: u64, mapped: bool) -> Option<u64> {
    if !kind::is_cell_bits(realm) || kind::cell_kind(realm) != kind::REALM.get() {
        return None;
    }
    let s = object::record_get(realm, shape_field(mapped));
    if s == UNDEFINED || shape::proto(s) != rt.root(Root::ObjectPrototype) {
        return None;
    }
    debug_assert!(
        fits(rt, s, mapped),
        "a cached arguments shape of another form"
    );
    Some(s)
}

/// Stores the shape of `o` (an arguments object [`create_slow`] just made in the current
/// realm `realm`) as the realm's cached shape for `mapped`, if it has the form
/// [`create_fast`] writes ([`fits`]). Not a GC point.
fn remember_shape(rt: &Runtime, realm: u64, mapped: bool, o: u64) {
    if !kind::is_cell_bits(realm) || kind::cell_kind(realm) != kind::REALM.get() {
        return;
    }
    let s = shape::shape_of(o);
    if fits(rt, s, mapped) {
        object::record_set(rt, realm, shape_field(mapped), s);
    }
}

/// True if `s` is a shape [`create_fast`] may give a new arguments object: a fast shape of
/// the transition tree (not a dictionary, not unique, not a prototype's) with exactly the 3
/// properties `length`, @@iterator and `callee` in that order, in fields 0, 1 and 2 of the
/// in-object slots, `callee` an accessor exactly when not `mapped`. (A realm without
/// %ThrowTypeError% gives an unmapped object no `callee`: its shape does not fit, so that
/// realm keeps the slow path.) Not a GC point.
fn fits(rt: &Runtime, s: u64, mapped: bool) -> bool {
    let fl = shape::flags(s);
    if fl & (shape::flag::DICTIONARY | shape::flag::UNIQUE | shape::flag::PROTOTYPE) != 0
        || shape::object_kind(s) != kind::ARGUMENTS.get()
        || shape::count(s) != 3
        || usize::from(shape::capacity(s)) < 3
    {
        return false;
    }
    let keys = [
        rt.atom(Atom::Length),
        symbol::well_known(rt, WellKnownSymbol::Iterator),
        rt.atom(Atom::Callee),
    ];
    keys.iter().enumerate().all(|(i, &k)| {
        let d = shape::details_at(s, i);
        let accessor = i == 2 && !mapped;
        shape::key_at(s, i) == k
            && usize::from(d.field()) == i
            && (d.attrs() & attr::ACCESSOR != 0) == accessor
    })
}

/// The arguments object made the spec way (§10.4.4.6 steps 4-9, §10.4.4.7 steps 15-23): a
/// new object from the root shape, its elements, then the ordinary properties `length`,
/// @@iterator and `callee` added one by one. GC point.
fn create_slow(rt: &Runtime, f: FrameRef, mapped: bool, map: &[Option<u32>]) -> u64 {
    let argc = f.argc();
    // GC points below: `callee` and `context` are on the frame, the new cells live in Rust
    // locals, and the argument values are read from the frame after each allocation.
    let o = object::new_js_object(
        rt,
        kind::ARGUMENTS,
        rt.root(Root::ObjectPrototype),
        argsf::CAPACITY,
    );
    let mc = map.len();
    // SAFETY: header slots of a live frame.
    let (callee, context) = unsafe { (*f.0.offset(slot::CALLEE), *f.0.offset(slot::CONTEXT)) };
    init(o, argsf::MAP, if mc > 0 { context } else { UNDEFINED });
    init(o, argsf::MAPPED_COUNT, int32_bits(mc as i32));
    init(o, obj::ELEMENTS, UNDEFINED);
    let o = o.to_value().to_bits();
    let n = argc as usize;
    if n > 0 {
        let e = new_elements(rt, f, map);
        object::wr(&rt.heap, object::cell(o), obj::ELEMENTS, e);
    }
    // The ordinary properties, in the spec's creation order (§10.4.4.6 steps 4-9, §10.4.4.7
    // steps 15-23): `length`, @@iterator, `callee`.
    let length = rt.atom(Atom::Length);
    props::own_add(rt, o, length, int32_bits(argc as i32), attr::HIDDEN);
    let values = array_values(rt);
    let iterator = symbol::well_known(rt, WellKnownSymbol::Iterator);
    props::own_add(rt, o, iterator, values, attr::HIDDEN);
    let callee_key = rt.atom(Atom::Callee);
    if mapped {
        props::own_add(rt, o, callee_key, callee, attr::HIDDEN);
    } else {
        // {[[Get]]: %ThrowTypeError%, [[Set]]: %ThrowTypeError%, e: false, c: false}. A realm
        // without %ThrowTypeError% (none makes it before VM-ARGS step 2) gets no `callee`:
        // an accessor with undefined halves would let tests of %ThrowTypeError% pass falsely.
        if let Some(t) = throw_type_error(rt) {
            let pair = accessor::new_pair(rt, t, t);
            props::own_add(rt, o, callee_key, pair, attr::ACCESSOR);
        }
    }
    o
}

/// The ARGS fast path: a new arguments object with the realm's cached shape `s` ([`fits`]),
/// the same object [`create_slow`] makes. Every new cell is made first (the @@iterator
/// function on its first use, the `callee` accessor pair, the elements list), then the
/// object, then only init stores (E2: no GC point between the object's allocation and its
/// last store). GC point.
fn create_fast(rt: &Runtime, f: FrameRef, s: u64, mapped: bool, map: &[Option<u32>]) -> u64 {
    #[cfg(test)]
    FAST_PATH_OBJECTS.with(|n| n.set(n.get() + 1));
    let argc = f.argc();
    // GC points below: `s` is held by the REALM record, `values`, `callee_v` and `e` live in
    // Rust locals (CSS), and the argument values are read from the frame after the
    // allocations.
    let values = array_values(rt);
    let callee_v = if mapped {
        UNDEFINED // read from the frame below, after the last GC point
    } else {
        // The cached unmapped shape has a `callee`, so this realm has %ThrowTypeError%.
        let t = throw_type_error(rt).expect("a cached unmapped shape has a callee");
        accessor::new_pair(rt, t, t)
    };
    let e = if argc > 0 {
        new_elements(rt, f, map)
    } else {
        UNDEFINED
    };
    let o = object::alloc_from_shape(rt, kind::ARGUMENTS, s);
    // No GC point from here on.
    // SAFETY: header slots of a live frame.
    let (callee, context) = unsafe { (*f.0.offset(slot::CALLEE), *f.0.offset(slot::CONTEXT)) };
    let mc = map.len();
    init(o, obj::ELEMENTS, e);
    init(o, argsf::MAP, if mc > 0 { context } else { UNDEFINED });
    init(o, argsf::MAPPED_COUNT, int32_bits(mc as i32));
    let fields = [
        int32_bits(argc as i32),
        values,
        if mapped { callee } else { callee_v },
    ];
    let first = obj::FIXED + 8 * argsf::FIXED;
    for (i, v) in fields.into_iter().enumerate() {
        // The field store hook of `props::field_set` (C-02 representation and const bits),
        // so this store counts as the store of an add.
        crate::field_rep::before_field_store(rt, s, i, v);
        init(o, first + 8 * i, v);
    }
    o.to_value().to_bits()
}

#[cfg(test)]
thread_local! {
    /// The objects [`create_fast`] made on this thread (the tests check the path ran).
    pub(crate) static FAST_PATH_OBJECTS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// The ELEMENTS list of a new arguments object for the frame `f` (n = argc > 0 arguments,
/// the parameter map `map`): the n values, n attribute words {w, e, c}, then the context slot
/// of each mapped index (HOLE for one a later parameter of the same name shadows). GC point
/// (the list).
fn new_elements(rt: &Runtime, f: FrameRef, map: &[Option<u32>]) -> u64 {
    let n = f.argc() as usize;
    let e = object::new_list(rt, 2 * n + map.len(), int32_bits(attr::DEFAULT));
    // No GC point from here on: the stores are init stores into the new list.
    let c = object::cell(e);
    for i in 0..n {
        init(c, elems::DATA + 8 * i, f.arg(i as u32));
    }
    for (j, s) in map.iter().enumerate() {
        let v = s.map_or(HOLE, |s| int32_bits(s as i32));
        init(c, elems::DATA + 8 * (2 * n + j), v);
    }
    e
}

/// %Array.prototype.values% for the @@iterator property (§10.4.4.6 step 7, §10.4.4.7 step
/// 20), made on its first use (a GC point then).
#[inline]
fn array_values(rt: &Runtime) -> u64 {
    builtins::function_object(
        rt,
        BuiltinId::new(natives::VM_BLOCK, natives::vm::ARRAY_VALUES),
    )
}

/// The current realm's %ThrowTypeError% (§10.2.4.1), or `None` before a table makes it. On
/// the first use in a realm it gets the attributes the spec gives it: `length` and `name`
/// {w: false, e: false, c: false} (a table gives a builtin function {c}) and
/// `[[Extensible]]` false. The strict `callee` accessor is the only way JS code reaches it
/// (no month-3 role builds the Function.prototype `caller` and `arguments` accessors), so
/// no code sees it before this runs. GC point (the dictionary move of the redefinitions).
fn throw_type_error(rt: &Runtime) -> Option<u64> {
    let t = builtins::intrinsic(rt, IntrinsicId::ThrowTypeError);
    if !kind::is_callable(t) {
        return None;
    }
    if property::is_extensible(t) {
        for a in [Atom::Length, Atom::Name] {
            let key = rt.atom(a);
            if let Some((v, _)) = props::own_get(t, key) {
                props::own_define(rt, t, key, v, 0);
            }
        }
        property::prevent_extensions(rt, t);
    }
    Some(t)
}

/// The parameter map of a mapped arguments object for `argc` arguments: for index
/// `j < min(argc, formal count)`, the context slot of formal parameter `j`, or `None` when a
/// later parameter of the same name binds that name (only the last one is mapped). Empty when
/// the function has no list (no formal parameter) or the current context is not the list's
/// scope (the verifier rules out the second case). A slice of the unit's list: no copy.
fn parameter_map<'a>(f: FrameRef, context: u64, argc: u32) -> &'a [Option<u32>] {
    let fi = f.info();
    let Some(u) = fi.loaded_unit() else {
        return &[];
    };
    let Some(m) = &u.unit.functions[fi.function_id as usize].mapped_parameters else {
        return &[];
    };
    let ok = kind::is_context(context) && {
        let (cu, scope) = object::context_scope(context);
        core::ptr::eq(cu, u) && core::ptr::eq(scope, &u.unit.scopes[m.scope.index()])
    };
    debug_assert!(ok, "CreateMappedArguments outside its function context");
    if !ok {
        return &[];
    }
    let n = (argc as usize).min(m.slots.len());
    &m.slots[..n]
}

/// A new unmapped ARGUMENTS object with prototype `proto`: no parameter map, mapped count 0,
/// no elements, its 3 in-object slots undefined. For the layout tests. GC point.
#[cfg(test)]
pub(crate) fn new_arguments_object(rt: &Runtime, proto: u64) -> u64 {
    let c = object::new_js_object(rt, kind::ARGUMENTS, proto, argsf::CAPACITY);
    init(c, argsf::MAP, UNDEFINED);
    init(c, argsf::MAPPED_COUNT, int32_bits(0));
    c.to_value().to_bits()
}

// ---- the element store ----------------------------------------------------------------------

/// The element store of the arguments object `o`: (the ELEMENTS list, argument count n,
/// mapped count mc). `None` when `o` has no elements. Not a GC point.
#[inline]
fn store(o: u64) -> Option<(u64, usize, usize)> {
    let e = object::rd(object::cell(o), obj::ELEMENTS);
    if e == UNDEFINED {
        return None;
    }
    let mc = int32_of(object::fixed_get(o, 1)) as usize;
    let n = (object::list_len(e) - mc) / 2;
    Some((e, n, mc))
}

/// Element `i` of `o`: (value, attribute bits), or `None` if `i` is not an element. The value
/// is the stored one (old while `i` is mapped: see [`value`]). Not a GC point.
fn element(o: u64, i: u32) -> Option<(u64, i32)> {
    let (e, n, _) = store(o)?;
    let i = i as usize;
    if i >= n {
        return None;
    }
    let v = object::list_get(e, i);
    (v != HOLE).then(|| (v, int32_of(object::list_get(e, n + i))))
}

/// The context slot that index `i` of `o` is mapped to, if it is mapped. Not a GC point.
fn mapped_slot(o: u64, i: u32) -> Option<u32> {
    let (e, n, mc) = store(o)?;
    let i = i as usize;
    if i >= mc {
        return None;
    }
    let s = object::list_get(e, 2 * n + i);
    (s != HOLE).then(|| int32_of(s) as u32)
}

/// The current value of element `i` (value `v`): the parameter's context slot while `i` is
/// mapped, else `v`. Not a GC point.
fn value(o: u64, i: u32, v: u64) -> u64 {
    match mapped_slot(o, i) {
        Some(s) => object::context_get(object::fixed_get(o, 0), s).expect("a mapped slot"),
        None => v,
    }
}

/// Writes the parameter that index `i` is mapped to (`map.[[Set]]`). Not a GC point.
fn set_mapped(rt: &Runtime, o: u64, i: u32, v: u64) {
    if let Some(s) = mapped_slot(o, i) {
        let ok = object::context_set(rt, object::fixed_get(o, 0), s, v);
        debug_assert!(ok, "a mapped slot");
    }
}

/// Removes index `i` from the parameter map (`map.[[Delete]]`). Not a GC point.
fn unmap(rt: &Runtime, o: u64, i: u32) {
    if mapped_slot(o, i).is_some() {
        let (e, n, _) = store(o).expect("a mapped index has a store");
        object::list_set(rt, e, 2 * n + i as usize, HOLE);
    }
}

/// Stores element `i` (value and attribute bits); HOLE as the value removes it. Not a GC
/// point.
fn set_element(rt: &Runtime, o: u64, i: u32, v: u64, attrs: i32) {
    let (e, n, _) = store(o).expect("an element");
    object::list_set(rt, e, i as usize, v);
    object::list_set(rt, e, n + i as usize, int32_bits(attrs));
}

// ---- integer-index access without a key (M4-P3 KEYED; `property.rs`, `abi/mod.rs`) ----------

/// The current value of element `i` of the arguments object `o` (a mapped index reads its
/// parameter, §10.4.4.3), or `None` if `i` is not an element: then the ordinary storage or
/// the chain decides, with the key. The [`get`] hook for an index key, without the key. Not
/// a GC point.
#[inline]
pub(crate) fn element_value(o: u64, i: u32) -> Option<u64> {
    debug_assert!(kind::cell_kind(o) == kind::ARGUMENTS.get());
    let (v, _) = element(o, i)?;
    Some(value(o, i, v))
}

/// `length` of the arguments object `o` when `o` still has the shape it was made with in the
/// current realm (one of the realm's cached shapes, [`fits`]: `length` is the data property in
/// field 0, no own property was added, removed or redefined since) and holds an int32 at or
/// above 0, which is then `ToLength(Get(o, "length"))` (§7.3.19 step 2) with no observable
/// step. `None`: take the generic path. The CreateListFromArrayLike fast path
/// (`abi::Cx::create_list_from_array_like`, V10-D11). Not a GC point.
#[inline]
pub(crate) fn unmodified_length(rt: &Runtime, o: u64) -> Option<u32> {
    debug_assert!(kind::cell_kind(o) == kind::ARGUMENTS.get());
    let realm = rt.root(Root::Realm);
    if !kind::is_cell_bits(realm) || kind::cell_kind(realm) != kind::REALM.get() {
        return None;
    }
    let s = shape::shape_of(o);
    if s != object::record_get(realm, realmf::MAPPED_ARGUMENTS_SHAPE)
        && s != object::record_get(realm, realmf::UNMAPPED_ARGUMENTS_SHAPE)
    {
        return None;
    }
    let len = object::rd(object::cell(o), obj::FIXED + 8 * argsf::FIXED);
    let len = crate::runtime::val(len).as_int32()?;
    u32::try_from(len).ok()
}

/// `ToLength(Get(o, "length"))` of CreateListFromArrayLike (§7.3.19 step 2) read in place
/// when `o` (an object) is an arguments object with an [`unmodified_length`]; `None`: the
/// caller takes Get and ToLength. The `length` fast path of
/// `abi::Cx::create_list_from_array_like` (V10-D11). Not a GC point.
#[inline]
pub(crate) fn list_length(rt: &Runtime, o: u64) -> Option<u64> {
    if kind::cell_kind(o) != kind::ARGUMENTS.get() {
        return None;
    }
    unmodified_length(rt, o).map(u64::from)
}

/// `Get(o, ! ToString(𝔽(i)))` of CreateListFromArrayLike (§7.3.19 step 6.b) read in place
/// when `i` is a present element of the array or arguments object `o` (an object; a mapped
/// index reads its parameter, [`element_value`]); `None`: the caller reads it through
/// `property::get_int` (a hole, an index past the elements, an accessor, the chain, any other
/// kind). Not a GC point.
#[inline]
pub(crate) fn list_element(o: u64, i: u32) -> Option<u64> {
    if kind::is_array(o) {
        object::array_get(o, i)
    } else if kind::cell_kind(o) == kind::ARGUMENTS.get() {
        element_value(o, i)
    } else {
        None
    }
}

/// `o.[[Set]](i, v, o)` (§10.4.4.4, receiver `o`) for a writable element `i` of the arguments
/// object `o`, without the key: writes the parameter while `i` is mapped and the element, as
/// the [`set`] hook does. False, with nothing stored, when `i` is not an element or not
/// writable: the caller takes the key path, where the hook decides. Not a GC point.
#[inline]
pub(crate) fn set_element_value(rt: &Runtime, o: u64, i: u32, v: u64) -> bool {
    debug_assert!(kind::cell_kind(o) == kind::ARGUMENTS.get());
    match element(o, i) {
        Some((_, a)) if a & attr::WRITABLE != 0 => {
            set_mapped(rt, o, i, v);
            set_element(rt, o, i, v, a);
            true
        }
        _ => false,
    }
}

// ---- exotic hooks (`exotic.rs`): `None` = no exotic own property, take the ordinary path ----

/// `[[Get]]` of an element (§10.4.4.3: a mapped index reads its parameter). `None` for every
/// other key. Never throws; not a GC point.
pub(crate) fn get(_cx: &mut Cx, o: u64, key: u64, _how: Probe) -> Option<Res<u64>> {
    let i = array_index_of(key)?;
    let (v, _) = element(o, i)?;
    Some(Ok(value(o, i, v)))
}

/// `[[GetOwnProperty]]` of an element (§10.4.4.1): (value, attribute bits); a mapped index
/// gives its parameter's value. `None` for every other key. Never throws; not a GC point.
pub(crate) fn own_property(_cx: &mut Cx, o: u64, key: u64, _how: Probe) -> Option<Res<(u64, i32)>> {
    let i = array_index_of(key)?;
    let (v, a) = element(o, i)?;
    Some(Ok((value(o, i, v), a)))
}

/// The attributes of an element, without making anything. `None` for every other key. Not a
/// GC point.
pub(crate) fn own_attrs(_rt: &Runtime, o: u64, key: u64, _how: Probe) -> Option<i32> {
    element(o, array_index_of(key)?).map(|(_, a)| a)
}

// ---- internal-method hooks (`property.rs`): `None` = the ordinary method ----

/// `[[Set]]` (§10.4.4.4) of an index key: a mapped index of the receiver writes its
/// parameter, then OrdinarySet. A writable element of the receiver takes the value in place
/// (the result of OrdinarySet there). `None` for every other key. GC point (OrdinarySet).
pub(crate) fn set(cx: &mut Cx, o: u64, key: u64, v: u64, receiver: u64) -> Option<Res<bool>> {
    let i = array_index_of(key)?;
    if receiver == o
        && let Some((_, a)) = element(o, i)
    {
        if a & attr::WRITABLE == 0 {
            // OrdinarySetWithOwnDescriptor step 2.a (a mapped index is writable).
            return Some(Ok(false));
        }
        let rt = cx.rt();
        set_mapped(rt, o, i, v);
        set_element(rt, o, i, v, a);
        return Some(Ok(true));
    }
    // Not the receiver's element: isMapped is false (step 1), so OrdinarySet decides.
    Some(property::ordinary_set_with_receiver(
        cx, o, key, v, receiver,
    ))
}

/// `[[DefineOwnProperty]]` (§10.4.4.2) of an element: OrdinaryDefineOwnProperty on the
/// element (ValidateAndApplyPropertyDescriptor), then the parameter map steps. `None` for
/// every other key (a new index, or one in the ordinary storage: the ordinary method; such an
/// index is never mapped). GC point (an accessor pair).
pub(crate) fn define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Option<Res<bool>> {
    let i = array_index_of(key)?;
    let (cur_v, cur_a) = element(o, i)?;
    let is_mapped = mapped_slot(o, i).is_some();
    // Step 3: a mapped index made non-writable without a value keeps the parameter's value.
    let mut new_desc = desc;
    if is_mapped && desc.is_data() && desc.value.is_none() && desc.writable == Some(false) {
        new_desc.value = Some(value(o, i, cur_v));
    }
    // Step 4: OrdinaryDefineOwnProperty against the element {cur_v, cur_a} (a data property).
    let current = Desc::data(cur_v, cur_a);
    if !validate(&new_desc, &current) {
        return Some(Ok(false));
    }
    let rt = cx.rt();
    if new_desc.is_accessor() {
        // §10.1.6.3 step 6.a: data to accessor; absent halves are undefined, enumerable and
        // configurable stay. The property moves to the ordinary storage.
        let get = new_desc.get.unwrap_or(UNDEFINED);
        let set = new_desc.set.unwrap_or(UNDEFINED);
        let ec = new_desc.merge_attrs(cur_a) & (attr::ENUMERABLE | attr::CONFIGURABLE);
        set_element(rt, o, i, HOLE, 0);
        unmap(rt, o, i);
        // GC points: `o`, `key`, `get` and `set` live in Rust locals.
        let pair = accessor::new_pair(rt, get, set);
        let key = crate::key::intern_key(rt, key);
        props::own_define(rt, o, key, pair, attr::ACCESSOR | ec);
        return Some(Ok(true));
    }
    // Step 6.c: data to data (a generic descriptor keeps the value and the kind).
    let v = new_desc.value.unwrap_or(cur_v);
    set_element(rt, o, i, v, new_desc.merge_attrs(cur_a));
    // Step 6 of §10.4.4.2: the parameter map.
    if is_mapped {
        if let Some(v) = desc.value {
            set_mapped(rt, o, i, v);
        }
        if desc.writable == Some(false) {
            unmap(rt, o, i);
        }
    }
    Some(Ok(true))
}

/// ValidateAndApplyPropertyDescriptor (§10.1.6.3) steps 4-5 for an existing data property
/// `current` (complete): may `desc` change it? (The rule of `property::validate`, for a data
/// property.)
fn validate(desc: &Desc, current: &Desc) -> bool {
    if current.configurable == Some(true) {
        return true;
    }
    // 5.a-b.
    if desc.configurable == Some(true) {
        return false;
    }
    if desc
        .enumerable
        .is_some_and(|e| Some(e) != current.enumerable)
    {
        return false;
    }
    // 5.c: a non-configurable data property cannot become an accessor.
    if desc.is_accessor() {
        return false;
    }
    // 5.e.
    if current.writable == Some(false) {
        if desc.writable == Some(true) {
            return false;
        }
        if let Some(v) = desc.value
            && !property::same_value(v, current.value.unwrap_or(UNDEFINED))
        {
            return false;
        }
    }
    true
}

/// `[[Delete]]` (§10.4.4.5) of an element: false for a non-configurable one; else it goes
/// and its index is unmapped. `None` for every other key (OrdinaryDelete: such an index is
/// never mapped). Not a GC point.
pub(crate) fn delete(cx: &mut Cx, o: u64, key: u64) -> Option<Res<bool>> {
    let i = array_index_of(key)?;
    let (_, a) = element(o, i)?;
    if a & attr::CONFIGURABLE == 0 {
        return Some(Ok(false));
    }
    let rt = cx.rt();
    set_element(rt, o, i, HOLE, 0);
    unmap(rt, o, i);
    Some(Ok(true))
}

/// `[[OwnPropertyKeys]]` (OrdinaryOwnPropertyKeys, §10.1.11.1) as a new array: the element
/// indices and the stored index keys ascending, then the stored strings and symbols in
/// creation order. `None` (the ordinary method) when `o` has no element. GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> Option<u64> {
    let (e, n, _) = store(o)?;
    let mut indices: Vec<u32> = (0..n as u32)
        .filter(|&i| object::list_get(e, i as usize) != HOLE)
        .collect();
    if indices.is_empty() {
        return None;
    }
    // GC: `ordinary` and `keys` are Rust locals; the keys in them stay reachable from them.
    let ordinary = property::ordinary_own_property_keys(cx, o);
    let len = object::array_len(ordinary);
    let get = |j: u32| object::array_get(ordinary, j).expect("a key");
    // The stored index keys lead the ordinary list (ascending); an element index is never
    // stored too.
    let stored = (0..len)
        .take_while(|&j| array_index_of(get(j)).is_some())
        .count() as u32;
    indices.extend((0..stored).filter_map(|j| array_index_of(get(j))));
    indices.sort_unstable();
    let rt = cx.rt();
    let keys = object::new_array(rt, indices.len() + (len - stored) as usize);
    let mut at = 0;
    for i in indices {
        let k = crate::key::index_key(rt, i);
        object::array_set(rt, keys, at, k);
        at += 1;
    }
    for j in stored..len {
        object::array_set(rt, keys, at, get(j));
        at += 1;
    }
    std::hint::black_box(ordinary);
    Some(keys)
}
