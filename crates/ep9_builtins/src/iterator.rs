//! The Iterator constructor and the iterator helpers (§27.1.3; part 1 M7-P1 SPEC-ITER, part 2
//! M7-P2 SPEC-ITER2): module `iterator`, block 28. Makes `%Iterator%`,
//! `%IteratorHelperPrototype%` and `%WrapForValidIteratorPrototype%` (`IntrinsicId` 95-97;
//! `ObjectClass::IteratorHelper` 23).
//!
//! ```text
//! Iterator ( )                      abstract: new Iterator() and Iterator() throw
//! Iterator.from ( O )               GetIteratorFlattenable; a wrapper unless O inherits from
//!                                   %Iterator.prototype%
//! Iterator.concat ( ...items )      a helper over the items in turn (iterator sequencing)
//! Iterator.zip ( iterables [ , options ] )       a helper: one array per step (joint
//! Iterator.zipKeyed ( iterables [ , options ] )  iteration); zipKeyed: one null-prototype
//!                                                object per step
//! Iterator.prototype                %Iterator.prototype% (realm init; @@iterator and
//!                                   @@toStringTag from the VM table)
//!   constructor                     accessor (SetterThatIgnoresPrototypeProperties)
//!   map filter take drop flatMap    a helper object (lazy: one step per next())
//!   chunks windows                  a helper object (iterator chunking)
//!   reduce toArray forEach some every find   eager: walk the iterator now
//!   includes join                   eager (iterator includes, iterator join)
//!   [Symbol.dispose]                calls `return` (explicit resource management)
//! %IteratorHelperPrototype%         next, return, @@toStringTag "Iterator Helper"
//! %WrapForValidIteratorPrototype%   next, return
//! ```
//!
//! A helper object is a SLOT_OBJECT of `ObjectClass::IteratorHelper`. The spec writes each
//! lazy helper as a generator closure (CreateIteratorFromClosure); here `next` runs one step of
//! a state machine, and the state slot keeps the GeneratorState (suspended-start,
//! suspended-yield, executing, completed). The wrapper of `Iterator.from` is an object of the
//! same class with the kind `kind::WRAP`: it has `[[Iterated]]` and no
//! `[[UnderlyingIterator]]`, so each `next` and `return` checks the kind. All state is in
//! slots (a heap cell), never in Rust memory (plan rule 8): a list (the iterators of zip, the
//! items of concat, the buffer of windows) is a JS array that only the helper sees.
//!
//! `[[UnderlyingIterators]]` (the joint iteration form of `%IteratorHelperPrototype%.return`):
//! the one underlying iterator of map, filter, take, drop, flatMap, chunks and windows; none
//! for concat (its inner iterator is closure state); the open iterators of zip and zipKeyed.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-iterator.txt`.

use ep9_vm::abi::{JsObject, ObjectClass};

use crate::string::{push_units, units};
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, JsSymbol, Local, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 28;

mod def {
    pub const ITERATOR: u16 = 0;
    pub const FROM: u16 = 1;
    pub const CONSTRUCTOR_GET: u16 = 2;
    pub const CONSTRUCTOR_SET: u16 = 3;
    pub const MAP: u16 = 4;
    pub const FILTER: u16 = 5;
    pub const TAKE: u16 = 6;
    pub const DROP: u16 = 7;
    pub const FLAT_MAP: u16 = 8;
    pub const REDUCE: u16 = 9;
    pub const TO_ARRAY: u16 = 10;
    pub const FOR_EACH: u16 = 11;
    pub const SOME: u16 = 12;
    pub const EVERY: u16 = 13;
    pub const FIND: u16 = 14;
    pub const HELPER_NEXT: u16 = 15;
    pub const HELPER_RETURN: u16 = 16;
    pub const WRAP_NEXT: u16 = 17;
    pub const WRAP_RETURN: u16 = 18;
    pub const CONCAT: u16 = 19;
    pub const ZIP: u16 = 20;
    pub const ZIP_KEYED: u16 = 21;
    pub const CHUNKS: u16 = 22;
    pub const WINDOWS: u16 = 23;
    pub const INCLUDES: u16 = 24;
    pub const JOIN: u16 = 25;
    pub const DISPOSE: u16 = 26;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Iterator", 0, iterator_ctor), // 0
    BuiltinDef::method("from", 1, from),                   // 1
    BuiltinDef::getter("constructor", constructor_get),    // 2
    BuiltinDef::setter("constructor", constructor_set),    // 3
    BuiltinDef::method("map", 1, map),                     // 4
    BuiltinDef::method("filter", 1, filter),               // 5
    BuiltinDef::method("take", 1, take),                   // 6
    BuiltinDef::method("drop", 1, drop),                   // 7
    BuiltinDef::method("flatMap", 1, flat_map),            // 8
    BuiltinDef::method("reduce", 1, reduce),               // 9
    BuiltinDef::method("toArray", 0, to_array),            // 10
    BuiltinDef::method("forEach", 1, for_each),            // 11
    BuiltinDef::method("some", 1, some),                   // 12
    BuiltinDef::method("every", 1, every),                 // 13
    BuiltinDef::method("find", 1, find),                   // 14
    BuiltinDef::method("next", 0, helper_next),            // 15
    BuiltinDef::method("return", 0, helper_return),        // 16
    BuiltinDef::method("next", 0, wrap_next),              // 17
    BuiltinDef::method("return", 0, wrap_return),          // 18
    BuiltinDef::method("concat", 0, concat),               // 19
    BuiltinDef::method("zip", 1, zip),                     // 20
    BuiltinDef::method("zipKeyed", 1, zip_keyed),          // 21
    BuiltinDef::method("chunks", 1, chunks),               // 22
    BuiltinDef::method("windows", 1, windows),             // 23
    BuiltinDef::method("includes", 1, includes),           // 24
    BuiltinDef::method("join", 1, join),                   // 25
    BuiltinDef::method("[Symbol.dispose]", 0, dispose),    // 26
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::Iterator,
        value: IntrinsicValue::Builtin(def::ITERATOR),
    },
    IntrinsicDef {
        id: IntrinsicId::IteratorHelperPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::WrapForValidIteratorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
];

const fn method(name: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), d)
}

/// The properties this module adds, per object (keys in the order of node v26's own keys;
/// the methods node v26 does not have come after them).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Iterator,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IteratorPrototype,
                PropAttrs::NONE,
            ),
            method("from", def::FROM),
            method("concat", def::CONCAT),
            method("zip", def::ZIP),
            method("zipKeyed", def::ZIP_KEYED),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IteratorPrototype,
        props: &[
            // §27.1.4.1 (ES2025): an accessor {e: false, c: true}.
            PropDef::accessor(
                PropKey::Atom(Atom::Constructor),
                Some(def::CONSTRUCTOR_GET),
                Some(def::CONSTRUCTOR_SET),
            ),
            method("reduce", def::REDUCE),
            method("toArray", def::TO_ARRAY),
            method("forEach", def::FOR_EACH),
            method("some", def::SOME),
            method("every", def::EVERY),
            method("find", def::FIND),
            method("map", def::MAP),
            method("filter", def::FILTER),
            method("take", def::TAKE),
            method("drop", def::DROP),
            method("flatMap", def::FLAT_MAP),
            method("chunks", def::CHUNKS),
            method("windows", def::WINDOWS),
            method("includes", def::INCLUDES),
            method("join", def::JOIN),
            // After @@iterator and @@toStringTag (the VM table makes them first), as node.
            PropDef::method(PropKey::Symbol(WellKnownSymbol::DISPOSE), def::DISPOSE),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IteratorHelperPrototype,
        props: &[
            method("next", def::HELPER_NEXT),
            method("return", def::HELPER_RETURN),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Iterator Helper"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::WrapForValidIteratorPrototype,
        props: &[
            method("next", def::WRAP_NEXT),
            method("return", def::WRAP_RETURN),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[PropDef::intrinsic(
            PropKey::Str("Iterator"),
            IntrinsicId::Iterator,
            PropAttrs::METHOD,
        )],
    },
];

/// The module of block [`BLOCK`].
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "iterator",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the helper object ------------------------------------------------------------------------

const CLASS: ObjectClass = ObjectClass::IteratorHelper;

/// The slots of a helper object (and of a wrapper: slots 0-2 only). A kind gives slots 3-7
/// its own names.
mod slot {
    /// The kind ([`super::kind`]) in bits 0-3, the GeneratorState ([`super::state`]) in bits
    /// 4-5 and [`super::state::LAST`] in bit 6, an int32.
    pub const STATE: usize = 0;
    /// `[[UnderlyingIterator]].[[Iterator]]` (a wrapper: `[[Iterated]].[[Iterator]]`; concat:
    /// the inner iterator while it is alive, else undefined; zip: undefined).
    pub const ITERATOR: usize = 1;
    /// `[[UnderlyingIterator]].[[NextMethod]]` (a wrapper: `[[Iterated]].[[NextMethod]]`;
    /// concat: the `next` method of the inner iterator).
    pub const NEXT: usize = 2;
    /// The mapper or the predicate (map, filter, flatMap).
    pub const FUNC: usize = 3;
    /// The counter (map, filter, flatMap) or the remaining count (take, drop; may be +∞), a
    /// Number. chunks, windows: the size.
    pub const COUNT: usize = 4;
    /// flatMap: the inner iterator while it is alive, else undefined.
    pub const INNER: usize = 5;
    /// flatMap: the `next` method of the inner iterator.
    pub const INNER_NEXT: usize = 6;
    /// concat: the list of Records { `[[OpenMethod]]`, `[[Iterable]]` }, an array of pairs
    /// (method at 2i, iterable at 2i + 1).
    pub const ITEMS: usize = 3;
    /// concat: the index of the next Record, a Number (in COUNT).
    pub const ITEM_INDEX: usize = 4;
    /// concat: the number of Records, a Number.
    pub const ITEM_COUNT: usize = 5;
    /// zip: `iters`, an array of pairs (iterator at 2i, `next` at 2i + 1). `openIters` is the
    /// same list: an iterator that leaves it becomes null at 2i (in longest mode that is the
    /// "`iters[i]` is null" of the spec too).
    pub const ITERS: usize = 3;
    /// zip: the mode ([`super::mode`]), an int32.
    pub const MODE: usize = 4;
    /// zip: `padding`, an array of iterCount values (longest mode), else undefined.
    pub const PADDING: usize = 5;
    /// zipKeyed: `keys`, an array of iterCount property keys, else undefined.
    pub const KEYS: usize = 6;
    /// zip: iterCount, a Number.
    pub const ITER_COUNT: usize = 7;
    /// windows: `buffer`, an array used as a ring of up to windowSize values.
    pub const BUFFER: usize = 3;
    /// windows: the index of the first value of the ring, a Number.
    pub const START: usize = 5;
    /// windows: the number of values in the ring, a Number.
    pub const LEN: usize = 6;
    /// windows: true for undersized "allow-partial".
    pub const PARTIAL: usize = 7;
    /// The slot count of a wrapper.
    pub const WRAP_COUNT: usize = 3;
    /// The slot count of map, filter, take, drop and chunks.
    pub const HELPER_COUNT: usize = 5;
    /// The slot count of concat.
    pub const CONCAT_COUNT: usize = 6;
    /// The slot count of flatMap.
    pub const FLAT_MAP_COUNT: usize = 7;
    /// The slot count of zip, zipKeyed and windows.
    pub const ZIP_COUNT: usize = 8;
}

/// The kinds of helper object.
mod kind {
    pub const MAP: i32 = 0;
    pub const FILTER: i32 = 1;
    pub const TAKE: i32 = 2;
    pub const DROP: i32 = 3;
    pub const FLAT_MAP: i32 = 4;
    /// The wrapper of `Iterator.from` (`[[Iterated]]`).
    pub const WRAP: i32 = 5;
    pub const CONCAT: i32 = 6;
    pub const ZIP: i32 = 7;
    pub const ZIP_KEYED: i32 = 8;
    pub const CHUNKS: i32 = 9;
    pub const WINDOWS: i32 = 10;
    pub const MASK: i32 = 0xf;
}

/// The GeneratorState of a helper (bits 4-5 of the state slot).
mod state {
    pub const START: i32 = 0 << 4;
    pub const YIELD: i32 = 1 << 4;
    pub const RUNNING: i32 = 2 << 4;
    pub const DONE: i32 = 3 << 4;
    pub const MASK: i32 = 3 << 4;
    /// With YIELD: the closure stopped at its last Yield (chunks and windows after the
    /// underlying iterator is done). Its next resume returns, whatever the completion, and
    /// closes nothing (the spec ignores the completion of that Yield).
    pub const LAST: i32 = 1 << 6;
}

/// The modes of zip (the `mode` option).
mod mode {
    pub const SHORTEST: i32 = 0;
    pub const LONGEST: i32 = 1;
    pub const STRICT: i32 = 2;
}

/// What one run of a helper closure ends with.
#[derive(Clone, Copy)]
enum Resumed<'s> {
    /// A Yield of the value.
    Yield(Local<'s>),
    /// The last Yield of the closure ([`state::LAST`]).
    Last(Local<'s>),
    /// The closure returns (ReturnCompletion(undefined)).
    Return,
}

impl<'s> From<Option<Local<'s>>> for Resumed<'s> {
    fn from(v: Option<Local<'s>>) -> Self {
        match v {
            Some(v) => Resumed::Yield(v),
            None => Resumed::Return,
        }
    }
}

/// CreateIteratorResultObject ( value, done ) (§7.4.14).
fn iter_result<'s>(cx: &mut Cx<'s>, value: Local<'s>, done: bool) -> JsResult<Local<'s>> {
    let o = cx.new_object();
    cx.create_data_property_or_throw(o, "value", value)?;
    cx.create_data_property_or_throw(o, "done", Local::boolean(done))?;
    Ok(o.value())
}

/// The well-known symbol `Symbol.<name>`, read from `%Symbol%` (its well-known symbol
/// properties are non-writable and non-configurable, §20.4.2).
fn well_known<'s>(cx: &mut Cx<'s>, name: &str) -> JsResult<Local<'s, JsSymbol>> {
    if !cx.has_intrinsic(IntrinsicId::Symbol) {
        return Err(cx.not_implemented("well-known symbols without the Symbol module"));
    }
    let symbol = cx.intrinsic(IntrinsicId::Symbol);
    let v = cx.get(symbol, name)?;
    match v.as_symbol() {
        Some(s) => Ok(s),
        None => Err(cx.throw_type_error("Symbol.<name> is not a symbol")),
    }
}

