//! Explicit resource management (the TC39 proposal; test262 feature
//! `explicit-resource-management`): module `disposable`, block 29 (M7-P3 SPEC-ERM). Makes
//! `%SuppressedError%`, `%SuppressedError.prototype%`, `%DisposableStack%`,
//! `%DisposableStack.prototype%`, `%AsyncDisposableStack%` and
//! `%AsyncDisposableStack.prototype%` (`IntrinsicId` 98-103; `ObjectClass::DisposableStack`
//! 24 and `ObjectClass::AsyncDisposableStack` 25), and adds
//! `%AsyncIteratorPrototype%[@@asyncDispose]`.
//!
//! ```text
//! SuppressedError ( error, suppressed, message )       [[ErrorData]]; length 3
//! DisposableStack ( )                                  new only; length 0
//!   prototype: use adopt defer move dispose (= @@dispose) get disposed, @@toStringTag
//! AsyncDisposableStack ( )                             new only; length 0
//!   prototype: use adopt defer move disposeAsync (= @@asyncDispose) get disposed,
//!   @@toStringTag
//! %AsyncIteratorPrototype% [ @@asyncDispose ] ( )      a promise of `return()`, undefined
//! ```
//!
//! The `using` and `await using` declarations are BytecodeGen (`ep9_frontend` `stmt.rs`,
//! "using") with the runtime ids 48-49 of ABI 6; they need no builtin of this file but the
//! SuppressedError prototype (id 49 reads `%SuppressedError.prototype%`).
//!
//! A stack is a SLOT_OBJECT of its class: slot 0 `[[DisposableState]]` (true = disposed), slot
//! 1 `[[DisposeCapability]]`, the resource list: an internal JS array that only this file
//! sees, 3 entries per resource (module `res`: a kind, then 2 values). All state is in slots and
//! arrays (heap cells), never in Rust memory (plan rule 8). `disposeAsync` runs
//! DisposeResources as a state machine: a record (a `PromiseReaction`-class slot object, null
//! prototype, as `promise.rs` keeps its captures) and two builtin closures that continue it
//! after each Await, with the same ticks as the spec's Await (PerformPromiseThen on
//! PromiseResolve(%Promise%, v)).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-disposable.txt`.

use ep9_vm::abi::{JsObject, ObjectClass, PromiseCapability};
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, ErrorKind, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, PropertyDescriptor,
    WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 29;

mod def {
    pub const SUPPRESSED_ERROR: u16 = 0;
    pub const STACK: u16 = 1;
    pub const STACK_USE: u16 = 2;
    pub const STACK_ADOPT: u16 = 3;
    pub const STACK_DEFER: u16 = 4;
    pub const STACK_MOVE: u16 = 5;
    pub const STACK_DISPOSE: u16 = 6;
    pub const STACK_DISPOSED: u16 = 7;
    pub const ASYNC_STACK: u16 = 8;
    pub const ASYNC_USE: u16 = 9;
    pub const ASYNC_ADOPT: u16 = 10;
    pub const ASYNC_DEFER: u16 = 11;
    pub const ASYNC_MOVE: u16 = 12;
    pub const ASYNC_DISPOSE: u16 = 13;
    pub const ASYNC_DISPOSED: u16 = 14;
    pub const ASYNC_ITERATOR_DISPOSE: u16 = 15;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("SuppressedError", 3, suppressed_error), // 0
    BuiltinDef::constructor("DisposableStack", 0, stack_ctor),       // 1
    BuiltinDef::method("use", 1, stack_use),                         // 2
    BuiltinDef::method("adopt", 2, stack_adopt),                     // 3
    BuiltinDef::method("defer", 1, stack_defer),                     // 4
    BuiltinDef::method("move", 0, stack_move),                       // 5
    BuiltinDef::method("dispose", 0, stack_dispose),                 // 6
    BuiltinDef::getter("disposed", stack_disposed),                  // 7
    BuiltinDef::constructor("AsyncDisposableStack", 0, async_ctor),  // 8
    BuiltinDef::method("use", 1, async_use),                         // 9
    BuiltinDef::method("adopt", 2, async_adopt),                     // 10
    BuiltinDef::method("defer", 1, async_defer),                     // 11
    BuiltinDef::method("move", 0, async_move),                       // 12
    BuiltinDef::method("disposeAsync", 0, async_dispose),            // 13
    BuiltinDef::getter("disposed", async_disposed),                  // 14
    BuiltinDef::method("[Symbol.asyncDispose]", 0, async_iterator_dispose), // 15
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::SuppressedErrorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ErrorPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::SuppressedError,
        value: IntrinsicValue::Builtin(def::SUPPRESSED_ERROR),
    },
    IntrinsicDef {
        id: IntrinsicId::DisposableStackPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::DisposableStack,
        value: IntrinsicValue::Builtin(def::STACK),
    },
    IntrinsicDef {
        id: IntrinsicId::AsyncDisposableStackPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::AsyncDisposableStack,
        value: IntrinsicValue::Builtin(def::ASYNC_STACK),
    },
];

