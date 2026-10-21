//! `CallRuntime(id, args)`.
//!
//! | Id | Function | Spec |
//! |---|---|---|
//! | 6 | `InitializeGlobal` | CreateGlobalFunctionBinding §9.1.1.4.18 (functions); lexical bindings |
//! | 7 | `ResolvePossiblyDirectEval` | §13.3.6.1 step 6 (`slow/eval.rs`) |
//! | 8 | `DeleteLookupSlot` | §13.5.1.2 for a dynamic name (`slow/lookup.rs`) |
//! | 9, 10 | `CopyDataProperties`, `CopyDataPropertiesExcluding` | §7.3.25 (getters run) |
//! | 11 | `SetPrototypeInLiteral` | §13.2.5.5 `__proto__: v` |
//! | 12, 16 | `DefineAccessorInLiteral`, `DefineClassAccessor` | §15.4.5 MethodDefinitionEvaluation (get, set) |
//! | 15 | `DefineClassMethod` | DefineMethodProperty §10.2.8 |
//! | 17, 18 | `LoadKeyedFromSuper`, `StoreToSuper` | §13.3.7 super references, receiver = `this` |
//! | 19-22 | `CreateGeneratorObject`, `GeneratorGetResumeMode`, `GeneratorClose`, `ThrowThrowMethodMissing` | §27.5 generators (`slow/generator.rs`) |
//! | 23 | `CreatePrivateName` | Private Names (`slow/private.rs`) |
//! | 24, 25 | `SetClassFieldsInitializer`, `RunClassFieldsInitializer` | §15.7.14 fields, InitializeInstanceElements (`slow/class_fields.rs`) |
//! | 26 | `LoadLookupSlotForCall` | §9.1.1.2.10 WithBaseObject; a pair, only through `CallRuntimeForPair` ([`call_runtime_pair`]; `slow/lookup.rs`) |
//! | 27, 28 | `DeclareEvalVar`, `DeclareEvalFunction` | §19.2.1.3 EvalDeclarationInstantiation (`slow/eval.rs`) |
//! | 29-31 | `ResolveLookupSlot`, `LoadLookupSlotResolved`, `StoreLookupSlotResolved` | §9.4.2 ResolveBinding, then GetValue / PutValue of that reference (ABI 4, `slow/lookup.rs`) |
//!
//! Ids 19-28 (ABI 3) are stubs in M2-P0 that throw `not implemented in ep9 v0: <name>`, as
//! the catch-all arm did before they were routed; their owners fill them.

use ep9_bytecode::RuntimeId;

use crate::abi::{self, JsObject, JsValue, Local};
use crate::conv::{self, Res, describe};
use crate::descriptor::Desc;
use crate::error::{not_implemented, throw_error};
use crate::frame::{HOLE, NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::key_text;
use crate::object::{self, attr, is_array, is_function, is_object, is_string, own_define, own_get};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root};

use super::property::{super_get_with_receiver, super_set};
use super::{enter, info, ret, strict};

// ---- CallRuntime ---------------------------------------------------------------------------