/// The this value as an object, else the TypeError of "If O is not an Object".
fn this_object<'s>(cx: &mut Cx<'s>, this: Local<'s>, name: &str) -> JsResult<Local<'s, JsObject>> {
    match this.as_object() {
        Some(o) => Ok(o),
        None => {
            let shown = cx.display(this);
            let msg = format!("Iterator.prototype.{name} called on non-object {shown}");
            Err(cx.throw_type_error(&msg))
        }
    }
}

/// GetIteratorDirect ( obj ) (§7.4.2): the `next` method of `obj` (any value; a value that
/// is not callable throws at the first step).
fn get_iterator_direct<'s>(cx: &mut Cx<'s>, obj: Local<'s, JsObject>) -> JsResult<Local<'s>> {
    // 1. Let nextMethod be ? Get(obj, "next").
    // 2-3. The record { obj, nextMethod, false }.
    cx.get(obj, "next")
}

/// IteratorNext and IteratorComplete (§7.4.6, §7.4.7): the result object of one `next` call,
/// or `None` if it is done. Takes one interrupt budget unit (a host `next` never polls).
fn step<'s>(
    cx: &mut Cx<'s>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s, JsObject>>> {
    cx.poll_interrupt()?;
    // IteratorNext: 1. Let result be ? Call(nextMethod, iterator).
    let result = cx.call(next, iterator, &[])?;
    // 3. If result is not an Object, throw a TypeError exception.
    let Some(result) = result.as_object() else {
        let shown = cx.display(result);
        let msg = format!("Iterator result {shown} is not an object");
        return Err(cx.throw_type_error(&msg));
    };
    // IteratorComplete: Return ToBoolean(? Get(iterResult, "done")).
    let done = cx.get(result, "done")?;
    if cx.to_boolean(done) {
        return Ok(None);
    }
    Ok(Some(result))
}

/// IteratorStepValue ( iteratorRecord ) (§7.4.10): the next value, or `None` once done. A
/// throw means the iterator is done: the caller must not close it.
fn step_value<'s>(
    cx: &mut Cx<'s>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s>>> {
    match step(cx, iterator, next)? {
        // IteratorValue: Return ? Get(iterResult, "value").
        Some(result) => cx.get(result, "value").map(Some),
        None => Ok(None),
    }
}

/// GetMethod(iterator, "return") and its call (IteratorClose steps 3-4): `None` if the
/// method is undefined or null.
fn call_return<'s>(cx: &mut Cx<'s>, iterator: Local<'s, JsObject>) -> JsResult<Option<Local<'s>>> {
    let method = cx.get(iterator, "return")?;
    if method.is_undefined_or_null() {
        return Ok(None);
    }
    if !cx.is_callable(method) {
        let shown = cx.display(method);
        let msg = format!("{shown} is not a function");
        return Err(cx.throw_type_error(&msg));
    }
    cx.call(method, iterator, &[]).map(Some)
}

/// IteratorClose ( iteratorRecord, completion ) (§7.4.11) of the iterator `iterator`. A throw
/// completion (`Err`, its exception pending) wins over anything `return` does; for a normal
/// (or return) completion a throw from `return`, or a result that is not an object, is the
/// result. A termination passes on without running `return`.
fn close<'s, T>(
    cx: &mut Cx<'s>,
    iterator: Local<'s, JsObject>,
    completion: JsResult<T>,
) -> JsResult<T> {
    match completion {
        Err(t) => {
            // The thrown value waits in a Rust local (a root by the stack scan) while
            // `return` runs. catch_exception gives Err back while the isolate terminates.
            let exception = cx.catch_exception(t)?;
            // 3-4. innerResult = Completion(GetMethod(iterator, "return")), then its call.
            if let Err(t) = call_return(cx, iterator) {
                cx.catch_exception(t)?;
            }
            // 5. If completion is a throw completion, return ? completion.
            Err(cx.throw(exception))
        }
        Ok(v) => {
            // 6. If innerResult is a throw completion, return ? innerResult.
            match call_return(cx, iterator)? {
                // 7. If innerResult.[[Value]] is not an Object, throw a TypeError exception.
                Some(r) if !r.is_object() => {
                    let shown = cx.display(r);
                    let msg = format!("Iterator result {shown} is not an object");
                    Err(cx.throw_type_error(&msg))
                }
                // 8. Return ? completion.
                _ => Ok(v),
            }
        }
    }
}

/// The TypeError for a callback argument that is not callable, after IteratorClose of `o`
/// (the argument-validation step of every helper: the record is { O, undefined, false }).
fn not_callable_close<'s, T>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, f: Local<'s>) -> JsResult<T> {
    // a. Let error be ThrowCompletion(a newly created TypeError object).
    let shown = cx.display(f);
    let error = Err(cx.throw_type_error(&format!("{shown} is not a function")));
    // b. Return ? IteratorClose(iterated, error).
    close(cx, o, error)
}

/// The state slot of `o`, which is an object of [`CLASS`].
fn state_of<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<i32> {
    Ok(cx
        .slot(o, CLASS, slot::STATE)?
        .as_int32()
        .expect("the state is an int32"))
}

fn set_state<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, bits: i32) -> JsResult<()> {
    cx.set_slot(o, CLASS, slot::STATE, Local::int32(bits))
}

/// CreateIteratorFromClosure ( closure, "Iterator Helper", %IteratorHelperPrototype%,
/// « `[[UnderlyingIterator]]` » ) for a helper of `kind`: suspended-start, with
/// `[[UnderlyingIterator]]` = { iterator, next, false }.
fn new_helper<'s>(
    cx: &mut Cx<'s>,
    kind: i32,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
    func: Local<'s>,
    count: f64,
) -> JsResult<Local<'s>> {
    new_helper_object(cx, kind, iterator.value(), next, func, count).map(|o| o.value())
}

/// [`new_helper`] for any kind: slots 1-4 are `a`, `b`, `c` and the Number `count`; the other
/// slots of the kind are undefined.
fn new_helper_object<'s>(
    cx: &mut Cx<'s>,
    kind: i32,
    a: Local<'s>,
    b: Local<'s>,
    c: Local<'s>,
    count: f64,
) -> JsResult<Local<'s, JsObject>> {
    let n = match kind {
        kind::FLAT_MAP => slot::FLAT_MAP_COUNT,
        kind::CONCAT => slot::CONCAT_COUNT,
        kind::ZIP | kind::ZIP_KEYED | kind::WINDOWS => slot::ZIP_COUNT,
        _ => slot::HELPER_COUNT,
    };
    let proto = cx.intrinsic(IntrinsicId::IteratorHelperPrototype);
    let o = cx.new_object_with_slots(proto, CLASS, n);
    cx.set_slot(o, CLASS, slot::STATE, Local::int32(kind | state::START))?;
    cx.set_slot(o, CLASS, slot::ITERATOR, a)?;
    cx.set_slot(o, CLASS, slot::NEXT, b)?;
    cx.set_slot(o, CLASS, slot::FUNC, c)?;
    cx.set_slot(o, CLASS, slot::COUNT, Local::number(count))?;
    Ok(o)
}

/// A slot that holds a Number, as an f64.
fn number_slot<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, i: usize) -> JsResult<f64> {
    let v = cx.slot(o, CLASS, i)?;
    Ok(v.as_number().expect("the slot holds a Number"))
}

/// A slot that holds a list (an array that only the helper sees).
fn list_slot<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    i: usize,
) -> JsResult<Local<'s, JsObject>> {
    let v = cx.slot(o, CLASS, i)?;
    Ok(v.as_object().expect("the slot holds a list"))
}

/// A count slot as an f64 (an integral Number or +∞).
fn count_of<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<f64> {
    let v = cx.slot(o, CLASS, slot::COUNT)?;
    Ok(v.as_number().expect("the count is a Number"))
}

// ---- Iterator ---------------------------------------------------------------------------------

/// Iterator ( ) (§27.1.3.1).
fn iterator_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is either undefined or the active function object, throw a TypeError
    //    exception.
    let callee = args.callee().value();
    let nt = match new_target.as_object() {
        Some(nt) if !nt.value().is_same_bits(&callee) => nt,
        _ => return Err(cx.throw_type_error("Abstract class Iterator not directly constructable")),
    };
    // 2. Return ? OrdinaryCreateFromConstructor(NewTarget, "%Iterator.prototype%").
    let o = cx.ordinary_create_from_constructor(nt, IntrinsicId::IteratorPrototype)?;
    Ok(o.value())
}