const fn method(name: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), d)
}

/// The properties this module adds, per object (the order of node v26's own keys).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::SuppressedError,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::SuppressedErrorPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::SuppressedErrorPrototype,
        props: &[
            // constructor, name "SuppressedError", message "" ({w, c}).
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::SuppressedError,
                PropAttrs::METHOD,
            ),
            PropDef::constant(
                PropKey::Atom(Atom::Name),
                ConstValue::Str("SuppressedError"),
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
        target: IntrinsicId::DisposableStack,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::DisposableStackPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::DisposableStackPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::DisposableStack,
                PropAttrs::METHOD,
            ),
            method("use", def::STACK_USE),
            method("dispose", def::STACK_DISPOSE),
            method("adopt", def::STACK_ADOPT),
            method("defer", def::STACK_DEFER),
            method("move", def::STACK_MOVE),
            PropDef::accessor(PropKey::Str("disposed"), Some(def::STACK_DISPOSED), None),
            // DisposableStack.prototype [ @@dispose ] is the `dispose` function object.
            PropDef::method(
                PropKey::Symbol(WellKnownSymbol::DISPOSE),
                def::STACK_DISPOSE,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("DisposableStack"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::AsyncDisposableStack,
        props: &[PropDef::intrinsic(
            PropKey::Atom(Atom::Prototype),
            IntrinsicId::AsyncDisposableStackPrototype,
            PropAttrs::NONE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::AsyncDisposableStackPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::AsyncDisposableStack,
                PropAttrs::METHOD,
            ),
            method("use", def::ASYNC_USE),
            method("disposeAsync", def::ASYNC_DISPOSE),
            method("adopt", def::ASYNC_ADOPT),
            method("defer", def::ASYNC_DEFER),
            method("move", def::ASYNC_MOVE),
            PropDef::accessor(PropKey::Str("disposed"), Some(def::ASYNC_DISPOSED), None),
            // AsyncDisposableStack.prototype [ @@asyncDispose ] is `disposeAsync`.
            PropDef::method(
                PropKey::Symbol(WellKnownSymbol::ASYNC_DISPOSE),
                def::ASYNC_DISPOSE,
            ),
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("AsyncDisposableStack"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::AsyncIteratorPrototype,
        props: &[PropDef::method(
            PropKey::Symbol(WellKnownSymbol::ASYNC_DISPOSE),
            def::ASYNC_ITERATOR_DISPOSE,
        )],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        props: &[
            PropDef::intrinsic(
                PropKey::Str("SuppressedError"),
                IntrinsicId::SuppressedError,
                PropAttrs::METHOD,
            ),
            PropDef::intrinsic(
                PropKey::Str("DisposableStack"),
                IntrinsicId::DisposableStack,
                PropAttrs::METHOD,
            ),
            PropDef::intrinsic(
                PropKey::Str("AsyncDisposableStack"),
                IntrinsicId::AsyncDisposableStack,
                PropAttrs::METHOD,
            ),
        ],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "disposable",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the stack objects ------------------------------------------------------------------------

/// The slots of a stack object.
mod slot {
    /// `[[DisposableState]]` / `[[AsyncDisposableState]]`: true when disposed.
    pub const DISPOSED: usize = 0;
    /// `[[DisposeCapability]]`: the resource list (an internal array).
    pub const LIST: usize = 1;
    pub const COUNT: usize = 2;
}

/// The kinds of a resource list entry: `[kind, a, b]`.
mod res {
    /// `use(v)`: call `b` (the dispose method) with `this` = `a` (the value).
    pub const USE: i32 = 0;
    /// `defer(f)`: call `a` with `this` undefined and no argument.
    pub const DEFER: i32 = 1;
    /// `adopt(v, f)`: call `a` with `this` undefined and the argument `b`.
    pub const ADOPT: i32 = 2;
    /// `use(v)` of an AsyncDisposableStack whose value has only @@dispose: call `b` with
    /// `this` = `a`, then Await(undefined), or Await a rejection by its throw (the closure of
    /// GetDisposeMethod step 1.b.ii).
    pub const USE_SYNC: i32 = 3;
    /// `use(undefined)` or `use(null)` of an AsyncDisposableStack: no method; needsAwait.
    pub const AWAIT_ONLY: i32 = 4;
}

/// The class of a sync or an async stack.
const fn class(asynchronous: bool) -> ObjectClass {
    if asynchronous {
        ObjectClass::AsyncDisposableStack
    } else {
        ObjectClass::DisposableStack
    }
}

fn name(asynchronous: bool) -> &'static str {
    if asynchronous {
        "AsyncDisposableStack"
    } else {
        "DisposableStack"
    }
}

/// RequireInternalSlot(this, `[[DisposableState]]`): the stack, or a TypeError.
fn this_stack<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    asynchronous: bool,
    what: &str,
) -> JsResult<Local<'s, JsObject>> {
    match this.as_object() {
        Some(o) if cx.object_class(o) == Some(class(asynchronous)) => Ok(o),
        _ => {
            let msg = format!(
                "{}.prototype.{what} called on an incompatible receiver",
                name(asynchronous)
            );
            Err(cx.throw_type_error(&msg))
        }
    }
}

fn is_disposed<'s>(cx: &mut Cx<'s>, s: Local<'s, JsObject>, asynchronous: bool) -> JsResult<bool> {
    Ok(cx.slot(s, class(asynchronous), slot::DISPOSED)?.as_bool() == Some(true))
}

