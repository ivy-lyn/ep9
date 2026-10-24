//! Promise (§27.2) and AggregateError (§20.5.7): module `promise`, block 10. Owner:
//! P4-PROMISE. Makes `%Promise%`, `%Promise.prototype%`, `%AggregateError%` and
//! `%AggregateError.prototype%` (`IntrinsicId` 39-42).
//!
//! ```text
//! this file: the table (constructor, statics, prototype methods, AggregateError)
//!   built on the Promise operations of ep9_vm (`abi/jobs.rs`): new_promise,
//!   create_resolving_functions, new_promise_capability, promise_resolve,
//!   perform_promise_then, new_builtin_closure / closure_data, catch_exception
//! the job queue and its checkpoint: ep9_vm (`Isolate::with_cx_checkpoint` drains it)
//! ```
//!
//! Lists that outlive one call (the values of `Promise.all`, the errors of `Promise.any`)
//! are internal JS arrays that only the element functions write, through CreateDataProperty
//! (no prototype setter runs); the array becomes the result (CreateArrayFromList of the list)
//! once every element function ran, so no write follows it. Shared records (a remaining
//! count, an `[[AlreadyCalled]]` flag, a closure's captures) are internal slot objects of
//! the class `PromiseReaction` with a null prototype: JS never sees them.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-promise.txt`. Not built: `Promise.allKeyed` and `allSettledKeyed`
//! (the await-dictionary proposal).

use ep9_vm::abi::{JsObject, ObjectClass, PromiseCapability};
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, PropertyDescriptor,
    Throw, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 10;

mod def {
    pub const PROMISE: u16 = 0;
    pub const ALL: u16 = 1;
    pub const ALL_SETTLED: u16 = 2;
    pub const ANY: u16 = 3;
    pub const RACE: u16 = 4;
    pub const RESOLVE: u16 = 5;
    pub const REJECT: u16 = 6;
    pub const WITH_RESOLVERS: u16 = 7;
    pub const TRY: u16 = 8;
    pub const SPECIES: u16 = 9;
    pub const THEN: u16 = 10;
    pub const CATCH: u16 = 11;
    pub const FINALLY: u16 = 12;
    pub const AGGREGATE_ERROR: u16 = 13;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Promise", 1, promise_ctor), // 0
    BuiltinDef::method("all", 1, all),                   // 1
    BuiltinDef::method("allSettled", 1, all_settled),    // 2
    BuiltinDef::method("any", 1, any),                   // 3
    BuiltinDef::method("race", 1, race),                 // 4
    BuiltinDef::method("resolve", 1, resolve),           // 5
    BuiltinDef::method("reject", 1, reject),             // 6
    BuiltinDef::method("withResolvers", 0, with_resolvers), // 7
    BuiltinDef::method("try", 1, promise_try),           // 8
    BuiltinDef::getter("[Symbol.species]", species),     // 9
    BuiltinDef::method("then", 2, then),                 // 10
    BuiltinDef::method("catch", 1, catch),               // 11
    BuiltinDef::method("finally", 1, finally),           // 12
    BuiltinDef::constructor("AggregateError", 2, aggregate_error), // 13
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::PromisePrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::Promise,
        value: IntrinsicValue::Builtin(def::PROMISE),
    },
    IntrinsicDef {
        id: IntrinsicId::AggregateErrorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ErrorPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::AggregateError,
        value: IntrinsicValue::Builtin(def::AGGREGATE_ERROR),
    },
];

const fn method(name: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), d)
}

