//! The realm: its core intrinsics and global object (`init`), and the global declaration step
//! of scripts (`declare_globals`). Builtin functions and their properties come from tables
//! (`builtins.rs`): the VM table (`natives.rs`), then `IsolateConfig::builtins`.
//!
//! ```text
//! Isolate::new: builtins::check -> Runtime::new -> builtins::prepare
//!   -> realm::init: atoms, core intrinsics, global object, symbol::install
//!   -> builtins::install: every builtin function, the tables' intrinsics, their properties
//! ```
//!
//! | Core intrinsic | Made as |
//! |---|---|
//! | `%Object.prototype%` | an ordinary object, `[[Prototype]]` null |
//! | `%Function.prototype%` | VM builtin 0.0: a function that returns undefined (§20.2.3) |
//! | `%Array.prototype%` | an ordinary object (v0: not an Array exotic object, H18) |
//! | `%String.prototype%`, `%Number.prototype%`, `%Boolean.prototype%` | `wrapper::new_wrapper` of "", +0 and false (§22.1.3, §21.1.3, §20.3.3) |
//! | `%Iterator.prototype%`, `%ArrayIteratorPrototype%` | ordinary objects (`%StringIteratorPrototype%` is made by the VM table, `natives.rs`) |
//! | `%Error.prototype%`, the native error prototypes | ordinary objects |
//! | `%Symbol.prototype%` | `symbol::install` (P2-SYM) |
//! | the global object; the script lexical record | an ordinary object; a null-prototype object |

use crate::abi::HostFn;
use crate::builtins::{self, BuiltinId};
use crate::conv::{self, num};
use crate::error;
use crate::frame::{FALSE, HOLE, NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::key_text;
use crate::natives;
use crate::object::{self, attr, own_define, own_get};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, Runtime};
use crate::unit::FunctionInfo;
use crate::{atoms, symbol, wrapper};

/// A host function object for `f` outside the tables (an embedder's function, tests). Its
/// FunctionInfo lives in `rt.infos` (boxed, stable).
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

/// Makes the atoms, the core intrinsics, the global object and the symbols. Every new cell
/// goes into a root before the next allocation (GC-stress safe). `builtins::prepare` ran
/// first; `builtins::install` runs next.
pub(crate) fn init(rt: &Runtime) {
    atoms::init_static(rt);
    let object_proto = object::new_object(rt, NULL);
    rt.set_root(Root::ObjectPrototype, object_proto);
    // `new_function` takes a function's [[Prototype]] from this root, and the [[Prototype]]
    // of %Function.prototype% itself is %Object.prototype% (§20.2.3).
    rt.set_root(Root::FunctionPrototype, object_proto);
    let fp = builtins::function_object(
        rt,
        BuiltinId::new(natives::VM_BLOCK, natives::vm::FUNCTION_PROTOTYPE),
    );
    rt.set_root(Root::FunctionPrototype, fp);
    let array_proto = object::new_object(rt, object_proto);
    rt.set_root(Root::ArrayPrototype, array_proto);
    for (r, v) in [
        (Root::StringPrototype, rt.atom(Atom::Empty)),
        (Root::NumberPrototype, num(0.0)),
        (Root::BooleanPrototype, FALSE),
    ] {
        let p = wrapper::new_wrapper(rt, object_proto, v);
        rt.set_root(r, p);
    }
    let iterator_proto = object::new_object(rt, object_proto);
    rt.set_root(Root::IteratorPrototype, iterator_proto);
    let array_iterator_proto = object::new_object(rt, iterator_proto);
    rt.set_root(Root::ArrayIteratorPrototype, array_iterator_proto);
    let error_proto = object::new_object(rt, object_proto);
    rt.set_root(Root::ErrorPrototype, error_proto);
    for kind in ErrorKind::ALL {
        if kind != ErrorKind::Error {
            let p = object::new_object(rt, error_proto);
            rt.set_root(kind.prototype_root(), p);
        }
    }
    let lexical = object::new_object(rt, NULL);
    rt.set_root(Root::Lexical, lexical);
    let global = object::new_object(rt, object_proto);
    rt.set_root(Root::GlobalObject, global);
    symbol::install(rt);
}