/// The ReferenceError of a disposed stack (the methods' step 3).
fn disposed_error<'s>(cx: &mut Cx<'s>, asynchronous: bool, what: &str) -> ep9_vm::Throw {
    let msg = format!(
        "{}.prototype.{what}: the stack is disposed",
        name(asynchronous)
    );
    cx.throw_error(ErrorKind::ReferenceError, &msg)
}

/// A new stack of `proto` with a new empty DisposeCapability (state pending).
fn new_stack<'s>(
    cx: &mut Cx<'s>,
    proto: Local<'s, JsObject>,
    asynchronous: bool,
) -> JsResult<Local<'s, JsObject>> {
    let s = cx.new_object_with_slots(proto, class(asynchronous), slot::COUNT);
    let list: Local<'s, JsObject> = cx.new_array().into();
    cx.set_slot(
        s,
        class(asynchronous),
        slot::DISPOSED,
        Local::boolean(false),
    )?;
    cx.set_slot(s, class(asynchronous), slot::LIST, list)?;
    Ok(s)
}

/// The resource list of a stack.
fn list_of<'s>(
    cx: &mut Cx<'s>,
    s: Local<'s, JsObject>,
    asynchronous: bool,
) -> JsResult<Local<'s, JsObject>> {
    Ok(cx
        .slot(s, class(asynchronous), slot::LIST)?
        .as_object()
        .expect("a stack has its list"))
}

/// Appends the resource `[kind, a, b]` (AddDisposableResource step 3).
fn push<'s>(
    cx: &mut Cx<'s>,
    list: Local<'s, JsObject>,
    kind: i32,
    a: Local<'s>,
    b: Local<'s>,
) -> JsResult<()> {
    let n = cx.get(list, "length")?.as_number().unwrap_or(0.0) as u64;
    for (i, v) in [Local::int32(kind), a, b].into_iter().enumerate() {
        let key = cx.to_property_key(Local::number((n + i as u64) as f64))?;
        cx.create_data_property_or_throw(list, key, v)?;
    }
    Ok(())
}

/// The constructor (both kinds): step 1 (no NewTarget: TypeError), then
/// OrdinaryCreateFromConstructor(NewTarget, the prototype) with a new DisposeCapability.
fn construct<'s>(
    cx: &mut Cx<'s>,
    new_target: Local<'s>,
    asynchronous: bool,
) -> JsResult<Local<'s>> {
    let Some(nt) = new_target.as_object() else {
        let msg = format!("Constructor {} requires 'new'", name(asynchronous));
        return Err(cx.throw_type_error(&msg));
    };
    let default = if asynchronous {
        IntrinsicId::AsyncDisposableStackPrototype
    } else {
        IntrinsicId::DisposableStackPrototype
    };
    let proto = cx.get_prototype_from_constructor(nt, default)?;
    Ok(new_stack(cx, proto, asynchronous)?.value())
}