/// `CallRuntime(id, args)`. `first` is the slot offset of the first argument register.
#[cold]
#[inline(never)]
pub(crate) fn call_runtime(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    id: u16,
    first: isize,
    argc: u32,
) -> u64 {
    enter(cx, fp, pc);
    let a = |i: u32| -> u64 {
        if i < argc {
            // SAFETY: the verifier checked that the register list lies in the frame `fp`.
            unsafe { *fp.offset(first + i as isize) }
        } else {
            UNDEFINED
        }
    };
    use RuntimeId as Id;
    let r: Res<u64> = match RuntimeId::from_u16(id) {
        Some(Id::ThrowConstAssignError) => Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Assignment to constant variable.",
        )),
        Some(Id::ThrowIteratorResultNotAnObject) => {
            let msg = format!("Iterator result {} is not an object", describe(a(0)));
            Err(throw_error(cx, ErrorKind::TypeError, &msg))
        }
        Some(Id::ThrowSymbolIteratorInvalid) => Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Result of the Symbol.iterator method is not an object",
        )),
        Some(Id::CreateIterResultObject) => {
            let rt = cx.rt();
            let o = object::new_plain_object(rt);
            own_define(rt, o, rt.atom(Atom::ValueKey), a(0), attr::DEFAULT);
            let done = conv::boolean(conv::to_boolean(a(1)));
            own_define(rt, o, rt.atom(Atom::Done), done, attr::DEFAULT);
            Ok(o)
        }
        Some(Id::DefineClass) => define_class(cx, a(0), a(1)),
        Some(Id::DeclareGlobals) => {
            let fi = info(fp);
            match fi.loaded_unit() {
                Some(u) => {
                    crate::realm::declare_globals(cx, &u.unit, &|i| u.string(i)).map(|_| UNDEFINED)
                }
                None => Ok(UNDEFINED),
            }
        }
        Some(Id::InitializeGlobal) => initialize_global(cx, a(0), a(1)),
        // slow/eval.rs (P3-EVAL-VM); P0: %eval% is not installed, so it gives the callee.
        Some(Id::ResolvePossiblyDirectEval) => {
            super::eval::resolve_possibly_direct_eval(cx, fp, a(0), a(1), a(2), a(3), a(4))
        }
        // slow/lookup.rs (P2-LOOKUP).
        Some(Id::DeleteLookupSlot) => super::lookup::delete_lookup_slot(cx, fp, a(0)),
        Some(Id::CopyDataProperties) => {
            copy_data_properties(cx, a(0), a(1), UNDEFINED).map(|_| a(0))
        }
        Some(Id::CopyDataPropertiesExcluding) => (|| -> Res<u64> {
            let ex = key_array(cx, fp, first + 1, argc.saturating_sub(1))?;
            let o = object::new_plain_object(cx.rt());
            copy_data_properties(cx, o, a(0), ex)?;
            Ok(o)
        })(),
        // §13.2.5.5 PropertyDefinitionEvaluation of `__proto__: v`: if `v` is an object or
        // null, `! object.[[SetPrototypeOf]](v)`. The literal is new, extensible and in no
        // chain, so it cannot fail.
        Some(Id::SetPrototypeInLiteral) => {
            let (o, p) = (a(0), a(1));
            if is_object(o) && (is_object(p) || p == NULL) {
                let done = property::set_prototype_of(cx.rt(), o, p);
                debug_assert!(done, "SetPrototypeOf of a new literal");
            }
            Ok(UNDEFINED)
        }
        Some(Id::DefineAccessorInLiteral) => define_accessor(cx, a(0), a(1), a(2), a(3), false),
        Some(Id::DefineClassMethod) => define_class_method(cx, a(0), a(1), a(2)),
        Some(Id::DefineClassAccessor) => define_accessor(cx, a(0), a(1), a(2), a(3), true),
        // MakeMethod(F, homeObject) (§10.2.7) for object literal methods that use `super`.
        Some(Id::SetHomeObject) => {
            let (f, home) = (a(0), a(1));
            if is_function(f) {
                object::set_home_object(cx.rt(), f, home);
            }
            Ok(UNDEFINED)
        }
        Some(Id::GetHomeObject) => {
            let f = a(0);
            Ok(if is_function(f) {
                object::home_object(f)
            } else {
                UNDEFINED
            })
        }
        // `super[key]` (§13.3.7.1 MakeSuperPropertyReference + GetValue): `[[Get]]` on the
        // home object's prototype with receiver = `this` (a getter sees `this`).
        Some(Id::LoadKeyedFromSuper) => {
            let (receiver, home, k) = (a(0), a(1), a(2));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                super_get_with_receiver(cx, home, key, receiver)
            })()
        }
        // `super[key] = value` (§13.3.7.1, then PutValue): `[[Set]]` on the home object's
        // prototype with receiver = `this` (§10.1.9.2: a setter on the chain runs with
        // `this`; a data property lands on the receiver). A failed store throws in strict
        // code. Returns the value.
        Some(Id::StoreToSuper) => {
            let (receiver, home, k, v) = (a(0), a(1), a(2), a(3));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                super_set(cx, home, key, v, receiver, strict(fp))?;
                Ok(v)
            })()
        }
        // ---- ABI 3 ----
        // slow/generator.rs (P1-GEN-VM).
        Some(Id::CreateGeneratorObject) => {
            super::generator::create_generator_object(cx, fp, a(0), a(1))
        }
        Some(Id::GeneratorGetResumeMode) => super::generator::generator_get_resume_mode(cx, a(0)),
        Some(Id::GeneratorClose) => super::generator::generator_close(cx, a(0)),
        Some(Id::ThrowThrowMethodMissing) => super::generator::throw_throw_method_missing(cx),
        // slow/private.rs and slow/class_fields.rs (P2-CLS-VM).
        Some(Id::CreatePrivateName) => super::private::create_private_name(cx, a(0)),
        Some(Id::SetClassFieldsInitializer) => {
            super::class_fields::set_class_fields_initializer(cx, a(0), a(1))
        }
        Some(Id::RunClassFieldsInitializer) => {
            super::class_fields::run_class_fields_initializer(cx, a(0), a(1))
        }
        // A pair function: the verifier lets only `CallRuntimeForPair` call it
        // (`VerifyErrorKind::RuntimePair`), so this arm never runs for verified code.
        Some(Id::LoadLookupSlotForCall) => Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: CallRuntime of a pair runtime function",
        )),
        // slow/eval.rs (P3-EVAL-VM).
        Some(Id::DeclareEvalVar) => super::eval::declare_eval_var(cx, fp, a(0)),
        Some(Id::DeclareEvalFunction) => super::eval::declare_eval_function(cx, fp, a(0), a(1)),
        // ---- ABI 4 ----
        // slow/lookup.rs: a dynamic reference resolved before the right-hand side.
        Some(Id::ResolveLookupSlot) => super::lookup::resolve_lookup_slot(cx, fp, a(0)),
        Some(Id::LoadLookupSlotResolved) => {
            super::lookup::load_lookup_slot_resolved(cx, fp, a(0), a(1))
        }
        Some(Id::StoreLookupSlotResolved) => {
            super::lookup::store_lookup_slot_resolved(cx, a(0), a(1), a(2), a(3))
        }
        // An id that a later ABI appends and the VM does not route yet. At ABI 4 every id is
        // routed above, so the arm is unreachable until the next ABI adds ids.
        #[allow(unreachable_patterns)]
        Some(other) => Err(not_implemented(cx, other.info().name)),
        None => Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: unknown runtime function",
        )),
    };
    ret(r)
}