/// The properties this module adds, per object (the order of node v24's own keys).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::Promise,
        props: &[
            // §27.2.4.4 Promise.prototype ({}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::PromisePrototype,
                PropAttrs::NONE,
            ),
            method("all", def::ALL),
            method("allSettled", def::ALL_SETTLED),
            method("any", def::ANY),
            method("race", def::RACE),
            method("resolve", def::RESOLVE),
            method("reject", def::REJECT),
            method("withResolvers", def::WITH_RESOLVERS),
            method("try", def::TRY),
            // §27.2.4.9 get Promise [ %Symbol.species% ] ({c}).
            PropDef::accessor(
                PropKey::Symbol(WellKnownSymbol::SPECIES),
                Some(def::SPECIES),
                None,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::PromisePrototype,
        props: &[
            // §27.2.5.2 Promise.prototype.constructor.
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::Promise,
                PropAttrs::METHOD,
            ),
            method("then", def::THEN),
            method("catch", def::CATCH),
            method("finally", def::FINALLY),
            // §27.2.5.5 Promise.prototype [ %Symbol.toStringTag% ] = "Promise" ({c}).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Promise"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::AggregateError,
        // §20.5.7.2.1 AggregateError.prototype ({}).
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::AggregateErrorPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::AggregateErrorPrototype,
        props: &[
            // §20.5.7.3.1-3: constructor, message "", name "AggregateError" ({w, c}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::AggregateError,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::Name),
                ConstValue::Str("AggregateError"),
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::Message),
                ConstValue::Str(""),
                PropAttrs::METHOD,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 Promise ( . . . ), AggregateError ( . . . ) ({w, c}).
        props: &[
            PropDef::intrinsic(
                PropKey::Str("AggregateError"),
                IntrinsicId::AggregateError,
                PropAttrs::METHOD,
            ),
            PropDef::intrinsic(
                PropKey::Str("Promise"),
                IntrinsicId::Promise,
                PropAttrs::METHOD,
            ),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "promise",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- helpers ----------------------------------------------------------------------------------

/// A new internal record of `n` slots (a `PromiseReaction`-class object, null prototype).
fn record<'s>(cx: &mut Cx<'s>, n: usize) -> Local<'s, JsObject> {
    cx.new_object_with_slots(Local::null(), ObjectClass::PromiseReaction, n)
}

fn rec_get<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, i: usize) -> JsResult<Local<'s>> {
    cx.slot(o, ObjectClass::PromiseReaction, i)
}

fn rec_set<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    i: usize,
    v: impl Into<Local<'s>>,
) -> JsResult<()> {
    cx.set_slot(o, ObjectClass::PromiseReaction, i, v)
}

/// The record a closure captured (its closure data).
fn captured<'s>(cx: &Cx<'s>, args: &Args<'s>) -> Local<'s, JsObject> {
    cx.closure_data(args.callee())
        .as_object()
        .expect("a promise closure has its record")
}

/// The `this` value as an object, or a TypeError that names the method.
fn this_object<'s>(cx: &mut Cx<'s>, this: Local<'s>, what: &str) -> JsResult<Local<'s, JsObject>> {
    match this.as_object() {
        Some(o) => Ok(o),
        None => {
            let msg = format!("{what} called on a non-object");
            Err(cx.throw_type_error(&msg))
        }
    }
}

/// Invoke(V, P, argumentsList) (§7.3.20).
fn invoke<'s>(
    cx: &mut Cx<'s>,
    v: Local<'s>,
    name: &str,
    args: &[Local<'s>],
) -> JsResult<Local<'s>> {
    let f = cx.get_v(v, name)?;
    cx.call(f, v, args)
}

/// CreateDataPropertyOrThrow(list, ToString(index), v) on an internal list array.
fn list_set<'s>(
    cx: &mut Cx<'s>,
    list: Local<'s, JsObject>,
    index: f64,
    v: impl Into<Local<'s>>,
) -> JsResult<()> {
    let key = cx.to_property_key(Local::number(index))?;
    cx.create_data_property_or_throw(list, key, v)
}

/// SpeciesConstructor(O, defaultConstructor) (§7.3.22).
fn species_constructor<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s, JsObject>,
    default: IntrinsicId,
) -> JsResult<Local<'s>> {
    // 1. Let C be ? Get(O, "constructor").
    let c = cx.get(o, "constructor")?;
    // 2. If C is undefined, return defaultConstructor.
    if c.is_undefined() {
        return Ok(cx.intrinsic(default).value());
    }
    // 3. If C is not an Object, throw a TypeError exception.
    let Some(c) = c.as_object() else {
        return Err(cx.throw_type_error("The .constructor property is not an object"));
    };
    // 4. Let S be ? Get(C, %Symbol.species%). (`Symbol.species` is non-writable and
    // non-configurable, so reading it from %Symbol% runs no JS.)
    let symbol = cx.intrinsic(IntrinsicId::Symbol);
    let species = cx.get(symbol, "species")?;
    let Some(species) = species.as_symbol() else {
        return Err(cx.throw_type_error("Symbol.species is not a symbol"));
    };
    let s = cx.get(c, species)?;
    // 5. If S is either undefined or null, return defaultConstructor.
    if s.is_undefined_or_null() {
        return Ok(cx.intrinsic(default).value());
    }
    // 6. If IsConstructor(S) is true, return S.
    if cx.is_constructor(s) {
        return Ok(s);
    }
    // 7. Throw a TypeError exception.
    Err(cx.throw_type_error("object.constructor[Symbol.species] is not a constructor"))
}