/// GetMethod(v, @@key) (§7.3.10): `None` for undefined or null, else the callable value or
/// a TypeError (node's text, as `ep9_vm` runtime id 48 gives it).
fn get_method<'s>(cx: &mut Cx<'s>, v: Local<'s>, key: &str) -> JsResult<Option<Local<'s>>> {
    let sym = crate::string::well_known(cx, key)?;
    let m = cx.get_v(v, sym)?;
    if m.is_undefined_or_null() {
        return Ok(None);
    }
    if !cx.is_callable(m) {
        return Err(not_a_method(cx, key));
    }
    Ok(Some(m))
}

/// The TypeError of a missing or non-callable dispose method (node's text).
fn not_a_method(cx: &mut Cx<'_>, key: &str) -> ep9_vm::Throw {
    let msg = format!("Symbol(Symbol.{key}) is not a function");
    cx.throw_type_error(&msg)
}

/// `use(value)` (both kinds): AddDisposableResource(capability, value, hint), then value.
fn use_value<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    value: Local<'s>,
    asynchronous: bool,
) -> JsResult<Local<'s>> {
    let s = this_stack(cx, this, asynchronous, "use")?;
    if is_disposed(cx, s, asynchronous)? {
        return Err(disposed_error(cx, asynchronous, "use"));
    }
    let list = list_of(cx, s, asynchronous)?;
    // CreateDisposableResource step 1.a: undefined or null.
    if value.is_undefined_or_null() {
        if asynchronous {
            push(
                cx,
                list,
                res::AWAIT_ONLY,
                Local::undefined(),
                Local::undefined(),
            )?;
        }
        return Ok(value);
    }
    // Step 1.b.i.
    if value.as_object().is_none() {
        return Err(cx.throw_type_error("An object is expected with `using` declarations"));
    }
    // GetDisposeMethod(value, hint).
    let (kind, method) = if asynchronous {
        match get_method(cx, value, "asyncDispose")? {
            Some(m) => (res::USE, Some(m)),
            None => (res::USE_SYNC, get_method(cx, value, "dispose")?),
        }
    } else {
        (res::USE, get_method(cx, value, "dispose")?)
    };
    let Some(method) = method else {
        return Err(not_a_method(cx, "dispose"));
    };
    push(cx, list, kind, value, method)?;
    Ok(value)
}

/// `adopt(value, onDispose)` (both kinds).
fn adopt<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: &Args<'s>,
    asynchronous: bool,
) -> JsResult<Local<'s>> {
    let s = this_stack(cx, this, asynchronous, "adopt")?;
    if is_disposed(cx, s, asynchronous)? {
        return Err(disposed_error(cx, asynchronous, "adopt"));
    }
    let (value, on_dispose) = (args.get(0), args.get(1));
    if !cx.is_callable(on_dispose) {
        let shown = cx.display(on_dispose);
        let msg = format!("{shown} is not a function");
        return Err(cx.throw_type_error(&msg));
    }
    let list = list_of(cx, s, asynchronous)?;
    push(cx, list, res::ADOPT, on_dispose, value)?;
    Ok(value)
}

/// `defer(onDispose)` (both kinds).
fn defer<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    on_dispose: Local<'s>,
    asynchronous: bool,
) -> JsResult<Local<'s>> {
    let s = this_stack(cx, this, asynchronous, "defer")?;
    if is_disposed(cx, s, asynchronous)? {
        return Err(disposed_error(cx, asynchronous, "defer"));
    }
    if !cx.is_callable(on_dispose) {
        let shown = cx.display(on_dispose);
        let msg = format!("{shown} is not a function");
        return Err(cx.throw_type_error(&msg));
    }
    let list = list_of(cx, s, asynchronous)?;
    push(cx, list, res::DEFER, on_dispose, Local::undefined())?;
    Ok(Local::undefined())
}