/// GetIteratorFlattenable ( obj, primitiveHandling ) (§7.4.5): the iterator and its `next`.
fn get_iterator_flattenable<'s>(
    cx: &mut Cx<'s>,
    obj: Local<'s>,
    iterate_strings: bool,
) -> JsResult<(Local<'s, JsObject>, Local<'s>)> {
    // 1. If obj is not an Object, then
    //    a. If primitiveHandling is reject-primitives, throw a TypeError exception.
    //    b. Assert: primitiveHandling is iterate-string-primitives.
    //    c. If obj is not a String, throw a TypeError exception.
    if !obj.is_object() && !(iterate_strings && obj.is_string()) {
        let shown = cx.display(obj);
        let msg = format!("{shown} is not an object");
        return Err(cx.throw_type_error(&msg));
    }
    // 2. Let method be ? GetMethod(obj, %Symbol.iterator%).
    let key = well_known(cx, "iterator")?;
    let method = cx.get_v(obj, key)?;
    // 3. If method is undefined, then let iterator be obj.
    // 4. Else, let iterator be ? Call(method, obj).
    let iterator = if method.is_undefined_or_null() {
        obj
    } else {
        if !cx.is_callable(method) {
            let shown = cx.display(method);
            let msg = format!("{shown} is not a function");
            return Err(cx.throw_type_error(&msg));
        }
        cx.call(method, obj, &[])?
    };
    // 5. If iterator is not an Object, throw a TypeError exception.
    let Some(iterator) = iterator.as_object() else {
        let shown = cx.display(iterator);
        let msg = format!("{shown} is not an object");
        return Err(cx.throw_type_error(&msg));
    };
    // 6. Return ? GetIteratorDirect(iterator).
    let next = get_iterator_direct(cx, iterator)?;
    Ok((iterator, next))
}

/// Iterator.from ( O ) (§27.1.3.2.1).
fn from<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let iteratorRecord be ? GetIteratorFlattenable(O, iterate-string-primitives).
    let (iterator, next) = get_iterator_flattenable(cx, args.get(0), true)?;
    // 2. Let hasInstance be ? OrdinaryHasInstance(%Iterator%, iteratorRecord.[[Iterator]]).
    //    (%Iterator%.prototype is non-writable and non-configurable: the Get gives
    //    %Iterator.prototype% and runs no JS.) Each step polls and counts the proxies it asks:
    //    a proxy can make the chain a cycle, which ends in a RangeError, as in V8 (Codex r1
    //    SPEC-ITER 1).
    let p = cx.intrinsic(IntrinsicId::IteratorPrototype).value();
    let mut o = iterator;
    let mut hops = 0;
    let has_instance = loop {
        // a. Set O to ? O.[[GetPrototypeOf]]().
        let proto = cx.prototype_walk_step(o, &mut hops)?;
        // b. If O is null, return false.
        let Some(proto) = proto.as_object() else {
            break false;
        };
        // c. If SameValue(P, O) is true, return true.
        if proto.value().is_same_bits(&p) {
            break true;
        }
        o = proto;
    };
    // 3. If hasInstance is true, then return iteratorRecord.[[Iterator]].
    if has_instance {
        return Ok(iterator.value());
    }
    // 4. Let wrapper be OrdinaryObjectCreate(%WrapForValidIteratorPrototype%, « [[Iterated]] »).
    let proto = cx.intrinsic(IntrinsicId::WrapForValidIteratorPrototype);
    let wrapper = cx.new_object_with_slots(proto, CLASS, slot::WRAP_COUNT);
    cx.set_slot(wrapper, CLASS, slot::STATE, Local::int32(kind::WRAP))?;
    // 5. Set wrapper.[[Iterated]] to iteratorRecord.
    cx.set_slot(wrapper, CLASS, slot::ITERATOR, iterator)?;
    cx.set_slot(wrapper, CLASS, slot::NEXT, next)?;
    // 6. Return wrapper.
    Ok(wrapper.value())
}

/// The wrapper `this` (`RequireInternalSlot(O, [[Iterated]])`), else a TypeError.
fn this_wrapper<'s>(cx: &mut Cx<'s>, this: Local<'s>, name: &str) -> JsResult<Local<'s, JsObject>> {
    if let Some(o) = this.as_object()
        && cx.object_class(o) == Some(CLASS)
        && state_of(cx, o)? & kind::MASK == kind::WRAP
    {
        return Ok(o);
    }
    let shown = cx.display(this);
    let msg =
        format!("%WrapForValidIteratorPrototype%.{name} called on incompatible receiver {shown}");
    Err(cx.throw_type_error(&msg))
}

/// %WrapForValidIteratorPrototype%.next ( ) (§27.1.3.2.1.1.1).
fn wrap_next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be this value.
    // 2. Perform ? RequireInternalSlot(O, [[Iterated]]).
    let o = this_wrapper(cx, this, "next")?;
    // 3. Let iteratorRecord be O.[[Iterated]].
    let iterator = cx.slot(o, CLASS, slot::ITERATOR)?;
    let next = cx.slot(o, CLASS, slot::NEXT)?;
    // 4. Return ? Call(iteratorRecord.[[NextMethod]], iteratorRecord.[[Iterator]]).
    cx.call(next, iterator, &[])
}

/// %WrapForValidIteratorPrototype%.return ( ) (§27.1.3.2.1.1.2).
fn wrap_return<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be this value.
    // 2. Perform ? RequireInternalSlot(O, [[Iterated]]).
    let o = this_wrapper(cx, this, "return")?;
    // 3. Let iterator be O.[[Iterated]].[[Iterator]].
    let iterator = cx.slot(o, CLASS, slot::ITERATOR)?;
    let iterator = iterator.as_object().expect("[[Iterator]] is an object");
    // 4. Assert: iterator is an Object.
    // 5. Let returnMethod be ? GetMethod(iterator, "return").
    // 6. If returnMethod is undefined, then
    //    a. Return CreateIteratorResultObject(undefined, true).
    // 7. Return ? Call(returnMethod, iterator).
    match call_return(cx, iterator)? {
        Some(r) => Ok(r),
        None => iter_result(cx, Local::undefined(), true),
    }
}

/// get Iterator.prototype.constructor (§27.1.4.1.1).
fn constructor_get<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Return %Iterator%.
    Ok(cx.intrinsic(IntrinsicId::Iterator).value())
}

/// set Iterator.prototype.constructor (§27.1.4.1.2).
fn constructor_set<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Perform ? SetterThatIgnoresPrototypeProperties(this value, %Iterator.prototype%,
    //    "constructor", v).
    // SetterThatIgnoresPrototypeProperties ( thisValue, home, p, v ):
    // 1. If thisValue is not an Object, throw a TypeError exception.
    let Some(o) = this.as_object() else {
        return Err(cx.throw_type_error("set Iterator.prototype.constructor called on non-object"));
    };
    // 2. If SameValue(thisValue, home) is true, throw a TypeError exception.
    let home = cx.intrinsic(IntrinsicId::IteratorPrototype);
    if o.value().is_same_bits(&home.value()) {
        return Err(
            cx.throw_type_error("Cannot assign to read only property 'constructor' of object")
        );
    }
    let v = args.get(0);
    // 3. Let desc be ? thisValue.[[GetOwnProperty]](p).
    let desc = cx.get_own_property(o, "constructor")?;
    if desc.is_none() {
        // 4. If desc is undefined, then
        //    a. Perform ? CreateDataPropertyOrThrow(thisValue, p, v).
        cx.create_data_property_or_throw(o, "constructor", v)?;
    } else {
        // 5. Else, a. Perform ? Set(thisValue, p, v, true).
        cx.set(o, "constructor", v, true)?;
    }
    // 2. Return undefined.
    Ok(Local::undefined())
}

// ---- the lazy helpers -------------------------------------------------------------------------

/// The shared steps 1-5 of map, filter and flatMap: the this object and its `next`, after
/// the callback check (which closes the iterator on failure).
fn begin_with_callback<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    f: Local<'s>,
    name: &str,
) -> JsResult<(Local<'s, JsObject>, Local<'s>)> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, name)?;
    // 3. Let iterated be the Iterator Record { [[Iterator]]: O, [[NextMethod]]: undefined,
    //    [[Done]]: false }.
    // 4. If IsCallable(f) is false, then
    //    a. Let error be ThrowCompletion(a newly created TypeError object).
    //    b. Return ? IteratorClose(iterated, error).
    if !cx.is_callable(f) {
        return not_callable_close(cx, o, f);
    }
    // 5. Set iterated to ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    Ok((o, next))
}

/// Iterator.prototype.map ( mapper ) (§27.1.4.8).
fn map<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let mapper = args.get(0);
    let (o, next) = begin_with_callback(cx, this, mapper, "map")?;
    // 6-9. The closure, CreateIteratorFromClosure, [[UnderlyingIterator]] = iterated.
    new_helper(cx, kind::MAP, o, next, mapper, 0.0)
}

/// Iterator.prototype.filter ( predicate ) (§27.1.4.4).
fn filter<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let predicate = args.get(0);
    let (o, next) = begin_with_callback(cx, this, predicate, "filter")?;
    new_helper(cx, kind::FILTER, o, next, predicate, 0.0)
}

/// Iterator.prototype.flatMap ( mapper ) (§27.1.4.6).
fn flat_map<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let mapper = args.get(0);
    let (o, next) = begin_with_callback(cx, this, mapper, "flatMap")?;
    new_helper(cx, kind::FLAT_MAP, o, next, mapper, 0.0)
}

/// The shared steps 1-10 of take and drop: the this object, its `next` and the limit (an
/// integer ≥ 0 or +∞).
fn begin_with_limit<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    limit: Local<'s>,
    name: &str,
) -> JsResult<(Local<'s, JsObject>, Local<'s>, f64)> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, name)?;
    // 3. Let iterated be the Iterator Record { [[Iterator]]: O, [[NextMethod]]: undefined,
    //    [[Done]]: false }.
    // 4. Let numLimit be Completion(ToNumber(limit)).
    let num_limit = cx.to_number(limit);
    // 5. IfAbruptCloseIterator(numLimit, iterated).
    let num_limit = close_if_abrupt(cx, o, num_limit)?;
    // 6. If numLimit is NaN, then a RangeError, after IteratorClose(iterated, error).
    // 7. If numLimit is finite and numLimit > 𝔽(2**53 - 1), the same.
    // 8. Let integerLimit be ! ToIntegerOrInfinity(numLimit).
    // 9. If integerLimit < 0, the same.
    let integer_limit = if num_limit.is_finite() {
        num_limit.trunc() + 0.0
    } else {
        num_limit
    };
    if num_limit.is_nan()
        || (num_limit.is_finite() && num_limit > 9_007_199_254_740_991.0)
        || integer_limit < 0.0
    {
        let msg = format!("{} must be positive", cx.display(limit));
        let error = Err(cx.throw_range_error(&msg));
        return close(cx, o, error);
    }
    // 10. Set iterated to ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    Ok((o, next, integer_limit))
}

/// IfAbruptCloseIterator ( value, iteratorRecord ) (§7.4.12): an abrupt `value` closes the
/// iterator and passes on; a normal one is the result.
fn close_if_abrupt<'s, T>(
    cx: &mut Cx<'s>,
    iterator: Local<'s, JsObject>,
    value: JsResult<T>,
) -> JsResult<T> {
    match value {
        Ok(v) => Ok(v),
        Err(t) => close(cx, iterator, Err(t)),
    }
}

/// Iterator.prototype.take ( limit ) (§27.1.4.11).
fn take<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (o, next, limit) = begin_with_limit(cx, this, args.get(0), "take")?;
    // 11-14. The closure, CreateIteratorFromClosure, [[UnderlyingIterator]] = iterated.
    new_helper(cx, kind::TAKE, o, next, Local::undefined(), limit)
}

/// Iterator.prototype.drop ( limit ) (§27.1.4.2).
fn drop<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (o, next, limit) = begin_with_limit(cx, this, args.get(0), "drop")?;
    new_helper(cx, kind::DROP, o, next, Local::undefined(), limit)
}