/// IfAbruptRejectPromise(value, capability) (§27.2.1.1.1) for the throw completion `t`.
fn reject_with<'s>(cx: &mut Cx<'s>, t: Throw, cap: PromiseCapability<'s>) -> JsResult<Local<'s>> {
    let e = cx.catch_exception(t)?;
    cx.call(cap.reject, Local::undefined(), &[e])?;
    Ok(cap.promise.value())
}

// ---- the constructor ---------------------------------------------------------------------------

/// Promise ( executor ) (§27.2.3.1).
fn promise_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Promise constructor cannot be invoked without 'new'"));
    };
    // 2. If IsCallable(executor) is false, throw a TypeError exception.
    let executor = args.get(0);
    if !executor.is_callable() {
        return Err(cx.throw_type_error("Promise resolver is not a function"));
    }
    // 3-7. OrdinaryCreateFromConstructor(NewTarget, "%Promise.prototype%"), pending.
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::PromisePrototype)?;
    let p = cx.new_promise(proto);
    // 8. Let resolvingFunctions be CreateResolvingFunctions(promise).
    let (res, rej) = cx.create_resolving_functions(p);
    // 9. Let completion be Completion(Call(executor, undefined, « resolve, reject »)).
    if let Err(t) = cx.call(executor, Local::undefined(), &[res.into(), rej.into()]) {
        // 10. If completion is an abrupt completion, then
        //     a. Perform ? Call(resolvingFunctions.[[Reject]], undefined, « completion.[[Value]] »).
        let e = cx.catch_exception(t)?;
        cx.call(rej, Local::undefined(), &[e])?;
    }
    // 11. Return promise.
    Ok(p.value())
}

// ---- the statics -------------------------------------------------------------------------------

/// GetPromiseResolve(promiseConstructor) (§27.2.4.1.1).
fn get_promise_resolve<'s>(cx: &mut Cx<'s>, c: Local<'s>) -> JsResult<Local<'s>> {
    // 1. Let promiseResolve be ? Get(promiseConstructor, "resolve").
    let r = cx.get_v(c, "resolve")?;
    // 2. If IsCallable(promiseResolve) is false, throw a TypeError exception.
    if !r.is_callable() {
        return Err(cx.throw_type_error("Promise resolve is not a function"));
    }
    Ok(r)
}

/// Which combinator `Promise.all`, `allSettled` or `any` runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Combinator {
    All,
    AllSettled,
    Any,
    Race,
}

/// Promise.all / allSettled / any / race (§27.2.4.1, .2, .3, .5): steps 1-9, the shared part.
fn combinator<'s>(
    cx: &mut Cx<'s>,
    c: Local<'s>,
    iterable: Local<'s>,
    kind: Combinator,
) -> JsResult<Local<'s>> {
    // 1-2. Let C be the this value; let promiseCapability be ? NewPromiseCapability(C).
    let cap = cx.new_promise_capability(c)?;
    // 3-4. promiseResolve, with IfAbruptRejectPromise.
    let promise_resolve = match get_promise_resolve(cx, c) {
        Ok(r) => r,
        Err(t) => return reject_with(cx, t, cap),
    };
    // 5-6. GetIterator(iterable, sync), with IfAbruptRejectPromise.
    let mut rec = match cx.get_iterator(iterable) {
        Ok(r) => r,
        Err(t) => return reject_with(cx, t, cap),
    };
    // 7. Let result be Completion(PerformPromiseX(...)).
    let result = match kind {
        Combinator::Race => perform_race(cx, &mut rec, c, cap, promise_resolve),
        _ => perform_list(cx, &mut rec, c, cap, promise_resolve, kind),
    };
    match result {
        Ok(v) => Ok(v),
        // 8. If result is an abrupt completion, then
        Err(t) => {
            // a. If iteratorRecord.[[Done]] is false, set result to
            //    Completion(IteratorClose(iteratorRecord, result)).
            let r = if rec.is_done() {
                Err(t)
            } else {
                cx.iterator_close(&rec, Err::<Local<'s>, _>(t))
            };
            // b. IfAbruptRejectPromise(result, promiseCapability).
            match r {
                Ok(v) => Ok(v),
                Err(t) => reject_with(cx, t, cap),
            }
        }
    }
}