/// `move()` (both kinds): a new stack of %(Async)DisposableStack.prototype% takes the
/// capability; this stack gets a new empty one and is disposed.
fn move_stack<'s>(cx: &mut Cx<'s>, this: Local<'s>, asynchronous: bool) -> JsResult<Local<'s>> {
    let s = this_stack(cx, this, asynchronous, "move")?;
    if is_disposed(cx, s, asynchronous)? {
        return Err(disposed_error(cx, asynchronous, "move"));
    }
    let proto = cx.intrinsic(if asynchronous {
        IntrinsicId::AsyncDisposableStackPrototype
    } else {
        IntrinsicId::DisposableStackPrototype
    });
    let fresh = new_stack(cx, proto, asynchronous)?;
    let list = list_of(cx, s, asynchronous)?;
    let empty = list_of(cx, fresh, asynchronous)?;
    let c = class(asynchronous);
    cx.set_slot(fresh, c, slot::LIST, list)?;
    cx.set_slot(s, c, slot::LIST, empty)?;
    cx.set_slot(s, c, slot::DISPOSED, Local::boolean(true))?;
    Ok(fresh.value())
}

/// `get disposed` (both kinds).
fn disposed<'s>(cx: &mut Cx<'s>, this: Local<'s>, asynchronous: bool) -> JsResult<Local<'s>> {
    let s = this_stack(cx, this, asynchronous, "disposed")?;
    Ok(Local::boolean(is_disposed(cx, s, asynchronous)?))
}

/// A new SuppressedError of DisposeResources step 3.e.iii.1 (no `message` in the spec; node's
/// text, as `ep9_vm` runtime id 49 gives it).
fn suppressed_of<'s>(
    cx: &mut Cx<'s>,
    error: Local<'s>,
    suppressed: Local<'s>,
) -> JsResult<Local<'s>> {
    let proto = cx.intrinsic(IntrinsicId::SuppressedErrorPrototype);
    let message = cx.new_string("An error was suppressed during disposal")?;
    // `stack` first, then `message`, `error`, `suppressed` (as `suppressed_error`).
    let e = cx.new_error_object(proto, None);
    hidden(cx, e, "message", message.into())?;
    hidden(cx, e, "error", error)?;
    hidden(cx, e, "suppressed", suppressed)?;
    Ok(e.value())
}

/// CreateNonEnumerableDataPropertyOrThrow(o, key, v).
fn hidden<'s>(cx: &mut Cx<'s>, o: Local<'s, JsObject>, key: &str, v: Local<'s>) -> JsResult<()> {
    let desc = PropertyDescriptor::data(v, true, false, true);
    cx.define_property_or_throw(o, key, desc)?;
    Ok(())
}

/// DisposeResources step 3.e.iii: the thrown value `e` joins the completion `error` (`None`:
/// a normal completion).
fn join<'s>(cx: &mut Cx<'s>, error: Option<Local<'s>>, e: Local<'s>) -> JsResult<Local<'s>> {
    match error {
        Some(old) => suppressed_of(cx, e, old),
        None => Ok(e),
    }
}

/// Reads resource `i` of `list`: (kind, a, b).
fn resource<'s>(
    cx: &mut Cx<'s>,
    list: Local<'s, JsObject>,
    i: u64,
) -> JsResult<(i32, Local<'s>, Local<'s>)> {
    let kind = cx.get_index(list, 3 * i)?.as_int32().unwrap_or(-1);
    let a = cx.get_index(list, 3 * i + 1)?;
    let b = cx.get_index(list, 3 * i + 2)?;
    Ok((kind, a, b))
}

/// The call of one resource's dispose operation.
fn call_resource<'s>(
    cx: &mut Cx<'s>,
    kind: i32,
    a: Local<'s>,
    b: Local<'s>,
) -> JsResult<Local<'s>> {
    match kind {
        res::DEFER => cx.call(a, Local::undefined(), &[]),
        res::ADOPT => cx.call(a, Local::undefined(), &[b]),
        _ => cx.call(b, a, &[]),
    }
}

fn count<'s>(cx: &mut Cx<'s>, list: Local<'s, JsObject>) -> JsResult<u64> {
    Ok(cx.get(list, "length")?.as_number().unwrap_or(0.0) as u64 / 3)
}

// ---- SuppressedError --------------------------------------------------------------------------

