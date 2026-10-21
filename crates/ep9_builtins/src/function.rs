//! Function builtins (§20.2): module `function`, block 2. Targets `%Function%` and
//! `%Function.prototype%` (`IntrinsicId::Function`, `IntrinsicId::FunctionPrototype`).
//!
//! `%Function.prototype%` itself and the `Function` constructor (v0: throws) are VM builtins
//! (block 0). Week 4 (P3-BLTA): `Function.prototype.call`, `apply`, `bind`.
//!
//! | Builtin | Template | Pattern |
//! |---|---|---|
//! | `call` | 6 | IsCallable(this); forward `args.rest(1)` with `Cx::call_args` (one copy, into the callee frame) |
//! | `apply` | companion | CreateListFromArrayLike into an `ArgsBuilder` (VM stack), then `Cx::call_built` |
//! | `bind` | 7 | BoundFunctionCreate (`Cx::new_bound_function`: a BOUND_FUNCTION cell; its `[[Call]]` and `[[Construct]]` are in the VM), then SetFunctionLength and SetFunctionName |
//! | `toString` | companion | v0: the NativeFunction form for every callable (no `[[SourceText]]` is kept yet), a builtin's named by its `[[InitialName]]` (`Cx::initial_name`); TypeError otherwise |
//!
//! Owner of the templates: P3-BLTA (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-function.txt`.

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, Cx, IntrinsicDef, IntrinsicId, JsResult, Local,
    ObjectProps, PropDef, PropKey,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 2;

/// Definition indices (ids `BLOCK.i`).
mod def {
    pub const CALL: u16 = 0;
    pub const APPLY: u16 = 1;
    pub const BIND: u16 = 2;
    pub const TO_STRING: u16 = 3;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("call", 1, call),          // 2.0
    BuiltinDef::method("apply", 2, apply),        // 2.1
    BuiltinDef::method("bind", 1, bind),          // 2.2
    BuiltinDef::method("toString", 0, to_string), // 2.3
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[ObjectProps {
    target: IntrinsicId::FunctionPrototype,
    props: &[
        PropDef::method(PropKey::Str("apply"), def::APPLY),
        PropDef::method(PropKey::Str("bind"), def::BIND),
        PropDef::method(PropKey::Str("call"), def::CALL),
        PropDef::method(PropKey::Atom(Atom::ToStringKey), def::TO_STRING),
    ],
}];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "function",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The TypeError of a `Function.prototype` method called on a value that is not callable.
fn not_callable<'s>(cx: &mut Cx<'s>, method: &str, this: Local<'s>) -> ep9_vm::Throw {
    let what = cx.type_of(this);
    let msg =
        format!("Function.prototype.{method} called on a value that is not a function ({what})");
    cx.throw_type_error(&msg)
}

/// Template 6. Function.prototype.call ( thisArg, ...args ) (§20.2.3.3).
fn call<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let func be the this value.
    let func = this;
    // 2. If IsCallable(func) is false, throw a TypeError exception.
    if !func.is_callable() {
        return Err(not_callable(cx, "call", func));
    }
    // 3. Perform PrepareForTailCall() (v0: no tail call; H20-Q3).
    // 4. Return ? Call(func, thisArg, args). The arguments after thisArg go to the callee
    //    frame in one copy (`call_args`), never through a heap list.
    cx.call_args(func, args.get(0), args.rest(1))
}

/// Function.prototype.apply ( thisArg, argArray ) (§20.2.3.1).
fn apply<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let func be the this value.
    let func = this;
    // 2. If IsCallable(func) is false, throw a TypeError exception.
    if !func.is_callable() {
        return Err(not_callable(cx, "apply", func));
    }
    let (this_arg, arg_array) = (args.get(0), args.get(1));
    // 3. If argArray is either undefined or null, then
    if arg_array.is_undefined_or_null() {
        // a. Perform PrepareForTailCall().
        // b. Return ? Call(func, thisArg).
        return cx.call(func, this_arg, &[]);
    }
    // 4. Let argList be ? CreateListFromArrayLike(argArray). It is built on the VM stack
    //    (an ArgsBuilder: precise roots, no heap list).
    let arg_list = cx.create_list_from_array_like(arg_array)?;
    // 5. Perform PrepareForTailCall().
    // 6. Return ? Call(func, thisArg, argList).
    cx.call_built(func, this_arg, arg_list)
}