/// The slots of an element function's record (the captures of §27.2.4.1.3, .2.2, .2.3,
/// .3.2).
mod element {
    /// `[[AlreadyCalled]]`: a record with one boolean slot (shared by the two functions of
    /// `allSettled`).
    pub const ALREADY_CALLED: usize = 0;
    pub const INDEX: usize = 1;
    /// `[[Values]]` or `[[Errors]]`: the internal list array.
    pub const LIST: usize = 2;
    /// `[[Capability]]`.`[[Resolve]]` (all, allSettled) or `.[[Reject]]` (any).
    pub const SETTLE: usize = 3;
    /// `[[RemainingElements]]`: a record with one number slot.
    pub const REMAINING: usize = 4;
    pub const COUNT: usize = 5;
}

/// `remainingElementsCount.[[Value]] - 1`, stored; returns the new value.
fn count_down<'s>(cx: &mut Cx<'s>, remaining: Local<'s, JsObject>) -> JsResult<f64> {
    let n = rec_get(cx, remaining, 0)?.as_number().unwrap_or(0.0) - 1.0;
    rec_set(cx, remaining, 0, Local::number(n))?;
    Ok(n)
}

/// A newly created AggregateError object whose `errors` is `list` (PerformPromiseAny step
/// 4.b.ii, Promise.any Reject Element Functions step 10).
fn aggregate_error_of<'s>(cx: &mut Cx<'s>, list: Local<'s, JsObject>) -> JsResult<Local<'s>> {
    let proto = cx.intrinsic(IntrinsicId::AggregateErrorPrototype);
    // The spec leaves the message out ("a newly created AggregateError"); node's text, so the
    // CLI prints what node prints.
    let message = cx.new_string("All promises were rejected")?;
    let error = cx.new_error_object(proto, Some(message));
    let desc = PropertyDescriptor::data(list.value(), true, false, true);
    cx.define_property_or_throw(error, "errors", desc)?;
    Ok(error.value())
}