/// SuppressedError ( error, suppressed, message ).
fn suppressed_error<'s>(
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
    // 2. Let O be ? OrdinaryCreateFromConstructor(newTarget, "%SuppressedError.prototype%",
    //    « [[ErrorData]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::SuppressedErrorPrototype)?;
    // 3. If message is not undefined: msg = ? ToString(message), a non-enumerable `message`.
    let message = args.get(2);
    let msg = if message.is_undefined() {
        None
    } else {
        Some(cx.to_string(message)?)
    };
    // The own `stack` (implementation-defined) comes first, so `message`, `error` and
    // `suppressed` are adjacent in key order (as test262 order-of-args-evaluation.js asks).
    let o = cx.new_error_object(proto, None);
    if let Some(msg) = msg {
        hidden(cx, o, "message", msg.into())?;
    }
    // 4-5. The non-enumerable `error` and `suppressed`.
    hidden(cx, o, "error", args.get(0))?;
    hidden(cx, o, "suppressed", args.get(1))?;
    Ok(o.value())
}

// ---- DisposableStack --------------------------------------------------------------------------

fn stack_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    construct(cx, new_target, false)
}

fn stack_use<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    use_value(cx, this, args.get(0), false)
}

fn stack_adopt<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    adopt(cx, this, &args, false)
}

fn stack_defer<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    defer(cx, this, args.get(0), false)
}

fn stack_move<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    move_stack(cx, this, false)
}

fn stack_disposed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    disposed(cx, this, false)
}

/// DisposableStack.prototype.dispose ( ): DisposeResources with a normal completion, every
/// resource in reverse order, with no Await.
fn stack_dispose<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let s = this_stack(cx, this, false, "dispose")?;
    if is_disposed(cx, s, false)? {
        return Ok(Local::undefined());
    }
    cx.set_slot(s, class(false), slot::DISPOSED, Local::boolean(true))?;
    let list = list_of(cx, s, false)?;
    let mut error: Option<Local<'s>> = None;
    let mut i = count(cx, list)?;
    while i > 0 {
        i -= 1;
        let (kind, a, b) = resource(cx, list, i)?;
        if let Err(t) = call_resource(cx, kind, a, b) {
            let e = cx.catch_exception(t)?;
            error = Some(join(cx, error, e)?);
        }
    }
    // Step 6: a new empty list.
    let empty: Local<'s, JsObject> = cx.new_array().into();
    cx.set_slot(s, class(false), slot::LIST, empty)?;
    match error {
        Some(e) => Err(cx.throw(e)),
        None => Ok(Local::undefined()),
    }
}

// ---- AsyncDisposableStack ---------------------------------------------------------------------

fn async_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    construct(cx, new_target, true)
}

fn async_use<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    use_value(cx, this, args.get(0), true)
}

fn async_adopt<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    adopt(cx, this, &args, true)
}

fn async_defer<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    defer(cx, this, args.get(0), true)
}

fn async_move<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    move_stack(cx, this, true)
}

fn async_disposed<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    disposed(cx, this, true)
}

/// The slots of the record of one `disposeAsync` run.
mod run {
    /// The resource list.
    pub const LIST: usize = 0;
    /// The resources not yet disposed (a Number; the next one is `INDEX - 1`).
    pub const INDEX: usize = 1;
    /// The completion: undefined for a normal one, else `[error]` in ERROR.
    pub const HAS_ERROR: usize = 2;
    pub const ERROR: usize = 3;
    /// needsAwait, hasAwaited (booleans).
    pub const NEEDS_AWAIT: usize = 4;
    pub const HAS_AWAITED: usize = 5;
    /// The promise capability of `disposeAsync`.
    pub const RESOLVE: usize = 6;
    pub const REJECT: usize = 7;
    pub const COUNT: usize = 8;
}

fn rec_get<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, i: usize) -> JsResult<Local<'s>> {
    cx.slot(r, ObjectClass::PromiseReaction, i)
}

fn rec_set<'s>(
    cx: &mut Cx<'s>,
    r: Local<'s, JsObject>,
    i: usize,
    v: impl Into<Local<'s>>,
) -> JsResult<()> {
    cx.set_slot(r, ObjectClass::PromiseReaction, i, v)
}

fn rec_flag<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, i: usize) -> JsResult<bool> {
    Ok(rec_get(cx, r, i)?.as_bool() == Some(true))
}