/// Template 7. Function.prototype.bind ( thisArg, ...args ) (§20.2.3.2).
fn bind<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let Target be the this value.
    // 2. If IsCallable(Target) is false, throw a TypeError exception.
    let Some(target) = this.as_function() else {
        return Err(cx.throw_type_error("Bind must be called on a function"));
    };
    // 3. Let F be ? BoundFunctionCreate(Target, thisArg, args). `args` is a tail of this
    //    call's arguments; the VM copies it into F's bound-argument list.
    let bound_args = args.rest(1);
    let f = cx.new_bound_function(target, args.get(0), bound_args)?;
    // 4. Let L be 0.
    let mut l = 0.0;
    // 5. Let targetHasLength be ? HasOwnProperty(Target, "length").
    // 6. If targetHasLength is true, then
    if cx.has_own_property(target, "length")? {
        // a. Let targetLen be ? Get(Target, "length").
        let target_len = cx.get(target, "length")?;
        // b. If targetLen is a Number, then
        if let Some(n) = target_len.as_number() {
            if n == f64::INFINITY {
                // i. If targetLen is +∞𝔽, set L to +∞.
                l = f64::INFINITY;
            } else if n == f64::NEG_INFINITY {
                // ii. Else if targetLen is -∞𝔽, set L to 0.
                l = 0.0;
            } else {
                // iii. Else,
                // 1. Let targetLenAsInt be ! ToIntegerOrInfinity(targetLen).
                let target_len_as_int = cx.to_integer_or_infinity(target_len)?;
                // 2. Assert: targetLenAsInt is finite.
                debug_assert!(target_len_as_int.is_finite());
                // 3. Let argCount be the number of elements in args.
                let arg_count = bound_args.len() as f64;
                // 4. Set L to max(targetLenAsInt - argCount, 0) (+0, never -0).
                let d = target_len_as_int - arg_count;
                l = if d > 0.0 { d } else { 0.0 };
            }
        }
    }
    // 7. Perform SetFunctionLength(F, L).
    cx.set_function_length(f, l)?;
    // 8. Let targetName be ? Get(Target, "name").
    let mut target_name = cx.get(target, "name")?;
    // 9. If targetName is not a String, set targetName to the empty String.
    if !target_name.is_string() {
        target_name = cx.key("").value();
    }
    // 10. Perform SetFunctionName(F, targetName, "bound").
    cx.set_function_name(f, target_name, Some("bound"))?;
    // 11. Return F.
    Ok(f.value())
}

/// Is `s` an ASCII IdentifierName (`[A-Za-z$_][A-Za-z0-9$_]*`)? Other names are left out of
/// the NativeFunction text rather than checked against the Unicode ID tables.
fn is_ascii_identifier_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '$' || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '$' || c == '_')
}

/// Function.prototype.toString ( ) (§20.2.3.5).
fn to_string<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let func be the this value.
    let func = this;
    // 2. If func is an Object, func has a [[SourceText]] internal slot, func.[[SourceText]]
    //    is a sequence of Unicode code points, and HostHasSourceTextAvailable(func) is true,
    //    then return CodePointsToString(func.[[SourceText]]).
    // v0: no function keeps its [[SourceText]] (HostHasSourceTextAvailable is false), so
    // steps 3-4 give every callable the NativeFunction form.
    // 5. Throw a TypeError exception (moved up: a non-callable reaches no other step).
    let Some(func) = func.as_function() else {
        let what = cx.type_of(func);
        let msg =
            format!("Function.prototype.toString requires that 'this' be a Function ({what})");
        return Err(cx.throw_type_error(&msg));
    };
    // 3. If func is a built-in function object, return an implementation-defined String
    //    source code representation of func. The representation must have the syntax of a
    //    NativeFunction. Additionally, if func has an [[InitialName]] internal slot and
    //    func.[[InitialName]] is a String, the portion of the returned String that would be
    //    matched by NativeFunctionAccessor_opt PropertyName must be the value of
    //    func.[[InitialName]].
    //    (Not the `name` property: it may have been redefined or deleted.)
    let name = if let Some(initial_name) = cx.initial_name(func) {
        cx.to_rust_string(initial_name)?
    } else {
        // 4. If func is an Object and IsCallable(func) is true, return an
        //    implementation-defined String source code representation of func. The
        //    representation must have the syntax of a NativeFunction.
        // v0 name: the own data property `name` (no getter runs) when it is a string of the
        // NativeFunction syntax: an IdentifierName, or "get " / "set " and one. Else none.
        let mut name = String::new();
        if let Some(d) = cx.get_own_property(func, "name")?
            && let Some(v) = d.value
            && v.is_string()
        {
            let text = cx.to_rust_string(v)?;
            let bare = text
                .strip_prefix("get ")
                .or_else(|| text.strip_prefix("set "))
                .unwrap_or(&text);
            if is_ascii_identifier_name(bare) {
                name = text;
            }
        }
        name
    };
    // As V8 prints it: `function f() { [native code] }`, `function () { [native code] }`.
    let text = format!("function {name}() {{ [native code] }}");
    Ok(cx.new_string(&text)?.value())
}