/// PerformPromiseAll (§27.2.4.1.2), PerformPromiseAllSettled (§27.2.4.2.1) and
/// PerformPromiseAny (§27.2.4.3.1).
fn perform_list<'s>(
    cx: &mut Cx<'s>,
    rec: &mut ep9_vm::abi::IteratorRecord<'s>,
    c: Local<'s>,
    cap: PromiseCapability<'s>,
    promise_resolve: Local<'s>,
    kind: Combinator,
) -> JsResult<Local<'s>> {
    // 1. Let values (errors) be a new empty List.
    let list: Local<'s, JsObject> = cx.new_array().into();
    // 2. Let remainingElementsCount be the Record { [[Value]]: 1 }.
    let remaining = record(cx, 1);
    rec_set(cx, remaining, 0, Local::number(1.0))?;
    // 3. Let index be 0.
    let mut index = 0.0f64;
    // 4. Repeat,
    loop {
        // a. Let next be ? IteratorStepValue(iteratorRecord).
        let Some(next) = cx.iterator_step_value(rec)? else {
            // b. If next is done, then
            //    i-ii. remaining - 1; if 0, settle with the list.
            if count_down(cx, remaining)? == 0.0 {
                if kind == Combinator::Any {
                    let error = aggregate_error_of(cx, list)?;
                    return Err(cx.throw(error));
                }
                cx.call(cap.resolve, Local::undefined(), &[list.value()])?;
            }
            //    iii. Return resultCapability.[[Promise]].
            return Ok(cap.promise.value());
        };
        // c. Append undefined to values (errors).
        list_set(cx, list, index, Local::undefined())?;
        // d. Let nextPromise be ? Call(promiseResolve, constructor, « next »).
        let next_promise = cx.call(promise_resolve, c, &[next])?;
        // e-k. The element function(s) and their captures.
        let already = record(cx, 1);
        rec_set(cx, already, 0, Local::boolean(false))?;
        let data = record(cx, element::COUNT);
        rec_set(cx, data, element::ALREADY_CALLED, already)?;
        rec_set(cx, data, element::INDEX, Local::number(index))?;
        rec_set(cx, data, element::LIST, list)?;
        let settle = if kind == Combinator::Any {
            cap.reject
        } else {
            cap.resolve
        };
        rec_set(cx, data, element::SETTLE, settle)?;
        rec_set(cx, data, element::REMAINING, remaining)?;
        let (on_fulfilled, on_rejected): (Local<'s>, Local<'s>) = match kind {
            Combinator::All => (
                cx.new_builtin_closure(all_resolve_element, "", 1, data)
                    .into(),
                cap.reject,
            ),
            // The two functions share [[AlreadyCalled]] and every other capture: one record.
            Combinator::AllSettled => (
                cx.new_builtin_closure(all_settled_resolve_element, "", 1, data)
                    .into(),
                cx.new_builtin_closure(all_settled_reject_element, "", 1, data)
                    .into(),
            ),
            Combinator::Any | Combinator::Race => (
                cap.resolve,
                cx.new_builtin_closure(any_reject_element, "", 1, data)
                    .into(),
            ),
        };
        // l. Set remainingElementsCount.[[Value]] to remainingElementsCount.[[Value]] + 1.
        let n = rec_get(cx, remaining, 0)?.as_number().unwrap_or(0.0);
        rec_set(cx, remaining, 0, Local::number(n + 1.0))?;
        // m. Perform ? Invoke(nextPromise, "then", « onFulfilled, onRejected »).
        invoke(cx, next_promise, "then", &[on_fulfilled, on_rejected])?;
        // n. Set index to index + 1.
        index += 1.0;
    }
}

/// PerformPromiseRace (§27.2.4.5.1).
fn perform_race<'s>(
    cx: &mut Cx<'s>,
    rec: &mut ep9_vm::abi::IteratorRecord<'s>,
    c: Local<'s>,
    cap: PromiseCapability<'s>,
    promise_resolve: Local<'s>,
) -> JsResult<Local<'s>> {
    loop {
        // a-b. Let next be ? IteratorStepValue(iteratorRecord); done: return the promise.
        let Some(next) = cx.iterator_step_value(rec)? else {
            return Ok(cap.promise.value());
        };
        // c. Let nextPromise be ? Call(promiseResolve, constructor, « next »).
        let next_promise = cx.call(promise_resolve, c, &[next])?;
        // d. Perform ? Invoke(nextPromise, "then", « resolve, reject »).
        invoke(cx, next_promise, "then", &[cap.resolve, cap.reject])?;
    }
}

/// The shared steps of the element functions: `None` if `[[AlreadyCalled]]` was set (step
/// 3), else sets it (step 4) and gives (index, list, settle function, remaining record).
#[allow(clippy::type_complexity)]
fn element_call<'s>(
    cx: &mut Cx<'s>,
    args: &Args<'s>,
) -> JsResult<Option<(f64, Local<'s, JsObject>, Local<'s>, Local<'s, JsObject>)>> {
    let data = captured(cx, args);
    let already = rec_get(cx, data, element::ALREADY_CALLED)?
        .as_object()
        .expect("the [[AlreadyCalled]] record");
    if rec_get(cx, already, 0)?.as_bool() == Some(true) {
        return Ok(None);
    }
    rec_set(cx, already, 0, Local::boolean(true))?;
    let index = rec_get(cx, data, element::INDEX)?
        .as_number()
        .unwrap_or(0.0);
    let list = rec_get(cx, data, element::LIST)?
        .as_object()
        .expect("the list array");
    let settle = rec_get(cx, data, element::SETTLE)?;
    let remaining = rec_get(cx, data, element::REMAINING)?
        .as_object()
        .expect("the remaining record");
    Ok(Some((index, list, settle, remaining)))
}

