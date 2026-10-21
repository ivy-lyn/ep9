//! The v0 realm: intrinsics, the global object and a few builtins (M0 week 3). Week 4 brings
//! the builtin ABI (H20) and template builtins; these are only what the VM itself needs. The
//! host functions are in `natives.rs`.
//!
//! | Global | Value |
//! |---|---|
//! | `globalThis`, `undefined`, `NaN`, `Infinity` | as in §19.1 |
//! | `Object`, `Array` | constructors (§20.1.1, §23.1.1; v0: no wrapper objects) |
//! | `Error`, `TypeError`, `RangeError`, `ReferenceError`, `SyntaxError`, `EvalError`, `URIError` | constructors (§20.5) |
//!
//! Prototype methods: `Object.prototype.toString`/`valueOf`, `Error.prototype.toString`,
//! `Array.prototype.join`/`toString`/`push`.
//!
//! Init order: static atoms, intrinsics and globals, symbols (`symbol::install`), then the
//! embedder's builtin table (`builtins::install`).

use crate::abi::HostFn;
use crate::builtins::{self, BuiltinTable};
use crate::conv::{self, num};
use crate::error;
use crate::frame::{NULL, UNDEFINED};
use crate::interp::Cx;
use crate::natives;
use crate::object::{self, attr, own_define};
use crate::runtime::{Atom, ErrorKind, Root, Runtime};
use crate::unit::FunctionInfo;
use crate::{atoms, symbol};

/// A host function object for `f`. Its FunctionInfo lives in `rt.infos` (boxed, stable).
pub(crate) fn new_host_function(
    rt: &Runtime,
    f: HostFn,
    name: &str,
    length: u32,
    constructor: bool,
) -> u64 {
    let info = Box::new(FunctionInfo::host(f, name, length, constructor));
    let p: *const FunctionInfo = &*info;
    rt.infos.borrow_mut().push(info);
    object::new_function(rt, p, UNDEFINED)
}

/// Defines `holder.name = <host function>` (not enumerable).
pub(crate) fn define_host_method(
    rt: &Runtime,
    holder: u64,
    name: &str,
    f: HostFn,
    length: u32,
) -> u64 {
    let func = new_host_function(rt, f, name, length, false);
    let key = atoms::intern_str(rt, name);
    own_define(rt, holder, key, func, attr::HIDDEN);
    func
}

/// Builds the realm: atoms, intrinsics, the global object, then the embedder's builtins.
/// Every new cell goes into a root before the next allocation (GC-stress safe).
pub(crate) fn init(rt: &Runtime, table: Option<&'static BuiltinTable>) {
    atoms::init_static(rt);
    let object_proto = object::new_object(rt, NULL);
    rt.set_root(Root::ObjectPrototype, object_proto);
    for (r, p) in [
        (Root::FunctionPrototype, object_proto),
        (Root::ArrayPrototype, object_proto),
        (Root::StringPrototype, object_proto),
        (Root::NumberPrototype, object_proto),
        (Root::BooleanPrototype, object_proto),
        (Root::ErrorPrototype, object_proto),
    ] {
        // v0: plain objects (Function.prototype is not callable, Array.prototype is not an
        // array exotic object).
        let o = object::new_object(rt, p);
        rt.set_root(r, o);
    }
    let lexical = object::new_object(rt, NULL);
    rt.set_root(Root::Lexical, lexical);
    let global = object::new_object(rt, object_proto);
    rt.set_root(Root::GlobalObject, global);

    own_define(rt, global, rt.atom(Atom::GlobalThis), global, attr::HIDDEN);
    own_define(rt, global, rt.atom(Atom::Undefined), UNDEFINED, 0);
    own_define(rt, global, rt.atom(Atom::NaN), num(f64::NAN), 0);
    own_define(rt, global, rt.atom(Atom::Infinity), num(f64::INFINITY), 0);

    define_host_method(rt, object_proto, "toString", natives::object_to_string, 0);
    define_host_method(rt, object_proto, "valueOf", natives::object_value_of, 0);
    let array_proto = rt.root(Root::ArrayPrototype);
    define_host_method(rt, array_proto, "join", natives::array_join, 1);
    define_host_method(rt, array_proto, "toString", natives::array_to_string, 0);
    define_host_method(rt, array_proto, "push", natives::array_push, 1);

    // v0: one prototype for array and string iterators; `next` only (no Symbol.iterator).
    let iterator_proto = object::new_object(rt, object_proto);
    rt.set_root(Root::ArrayIteratorPrototype, iterator_proto);
    define_host_method(rt, iterator_proto, "next", natives::iterator_next, 0);

    define_constructor(
        rt,
        global,
        "Object",
        natives::object_constructor,
        object_proto,
    );
    define_constructor(rt, global, "Array", natives::array_constructor, array_proto);

    let error_proto = rt.root(Root::ErrorPrototype);
    define_host_method(rt, error_proto, "toString", natives::error_to_string, 0);
    for kind in ErrorKind::ALL {
        let proto = if kind == ErrorKind::Error {
            error_proto
        } else {
            let p = object::new_object(rt, error_proto);
            rt.set_root(kind.prototype_root(), p);
            p
        };
        let name = object::new_string(rt, kind.name());
        own_define(rt, proto, rt.atom(Atom::Name), name, attr::HIDDEN);
        own_define(
            rt,
            proto,
            rt.atom(Atom::Message),
            rt.atom(Atom::Empty),
            attr::HIDDEN,
        );
        let ctor = new_host_function(rt, natives::error_constructor(kind), kind.name(), 1, true);
        own_define(rt, ctor, rt.atom(Atom::Prototype), proto, 0);
        own_define(rt, proto, rt.atom(Atom::Constructor), ctor, attr::HIDDEN);
        let gname = atoms::intern_str(rt, kind.name());
        own_define(rt, global, gname, ctor, attr::HIDDEN);
    }
    symbol::install(rt);
    builtins::install(rt, table);
}

