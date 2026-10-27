//! WeakRef and FinalizationRegistry (§26.1, §26.2): module `weakref`, block 17. Owner: M4-P4
//! WEAKREF. Makes `%WeakRef%`, `%WeakRef.prototype%`, `%FinalizationRegistry%` and
//! `%FinalizationRegistry.prototype%` (`IntrinsicId` 68-71; `ObjectClass::WeakRef` 11 and
//! `FinalizationRegistry` 12).
//!
//! ```text
//! this file: the spec steps that throw (NewTarget, CanBeHeldWeakly, IsCallable, SameValue)
//! ep9_vm abi/weakref.rs: new_weak_ref, weak_ref_deref, new_finalization_registry,
//!   finalization_registry_register, finalization_registry_unregister (the weak cells,
//!   AddToKeptObjects); the job loop's clear_kept_objects and cleanup_finalization_registries
//! ```
//!
//! A target or an unregister token must satisfy CanBeHeldWeakly (§9.13): an object, or a
//! symbol that `Symbol.for` did not make. The VM holds both weakly; a held value is strong.
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `tests/golden/ids-weakref.txt`.

use ep9_vm::abi::ObjectClass;
use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 17;

mod def {
    pub const WEAK_REF: u16 = 0;
    pub const WEAK_REF_DEREF: u16 = 1;
    pub const FINALIZATION_REGISTRY: u16 = 2;
    pub const REGISTER: u16 = 3;
    pub const UNREGISTER: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("WeakRef", 1, weak_ref_ctor), // 0
    BuiltinDef::method("deref", 0, deref),                // 1
    BuiltinDef::constructor("FinalizationRegistry", 1, finalization_registry_ctor), // 2
    BuiltinDef::method("register", 2, register),          // 3
    BuiltinDef::method("unregister", 1, unregister),      // 4
];

const fn object(id: IntrinsicId, proto: IntrinsicId) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Object { proto: Some(proto) },
    }
}

const fn builtin(id: IntrinsicId, d: u16) -> IntrinsicDef {
    IntrinsicDef {
        id,
        value: IntrinsicValue::Builtin(d),
    }
}

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    object(IntrinsicId::WeakRefPrototype, IntrinsicId::ObjectPrototype),
    builtin(IntrinsicId::WeakRef, def::WEAK_REF),
    object(
        IntrinsicId::FinalizationRegistryPrototype,
        IntrinsicId::ObjectPrototype,
    ),
    builtin(
        IntrinsicId::FinalizationRegistry,
        def::FINALIZATION_REGISTRY,
    ),
];

const fn method(name: &'static str, d: u16) -> PropDef {
    PropDef::method(PropKey::Str(name), d)
}

/// `constructor` = `id` ({w, c}).
const fn constructor(id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Atom(Atom::Constructor), id, PropAttrs::METHOD)
}

/// `prototype` = `id` ({}).
const fn prototype(id: IntrinsicId) -> PropDef {
    PropDef::intrinsic(PropKey::Atom(Atom::Prototype), id, PropAttrs::NONE)
}

/// `@@toStringTag` = `tag` ({c}).
const fn to_string_tag(tag: &'static str) -> PropDef {
    PropDef::constant(
        PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
        ConstValue::Str(tag),
        PropAttrs::CONFIGURABLE,
    )
}

/// The properties this module adds, per object (the order of node v24's own keys).
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::WeakRef,
        props: &[prototype(IntrinsicId::WeakRefPrototype)],
    },
    ObjectProps {
        target: IntrinsicId::WeakRefPrototype,
        props: &[
            constructor(IntrinsicId::WeakRef),
            method("deref", def::WEAK_REF_DEREF),
            to_string_tag("WeakRef"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::FinalizationRegistry,
        props: &[prototype(IntrinsicId::FinalizationRegistryPrototype)],
    },
    ObjectProps {
        target: IntrinsicId::FinalizationRegistryPrototype,
        props: &[
            constructor(IntrinsicId::FinalizationRegistry),
            method("register", def::REGISTER),
            method("unregister", def::UNREGISTER),
            to_string_tag("FinalizationRegistry"),
        ],
    },
    ObjectProps {
        target: IntrinsicId::GlobalObject,
        // §19.3 WeakRef ( . . . ), FinalizationRegistry ( . . . ) ({w, c}).
        props: &[
            PropDef::intrinsic(
                PropKey::Str("WeakRef"),
                IntrinsicId::WeakRef,
                PropAttrs::METHOD,
            ),
            PropDef::intrinsic(
                PropKey::Str("FinalizationRegistry"),
                IntrinsicId::FinalizationRegistry,
                PropAttrs::METHOD,
            ),
        ],
    },
];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "weakref",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

/// The TypeError of a constructor called without `new` (§26.1.1.1 and §26.2.1.1 step 1).
fn requires_new<'s>(cx: &mut Cx<'s>, name: &str) -> ep9_vm::Throw {
    let msg = format!("Constructor {name} requires 'new'");
    cx.throw_type_error(&msg)
}