/// Stores `v` at the element's index; when it was the last element, settles with the list
/// (a resolve) or with an AggregateError of it (`any`, a reject).
fn element_done<'s>(
    cx: &mut Cx<'s>,
    args: &Args<'s>,
    v: Local<'s>,
    any: bool,
) -> JsResult<Local<'s>> {
    let Some((index, list, settle, remaining)) = element_call(cx, args)? else {
        return Ok(Local::undefined());
    };
    list_set(cx, list, index, v)?;
    if count_down(cx, remaining)? == 0.0 {
        let result = if any {
            aggregate_error_of(cx, list)?
        } else {
            list.value()
        };
        return cx.call(settle, Local::undefined(), &[result]);
    }
    Ok(Local::undefined())
}

/// Promise.all Resolve Element Functions (§27.2.4.1.3).
fn all_resolve_element<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    element_done(cx, &args, args.get(0), false)
}

/// A `{ status, value | reason }` object of `Promise.allSettled`.
fn settled_object<'s>(cx: &mut Cx<'s>, fulfilled: bool, x: Local<'s>) -> JsResult<Local<'s>> {
    let obj = cx.new_object();
    let status = cx.new_string(if fulfilled { "fulfilled" } else { "rejected" })?;
    cx.create_data_property_or_throw(obj, "status", status)?;
    let key = if fulfilled { "value" } else { "reason" };
    cx.create_data_property_or_throw(obj, key, x)?;
    Ok(obj.value())
}

/// Promise.allSettled Resolve Element Functions (§27.2.4.2.2).
fn all_settled_resolve_element<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // The object is made after the [[AlreadyCalled]] check (step 5 follows steps 3-4).
    let data = captured(cx, &args);
    let already = rec_get(cx, data, element::ALREADY_CALLED)?
        .as_object()
        .expect("the [[AlreadyCalled]] record");
    if rec_get(cx, already, 0)?.as_bool() == Some(true) {
        return Ok(Local::undefined());
    }
    let obj = settled_object(cx, true, args.get(0))?;
    element_done(cx, &args, obj, false)
}

/// Promise.allSettled Reject Element Functions (§27.2.4.2.3).
fn all_settled_reject_element<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let data = captured(cx, &args);
    let already = rec_get(cx, data, element::ALREADY_CALLED)?
        .as_object()
        .expect("the [[AlreadyCalled]] record");
    if rec_get(cx, already, 0)?.as_bool() == Some(true) {
        return Ok(Local::undefined());
    }
    let obj = settled_object(cx, false, args.get(0))?;
    element_done(cx, &args, obj, false)
}

/// Promise.any Reject Element Functions (§27.2.4.3.2).
fn any_reject_element<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    element_done(cx, &args, args.get(0), true)
}

/// Promise.all ( iterable ) (§27.2.4.1).
fn all<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    combinator(cx, this, args.get(0), Combinator::All)
}

/// Promise.allSettled ( iterable ) (§27.2.4.2).
fn all_settled<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    combinator(cx, this, args.get(0), Combinator::AllSettled)
}

/// Promise.any ( iterable ) (§27.2.4.3).
fn any<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    combinator(cx, this, args.get(0), Combinator::Any)
}

/// Promise.race ( iterable ) (§27.2.4.5).
fn race<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    combinator(cx, this, args.get(0), Combinator::Race)
}

/// Promise.reject ( r ) (§27.2.4.6).
fn reject<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let C be the this value; let promiseCapability be ? NewPromiseCapability(C).
    let cap = cx.new_promise_capability(this)?;
    // 3. Perform ? Call(promiseCapability.[[Reject]], undefined, « r »).
    cx.call(cap.reject, Local::undefined(), &[args.get(0)])?;
    // 4. Return promiseCapability.[[Promise]].
    Ok(cap.promise.value())
}

/// Promise.resolve ( x ) (§27.2.4.7).
fn resolve<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let C be the this value; if C is not an Object, throw a TypeError exception.
    let c = this_object(cx, this, "Promise.resolve")?;
    // 3. Return ? PromiseResolve(C, x).
    cx.promise_resolve(c, args.get(0))
}