/// Defines the global constructor `name` with `prototype` (and `prototype.constructor`).
fn define_constructor(rt: &Runtime, global: u64, name: &str, f: HostFn, proto: u64) {
    let ctor = new_host_function(rt, f, name, 1, true);
    own_define(rt, ctor, rt.atom(Atom::Prototype), proto, 0);
    own_define(rt, proto, rt.atom(Atom::Constructor), ctor, attr::HIDDEN);
    let key = atoms::intern_str(rt, name);
    own_define(rt, global, key, ctor, attr::HIDDEN);
}

/// The global declaration step of `DeclareGlobals` (§16.1.7 GlobalDeclarationInstantiation,
/// v0): `var`/function names become global properties (undefined if new); `let`/`const`/
/// `class` names go to the lexical record as holes (TDZ; `const` is not writable).
/// `InitializeGlobal` stores their values later. Conflicts throw a SyntaxError.
pub(crate) fn declare_globals(
    cx: &mut Cx,
    unit: &ep9_bytecode::CompiledUnit,
    strings: &dyn Fn(usize) -> u64,
) -> conv::Res<()> {
    use ep9_bytecode::{ScopeKind, VariableMode};
    let rt = cx.rt();
    let Some(scope) = unit
        .scopes
        .iter()
        .find(|s| matches!(s.kind, ScopeKind::Script | ScopeKind::Eval))
    else {
        return Ok(());
    };
    let global = rt.global();
    let lexical = rt.root(Root::Lexical);
    for s in &scope.slots {
        let name = strings(s.name.index());
        let lexical_decl = matches!(
            s.mode,
            VariableMode::Let | VariableMode::Const | VariableMode::Class
        );
        let clash = object::own_get(lexical, name).is_some()
            || lexical_decl
                && object::own_get(global, name).is_some_and(|(_, a)| a & attr::CONFIGURABLE == 0);
        if clash {
            let n = object::str_view(name).to_string_lossy();
            return Err(error::throw_error(
                cx,
                ErrorKind::SyntaxError,
                &format!("Identifier '{n}' has already been declared"),
            ));
        }
        if lexical_decl {
            let a = if s.mode == VariableMode::Const {
                0
            } else {
                attr::WRITABLE
            };
            own_define(rt, lexical, name, crate::frame::HOLE, a);
        } else if object::own_get(global, name).is_none() {
            own_define(
                rt,
                global,
                name,
                UNDEFINED,
                attr::WRITABLE | attr::ENUMERABLE,
            );
        }
    }
    Ok(())
}