/// The helper `this` (GeneratorValidate with the brand "Iterator Helper", or
/// `RequireInternalSlot(O, [[UnderlyingIterator]])`), else a TypeError.
fn this_helper<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    name: &str,
) -> JsResult<(Local<'s, JsObject>, i32)> {
    if let Some(o) = this.as_object()
        && cx.object_class(o) == Some(CLASS)
    {
        let bits = state_of(cx, o)?;
        if bits & kind::MASK != kind::WRAP {
            return Ok((o, bits));
        }
    }
    let shown = cx.display(this);
    let msg = format!("%IteratorHelperPrototype%.{name} called on incompatible receiver {shown}");
    Err(cx.throw_type_error(&msg))
}

/// %IteratorHelperPrototype%.next ( ) (§27.1.2.1.1): GeneratorResume(this value, undefined,
/// "Iterator Helper").
fn helper_next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // GeneratorValidate: the brand, and not executing.
    let (o, bits) = this_helper(cx, this, "next")?;
    match bits & state::MASK {
        state::RUNNING => return Err(cx.throw_type_error("Generator is already running")),
        // GeneratorState completed: CreateIteratorResultObject(undefined, true).
        state::DONE => return iter_result(cx, Local::undefined(), true),
        _ => {}
    }
    let kind = bits & kind::MASK;
    // After its last Yield the closure returns (chunks and windows).
    if bits & state::LAST != 0 {
        set_state(cx, o, kind | state::DONE)?;
        release(cx, o, kind)?;
        return iter_result(cx, Local::undefined(), true);
    }
    set_state(cx, o, kind | state::RUNNING)?;
    let r = resume(cx, o, kind);
    // The closure stops at a Yield (suspended-yield), or it ends: a return or an abrupt
    // completion (completed).
    let after = match r {
        Ok(Resumed::Yield(_)) => state::YIELD,
        Ok(Resumed::Last(_)) => state::YIELD | state::LAST,
        _ => state::DONE,
    };
    set_state(cx, o, kind | after)?;
    if after == state::DONE {
        release(cx, o, kind)?;
    }
    match r? {
        Resumed::Yield(v) | Resumed::Last(v) => iter_result(cx, v, false),
        Resumed::Return => iter_result(cx, Local::undefined(), true),
    }
}

/// A helper of `kind` is completed: it drops the values that only its closure used (the
/// callback, flatMap's inner iterator, the lists of concat, zip and windows), as a completed
/// generator keeps no closure values (Codex r1 SPEC-ITER 2). The underlying iterator of a
/// one-iterator helper stays: `return` reads the state only once the helper is completed.
fn release<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, kind: i32) -> JsResult<()> {
    let undefined = Local::undefined();
    match kind {
        kind::MAP | kind::FILTER => cx.set_slot(o, CLASS, slot::FUNC, undefined)?,
        kind::FLAT_MAP => {
            cx.set_slot(o, CLASS, slot::FUNC, undefined)?;
            cx.set_slot(o, CLASS, slot::INNER, undefined)?;
            cx.set_slot(o, CLASS, slot::INNER_NEXT, undefined)?;
        }
        kind::CONCAT => {
            cx.set_slot(o, CLASS, slot::ITERATOR, undefined)?;
            cx.set_slot(o, CLASS, slot::NEXT, undefined)?;
            cx.set_slot(o, CLASS, slot::ITEMS, undefined)?;
        }
        kind::ZIP | kind::ZIP_KEYED => {
            cx.set_slot(o, CLASS, slot::ITERS, undefined)?;
            cx.set_slot(o, CLASS, slot::PADDING, undefined)?;
            cx.set_slot(o, CLASS, slot::KEYS, undefined)?;
        }
        kind::WINDOWS => cx.set_slot(o, CLASS, slot::BUFFER, undefined)?,
        _ => {}
    }
    Ok(())
}

/// One run of the closure of a helper of `kind` up to its next Yield, or to its end.
fn resume<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, kind: i32) -> JsResult<Resumed<'s>> {
    match kind {
        kind::CONCAT => return resume_concat(cx, o),
        kind::ZIP | kind::ZIP_KEYED => return resume_zip(cx, o, kind == kind::ZIP_KEYED),
        _ => {}
    }
    let iterator = cx.slot(o, CLASS, slot::ITERATOR)?;
    let iterator = iterator.as_object().expect("[[Iterator]] is an object");
    let next = cx.slot(o, CLASS, slot::NEXT)?;
    let r = match kind {
        kind::MAP => resume_map(cx, o, iterator, next),
        kind::FILTER => resume_filter(cx, o, iterator, next),
        kind::TAKE => resume_take(cx, o, iterator, next),
        kind::DROP => resume_drop(cx, o, iterator, next),
        kind::FLAT_MAP => resume_flat_map(cx, o, iterator, next),
        kind::CHUNKS => return resume_chunks(cx, o, iterator, next),
        kind::WINDOWS => return resume_windows(cx, o, iterator, next),
        _ => unreachable!("helper kind {kind}"),
    };
    r.map(Resumed::from)
}

/// map step 6.b (the closure): one value of the underlying iterator, mapped.
fn resume_map<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s>>> {
    // i. Let value be ? IteratorStepValue(iterated).
    // ii. If value is done, return ReturnCompletion(undefined).
    let Some(value) = step_value(cx, iterator, next)? else {
        return Ok(None);
    };
    // iii. Let mapped be Completion(Call(mapper, undefined, « value, 𝔽(counter) »)).
    let mapper = cx.slot(o, CLASS, slot::FUNC)?;
    let counter = count_of(cx, o)?;
    let mapped = cx.call(mapper, Local::undefined(), &[value, Local::number(counter)]);
    // iv. IfAbruptCloseIterator(mapped, iterated).
    let mapped = close_if_abrupt(cx, iterator, mapped)?;
    // vii. Set counter to counter + 1 (after the Yield; no JS sees the order).
    cx.set_slot(o, CLASS, slot::COUNT, Local::number(counter + 1.0))?;
    // v. Let completion be Completion(Yield(mapped)).
    Ok(Some(mapped))
}

/// filter step 6.b (the closure): the next value of the underlying iterator that the
/// predicate selects.
fn resume_filter<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s>>> {
    let predicate = cx.slot(o, CLASS, slot::FUNC)?;
    loop {
        // i. Let value be ? IteratorStepValue(iterated).
        // ii. If value is done, return ReturnCompletion(undefined).
        let Some(value) = step_value(cx, iterator, next)? else {
            return Ok(None);
        };
        // iii. Let selected be Completion(Call(predicate, undefined, « value, 𝔽(counter) »)).
        let counter = count_of(cx, o)?;
        let selected = cx.call(
            predicate,
            Local::undefined(),
            &[value, Local::number(counter)],
        );
        // iv. IfAbruptCloseIterator(selected, iterated).
        let selected = close_if_abrupt(cx, iterator, selected)?;
        // vi. Set counter to counter + 1.
        cx.set_slot(o, CLASS, slot::COUNT, Local::number(counter + 1.0))?;
        // v. If ToBoolean(selected) is true, then
        //    1. Let completion be Completion(Yield(value)).
        if cx.to_boolean(selected) {
            return Ok(Some(value));
        }
    }
}

/// take step 11 (the closure): the next value while the remaining count is above 0.
fn resume_take<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s>>> {
    let remaining = count_of(cx, o)?;
    // i. If remaining = 0, then
    //    1. Return ? IteratorClose(iterated, ReturnCompletion(undefined)).
    if remaining == 0.0 {
        return close(cx, iterator, Ok(None));
    }
    // ii. If remaining ≠ +∞, then
    //     1. Set remaining to remaining - 1.
    if remaining != f64::INFINITY {
        cx.set_slot(o, CLASS, slot::COUNT, Local::number(remaining - 1.0))?;
    }
    // iii. Let value be ? IteratorStepValue(iterated).
    // iv. If value is done, return ReturnCompletion(undefined).
    // v. Let completion be Completion(Yield(value)).
    step_value(cx, iterator, next)
}

/// drop step 11 (the closure): skip the remaining count, then the next value.
fn resume_drop<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s>>> {
    // b. Repeat, while remaining > 0,
    let mut remaining = count_of(cx, o)?;
    if remaining > 0.0 {
        while remaining > 0.0 {
            // i. If remaining ≠ +∞, then
            //    1. Set remaining to remaining - 1.
            if remaining != f64::INFINITY {
                remaining -= 1.0;
            }
            // ii. Let next be ? IteratorStep(iterated).
            // iii. If next is done, return ReturnCompletion(undefined).
            if step(cx, iterator, next)?.is_none() {
                return Ok(None);
            }
        }
        cx.set_slot(o, CLASS, slot::COUNT, Local::number(0.0))?;
    }
    // c. Repeat,
    //    i. Let value be ? IteratorStepValue(iterated).
    //    ii. If value is done, return ReturnCompletion(undefined).
    //    iii. Let completion be Completion(Yield(value)).
    step_value(cx, iterator, next)
}

/// flatMap step 6.b (the closure): the next value of the inner iterator; a done inner
/// iterator moves on to the next mapped value of the underlying iterator.
fn resume_flat_map<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Option<Local<'s>>> {
    loop {
        // viii. Repeat, while innerAlive is true,
        let inner = cx.slot(o, CLASS, slot::INNER)?;
        if let Some(inner) = inner.as_object() {
            let inner_next = cx.slot(o, CLASS, slot::INNER_NEXT)?;
            // 1. Let innerValue be Completion(IteratorStepValue(innerIterator)).
            let inner_value = step_value(cx, inner, inner_next);
            // 2. IfAbruptCloseIterator(innerValue, iterated).
            match close_if_abrupt(cx, iterator, inner_value)? {
                // 4. Else, a. Let completion be Completion(Yield(innerValue)).
                Some(v) => return Ok(Some(v)),
                // 3. If innerValue is done, then a. Set innerAlive to false.
                None => {
                    cx.set_slot(o, CLASS, slot::INNER, Local::undefined())?;
                    cx.set_slot(o, CLASS, slot::INNER_NEXT, Local::undefined())?;
                    // ix. Set counter to counter + 1.
                    let counter = count_of(cx, o)?;
                    cx.set_slot(o, CLASS, slot::COUNT, Local::number(counter + 1.0))?;
                }
            }
        }
        // i. Let value be ? IteratorStepValue(iterated).
        // ii. If value is done, return ReturnCompletion(undefined).
        let Some(value) = step_value(cx, iterator, next)? else {
            return Ok(None);
        };
        // iii. Let mapped be Completion(Call(mapper, undefined, « value, 𝔽(counter) »)).
        let mapper = cx.slot(o, CLASS, slot::FUNC)?;
        let counter = count_of(cx, o)?;
        let mapped = cx.call(mapper, Local::undefined(), &[value, Local::number(counter)]);
        // iv. IfAbruptCloseIterator(mapped, iterated).
        let mapped = close_if_abrupt(cx, iterator, mapped)?;
        // v. Let innerIterator be Completion(GetIteratorFlattenable(mapped,
        //    reject-primitives)).
        let inner = get_iterator_flattenable(cx, mapped, false);
        // vi. IfAbruptCloseIterator(innerIterator, iterated).
        let (inner, inner_next) = close_if_abrupt(cx, iterator, inner)?;
        // vii. Let innerAlive be true.
        cx.set_slot(o, CLASS, slot::INNER, inner)?;
        cx.set_slot(o, CLASS, slot::INNER_NEXT, inner_next)?;
    }
}