/// AsyncDisposableStack.prototype.disposeAsync ( ).
fn async_dispose<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 2. Let promiseCapability be ! NewPromiseCapability(%Promise%).
    let promise_ctor = cx.intrinsic(IntrinsicId::Promise);
    let cap = cx.new_promise_capability(promise_ctor)?;
    // 3. RequireInternalSlot: a rejected promise.
    let s = match this_stack(cx, this, true, "disposeAsync") {
        Ok(s) => s,
        Err(t) => {
            let e = cx.catch_exception(t)?;
            cx.call(cap.reject, Local::undefined(), &[e])?;
            return Ok(cap.promise.value());
        }
    };
    // 4. Disposed: resolve with undefined.
    if is_disposed(cx, s, true)? {
        cx.call(cap.resolve, Local::undefined(), &[Local::undefined()])?;
        return Ok(cap.promise.value());
    }
    // 5. Set the state to disposed.
    cx.set_slot(s, class(true), slot::DISPOSED, Local::boolean(true))?;
    let list = list_of(cx, s, true)?;
    let empty: Local<'s, JsObject> = cx.new_array().into();
    cx.set_slot(s, class(true), slot::LIST, empty)?;
    // 6-8. DisposeResources(capability, NormalCompletion(undefined)), then settle.
    let r = cx.new_object_with_slots(Local::null(), ObjectClass::PromiseReaction, run::COUNT);
    let n = count(cx, list)?;
    rec_set(cx, r, run::LIST, list)?;
    rec_set(cx, r, run::INDEX, Local::number(n as f64))?;
    rec_set(cx, r, run::HAS_ERROR, Local::boolean(false))?;
    rec_set(cx, r, run::NEEDS_AWAIT, Local::boolean(false))?;
    rec_set(cx, r, run::HAS_AWAITED, Local::boolean(false))?;
    rec_set(cx, r, run::RESOLVE, cap.resolve)?;
    rec_set(cx, r, run::REJECT, cap.reject)?;
    dispose_steps(cx, r)?;
    Ok(cap.promise.value())
}

/// DisposeResources step 3.e.iii for the run `r`: the thrown value `e` joins the completion.
fn run_error<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, e: Local<'s>) -> JsResult<()> {
    let old = if rec_flag(cx, r, run::HAS_ERROR)? {
        Some(rec_get(cx, r, run::ERROR)?)
    } else {
        None
    };
    let joined = join(cx, old, e)?;
    rec_set(cx, r, run::ERROR, joined)?;
    rec_set(cx, r, run::HAS_ERROR, Local::boolean(true))
}

/// Await(v) for the run `r`: PromiseResolve(%Promise%, v), then the two continuations. An
/// abrupt PromiseResolve is a throw completion of the dispose call (the step continues).
fn run_await<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, v: Local<'s>) -> JsResult<bool> {
    let promise_ctor = cx.intrinsic(IntrinsicId::Promise);
    let p = match cx.promise_resolve(promise_ctor, v) {
        Ok(p) => p,
        Err(t) => {
            let e = cx.catch_exception(t)?;
            run_error(cx, r, e)?;
            return Ok(false);
        }
    };
    let p = p.as_object().expect("PromiseResolve gives an object");
    let on_fulfilled = cx.new_builtin_closure(run_fulfilled, "", 1, r);
    let on_rejected = cx.new_builtin_closure(run_rejected, "", 1, r);
    cx.perform_promise_then(p, on_fulfilled, on_rejected, None);
    Ok(true)
}

/// The run's steps from its INDEX on, up to the next Await or the end (then it settles).
fn dispose_steps<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>) -> JsResult<()> {
    let list = rec_get(cx, r, run::LIST)?
        .as_object()
        .expect("a run has its list");
    loop {
        let i = rec_get(cx, r, run::INDEX)?.as_number().unwrap_or(0.0) as u64;
        if i == 0 {
            // 4. If needsAwait is true and hasAwaited is false, Await(undefined).
            if rec_flag(cx, r, run::NEEDS_AWAIT)? && !rec_flag(cx, r, run::HAS_AWAITED)? {
                rec_set(cx, r, run::NEEDS_AWAIT, Local::boolean(false))?;
                if run_await(cx, r, Local::undefined())? {
                    return Ok(());
                }
            }
            // 7-8. Reject with the error, or resolve with undefined.
            let (f, v) = if rec_flag(cx, r, run::HAS_ERROR)? {
                (rec_get(cx, r, run::REJECT)?, rec_get(cx, r, run::ERROR)?)
            } else {
                (rec_get(cx, r, run::RESOLVE)?, Local::undefined())
            };
            cx.call(f, Local::undefined(), &[v])?;
            return Ok(());
        }
        rec_set(cx, r, run::INDEX, Local::number((i - 1) as f64))?;
        let (kind, a, b) = resource(cx, list, i - 1)?;
        if kind == res::AWAIT_ONLY {
            // 3.f: no method.
            rec_set(cx, r, run::NEEDS_AWAIT, Local::boolean(true))?;
            continue;
        }
        let awaited = match call_resource(cx, kind, a, b) {
            // U3: the sync method's result is not awaited.
            Ok(_) if kind == res::USE_SYNC => Local::undefined(),
            Ok(v) => v,
            Err(t) => {
                let e = cx.catch_exception(t)?;
                if kind != res::USE_SYNC {
                    // 3.e.iii: no Await of a throw completion.
                    run_error(cx, r, e)?;
                    continue;
                }
                // The closure's promise is rejected by `e`: Await it.
                let promise_ctor = cx.intrinsic(IntrinsicId::Promise);
                let cap = cx.new_promise_capability(promise_ctor)?;
                cx.call(cap.reject, Local::undefined(), &[e])?;
                cap.promise.value()
            }
        };
        // 3.e.ii: Await(result); hasAwaited.
        rec_set(cx, r, run::HAS_AWAITED, Local::boolean(true))?;
        if run_await(cx, r, awaited)? {
            return Ok(());
        }
    }
}

