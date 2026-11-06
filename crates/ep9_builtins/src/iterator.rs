//! The Iterator constructor and the iterator helpers (§27.1.3, iterator helpers part 1):
//! module `iterator`, block 28. Owner: M7-P1 SPEC-ITER. Makes `%Iterator%`,
//! `%IteratorHelperPrototype%` and `%WrapForValidIteratorPrototype%` (`IntrinsicId` 95-97;
//! `ObjectClass::IteratorHelper` 23).
//!
//! ```text
//! Iterator ( )                      abstract: new Iterator() and Iterator() throw
//! Iterator.from ( O )               GetIteratorFlattenable; a wrapper unless O inherits from
//!                                   %Iterator.prototype%
//! Iterator.prototype                %Iterator.prototype% (realm init; @@iterator and
//!                                   @@toStringTag from the VM table)
//!   constructor                     accessor (SetterThatIgnoresPrototypeProperties)
//!   map filter take drop flatMap    a helper object (lazy: one step per next())
//!   reduce toArray forEach some every find   eager: walk the iterator now
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
//! slots (a heap cell), never in Rust memory (plan rule 8).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-iterator.txt`.

use ep9_vm::abi::{JsObject, ObjectClass};
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

/// The properties this module adds, per object (string keys in the order of node v26's own
/// keys; node's later-stage methods are left out).
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

/// The slots of a helper object (and of a wrapper: slots 0-2 only).
mod slot {
    /// The kind ([`super::kind`]) in bits 0-3 and the GeneratorState ([`super::state`]) in
    /// bits 4-5, an int32.
    pub const STATE: usize = 0;
    /// `[[UnderlyingIterator]].[[Iterator]]` (a wrapper: `[[Iterated]].[[Iterator]]`).
    pub const ITERATOR: usize = 1;
    /// `[[UnderlyingIterator]].[[NextMethod]]` (a wrapper: `[[Iterated]].[[NextMethod]]`).
    pub const NEXT: usize = 2;
    /// The mapper or the predicate (map, filter, flatMap).
    pub const FUNC: usize = 3;
    /// The counter (map, filter, flatMap) or the remaining count (take, drop; may be +∞), a
    /// Number.
    pub const COUNT: usize = 4;
    /// flatMap: the inner iterator while it is alive, else undefined.
    pub const INNER: usize = 5;
    /// flatMap: the `next` method of the inner iterator.
    pub const INNER_NEXT: usize = 6;
    /// The slot count of a wrapper.
    pub const WRAP_COUNT: usize = 3;
    /// The slot count of map, filter, take and drop.
    pub const HELPER_COUNT: usize = 5;
    /// The slot count of flatMap.
    pub const FLAT_MAP_COUNT: usize = 7;
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
    pub const MASK: i32 = 0xf;
}

/// The GeneratorState of a helper (bits 4-5 of the state slot).
mod state {
    pub const START: i32 = 0 << 4;
    pub const YIELD: i32 = 1 << 4;
    pub const RUNNING: i32 = 2 << 4;
    pub const DONE: i32 = 3 << 4;
    pub const MASK: i32 = 3 << 4;
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
    let n = if kind == kind::FLAT_MAP {
        slot::FLAT_MAP_COUNT
    } else {
        slot::HELPER_COUNT
    };
    let proto = cx.intrinsic(IntrinsicId::IteratorHelperPrototype);
    let o = cx.new_object_with_slots(proto, CLASS, n);
    cx.set_slot(o, CLASS, slot::STATE, Local::int32(kind | state::START))?;
    cx.set_slot(o, CLASS, slot::ITERATOR, iterator)?;
    cx.set_slot(o, CLASS, slot::NEXT, next)?;
    cx.set_slot(o, CLASS, slot::FUNC, func)?;
    cx.set_slot(o, CLASS, slot::COUNT, Local::number(count))?;
    Ok(o.value())
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
    set_state(cx, o, kind | state::RUNNING)?;
    let r = resume(cx, o, kind);
    // The closure stops at a Yield (suspended-yield), or it ends: a return or an abrupt
    // completion (completed).
    let after = if matches!(r, Ok(Some(_))) {
        state::YIELD
    } else {
        state::DONE
    };
    set_state(cx, o, kind | after)?;
    if after == state::DONE {
        release(cx, o, kind)?;
    }
    match r? {
        Some(v) => iter_result(cx, v, false),
        None => iter_result(cx, Local::undefined(), true),
    }
}

/// A helper of `kind` is completed: it drops the values that only its closure used, the
/// callback and (flatMap) the inner iterator with its `next`, as a completed generator keeps
/// no closure values (Codex r1 SPEC-ITER 2). The underlying iterator stays: `return` reads
/// the state only once the helper is completed.
fn release<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, kind: i32) -> JsResult<()> {
    cx.set_slot(o, CLASS, slot::FUNC, Local::undefined())?;
    if kind == kind::FLAT_MAP {
        cx.set_slot(o, CLASS, slot::INNER, Local::undefined())?;
        cx.set_slot(o, CLASS, slot::INNER_NEXT, Local::undefined())?;
    }
    Ok(())
}

/// One run of the closure of a helper of `kind` up to its next Yield: the yielded value, or
/// `None` when the closure returns.
fn resume<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, kind: i32) -> JsResult<Option<Local<'s>>> {
    let iterator = cx.slot(o, CLASS, slot::ITERATOR)?;
    let iterator = iterator.as_object().expect("[[Iterator]] is an object");
    let next = cx.slot(o, CLASS, slot::NEXT)?;
    match kind {
        kind::MAP => resume_map(cx, o, iterator, next),
        kind::FILTER => resume_filter(cx, o, iterator, next),
        kind::TAKE => resume_take(cx, o, iterator, next),
        kind::DROP => resume_drop(cx, o, iterator, next),
        kind::FLAT_MAP => resume_flat_map(cx, o, iterator, next),
        _ => unreachable!("helper kind {kind}"),
    }
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

/// %IteratorHelperPrototype%.return ( ) (§27.1.2.1.2).
fn helper_return<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let O be this value.
    // 2. Perform ? RequireInternalSlot(O, [[UnderlyingIterator]]).
    // 3. Assert: O has a [[GeneratorState]] slot.
    let (o, bits) = this_helper(cx, this, "return")?;
    let kind = bits & kind::MASK;
    let iterator = cx.slot(o, CLASS, slot::ITERATOR)?;
    let iterator = iterator.as_object().expect("[[Iterator]] is an object");
    match bits & state::MASK {
        // 4. If O.[[GeneratorState]] is suspended-start, then
        state::START => {
            // a. Set O.[[GeneratorState]] to completed.
            set_state(cx, o, kind | state::DONE)?;
            release(cx, o, kind)?;
            // c. Perform ? IteratorClose(O.[[UnderlyingIterator]], NormalCompletion(unused)).
            close(cx, iterator, Ok(()))?;
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
            let r = return_at_yield(cx, o, kind, iterator);
            set_state(cx, o, kind | state::DONE)?;
            release(cx, o, kind)?;
            r?;
            iter_result(cx, Local::undefined(), true)
        }
    }
}

/// The return completion at the Yield of a helper closure: IfAbruptCloseIterator(completion,
/// iterated); for flatMap, first IteratorClose(innerIterator, completion) (flatMap step
/// 6.b.viii.4.b).
fn return_at_yield<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    kind: i32,
    iterator: Local<'s, JsObject>,
) -> JsResult<()> {
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