/// %IteratorHelperPrototype%.return ( ) (§27.1.2.1.2, with the joint iteration form: step 4.c
/// is IteratorCloseAll(O.`[[UnderlyingIterators]]`, NormalCompletion(unused))).
fn helper_return<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be this value.
    // 2. Perform ? RequireInternalSlot(O, [[UnderlyingIterators]]).
    // 3. Assert: O has a [[GeneratorState]] slot.
    let (o, bits) = this_helper(cx, this, "return")?;
    let kind = bits & kind::MASK;
    match bits & state::MASK {
        // 4. If O.[[GeneratorState]] is suspended-start, then
        state::START => {
            // The underlying iterators (read before the state changes: release drops the
            // list of zip).
            let underlying = match kind {
                kind::CONCAT => Local::undefined(),
                kind::ZIP | kind::ZIP_KEYED => cx.slot(o, CLASS, slot::ITERS)?,
                _ => cx.slot(o, CLASS, slot::ITERATOR)?,
            };
            let count = if matches!(kind, kind::ZIP | kind::ZIP_KEYED) {
                number_slot(cx, o, slot::ITER_COUNT)? as u64
            } else {
                0
            };
            // a. Set O.[[GeneratorState]] to completed.
            set_state(cx, o, kind | state::DONE)?;
            release(cx, o, kind)?;
            // c. Perform ? IteratorCloseAll(O.[[UnderlyingIterators]], NormalCompletion(unused)).
            match kind {
                // concat: [[UnderlyingIterators]] is empty.
                kind::CONCAT => {}
                kind::ZIP | kind::ZIP_KEYED => {
                    let iters = underlying.as_object().expect("iters is a list");
                    close_all(cx, iters, count, None, Ok(()))?;
                }
                _ => {
                    let iterator = underlying.as_object().expect("[[Iterator]] is an object");
                    close(cx, iterator, Ok(()))?;
                }
            }
            // d. Return CreateIteratorResultObject(undefined, true).
            iter_result(cx, Local::undefined(), true)
        }
        // 5-6. GeneratorResumeAbrupt(O, return completion, "Iterator Helper"):
        // GeneratorValidate (executing: a TypeError).
        state::RUNNING => Err(cx.throw_type_error("Generator is already running")),
        // completed: CreateIteratorResultObject(undefined, true).
        state::DONE => iter_result(cx, Local::undefined(), true),
        // suspended-yield: the closure resumes with the return completion at its Yield.
        _ => {
            set_state(cx, o, kind | state::RUNNING)?;
            // At its last Yield the closure ignores the completion and returns.
            let r = if bits & state::LAST != 0 {
                Ok(())
            } else {
                return_at_yield(cx, o, kind)
            };
            set_state(cx, o, kind | state::DONE)?;
            release(cx, o, kind)?;
            r?;
            iter_result(cx, Local::undefined(), true)
        }
    }
}

/// The return completion at the Yield of a helper closure: IfAbruptCloseIterator(completion,
/// iterated); for flatMap, first IteratorClose(innerIterator, completion) (flatMap step
/// 6.b.viii.4.b); concat: IteratorClose of the inner iterator (step 3.a.v.3.b); zip:
/// IteratorCloseAll(openIters, completion) (IteratorZip step 3.b.vi).
fn return_at_yield<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, kind: i32) -> JsResult<()> {
    match kind {
        kind::CONCAT => {
            let inner = cx.slot(o, CLASS, slot::ITERATOR)?;
            let inner = inner
                .as_object()
                .expect("a Yield of concat has an inner iterator");
            return close(cx, inner, Ok(()));
        }
        kind::ZIP | kind::ZIP_KEYED => {
            let iters = list_slot(cx, o, slot::ITERS)?;
            let count = number_slot(cx, o, slot::ITER_COUNT)? as u64;
            return close_all(cx, iters, count, None, Ok(()));
        }
        _ => {}
    }
    let iterator = cx.slot(o, CLASS, slot::ITERATOR)?;
    let iterator = iterator.as_object().expect("[[Iterator]] is an object");
    if kind == kind::FLAT_MAP {
        let inner = cx.slot(o, CLASS, slot::INNER)?;
        if let Some(inner) = inner.as_object() {
            // i. Let backupCompletion be Completion(IteratorClose(innerIterator, completion)).
            let backup = close(cx, inner, Ok(()));
            // ii. IfAbruptCloseIterator(backupCompletion, iterated).
            close_if_abrupt(cx, iterator, backup)?;
            // iii. Return ? IteratorClose(iterated, completion).
        }
    }
    close(cx, iterator, Ok(()))
}

// ---- the eager methods ------------------------------------------------------------------------

/// Iterator.prototype.reduce ( reducer [ , initialValue ] ) (§27.1.4.9).
fn reduce<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let reducer = args.get(0);
    // 1-5. The this object, the reducer check, GetIteratorDirect.
    let (o, next) = begin_with_callback(cx, this, reducer, "reduce")?;
    // 6. If initialValue is not present, then
    let (mut accumulator, mut counter) = if args.len() < 2 {
        // a. Let accumulator be ? IteratorStepValue(iterated).
        // b. If accumulator is done, throw a TypeError exception.
        let Some(first) = step_value(cx, o, next)? else {
            return Err(cx.throw_type_error("Reduce of empty iterator with no initial value"));
        };
        // c. Let counter be 1.
        (first, 1.0)
    } else {
        // 7. Else, a. Let accumulator be initialValue. b. Let counter be 0.
        (args.get(1), 0.0)
    };
    // 8. Repeat,
    loop {
        // a. Let value be ? IteratorStepValue(iterated).
        // b. If value is done, return accumulator.
        let Some(value) = step_value(cx, o, next)? else {
            return Ok(accumulator);
        };
        // c. Let result be Completion(Call(reducer, undefined, « accumulator, value,
        //    𝔽(counter) »)).
        let result = cx.call(
            reducer,
            Local::undefined(),
            &[accumulator, value, Local::number(counter)],
        );
        // d. IfAbruptCloseIterator(result, iterated).
        // e. Set accumulator to result.
        accumulator = close_if_abrupt(cx, o, result)?;
        // f. Set counter to counter + 1.
        counter += 1.0;
    }
}

/// Iterator.prototype.toArray ( ) (§27.1.4.12).
fn to_array<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, "toArray")?;
    // 3. Let iterated be ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    // 4. Let items be a new empty List (a JS array: the list holds Values).
    let items = cx.new_array();
    let mut n: u64 = 0;
    // 5. Repeat,
    //    a. Let value be ? IteratorStepValue(iterated).
    //    b. If value is done, return CreateArrayFromList(items).
    //    c. Append value to items.
    while let Some(value) = step_value(cx, o, next)? {
        cx.create_data_property_index_or_throw(items, n, value)?;
        n += 1;
    }
    Ok(items.value())
}

/// Iterator.prototype.forEach ( procedure ) (§27.1.4.7).
fn for_each<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let procedure = args.get(0);
    let (o, next) = begin_with_callback(cx, this, procedure, "forEach")?;
    // 6. Let counter be 0.
    let mut counter = 0.0;
    // 7. Repeat,
    //    a. Let value be ? IteratorStepValue(iterated).
    //    b. If value is done, return undefined.
    while let Some(value) = step_value(cx, o, next)? {
        // c. Let result be Completion(Call(procedure, undefined, « value, 𝔽(counter) »)).
        let result = cx.call(
            procedure,
            Local::undefined(),
            &[value, Local::number(counter)],
        );
        // d. IfAbruptCloseIterator(result, iterated).
        close_if_abrupt(cx, o, result)?;
        // e. Set counter to counter + 1.
        counter += 1.0;
    }
    Ok(Local::undefined())
}

/// What some, every and find do with a value the predicate selects (or, for every, does not
/// select).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Search {
    Some,
    Every,
    Find,
}

/// Iterator.prototype.some, every and find (§27.1.4.10, §27.1.4.3, §27.1.4.5).
fn search<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    which: Search,
) -> JsResult<Local<'s>> {
    let predicate = args.get(0);
    let name = match which {
        Search::Some => "some",
        Search::Every => "every",
        Search::Find => "find",
    };
    let (o, next) = begin_with_callback(cx, this, predicate, name)?;
    // 6. Let counter be 0.
    let mut counter = 0.0;
    // 7. Repeat,
    //    a. Let value be ? IteratorStepValue(iterated).
    //    b. If value is done, return false (some), true (every) or undefined (find).
    while let Some(value) = step_value(cx, o, next)? {
        // c. Let result be Completion(Call(predicate, undefined, « value, 𝔽(counter) »)).
        let result = cx.call(
            predicate,
            Local::undefined(),
            &[value, Local::number(counter)],
        );
        // d. IfAbruptCloseIterator(result, iterated).
        let result = close_if_abrupt(cx, o, result)?;
        let selected = cx.to_boolean(result);
        // e. some: if ToBoolean(result) is true, return ? IteratorClose(iterated,
        //    NormalCompletion(true)). every: if it is false, return ? IteratorClose(iterated,
        //    NormalCompletion(false)). find: if it is true, return ? IteratorClose(iterated,
        //    NormalCompletion(value)).
        match (which, selected) {
            (Search::Some, true) => return close(cx, o, Ok(Local::boolean(true))),
            (Search::Every, false) => return close(cx, o, Ok(Local::boolean(false))),
            (Search::Find, true) => return close(cx, o, Ok(value)),
            _ => {}
        }
        // f. Set counter to counter + 1.
        counter += 1.0;
    }
    Ok(match which {
        Search::Some => Local::boolean(false),
        Search::Every => Local::boolean(true),
        Search::Find => Local::undefined(),
    })
}

/// Iterator.prototype.some ( predicate ) (§27.1.4.10).
fn some<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    search(cx, this, args, Search::Some)
}

/// Iterator.prototype.every ( predicate ) (§27.1.4.3).
fn every<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    search(cx, this, args, Search::Every)
}

/// Iterator.prototype.find ( predicate ) (§27.1.4.5).
fn find<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    search(cx, this, args, Search::Find)
}

// ---- part 2: lists, IteratorCloseAll -----------------------------------------------------------

/// Appends `v` at index `n` of the list `list` (an array that only this module sees).
fn list_push<'s>(
    cx: &mut Cx<'s>,
    list: Local<'s, JsObject>,
    n: &mut u64,
    v: Local<'s>,
) -> JsResult<()> {
    cx.create_data_property_index_or_throw(list, *n, v)?;
    *n += 1;
    Ok(())
}

/// IteratorCloseAll ( iters, completion ) (joint iteration): IteratorClose of each iterator of
/// the pair list `iters` (`count` pairs; a null iterator has left the list), in reverse list
/// order, each with the completion the last one left. `skip` is an index that has left the
/// list but is not null yet (its step threw: the spec removes it, then closes the rest).
fn close_all<'s, T>(
    cx: &mut Cx<'s>,
    iters: Local<'s, JsObject>,
    count: u64,
    skip: Option<u64>,
    completion: JsResult<T>,
) -> JsResult<T> {
    // A throw completion: its value waits in a Rust local (a root by the stack scan), so that
    // the reads of the list run with no pending exception. catch_exception gives Err back
    // while the isolate terminates.
    let mut completion = match completion {
        Ok(v) => Ok(v),
        Err(t) => Err(cx.catch_exception(t)?),
    };
    // 1. For each element iter of iters, in reverse List order, do
    for i in (0..count).rev() {
        // A pass that closes no iterator calls no JS: one budget unit per pass (H23).
        cx.poll_interrupt()?;
        if skip == Some(i) {
            continue;
        }
        let iterator = cx.get_index(iters, 2 * i)?;
        let Some(iterator) = iterator.as_object() else {
            continue;
        };
        // a. Set completion to Completion(IteratorClose(iter, completion)).
        completion = match completion {
            // IteratorClose with a throw completion: `return` runs, its result and its throw
            // are dropped.
            Err(e) => {
                if let Err(t) = call_return(cx, iterator) {
                    cx.catch_exception(t)?;
                }
                Err(e)
            }
            Ok(v) => match close(cx, iterator, Ok(v)) {
                Ok(v) => Ok(v),
                Err(t) => Err(cx.catch_exception(t)?),
            },
        };
    }
    // 2. Return ? completion.
    completion.map_err(|e| cx.throw(e))
}

// ---- Iterator.concat ----------------------------------------------------------------------------