/// RequireInternalSlot(o, …) (§10.1.15) for an object of `class`, before a step that throws
/// for an argument: a TypeError for any other value.
fn require<'s>(cx: &mut Cx<'s>, o: Local<'s>, class: ObjectClass) -> JsResult<()> {
    if cx.object_class(o) == Some(class) {
        return Ok(());
    }
    let msg = format!(
        "{}.prototype method called on an incompatible receiver",
        class.name()
    );
    Err(cx.throw_type_error(&msg))
}

/// WeakRef ( target ) (§26.1.1.1).
fn weak_ref_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let target = args.get(0);
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "WeakRef"));
    };
    // 2. If CanBeHeldWeakly(target) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(target) {
        return Err(cx.throw_type_error("WeakRef: invalid target"));
    }
    // 3. Let weakRef be ? OrdinaryCreateFromConstructor(NewTarget, "%WeakRef.prototype%",
    //    « [[WeakRefTarget]] »). 4. AddToKeptObjects(target). 5. Set
    //    weakRef.[[WeakRefTarget]] to target. 6. Return weakRef.
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::WeakRefPrototype)?;
    Ok(cx.new_weak_ref(proto, target).value())
}

/// WeakRef.prototype.deref ( ) (§26.1.3.2).
fn deref<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. RequireInternalSlot(weakRef, [[WeakRefTarget]]). 3. Return WeakRefDeref(weakRef).
    cx.weak_ref_deref(this)
}

/// FinalizationRegistry ( cleanupCallback ) (§26.2.1.1).
fn finalization_registry_ctor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let cleanup = args.get(0);
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(requires_new(cx, "FinalizationRegistry"));
    };
    // 2. If IsCallable(cleanupCallback) is false, throw a TypeError exception.
    if !cx.is_callable(cleanup) {
        return Err(cx.throw_type_error("FinalizationRegistry: cleanup must be callable"));
    }
    // 3. Let finalizationRegistry be ? OrdinaryCreateFromConstructor(NewTarget,
    //    "%FinalizationRegistry.prototype%", « [[Realm]], [[CleanupCallback]], [[Cells]] »).
    //    4-8. The realm, the callback, an empty [[Cells]]. 9. Return finalizationRegistry.
    let proto =
        cx.get_prototype_from_constructor(nt, IntrinsicId::FinalizationRegistryPrototype)?;
    Ok(cx.new_finalization_registry(proto, cleanup).value())
}

/// FinalizationRegistry.prototype.register ( target, heldValue \[ , unregisterToken \] )
/// (§26.2.3.2).
fn register<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let (target, held, token) = (args.get(0), args.get(1), args.get(2));
    // 1-2. RequireInternalSlot(finalizationRegistry, [[Cells]]).
    require(cx, this, ObjectClass::FinalizationRegistry)?;
    // 3. If CanBeHeldWeakly(target) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(target) {
        return Err(cx.throw_type_error("FinalizationRegistry.prototype.register: invalid target"));
    }
    // 4. If SameValue(target, heldValue) is true, throw a TypeError exception.
    if cx.same_value(target, held) {
        return Err(cx.throw_type_error(
            "FinalizationRegistry.prototype.register: target and holdings must not be same",
        ));
    }
    // 5. If CanBeHeldWeakly(unregisterToken) is false, then a. if unregisterToken is not
    //    undefined, throw a TypeError exception; b. set unregisterToken to empty.
    if !token.is_undefined() && !cx.can_be_held_weakly(token) {
        return Err(cx.throw_type_error(
            "FinalizationRegistry.prototype.register: invalid unregister token",
        ));
    }
    // 6-7. Append the cell to [[Cells]]. 8. Return undefined.
    cx.finalization_registry_register(this, target, held, token)?;
    Ok(Local::undefined())
}

/// FinalizationRegistry.prototype.unregister ( unregisterToken ) (§26.2.3.3).
fn unregister<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let token = args.get(0);
    // 1-2. RequireInternalSlot(finalizationRegistry, [[Cells]]).
    require(cx, this, ObjectClass::FinalizationRegistry)?;
    // 3. If CanBeHeldWeakly(unregisterToken) is false, throw a TypeError exception.
    if !cx.can_be_held_weakly(token) {
        return Err(cx.throw_type_error(
            "FinalizationRegistry.prototype.unregister: invalid unregister token",
        ));
    }
    // 4-6. Remove every cell with this token; return whether one was removed.
    Ok(Local::boolean(
        cx.finalization_registry_unregister(this, token)?,
    ))
}
