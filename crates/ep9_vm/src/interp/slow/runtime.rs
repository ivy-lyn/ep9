//! `CallRuntime(id, args)`.

use ep9_bytecode::RuntimeId;

use crate::conv::{self, Res, describe};
use crate::error::{not_implemented, throw_error};
use crate::frame::{HOLE, NULL, UNDEFINED};
use crate::interp::Cx;
use crate::object::{self, attr, is_array, is_function, is_object, is_string, own_define, own_get};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root};

use super::property::super_get;
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
        // v0: there is no %eval%, so the callee is never %eval% and the call goes ahead.
        Some(Id::ResolvePossiblyDirectEval) => Ok(a(0)),
        Some(Id::CopyDataProperties) => {
            copy_data_properties(cx, a(0), a(1), UNDEFINED).map(|_| a(0))
        }
        Some(Id::CopyDataPropertiesExcluding) => (|| -> Res<u64> {
            let ex = key_array(cx, fp, first + 1, argc.saturating_sub(1))?;
            let o = object::new_plain_object(cx.rt());
            copy_data_properties(cx, o, a(0), ex)?;
            Ok(o)
        })(),
        Some(Id::SetPrototypeInLiteral) => {
            let (o, p) = (a(0), a(1));
            if is_object(o) && (is_object(p) || p == NULL) {
                // A cycle leaves the prototype as it is (as V8 does for `__proto__`).
                object::set_proto(cx.rt(), o, p);
            }
            Ok(UNDEFINED)
        }
        Some(Id::DefineClassMethod) => {
            let (home, k, f) = (a(0), a(1), a(2));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                if !is_object(home) {
                    return Err(not_implemented(cx, "class method on a non-object"));
                }
                if is_function(f) {
                    object::set_home_object(cx.rt(), f, home);
                }
                own_define(cx.rt(), home, key, f, attr::HIDDEN);
                Ok(UNDEFINED)
            })()
        }
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
        // `super[key]` (§13.3.7.1 MakeSuperPropertyReference + GetValue). v0: no accessors,
        // so the receiver is not needed.
        Some(Id::LoadKeyedFromSuper) => {
            let (home, k) = (a(1), a(2));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                super_get(cx, home, key)
            })()
        }
        // `super[key] = value` (§10.1.9.2 OrdinarySetWithOwnDescriptor with receiver = this).
        // v0: no accessors, so the store goes to the receiver (strict: super is only in
        // methods, and a failed store throws in strict code).
        Some(Id::StoreToSuper) => {
            let (receiver, home, k, v) = (a(0), a(1), a(2), a(3));
            (|| -> Res<u64> {
                let key = conv::to_property_key(cx, k)?;
                if !is_object(home) {
                    return Err(not_implemented(
                        cx,
                        "super property store without a home object",
                    ));
                }
                property::set(cx, receiver, key, v, strict(fp))?;
                Ok(v)
            })()
        }
        Some(other) => Err(not_implemented(cx, other.info().name)),
        None => Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: unknown runtime function",
        )),
    };
    ret(r)
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

/// `InitializeGlobal(name, value)` (v0): initializes a lexical binding, or sets the global
/// object property of a function declaration.
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
    let g = rt.global();
    if !object::own_replace(rt, g, name, v) {
        own_define(rt, g, name, v, attr::WRITABLE | attr::ENUMERABLE);
    }
    Ok(UNDEFINED)
}

/// CopyDataProperties(target, source, excluded) (§7.3.25, v0: own enumerable data properties,
/// array elements and string characters). `excluded` is an array of property keys or
/// undefined. No JS runs here, so `source` keeps its keys and values alive; fresh index keys
/// live in Rust locals (conservative scan, H10) until `define` stores them.
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
            let k = conv::number_string(cx, i as f64);
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
    if is_array(source) {
        for i in 0..object::array_len(source) {
            if let Some(v) = object::array_get(source, i) {
                let k = conv::number_string(cx, f64::from(i));
                if !skip(k) {
                    property::define(cx, target, k, v)?;
                }
            }
        }
    }
    let mut i = 0;
    while i < object::own_count(source) {
        let (k, v, a) = object::own_entry(source, i);
        if a & attr::ENUMERABLE != 0 && !skip(k) {
            property::define(cx, target, k, v)?;
        }
        i += 1;
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