/// Iterator.concat ( ...items ) (iterator sequencing).
fn concat<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let iterables be a new empty List.
    let items = cx.new_array().into();
    let mut n = 0;
    let key = well_known(cx, "iterator")?;
    // 2. For each element item of items, do
    for item in args.iter() {
        // a. If item is not an Object, throw a TypeError exception.
        let Some(obj) = item.as_object() else {
            let shown = cx.display(item);
            let msg = format!("{shown} is not an object");
            return Err(cx.throw_type_error(&msg));
        };
        // b. Let method be ? GetMethod(item, %Symbol.iterator%).
        let method = cx.get(obj, key)?;
        // c. If method is undefined, throw a TypeError exception.
        if method.is_undefined_or_null() || !cx.is_callable(method) {
            let shown = cx.display(item);
            let msg = format!("{shown} is not iterable");
            return Err(cx.throw_type_error(&msg));
        }
        // d. Append the Record { [[OpenMethod]]: method, [[Iterable]]: item } to iterables.
        list_push(cx, items, &mut n, method)?;
        list_push(cx, items, &mut n, item)?;
    }
    // 3-6. The closure; CreateIteratorFromClosure(closure, "Iterator Helper",
    //      %IteratorHelperPrototype%, « [[UnderlyingIterators]] »); [[UnderlyingIterators]] is
    //      a new empty List.
    let undefined = Local::undefined();
    let o = new_helper_object(cx, kind::CONCAT, undefined, undefined, items.value(), 0.0)?;
    cx.set_slot(o, CLASS, slot::ITEM_COUNT, Local::number((n / 2) as f64))?;
    Ok(o.value())
}

/// concat step 3 (the closure): the next value of the inner iterator; a done inner iterator
/// moves on to the next Record.
fn resume_concat<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>) -> JsResult<Resumed<'s>> {
    loop {
        // v. Repeat, while innerAlive is true,
        let inner = cx.slot(o, CLASS, slot::ITERATOR)?;
        if let Some(inner) = inner.as_object() {
            let inner_next = cx.slot(o, CLASS, slot::NEXT)?;
            // 1. Let innerValue be ? IteratorStepValue(iteratorRecord).
            match step_value(cx, inner, inner_next)? {
                // 3. Else, a. Let completion be Completion(Yield(innerValue)).
                Some(v) => return Ok(Resumed::Yield(v)),
                // 2. If innerValue is done, then a. Set innerAlive to false.
                None => {
                    cx.set_slot(o, CLASS, slot::ITERATOR, Local::undefined())?;
                    cx.set_slot(o, CLASS, slot::NEXT, Local::undefined())?;
                }
            }
        }
        // a. For each Record iterable of iterables, do
        let index = number_slot(cx, o, slot::ITEM_INDEX)?;
        let count = number_slot(cx, o, slot::ITEM_COUNT)?;
        // b. Return ReturnCompletion(undefined).
        if index >= count {
            return Ok(Resumed::Return);
        }
        cx.set_slot(o, CLASS, slot::ITEM_INDEX, Local::number(index + 1.0))?;
        let items = list_slot(cx, o, slot::ITEMS)?;
        let i = index as u64;
        let method = cx.get_index(items, 2 * i)?;
        let iterable = cx.get_index(items, 2 * i + 1)?;
        // i. Let iter be ? Call(iterable.[[OpenMethod]], iterable.[[Iterable]]).
        let iter = cx.call(method, iterable, &[])?;
        // ii. If iter is not an Object, throw a TypeError exception.
        let Some(iter) = iter.as_object() else {
            let shown = cx.display(iter);
            let msg = format!("{shown} is not an object");
            return Err(cx.throw_type_error(&msg));
        };
        // iii. Let iteratorRecord be ? GetIteratorDirect(iter).
        let next = get_iterator_direct(cx, iter)?;
        // iv. Let innerAlive be true.
        cx.set_slot(o, CLASS, slot::ITERATOR, iter)?;
        cx.set_slot(o, CLASS, slot::NEXT, next)?;
    }
}

// ---- Iterator.zip and Iterator.zipKeyed ---------------------------------------------------------

/// GetOptionsObject ( options ) (ECMA-402 form, as joint iteration uses it).
fn get_options_object<'s>(cx: &mut Cx<'s>, options: Local<'s>) -> JsResult<Local<'s, JsObject>> {
    // 1. If options is undefined, then return OrdinaryObjectCreate(null).
    if options.is_undefined() {
        return Ok(cx.new_object_with_proto(Local::null()));
    }
    // 2. If options is an Object, then return options.
    if let Some(o) = options.as_object() {
        return Ok(o);
    }
    // 3. Throw a TypeError exception.
    let shown = cx.display(options);
    let msg = format!("Options {shown} is not an object");
    Err(cx.throw_type_error(&msg))
}

/// True if `v` is the String `text` (no coercion).
fn is_string_value<'s>(cx: &mut Cx<'s>, v: Local<'s>, text: &str) -> JsResult<bool> {
    if !v.is_string() {
        return Ok(false);
    }
    Ok(cx.to_rust_string(v)? == text)
}

/// The shared steps 2-7 of zip and zipKeyed: the options object, the mode and (longest
/// mode) the padding option.
fn zip_options<'s>(cx: &mut Cx<'s>, options: Local<'s>) -> JsResult<(i32, Local<'s>)> {
    // 2. Set options to ? GetOptionsObject(options).
    let options = get_options_object(cx, options)?;
    // 3. Let mode be ? Get(options, "mode").
    let m = cx.get(options, "mode")?;
    // 4. If mode is undefined, set mode to "shortest".
    // 5. If mode is not one of "shortest", "longest", or "strict", throw a TypeError
    //    exception.
    let mode = if m.is_undefined() || is_string_value(cx, m, "shortest")? {
        mode::SHORTEST
    } else if is_string_value(cx, m, "longest")? {
        mode::LONGEST
    } else if is_string_value(cx, m, "strict")? {
        mode::STRICT
    } else {
        let shown = cx.display(m);
        let msg = format!("{shown} is not a valid mode");
        return Err(cx.throw_type_error(&msg));
    };
    // 6. Let paddingOption be undefined.
    // 7. If mode is "longest", then
    //    a. Set paddingOption to ? Get(options, "padding").
    //    b. If paddingOption is not undefined and paddingOption is not an Object, throw a
    //       TypeError exception.
    let mut padding = Local::undefined();
    if mode == mode::LONGEST {
        padding = cx.get(options, "padding")?;
        if !padding.is_undefined() && !padding.is_object() {
            let shown = cx.display(padding);
            let msg = format!("Padding {shown} is not an object");
            return Err(cx.throw_type_error(&msg));
        }
    }
    Ok((mode, padding))
}

/// Iterator.zip ( iterables [ , options ] ) (joint iteration).
fn zip<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If iterables is not an Object, throw a TypeError exception.
    let iterables = args.get(0);
    if !iterables.is_object() {
        let shown = cx.display(iterables);
        let msg = format!("{shown} is not an object");
        return Err(cx.throw_type_error(&msg));
    }
    // 2-7. The options.
    let (mode, padding_option) = zip_options(cx, args.get(1))?;
    // 8. Let iters be a new empty List.
    let iters: Local<'s, JsObject> = cx.new_array().into();
    let mut n = 0;
    // 10. Let inputIter be ? GetIterator(iterables, sync).
    let mut input = cx.get_iterator(iterables)?;
    // 11. Let next be not-started.
    // 12. Repeat, while next is not done,
    loop {
        // a. Set next to Completion(IteratorStepValue(inputIter)).
        // b. IfAbruptCloseIterators(next, iters).
        let next = match cx.iterator_step_value(&mut input) {
            Ok(Some(v)) => v,
            Ok(None) => break,
            Err(t) => return close_all(cx, iters, n / 2, None, Err(t)),
        };
        // c. If next is not done, then
        //    i. Let iter be Completion(GetIteratorFlattenable(next, reject-strings)).
        //    ii. IfAbruptCloseIterators(iter, the list-concatenation of « inputIter » and
        //        iters).
        let (iter, iter_next) = match get_iterator_flattenable(cx, next, false) {
            Ok(r) => r,
            Err(t) => {
                let r = close_all(cx, iters, n / 2, None, Err::<(), _>(t));
                return cx.iterator_close(&input, r).map(|()| Local::undefined());
            }
        };
        //    iii. Append iter to iters.
        list_push(cx, iters, &mut n, iter.value())?;
        list_push(cx, iters, &mut n, iter_next)?;
    }
    // 13. Let iterCount be the number of elements in iters.
    let count = n / 2;
    // 14. If mode is "longest", then
    let padding = if mode == mode::LONGEST {
        let padding: Local<'s, JsObject> = cx.new_array().into();
        let mut p = 0;
        if padding_option.is_undefined() {
            // a. If paddingOption is undefined, then
            //    i. Perform the following steps iterCount times: Append undefined to padding.
            for _ in 0..count {
                cx.poll_interrupt()?;
                list_push(cx, padding, &mut p, Local::undefined())?;
            }
        } else {
            // b. Else,
            //    i. Let paddingIter be Completion(GetIterator(paddingOption, sync)).
            //    ii. IfAbruptCloseIterators(paddingIter, iters).
            let mut padding_iter = match cx.get_iterator(padding_option) {
                Ok(r) => r,
                Err(t) => return close_all(cx, iters, count, None, Err(t)),
            };
            //    iii. Let usingIterator be true.
            let mut using = true;
            //    iv. Perform the following steps iterCount times:
            for _ in 0..count {
                // 1. If usingIterator is true, then
                if using {
                    // a. Set next to Completion(IteratorStepValue(paddingIter)).
                    // b. IfAbruptCloseIterators(next, iters).
                    match cx.iterator_step_value(&mut padding_iter) {
                        // c. If next is done, then set usingIterator to false.
                        Ok(None) => using = false,
                        // d. Else, append next to padding.
                        Ok(Some(v)) => list_push(cx, padding, &mut p, v)?,
                        Err(t) => return close_all(cx, iters, count, None, Err(t)),
                    }
                }
                // 2. If usingIterator is false, append undefined to padding (no step: one
                //    budget unit, H23).
                if !using {
                    cx.poll_interrupt()?;
                    list_push(cx, padding, &mut p, Local::undefined())?;
                }
            }
            //    v. If usingIterator is true, then
            //       1. Let completion be Completion(IteratorClose(paddingIter,
            //          NormalCompletion(unused))).
            //       2. IfAbruptCloseIterators(completion, iters).
            if using {
                let completion = cx.iterator_close(&padding_iter, Ok(()));
                if let Err(t) = completion {
                    return close_all(cx, iters, count, None, Err(t));
                }
            }
        }
        padding.value()
    } else {
        Local::undefined()
    };
    // 15. finishResults: CreateArrayFromList(results).
    // 16. Return IteratorZip(iters, mode, padding, finishResults).
    iterator_zip(
        cx,
        kind::ZIP,
        iters,
        count,
        mode,
        padding,
        Local::undefined(),
    )
}