/// `DeclareGlobals`: GlobalDeclarationInstantiation (§16.1.7) of the script (or eval) scope
/// of `unit`. Every check runs before any binding is made, so a failed declaration leaves the
/// realm as it was:
///
/// | Steps | Check | Error |
/// |---|---|---|
/// | 3 | a lexical name is a lexical binding already, or HasRestrictedGlobalProperty (a non-configurable global; v0 has no `[[VarNames]]`, and a var binding is non-configurable) | SyntaxError |
/// | 4 | a var or function name is a lexical binding | SyntaxError |
/// | 5-8 | CanDeclareGlobalFunction (§9.1.1.4.16), last declaration first | TypeError |
/// | 9-10 | CanDeclareGlobalVar (§9.1.1.4.15) for vars that are not functions | TypeError |
///
/// Then (steps 15-17): `let`/`const`/`class` names go to the lexical record as holes (TDZ;
/// `const` is not writable); function, then var names become global properties (undefined
/// if new: CreateGlobalVarBinding §9.1.1.4.17; `InitializeGlobal` stores a function's value).
pub(crate) fn declare_globals(
    cx: &mut Cx,
    unit: &ep9_bytecode::CompiledUnit,
    strings: &dyn Fn(usize) -> u64,
) -> conv::Res<()> {
    use ep9_bytecode::{ScopeKind, VariableMode};
    let Some(scope) = unit
        .scopes
        .iter()
        .find(|s| matches!(s.kind, ScopeKind::Script | ScopeKind::Eval))
    else {
        return Ok(());
    };
    let rt = cx.rt();
    let global = rt.global();
    let lexical = rt.root(Root::Lexical);
    let is_lexical = |m: VariableMode| {
        matches!(
            m,
            VariableMode::Let | VariableMode::Const | VariableMode::Class
        )
    };
    let name_of = |s: &ep9_bytecode::ScopeSlot| strings(s.name.index());
    // Steps 3-4.
    for s in &scope.slots {
        let name = name_of(s);
        let clash = own_get(lexical, name).is_some()
            || is_lexical(s.mode) && has_restricted_global_property(cx, global, name);
        if clash {
            let msg = format!(
                "Identifier '{}' has already been declared",
                key_text(rt, name)
            );
            return Err(error::throw_error(cx, ErrorKind::SyntaxError, &msg));
        }
    }
    // Steps 5-8.
    let functions = || {
        scope
            .slots
            .iter()
            .filter(|s| s.mode == VariableMode::Function)
    };
    for s in functions().rev() {
        let name = name_of(s);
        if !can_declare_global_function(cx, global, name) {
            let n = key_text(rt, name);
            let msg = if own_get(global, name).is_some() {
                format!("Cannot redefine property: {n}")
            } else {
                format!("Cannot add property {n}, object is not extensible")
            };
            return Err(error::throw_error(cx, ErrorKind::TypeError, &msg));
        }
    }
    // Steps 9-10.
    let is_function_name = |name: u64| functions().any(|f| name_of(f) == name);
    let vars = || {
        scope
            .slots
            .iter()
            .filter(|s| s.mode == VariableMode::Var && !is_function_name(name_of(s)))
    };
    for s in vars() {
        let name = name_of(s);
        if !can_declare_global_var(cx, global, name) {
            let msg = format!(
                "Cannot add property {}, object is not extensible",
                key_text(rt, name)
            );
            return Err(error::throw_error(cx, ErrorKind::TypeError, &msg));
        }
    }
    // Steps 15-17. GC points: `name` is an atom of the unit's string table (a root range).
    for s in scope.slots.iter().filter(|s| is_lexical(s.mode)) {
        let a = if s.mode == VariableMode::Const {
            0
        } else {
            attr::WRITABLE
        };
        own_define(rt, lexical, name_of(s), HOLE, a);
    }
    for s in functions().chain(vars()) {
        // CreateGlobalVarBinding (§9.1.1.4.17): only a new name on an extensible global.
        let name = name_of(s);
        if own_get(global, name).is_none() && property::is_extensible(global) {
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

/// HasRestrictedGlobalProperty (§9.1.1.4.14): a non-configurable own property of the global.
fn has_restricted_global_property(cx: &mut Cx, global: u64, name: u64) -> bool {
    property::get_own_property(cx, global, name).is_some_and(|d| d.configurable == Some(false))
}

/// CanDeclareGlobalFunction (§9.1.1.4.16).
fn can_declare_global_function(cx: &mut Cx, global: u64, name: u64) -> bool {
    match property::get_own_property(cx, global, name) {
        // 3. No property: IsExtensible(globalObject).
        None => property::is_extensible(global),
        // 4. A configurable property can be replaced.
        Some(d) if d.configurable == Some(true) => true,
        // 5. A writable, enumerable data property keeps its attributes.
        Some(d) => !d.is_accessor() && d.writable == Some(true) && d.enumerable == Some(true),
    }
}

/// CanDeclareGlobalVar (§9.1.1.4.15).
fn can_declare_global_var(cx: &mut Cx, global: u64, name: u64) -> bool {
    property::has_own_property(cx, global, name) || property::is_extensible(global)
}