/// The run record of a continuation closure.
fn captured<'s>(cx: &Cx<'s>, args: &Args<'s>) -> Local<'s, JsObject> {
    cx.closure_data(args.callee())
        .as_object()
        .expect("a dispose continuation has its record")
}

/// The continuation after an Await that fulfilled.
fn run_fulfilled<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let r = captured(cx, &args);
    dispose_steps(cx, r)?;
    Ok(Local::undefined())
}

/// The continuation after an Await that rejected: a throw completion of that resource.
fn run_rejected<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let r = captured(cx, &args);
    run_error(cx, r, args.get(0))?;
    dispose_steps(cx, r)?;
    Ok(Local::undefined())
}

// ---- %AsyncIteratorPrototype% [ @@asyncDispose ] ----------------------------------------------

/// %AsyncIteratorPrototype% [ %Symbol.asyncDispose% ] ( ).
fn async_iterator_dispose<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 2. Let promiseCapability be ! NewPromiseCapability(%Promise%).
    let promise_ctor = cx.intrinsic(IntrinsicId::Promise);
    let cap = cx.new_promise_capability(promise_ctor)?;
    match iterator_dispose_steps(cx, this, cap) {
        Ok(()) => {}
        // IfAbruptRejectPromise (steps 4, 6.b, 6.d).
        Err(t) => {
            let e = cx.catch_exception(t)?;
            cx.call(cap.reject, Local::undefined(), &[e])?;
        }
    }
    // 7. Return promiseCapability.[[Promise]].
    Ok(cap.promise.value())
}

/// Steps 3-6 of `%AsyncIteratorPrototype% [ @@asyncDispose ]`: an error is an abrupt completion
/// that the caller turns into a rejection.
fn iterator_dispose_steps<'s>(
    cx: &mut Cx<'s>,
    o: Local<'s>,
    cap: PromiseCapability<'s>,
) -> JsResult<()> {
    // 3. Let return be Completion(GetMethod(O, "return")).
    let ret = cx.get_v(o, "return")?;
    // 5. If return is undefined: resolve with undefined.
    if ret.is_undefined_or_null() {
        cx.call(cap.resolve, Local::undefined(), &[Local::undefined()])?;
        return Ok(());
    }
    if !cx.is_callable(ret) {
        let shown = cx.display(ret);
        let msg = format!("{shown} is not a function");
        return Err(cx.throw_type_error(&msg));
    }
    // 6.a. Let result be Completion(Call(return, O, « »)).
    let result = cx.call(ret, o, &[])?;
    // 6.c. Let resultWrapper be Completion(PromiseResolve(%Promise%, result)).
    let promise_ctor = cx.intrinsic(IntrinsicId::Promise);
    let wrapper = cx.promise_resolve(promise_ctor, result)?;
    let wrapper = wrapper.as_object().expect("PromiseResolve gives an object");
    // 6.e-g. onFulfilled returns undefined; PerformPromiseThen(resultWrapper, onFulfilled,
    // undefined, promiseCapability).
    let unwrap = cx.new_builtin_closure(unwrap_undefined, "", 1, Local::undefined());
    cx.perform_promise_then(wrapper, unwrap, Local::undefined(), Some(cap));
    Ok(())
}

/// The `unwrap` closure of step 6.e: returns undefined.
fn unwrap_undefined<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::undefined())
}