/// Iterator.zipKeyed ( iterables [ , options ] ) (joint iteration).
fn zip_keyed<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If iterables is not an Object, throw a TypeError exception.
    let Some(iterables) = args.get(0).as_object() else {
        let shown = cx.display(args.get(0));
        let msg = format!("{shown} is not an object");
        return Err(cx.throw_type_error(&msg));
    };
    // 2-7. The options.
    let (mode, padding_option) = zip_options(cx, args.get(1))?;
    // 8. Let iters be a new empty List.
    let iters: Local<'s, JsObject> = cx.new_array().into();
    let mut n = 0;
    // 11. Let keys be a new empty List.
    let keys: Local<'s, JsObject> = cx.new_array().into();
    let mut k = 0;
    // 10. Let allKeys be ? iterables.[[OwnPropertyKeys]]().
    let all_keys = cx.own_property_keys(iterables)?;
    let all_len = cx.get(all_keys, "length")?;
    let all_len = all_len.as_number().expect("an array length is a Number") as u64;
    // 12. For each element key of allKeys, do (a key that is skipped calls no JS: one budget
    //     unit per pass, H23)
    for j in 0..all_len {
        cx.poll_interrupt()?;
        let key = cx.get_index(all_keys, j)?;
        let key = cx.to_property_key(key)?;
        // a. Let desc be Completion(iterables.[[GetOwnProperty]](key)).
        // b. IfAbruptCloseIterators(desc, iters).
        let desc = match cx.get_own_property(iterables, key) {
            Ok(d) => d,
            Err(t) => return close_all(cx, iters, n / 2, None, Err(t)),
        };
        // c. If desc is not undefined and desc.[[Enumerable]] is true, then
        if !desc.is_some_and(|d| d.enumerable == Some(true)) {
            continue;
        }
        // i. Let value be Completion(Get(iterables, key)).
        // ii. IfAbruptCloseIterators(value, iters).
        let value = match cx.get(iterables, key) {
            Ok(v) => v,
            Err(t) => return close_all(cx, iters, n / 2, None, Err(t)),
        };
        // iii. If value is not undefined, then
        if value.is_undefined() {
            continue;
        }
        // 1. Let iter be Completion(GetIteratorFlattenable(value, reject-strings)).
        // 2. IfAbruptCloseIterators(iter, iters).
        let (iter, iter_next) = match get_iterator_flattenable(cx, value, false) {
            Ok(r) => r,
            Err(t) => return close_all(cx, iters, n / 2, None, Err(t)),
        };
        // 3. Append key to keys.
        list_push(cx, keys, &mut k, key.value())?;
        // 4. Append iter to iters.
        list_push(cx, iters, &mut n, iter.value())?;
        list_push(cx, iters, &mut n, iter_next)?;
    }
    // 13. Let iterCount be the number of elements in iters.
    let count = n / 2;
    // 14. If mode is "longest", then
    let padding = if mode == mode::LONGEST {
        let padding: Local<'s, JsObject> = cx.new_array().into();
        let mut p = 0;
        for i in 0..count {
            cx.poll_interrupt()?;
            // a. If paddingOption is undefined, then
            //    i. Perform the following steps iterCount times: Append undefined to padding.
            // b. Else,
            //    i. For each element key of keys, do
            //       1. Let value be Completion(Get(paddingOption, key)).
            //       2. IfAbruptCloseIterators(value, iters).
            //       3. Append value to padding.
            let value = match padding_option.as_object() {
                None => Local::undefined(),
                Some(padding_option) => {
                    let key = cx.get_index(keys, i)?;
                    let key = cx.to_property_key(key)?;
                    match cx.get(padding_option, key) {
                        Ok(v) => v,
                        Err(t) => return close_all(cx, iters, count, None, Err(t)),
                    }
                }
            };
            list_push(cx, padding, &mut p, value)?;
        }
        padding.value()
    } else {
        Local::undefined()
    };
    // 15. finishResults: a null-prototype object with keys[i] = results[i].
    // 16. Return IteratorZip(iters, mode, padding, finishResults).
    iterator_zip(
        cx,
        kind::ZIP_KEYED,
        iters,
        count,
        mode,
        padding,
        keys.value(),
    )
}

/// IteratorZip ( iters, mode, padding, finishResults ) steps 1-6: the helper object, with
/// `[[UnderlyingIterators]]` = openIters (a copy of iters: the same pair list here).
fn iterator_zip<'s>(
    cx: &mut Cx<'s>,
    kind: i32,
    iters: Local<'s, JsObject>,
    count: u64,
    mode: i32,
    padding: Local<'s>,
    keys: Local<'s>,
) -> JsResult<Local<'s>> {
    let undefined = Local::undefined();
    let o = new_helper_object(cx, kind, undefined, undefined, iters.value(), 0.0)?;
    cx.set_slot(o, CLASS, slot::MODE, Local::int32(mode))?;
    cx.set_slot(o, CLASS, slot::PADDING, padding)?;
    cx.set_slot(o, CLASS, slot::KEYS, keys)?;
    cx.set_slot(o, CLASS, slot::ITER_COUNT, Local::number(count as f64))?;
    Ok(o.value())
}

/// Removes iterator `i` from the pair list `iters`: a null iterator and no cached `next`
/// method, so neither stays alive with the helper (the spec removes the whole record).
fn remove_iter<'s>(cx: &mut Cx<'s>, iters: Local<'s, JsObject>, i: u64) -> JsResult<()> {
    cx.set_index(iters, 2 * i, Local::null(), true)?;
    cx.set_index(iters, 2 * i + 1, Local::undefined(), true)?;
    Ok(())
}

/// The number of open iterators of the pair list `iters` (`count` pairs). One pass with no
/// JS call: one interrupt budget unit per pair (`Cx::poll_interrupt`, H23).
fn open_count<'s>(cx: &mut Cx<'s>, iters: Local<'s, JsObject>, count: u64) -> JsResult<u64> {
    let mut open = 0;
    for i in 0..count {
        cx.poll_interrupt()?;
        if cx.get_index(iters, 2 * i)?.is_object() {
            open += 1;
        }
    }
    Ok(open)
}

/// IteratorZip step 3 (the closure): one result of the zip (an array, or a null-prototype
/// object for zipKeyed), or the end.
fn resume_zip<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, keyed: bool) -> JsResult<Resumed<'s>> {
    let count = number_slot(cx, o, slot::ITER_COUNT)? as u64;
    // a. If iterCount = 0, return ReturnCompletion(undefined).
    if count == 0 {
        return Ok(Resumed::Return);
    }
    let iters = list_slot(cx, o, slot::ITERS)?;
    let mode = cx.slot(o, CLASS, slot::MODE)?;
    let mode = mode.as_int32().expect("the mode is an int32");
    // b. Repeat,
    //    i. Let results be a new empty List (here the result object itself: no JS sees it
    //       before the Yield).
    let results = if keyed {
        cx.new_object_with_proto(Local::null())
    } else {
        cx.new_array().into()
    };
    // The number of open iterators (longest mode): counted in one pass at the first input
    // that is done in this call, then 1 less for each later one (no scan per done input).
    let mut open: Option<u64> = None;
    //    iii. For each integer i such that 0 ≤ i < iterCount, in ascending order, do
    for i in 0..count {
        // 1. Let iter be iters[i].
        let iter = cx.get_index(iters, 2 * i)?;
        let result = match iter.as_object() {
            // 2. If iter is null, then
            //    a. Assert: mode is "longest".
            //    b. Let result be padding[i].
            None => {
                // No step: one budget unit (H23).
                cx.poll_interrupt()?;
                let padding = list_slot(cx, o, slot::PADDING)?;
                cx.get_index(padding, i)?
            }
            // 3. Else,
            Some(iter) => {
                let next = cx.get_index(iters, 2 * i + 1)?;
                // a. Let result be Completion(IteratorStepValue(iter)).
                match step_value(cx, iter, next) {
                    // b. If result is an abrupt completion, then
                    //    i. Remove iter from openIters.
                    //    ii. Return ? IteratorCloseAll(openIters, result).
                    Err(t) => return close_all(cx, iters, count, Some(i), Err(t)),
                    // c. Set result to ! result.
                    Ok(Some(v)) => v,
                    // d. If result is done, then
                    Ok(None) => {
                        // i. Remove iter from openIters.
                        remove_iter(cx, iters, i)?;
                        match mode {
                            // ii. If mode is "shortest", then
                            //     1. Return ? IteratorCloseAll(openIters,
                            //        ReturnCompletion(undefined)).
                            mode::SHORTEST => {
                                return close_all(cx, iters, count, None, Ok(Resumed::Return));
                            }
                            // iii. Else if mode is "strict", then
                            mode::STRICT => return zip_strict_end(cx, iters, count, i),
                            // iv. Else,
                            //     1. Assert: mode is "longest".
                            //     2. If openIters is empty, return ReturnCompletion(undefined).
                            //     3. Set iters[i] to null.
                            //     4. Set result to padding[i].
                            _ => {
                                let left = match open {
                                    Some(n) => n - 1,
                                    None => open_count(cx, iters, count)?,
                                };
                                open = Some(left);
                                if left == 0 {
                                    return Ok(Resumed::Return);
                                }
                                let padding = list_slot(cx, o, slot::PADDING)?;
                                cx.get_index(padding, i)?
                            }
                        }
                    }
                }
            }
        };
        // 4. Append result to results.
        if keyed {
            // finishResults: ! CreateDataPropertyOrThrow(obj, keys[i], results[i]).
            let keys = list_slot(cx, o, slot::KEYS)?;
            let key = cx.get_index(keys, i)?;
            let key = cx.to_property_key(key)?;
            cx.create_data_property_or_throw(results, key, result)?;
        } else {
            cx.create_data_property_index_or_throw(results, i, result)?;
        }
    }
    //    iv. Set results to finishResults(results).
    //    v. Let completion be Completion(Yield(results)).
    Ok(Resumed::Yield(results.value()))
}

/// IteratorZip step 3.b.iii.3.d.iii (strict mode): iterator `i` is done (and has left the
/// list). Every other one must be done too.
fn zip_strict_end<'s>(
    cx: &mut Cx<'s>,
    iters: Local<'s, JsObject>,
    count: u64,
    i: u64,
) -> JsResult<Resumed<'s>> {
    // 1. If i ≠ 0, then
    //    a. Return ? IteratorCloseAll(openIters, ThrowCompletion(a newly created TypeError
    //       object)).
    if i != 0 {
        let error =
            Err(cx.throw_type_error("Iterators passed to Iterator.zip have different lengths"));
        return close_all(cx, iters, count, None, error);
    }
    // 2. For each integer k such that 1 ≤ k < iterCount, in ascending order, do
    for k in 1..count {
        let iter = cx.get_index(iters, 2 * k)?;
        let iter = iter.as_object().expect("strict mode: iters[k] is open");
        let next = cx.get_index(iters, 2 * k + 1)?;
        // a. Let open be Completion(IteratorStep(iters[k])).
        match step(cx, iter, next) {
            // b. If open is an abrupt completion, then
            //    i. Remove iters[k] from openIters.
            //    ii. Return ? IteratorCloseAll(openIters, open).
            Err(t) => return close_all(cx, iters, count, Some(k), Err(t)),
            // d. If open is done, then
            //    i. Remove iters[k] from openIters.
            Ok(None) => {
                remove_iter(cx, iters, k)?;
            }
            // e. Else,
            //    i. Return ? IteratorCloseAll(openIters, ThrowCompletion(a newly created
            //       TypeError object)).
            Ok(Some(_)) => {
                let error = Err(
                    cx.throw_type_error("Iterators passed to Iterator.zip have different lengths")
                );
                return close_all(cx, iters, count, None, error);
            }
        }
    }
    // 3. Return ReturnCompletion(undefined).
    Ok(Resumed::Return)
}

// ---- chunks and windows -------------------------------------------------------------------------

/// The size check of chunks and windows (step 4): a TypeError if `size` is not an integral
/// Number, a RangeError if it is not in 1 to 2^32 - 1; each after IteratorClose of `o` (the
/// record { O, undefined, false }).
fn chunk_size<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    size: Local<'s>,
    name: &str,
) -> JsResult<f64> {
    let error = match size.as_number() {
        Some(x) if x.is_finite() && x.trunc() == x => {
            if (1.0..=4_294_967_295.0).contains(&x) {
                return Ok(x);
            }
            let shown = cx.display(size);
            cx.throw_range_error(&format!("{name} {shown} is out of range"))
        }
        _ => {
            let shown = cx.display(size);
            cx.throw_type_error(&format!("{name} {shown} is not an integral Number"))
        }
    };
    close(cx, o, Err(error))
}