/// `CallRuntimeForPair(id, args, out)` (ABI 3): calls the pair runtime function `id` and
/// writes its two results to the registers `out` and `out + 1` (slot offsets from `fp`).
/// Returns `acc` (the opcode leaves the accumulator as it was), or 0 if an exception is
/// pending; then neither register is written.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn call_runtime_pair(
    cx: &mut Cx,
    fp: *mut u64,
    pc: *const u8,
    id: u16,
    first: isize,
    argc: u32,
    out: isize,
    acc: u64,
) -> u64 {
    enter(cx, fp, pc);
    let a = |i: u32| -> u64 {
        if i < argc {
            // SAFETY: the verifier checked that the register list lies in the frame `fp`.
            unsafe { *fp.offset(first + i as isize) }
        } else {
            UNDEFINED
        }
    };
    use RuntimeId as Id;
    let r: Res<(u64, u64)> = match RuntimeId::from_u16(id) {
        // slow/lookup.rs (P2-LOOKUP).
        Some(Id::LoadLookupSlotForCall) => super::lookup::load_lookup_slot_for_call(cx, fp, a(0)),
        // The verifier lets `CallRuntimeForPair` call only pair functions, so these arms never
        // run for verified code.
        Some(_) => Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: CallRuntimeForPair of a runtime function that is not a pair",
        )),
        None => Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: unknown runtime function",
        )),
    };
    match r {
        Ok((v0, v1)) => {
            // SAFETY: the verifier checked that `out` and `out + 1` are locals of the frame
            // `fp` (`RegOutPair`); the call above may have run JS, but `fp` stays live.
            unsafe {
                *fp.offset(out) = v0;
                *fp.offset(out + 1) = v1;
            }
            debug_assert!(acc != 0);
            acc
        }
        Err(_) => 0,
    }
}

/// `DefineClass(constructor, heritage)` (§15.7.14 ClassDefinitionEvaluation steps 5-8, 14-16).
/// Returns the prototype.
fn define_class(cx: &mut Cx, ctor: u64, heritage: u64) -> Res<u64> {
    let rt = cx.rt();
    if !is_function(ctor) {
        return Err(not_implemented(
            cx,
            "DefineClass without a constructor function",
        ));
    }
    let (proto_parent, ctor_parent) = if heritage == HOLE {
        (
            rt.root(Root::ObjectPrototype),
            rt.root(Root::FunctionPrototype),
        )
    } else if heritage == NULL {
        (NULL, rt.root(Root::FunctionPrototype))
    } else if !object::is_constructor(heritage) {
        let msg = format!(
            "Class extends value {} is not a constructor or null",
            describe(heritage)
        );
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    } else {
        let p = property::get(cx, heritage, rt.atom(Atom::Prototype))?;
        if !is_object(p) && p != NULL {
            let msg = format!(
                "Class extends value does not have valid prototype property {}",
                describe(p)
            );
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        (p, heritage)
    };
    let proto = object::new_object(rt, proto_parent);
    own_define(rt, proto, rt.atom(Atom::Constructor), ctor, attr::HIDDEN);
    own_define(rt, ctor, rt.atom(Atom::Prototype), proto, 0);
    // The constructor is a method of the prototype: `super.x` inside it reads from the parent
    // prototype (§15.7.14 step 14, MakeMethod(F, proto)).
    object::set_home_object(rt, ctor, proto);
    if !object::set_proto(rt, ctor, ctor_parent) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "Cyclic __proto__ value",
        ));
    }
    Ok(proto)
}