/// Promise.try ( callback, ...args ) (§27.2.4.8), as amended by ecma262 PR #3883 (2026-08,
/// "make Promise.try use PromiseResolve in non-error case"): a returned promise of the
/// receiver is not wrapped, and the capability is made only for a throw. node v24 still
/// runs the ES2025 steps (a capability first, then a resolve); test262 tests the new ones
/// (`try/avoids-wrap.js`).
fn promise_try<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let C be the this value; if C is not an Object, throw a TypeError exception.
    let c = this_object(cx, this, "Promise.try")?;
    // 3. Let status be Completion(Call(callback, undefined, args)).
    match cx.call_args(args.get(0), Local::undefined(), args.rest(1)) {
        // 5. Return ? PromiseResolve(C, ! status).
        Ok(v) => cx.promise_resolve(c, v),
        // 4. If status is an abrupt completion, then
        Err(t) => {
            let e = cx.catch_exception(t)?;
            // a. Let promiseCapability be ? NewPromiseCapability(C).
            let cap = cx.new_promise_capability(c)?;
            // b. Perform ? Call(promiseCapability.[[Reject]], undefined, « status.[[Value]] »).
            cx.call(cap.reject, Local::undefined(), &[e])?;
            // c. Return promiseCapability.[[Promise]].
            Ok(cap.promise.value())
        }
    }
}

/// Promise.withResolvers ( ) (§27.2.4.10).
fn with_resolvers<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let C be the this value; let promiseCapability be ? NewPromiseCapability(C).
    let cap = cx.new_promise_capability(this)?;
    // 3-6. An ordinary object with promise, resolve and reject.
    let obj = cx.new_object();
    cx.create_data_property_or_throw(obj, "promise", cap.promise)?;
    cx.create_data_property_or_throw(obj, "resolve", cap.resolve)?;
    cx.create_data_property_or_throw(obj, "reject", cap.reject)?;
    Ok(obj.value())
}

/// get Promise [ %Symbol.species% ] (§27.2.4.9): the this value.
fn species<'s>(
    _cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(this)
}

// ---- the prototype methods ---------------------------------------------------------------------

/// Promise.prototype.then ( onFulfilled, onRejected ) (§27.2.5.4).
fn then<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let promise be the this value; if IsPromise(promise) is false, throw a TypeError.
    let p = match this.as_object() {
        Some(p) if cx.is_promise(p) => p,
        _ => {
            return Err(cx.throw_type_error(
                "Method Promise.prototype.then called on incompatible receiver",
            ));
        }
    };
    // 3. Let C be ? SpeciesConstructor(promise, %Promise%).
    let c = species_constructor(cx, p, IntrinsicId::Promise)?;
    // 4. Let resultCapability be ? NewPromiseCapability(C).
    let cap = cx.new_promise_capability(c)?;
    // 5. Return PerformPromiseThen(promise, onFulfilled, onRejected, resultCapability).
    Ok(cx.perform_promise_then(p, args.get(0), args.get(1), Some(cap)))
}

/// Promise.prototype.catch ( onRejected ) (§27.2.5.1).
fn catch<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Return ? Invoke(promise, "then", « undefined, onRejected »).
    invoke(cx, this, "then", &[Local::undefined(), args.get(0)])
}

/// The captures of thenFinally and catchFinally: onFinally and C.
mod finally_slot {
    pub const ON_FINALLY: usize = 0;
    pub const C: usize = 1;
    pub const COUNT: usize = 2;
}

/// Promise.prototype.finally ( onFinally ) (§27.2.5.3).
fn finally<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let promise be the this value; if it is not an Object, throw a TypeError.
    let p = this_object(cx, this, "Promise.prototype.finally")?;
    // 3. Let C be ? SpeciesConstructor(promise, %Promise%).
    let c = species_constructor(cx, p, IntrinsicId::Promise)?;
    let on_finally = args.get(0);
    // 5. If IsCallable(onFinally) is false, then thenFinally and catchFinally are onFinally.
    let (then_finally, catch_finally): (Local<'s>, Local<'s>) = if !on_finally.is_callable() {
        (on_finally, on_finally)
    } else {
        // 6. Else, the two closures over onFinally and C.
        let data = record(cx, finally_slot::COUNT);
        rec_set(cx, data, finally_slot::ON_FINALLY, on_finally)?;
        rec_set(cx, data, finally_slot::C, c)?;
        (
            cx.new_builtin_closure(then_finally, "", 1, data).into(),
            cx.new_builtin_closure(catch_finally, "", 1, data).into(),
        )
    };
    // 7. Return ? Invoke(promise, "then", « thenFinally, catchFinally »).
    invoke(cx, p.value(), "then", &[then_finally, catch_finally])
}