/// Iterator.prototype.chunks ( chunkSize ) (iterator chunking).
fn chunks<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, "chunks")?;
    // 3. Let iterated be the Iterator Record { [[Iterator]]: O, [[NextMethod]]: undefined,
    //    [[Done]]: false }.
    // 4. The chunkSize check (closes iterated on failure).
    let size = chunk_size(cx, o, args.get(0), "chunkSize")?;
    // 5. Set iterated to ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    // 6-9. The closure, CreateIteratorFromClosure, [[UnderlyingIterators]] = « iterated ».
    new_helper(cx, kind::CHUNKS, o, next, Local::undefined(), size)
}

/// chunks step 6 (the closure): the next chunk of chunkSize values, a last smaller one, or
/// the end.
fn resume_chunks<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Resumed<'s>> {
    let size = count_of(cx, o)? as u64;
    // a. Let buffer be a new empty List (here the chunk array itself: no JS sees it before
    //    the Yield, CreateArrayFromList(buffer)).
    let buffer: Local<'s, JsObject> = cx.new_array().into();
    let mut n = 0;
    // b. Repeat,
    loop {
        // i. Let value be ? IteratorStepValue(iterated).
        match step_value(cx, iterator, next)? {
            // ii. If value is done, then
            //     1. If buffer is not empty, then
            //        a. Perform Completion(Yield(CreateArrayFromList(buffer))).
            //     2. Return ReturnCompletion(undefined).
            None if n > 0 => return Ok(Resumed::Last(buffer.value())),
            None => return Ok(Resumed::Return),
            // iii. Append value to buffer.
            Some(v) => list_push(cx, buffer, &mut n, v)?,
        }
        // iv. If the number of elements in buffer is ℝ(chunkSize), then
        //     1. Let completion be Completion(Yield(CreateArrayFromList(buffer))).
        //     2. IfAbruptCloseIterator(completion, iterated).
        //     3. Set buffer to a new empty List.
        if n == size {
            return Ok(Resumed::Yield(buffer.value()));
        }
    }
}

/// Iterator.prototype.windows ( windowSize [ , undersized ] ) (iterator chunking).
fn windows<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, "windows")?;
    // 3. Let iterated be the Iterator Record { [[Iterator]]: O, [[NextMethod]]: undefined,
    //    [[Done]]: false }.
    // 4. The windowSize check (closes iterated on failure).
    let size = chunk_size(cx, o, args.get(0), "windowSize")?;
    // 5. If undersized is undefined, set undersized to "only-full".
    // 6. If undersized is neither "only-full" nor "allow-partial", then
    //    a. Let error be ThrowCompletion(a newly created TypeError object).
    //    b. Return ? IteratorClose(iterated, error).
    let undersized = args.get(1);
    let partial = if undersized.is_undefined() || is_string_value(cx, undersized, "only-full")? {
        false
    } else if is_string_value(cx, undersized, "allow-partial")? {
        true
    } else {
        let shown = cx.display(undersized);
        let error = Err(cx.throw_type_error(&format!("undersized {shown} is not valid")));
        return close(cx, o, error);
    };
    // 7. Set iterated to ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    // 8-11. The closure, CreateIteratorFromClosure, [[UnderlyingIterators]] = « iterated ».
    let buffer = cx.new_array();
    let h = new_helper_object(cx, kind::WINDOWS, o.value(), next, buffer.value(), size)?;
    cx.set_slot(h, CLASS, slot::START, Local::number(0.0))?;
    cx.set_slot(h, CLASS, slot::LEN, Local::number(0.0))?;
    cx.set_slot(h, CLASS, slot::PARTIAL, Local::boolean(partial))?;
    Ok(h.value())
}

/// A new array of the `len` values of the ring `buffer` (capacity `size`) from `start`:
/// CreateArrayFromList(buffer).
fn ring_array<'s>(
    cx: &mut Cx<'s>,
    buffer: Local<'s, JsObject>,
    size: u64,
    start: u64,
    len: u64,
) -> JsResult<Local<'s>> {
    let out: Local<'s, JsObject> = cx.new_array().into();
    let mut n = 0;
    for j in 0..len {
        cx.poll_interrupt()?;
        let v = cx.get_index(buffer, (start + j) % size)?;
        list_push(cx, out, &mut n, v)?;
    }
    Ok(out.value())
}

/// windows step 8 (the closure): the next window of windowSize values, a last smaller one
/// ("allow-partial"), or the end.
fn resume_windows<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    iterator: Local<'s, JsObject>,
    next: Local<'s>,
) -> JsResult<Resumed<'s>> {
    let size = count_of(cx, o)? as u64;
    let buffer = list_slot(cx, o, slot::BUFFER)?;
    let mut start = number_slot(cx, o, slot::START)? as u64;
    let mut len = number_slot(cx, o, slot::LEN)? as u64;
    // b. Repeat,
    loop {
        // i. Let value be ? IteratorStepValue(iterated).
        let Some(value) = step_value(cx, iterator, next)? else {
            // ii. If value is done, then
            //     1. If undersized is "allow-partial" and buffer is not empty and the number
            //        of elements in buffer < ℝ(windowSize), then
            //        a. Perform Completion(Yield(CreateArrayFromList(buffer))).
            //     2. Return ReturnCompletion(undefined).
            let partial = cx.slot(o, CLASS, slot::PARTIAL)?;
            if partial.as_bool() == Some(true) && len > 0 && len < size {
                return ring_array(cx, buffer, size, start, len).map(Resumed::Last);
            }
            return Ok(Resumed::Return);
        };
        if len == size {
            // iii. If the number of elements in buffer is ℝ(windowSize), then
            //      1. Remove the first element from buffer.
            // iv. Append value to buffer.
            cx.set_index(buffer, start, value, true)?;
            start = (start + 1) % size;
            cx.set_slot(o, CLASS, slot::START, Local::number(start as f64))?;
        } else {
            // iv. Append value to buffer (start is 0 until the ring is full).
            cx.create_data_property_index_or_throw(buffer, len, value)?;
            len += 1;
            cx.set_slot(o, CLASS, slot::LEN, Local::number(len as f64))?;
        }
        // v. If the number of elements in buffer is ℝ(windowSize), then
        //    1. Let completion be Completion(Yield(CreateArrayFromList(buffer))).
        //    2. IfAbruptCloseIterator(completion, iterated).
        if len == size {
            return ring_array(cx, buffer, size, start, len).map(Resumed::Yield);
        }
    }
}

// ---- includes, join, [Symbol.dispose] -----------------------------------------------------------

/// SameValueZero ( x, y ) (§7.2.11).
fn same_value_zero<'s>(cx: &Cx<'s>, x: Local<'s>, y: Local<'s>) -> bool {
    if let (Some(a), Some(b)) = (x.as_number(), y.as_number()) {
        return a == b || (a.is_nan() && b.is_nan());
    }
    cx.same_value(x, y)
}

/// Iterator.prototype.includes ( searchElement [ , skippedElements ] ) (iterator includes).
fn includes<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, "includes")?;
    // 3. Let iterated be the Iterator Record { [[Iterator]]: O, [[NextMethod]]: undefined,
    //    [[Done]]: false }.
    let search = args.get(0);
    let skipped = args.get(1);
    // 4. If skippedElements is undefined, let toSkip be 0.
    // 5. Else, if skippedElements is not +∞𝔽, -∞𝔽 or an integral Number, a TypeError after
    //    IteratorClose(iterated, error); else toSkip is its extended mathematical value.
    // 6. If toSkip < 0, or toSkip is finite and above 2^53 - 1, a RangeError after
    //    IteratorClose(iterated, error).
    let to_skip = if skipped.is_undefined() {
        0.0
    } else {
        match skipped.as_number() {
            Some(x) if x.is_infinite() || x.trunc() == x => {
                if x < 0.0 || (x.is_finite() && x > 9_007_199_254_740_991.0) {
                    let shown = cx.display(skipped);
                    let msg = format!("skippedElements {shown} is out of range");
                    let error = Err(cx.throw_range_error(&msg));
                    return close(cx, o, error);
                }
                x
            }
            _ => {
                let shown = cx.display(skipped);
                let msg = format!("skippedElements {shown} is not an integral Number");
                let error = Err(cx.throw_type_error(&msg));
                return close(cx, o, error);
            }
        }
    };
    // 7. Let skipped be 0.
    let mut skipped = 0.0;
    // 8. Set iterated to ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    // 9. Repeat,
    //    a. Let value be ? IteratorStepValue(iterated).
    //    b. If value is done, return false.
    while let Some(value) = step_value(cx, o, next)? {
        // c. If skipped < toSkip, set skipped to skipped + 1.
        if skipped < to_skip {
            skipped += 1.0;
        } else if same_value_zero(cx, value, search) {
            // d. Else if SameValueZero(value, searchElement) is true, then
            //    i. Return ? IteratorClose(iterated, NormalCompletion(true)).
            return close(cx, o, Ok(Local::boolean(true)));
        }
    }
    Ok(Local::boolean(false))
}

/// Iterator.prototype.join ( separator ) (iterator join).
fn join<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. If O is not an Object, throw a TypeError exception.
    let o = this_object(cx, this, "join")?;
    // 3. Let iterated be the Iterator Record { [[Iterator]]: O, [[NextMethod]]: undefined,
    //    [[Done]]: false }.
    // 4. If separator is undefined, let sep be ",".
    // 5. Else, let sep be Completion(ToString(separator)); IfAbruptCloseIterator(sep,
    //    iterated).
    let separator = args.get(0);
    let sep = if separator.is_undefined() {
        vec![u16::from(b',')]
    } else {
        let s = cx.to_string(separator);
        let s = close_if_abrupt(cx, o, s)?;
        units(cx, s)?
    };
    // 6. Set iterated to ? GetIteratorDirect(O).
    let next = get_iterator_direct(cx, o)?;
    // 7. Let R be the empty String.
    let mut r: Vec<u16> = Vec::new();
    // 8. Let first be true.
    let mut first = true;
    // 9. Repeat,
    //    a. Let value be ? IteratorStepValue(iterated).
    //    b. If value is done, return R.
    while let Some(value) = step_value(cx, o, next)? {
        // c. If first is false, set R to the string-concatenation of R and sep (a string
        //    past the length limit is a RangeError, which closes the iterator).
        if !first {
            let pushed = push_units(cx, &mut r, &sep);
            close_if_abrupt(cx, o, pushed)?;
        }
        // d. Set first to false.
        first = false;
        // e. If value is either undefined or null, let next be the empty String.
        if value.is_undefined_or_null() {
            continue;
        }
        // f. Else, let next be Completion(ToString(value)); IfAbruptCloseIterator(next,
        //    iterated).
        let s = cx.to_string(value);
        let s = close_if_abrupt(cx, o, s)?;
        let s = units(cx, s)?;
        // g. Set R to the string-concatenation of R and next.
        let pushed = push_units(cx, &mut r, &s);
        close_if_abrupt(cx, o, pushed)?;
    }
    Ok(cx.new_string_utf16(&r)?.value())
}

/// %Iterator.prototype% [ %Symbol.dispose% ] ( ) (explicit resource management).
fn dispose<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be the this value.
    // 2. Let return be ? GetMethod(O, "return").
    let method = cx.get_v(this, "return")?;
    // 3. If return is not undefined, then
    //    a. Perform ? Call(return, O, « »).
    if !method.is_undefined_or_null() {
        if !cx.is_callable(method) {
            let shown = cx.display(method);
            let msg = format!("{shown} is not a function");
            return Err(cx.throw_type_error(&msg));
        }
        cx.call(method, this, &[])?;
    }
    // 4. Return NormalCompletion(undefined).
    Ok(Local::undefined())
}