/// `InitializeGlobal(name, value)`: initializes a lexical binding (`let`, `const`, `class`),
/// or binds a function declaration on the global object (CreateGlobalFunctionBinding).
fn initialize_global(cx: &mut Cx, name: u64, v: u64) -> Res<u64> {
    let rt = cx.rt();
    if !is_string(name) {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: InitializeGlobal name is not a string",
        ));
    }
    let lexical = rt.root(Root::Lexical);
    if own_get(lexical, name).is_some() {
        object::own_replace(rt, lexical, name, v);
        return Ok(UNDEFINED);
    }
    create_global_function_binding(cx, name, v)?;
    Ok(UNDEFINED)
}

/// CreateGlobalFunctionBinding(N, V, D = false) (§9.1.1.4.18) for a script's function
/// declaration. `DeclareGlobals` checked CanDeclareGlobalFunction and made a missing binding
/// {w, e} (undefined); a configurable property (a builtin such as `parseInt`, or an
/// accessor) becomes the data property {w, e} here. GC point.
fn create_global_function_binding(cx: &mut Cx, name: u64, v: u64) -> Res<()> {
    let g = cx.rt().global();
    // Steps 3-5.
    let desc = match property::get_own_property(cx, g, name) {
        None => Desc::data(v, attr::WRITABLE | attr::ENUMERABLE),
        Some(d) if d.configurable == Some(true) => Desc::data(v, attr::WRITABLE | attr::ENUMERABLE),
        Some(_) => Desc {
            value: Some(v),
            ..Desc::default()
        },
    };
    // Step 6: DefinePropertyOrThrow.
    if !property::define_own_property(cx, g, name, desc)? {
        let msg = format!("Cannot redefine property: {}", key_text(cx.rt(), name));
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    // Step 7: Set(globalObject, N, V, false).
    property::set(cx, g, name, v, false)
}

/// True if the closure `f` is an anonymous function definition (its unit gives it no name):
/// the runtime names it from its property key (SetFunctionName). The front end names a
/// method or accessor with a literal key itself ("x", "get x").
fn is_anonymous(f: u64) -> bool {
    let fi = object::function_info(f);
    fi.loaded_unit()
        .is_some_and(|u| u.unit.functions[fi.function_id as usize].name.is_none())
}

/// SetFunctionName(f, key, prefix) (§10.2.9) through the builtin ABI. GC point.
fn set_function_name(cx: &mut Cx, f: u64, key: u64, prefix: Option<&str>) -> Res<()> {
    let mut acx = abi::Cx::new(cx);
    acx.set_function_name(
        Local::<JsObject>::from_bits(f),
        Local::<JsValue>::from_bits(key),
        prefix,
    )
}

/// MakeMethod and SetFunctionName of a class element or literal method `f` defined on
/// `home` under `key` (§15.4.4, §15.4.5): a class element gets `home` as its
/// `[[HomeObject]]` (an object literal method gets it from `SetHomeObject`, only when it uses
/// `super`); an anonymous `f` (a computed key) is named from `key`. GC point.
fn make_method(
    cx: &mut Cx,
    f: u64,
    home: u64,
    key: u64,
    class: bool,
    prefix: Option<&str>,
) -> Res<()> {
    if !is_function(f) {
        return Ok(());
    }
    if class {
        object::set_home_object(cx.rt(), f, home);
    }
    if is_anonymous(f) {
        set_function_name(cx, f, key, prefix)?;
    }
    Ok(())
}

/// `DefineClassMethod(home, key, function)`: DefineMethodProperty(home, key, f, false)
/// (§10.2.8), after MakeMethod and SetFunctionName. A rejected define throws a TypeError
/// (`static ['prototype']() {}` meets the non-configurable `prototype`).
fn define_class_method(cx: &mut Cx, home: u64, k: u64, f: u64) -> Res<u64> {
    let key = conv::to_property_key(cx, k)?;
    if !is_object(home) {
        return Err(not_implemented(cx, "class method on a non-object"));
    }
    make_method(cx, f, home, key, true, None)?;
    define_or_throw(cx, home, key, Desc::data(f, attr::HIDDEN))?;
    Ok(UNDEFINED)
}

/// `DefineAccessorInLiteral(object, key, getter, setter)` (enumerable) and
/// `DefineClassAccessor(home, key, getter, setter)` (not enumerable): MethodDefinitionEvaluation
/// of `get` or `set` (§15.4.5). The present half (the other is undefined) gets MakeMethod and
/// its name (`"get x"`, `"set x"`, `"get [desc]"` for a symbol key), then DefinePropertyOrThrow
/// with a descriptor of that half only, so a getter and a setter of one key combine into one
/// accessor property. GC point.
fn define_accessor(cx: &mut Cx, home: u64, k: u64, get: u64, set: u64, class: bool) -> Res<u64> {
    let key = conv::to_property_key(cx, k)?;
    if !is_object(home) {
        return Err(not_implemented(cx, "accessor on a non-object"));
    }
    make_method(cx, get, home, key, class, Some("get"))?;
    make_method(cx, set, home, key, class, Some("set"))?;
    let half = |f: u64| (f != UNDEFINED).then_some(f);
    let desc = Desc {
        get: half(get),
        set: half(set),
        enumerable: Some(!class),
        configurable: Some(true),
        ..Desc::default()
    };
    define_or_throw(cx, home, key, desc)?;
    Ok(UNDEFINED)
}

/// DefinePropertyOrThrow (§7.3.9). GC point.
fn define_or_throw(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Res<()> {
    if property::define_own_property(cx, o, key, desc)? {
        return Ok(());
    }
    let msg = format!("Cannot redefine property: {}", key_text(cx.rt(), key));
    Err(throw_error(cx, ErrorKind::TypeError, &msg))
}

/// CopyDataProperties(target, source, excluded) (§7.3.25): the own enumerable properties of
/// `source` (string characters for a string), in `[[OwnPropertyKeys]]` order; a getter runs
/// (Get). `excluded` is an array of property keys or undefined. GC: the key list is a new
/// array in a Rust local (conservative scan, H10), so the keys survive the getters, which
/// may change `source`; each key's descriptor is read again when its turn comes (step 5.c).
pub(super) fn copy_data_properties(
    cx: &mut Cx,
    target: u64,
    source: u64,
    excluded: u64,
) -> Res<()> {
    let rt = cx.rt();
    if source == UNDEFINED || source == NULL || !is_object(target) {
        return Ok(());
    }
    let skip = |k: u64| {
        is_array(excluded)
            && (0..object::array_len(excluded))
                .any(|j| object::array_get(excluded, j).is_some_and(|e| object::key_eq(e, k)))
    };
    if is_string(source) {
        for i in 0..object::str_len(source) {
            // A string is shorter than 2^29 units, so `i` fits a u32.
            let k = crate::key::index_key(rt, i as u32);
            if skip(k) {
                continue;
            }
            let u = [object::str_view(source).at(i)];
            let c = object::new_string_utf16(rt, &u);
            property::define(cx, target, k, c)?;
        }
        return Ok(());
    }
    if !is_object(source) {
        return Ok(());
    }
    let keys = property::own_property_keys(cx, source);
    for i in 0..object::array_len(keys) {
        let Some(k) = object::array_get(keys, i) else {
            continue;
        };
        if skip(k) {
            continue;
        }
        let enumerable =
            property::get_own_property(cx, source, k).is_some_and(|d| d.enumerable == Some(true));
        if enumerable {
            let v = property::get_from(cx, source, k)?;
            property::define(cx, target, k, v)?;
        }
    }
    Ok(())
}

/// The property keys of `count` registers from slot `first` of `fp`, as a new array.
fn key_array(cx: &mut Cx, fp: *mut u64, first: isize, count: u32) -> Res<u64> {
    let a = object::new_array(cx.rt(), count as usize);
    for i in 0..count {
        // SAFETY: the registers are part of a verified register list of the frame `fp`.
        let k = unsafe { *fp.offset(first + i as isize) };
        let key = conv::to_property_key(cx, k)?;
        object::array_set(cx.rt(), a, i, key);
    }
    Ok(a)
}