/// Steps a-c of thenFinallyClosure and catchFinallyClosure: Call(onFinally) and
/// PromiseResolve(C, result).
fn finally_promise<'s>(cx: &mut Cx<'s>, args: &Args<'s>) -> JsResult<Local<'s>> {
    let data = captured(cx, args);
    let on_finally = rec_get(cx, data, finally_slot::ON_FINALLY)?;
    let c = rec_get(cx, data, finally_slot::C)?
        .as_object()
        .expect("C is a constructor");
    // i. Let result be ? Call(onFinally, undefined).
    let result = cx.call(on_finally, Local::undefined(), &[])?;
    // ii. Let p be ? PromiseResolve(C, result).
    cx.promise_resolve(c, result)
}

/// thenFinallyClosure (§27.2.5.3 step 6.a).
fn then_finally<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let p = finally_promise(cx, &args)?;
    // iii-iv. returnValue: a closure that returns value.
    let value_thunk = cx.new_builtin_closure(return_value, "", 0, args.get(0));
    // v. Return ? Invoke(p, "then", « valueThunk »).
    invoke(cx, p, "then", &[value_thunk.into()])
}

/// catchFinallyClosure (§27.2.5.3 step 6.c).
fn catch_finally<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let p = finally_promise(cx, &args)?;
    // iii-iv. throwReason: a closure that throws reason.
    let thrower = cx.new_builtin_closure(throw_reason, "", 0, args.get(0));
    // v. Return ? Invoke(p, "then", « thrower »).
    invoke(cx, p, "then", &[thrower.into()])
}

/// returnValue (§27.2.5.3 step 6.a.iii): the captured value.
fn return_value<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.closure_data(args.callee()))
}

/// throwReason (§27.2.5.3 step 6.c.iii): throws the captured reason.
fn throw_reason<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let reason = cx.closure_data(args.callee());
    Err(cx.throw(reason))
}

// ---- AggregateError ----------------------------------------------------------------------------

/// AggregateError ( errors, message [ , options ] ) (§20.5.7.1.1).
fn aggregate_error<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, let newTarget be the active function object.
    let nt: Local<'s, JsObject> = match new_target.as_object() {
        Some(nt) => nt,
        None => args.callee().into(),
    };
    // 2. Let O be ? OrdinaryCreateFromConstructor(newTarget, "%AggregateError.prototype%",
    //    « [[ErrorData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::AggregateErrorPrototype)?;
    // 3. If message is not undefined: msg = ? ToString(message), a non-enumerable `message`.
    //    (O is made with it: nothing between steps 2 and 3 can observe O.)
    let message = args.get(1);
    let msg = if message.is_undefined() {
        None
    } else {
        Some(cx.to_string(message)?)
    };
    let o = cx.new_error_object(proto, msg);
    // 4. Perform ? InstallErrorCause(O, options).
    if let Some(options) = args.get(2).as_object()
        && cx.has_property(options, "cause")?
    {
        let cause = cx.get(options, "cause")?;
        let desc = PropertyDescriptor::data(cause, true, false, true);
        cx.define_property_or_throw(o, "cause", desc)?;
    }
    // 5. Let errorsList be ? IteratorToList(? GetIterator(errors, sync)).
    let list: Local<'s, JsObject> = cx.new_array().into();
    let mut rec = cx.get_iterator(args.get(0))?;
    let mut i = 0.0f64;
    while let Some(v) = cx.iterator_step_value(&mut rec)? {
        list_set(cx, list, i, v)?;
        i += 1.0;
    }
    // 6. Perform ! DefinePropertyOrThrow(O, "errors", { [[Configurable]]: true,
    //    [[Enumerable]]: false, [[Writable]]: true, [[Value]]: CreateArrayFromList(errorsList) }).
    let desc = PropertyDescriptor::data(list.value(), true, false, true);
    cx.define_property_or_throw(o, "errors", desc)?;
    // 7. Return O.
    Ok(o.value())
}
