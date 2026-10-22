//! Realms (H22-Q3): the REALM record, the core intrinsics and global object of a realm
//! (`init`), the install of the builtin tables into it (`install`), more realms (`create`), the
//! realm switch of a cross-realm call (`switch`), and the global declaration step of scripts
//! (`declare_globals`). Builtin functions and their properties come from tables
//! (`builtins.rs`): the VM table (`natives.rs`), then `IsolateConfig::builtins`.
//!
//! ```text
//! Isolate::new: builtins::check -> Runtime::new -> builtins::prepare
//!   -> realm::init: atoms, the REALM record, core intrinsics, global object, symbol::install
//!   -> realm::install: builtins::install (every table), %AggregateError%'s [[Prototype]],
//!                      fill_intrinsics (the REALM record's intrinsics list)
//! realm::create (Cx::create_realm, $262.createRealm): blank the current-realm roots
//!   -> init_core (a new REALM record, core intrinsics, global object, %Symbol.prototype%)
//!   -> realm::install (the same function, into the builtin install list) -> switch back
//! ```
//!
//! A realm is a REALM record (OBJECT_LAYOUT 3, `object::realmf`): its global object, its
//! global lexical record and its intrinsics list (index `IntrinsicId`). Every function stores
//! the realm it was made in as its `[[Realm]]` (`object::new_function` reads the `Root::Realm`
//! root). The global object holds its REALM record under a VM-internal private name
//! (`Root::RealmKey`), so the embedder API names a realm by its global object
//! (`abi/realm.rs`).
//!
//! **The current realm is the roots (V8-A10).** The realm roots of `runtime.rs`, the intrinsic
//! slots of `builtins.rs` and `%Symbol.prototype%` of `symbol.rs` always hold the values of
//! the current realm, and `Cx::realm`/`Cx::global_this` its record and global object. Code
//! that reads a root (object.rs, property.rs, ic.rs, conv.rs, exotic.rs, shape.rs, natives.rs,
//! interp/slow/*, abi/jobs.rs) needs no realm argument. A call of a function of another realm
//! switches: [`switch`] writes the callee realm's values from its record into those roots, and
//! the caller's go back on return or throw. The record is a copy of the roots: `realm::init`
//! fills it, and `builtins::set_intrinsic` writes both.
//!
//! | Call path | Where the realm switches |
//! |---|---|
//! | T0 `Call*` (`interp/ops.rs` `call_fast`) | a callee of another realm goes cold (one compare) |
//! | `slow::call` (`Call*`, `Construct*`) | a bytecode callee of another realm runs nested through `call::call_function_object` / `call::construct_function_object` (one Rust level per cross-realm call; a same-realm call stays a frame push) |
//! | `call::call_host` (every host function) | around the host call: a builtin runs in its own realm (H20 R5) |
//! | `call::call_function_object` (Rust -> JS) | around the nested run |
//! | `call::run_resumed` (generator resume) | around the nested run |
//!
//! Caches that hold a realm value: the function shape caches (`shape::function_shape`,
//! `generator_function_shape`) check the prototype on each hit, and [`switch`] empties them
//! (`shape::clear_realm_caches`), so they hold only a shape of the current realm and keep no
//! other realm live. Feedback vectors and template objects belong to one FunctionInfo, whose
//! functions are all of one realm. `builtins::existing_function` knows the first realm's
//! builtin functions only, so the identity fast paths that use it run their spec steps in a
//! later realm.
//!
//! | Core intrinsic | Made as |
//! |---|---|
//! | `%Object.prototype%` | an ordinary object, `[[Prototype]]` null |
//! | `%Function.prototype%` | VM builtin 0.0: a function that returns undefined (§20.2.3) |
//! | `%Array.prototype%` | an ordinary object (v0: not an Array exotic object, H18) |
//! | `%String.prototype%`, `%Number.prototype%`, `%Boolean.prototype%` | `wrapper::new_wrapper` of "", +0 and false (§22.1.3, §21.1.3, §20.3.3) |
//! | `%Iterator.prototype%`, `%ArrayIteratorPrototype%` | ordinary objects (`%StringIteratorPrototype%` is made by the VM table, `natives.rs`) |
//! | `%Error.prototype%`, the native error prototypes | ordinary objects |
//! | `%Symbol.prototype%` | `symbol::install` (first realm, P2-SYM), `symbol::install_prototype` (later realms) |
//! | the global object; the script lexical record | an ordinary object; a null-prototype object |

use crate::abi::HostFn;
use crate::builtins::{self, BuiltinId, IntrinsicId};
use crate::conv::{self, num};
use crate::error;
use crate::frame::{FALSE, HOLE, NULL, UNDEFINED};
use crate::interp::Cx;
use crate::key::key_text;
use crate::kinds;
use crate::natives;
use crate::object::{self, attr, own_define, own_get, realmf};
use crate::property;
use crate::runtime::{Atom, ErrorKind, Root, Runtime};
use crate::unit::FunctionInfo;
use crate::{atoms, symbol, wrapper};

/// A host function object for `f` outside the tables (an embedder's function, tests), in the
/// current realm. Its FunctionInfo lives in `rt.infos` (boxed, stable).
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

/// Makes the atoms, the first realm's core intrinsics, its global object, the symbols and the
/// realm key. Every new cell goes into a root before the next allocation (GC-stress safe).
/// `builtins::prepare` ran first; [`install`] runs next.
pub(crate) fn init(rt: &Runtime) {
    atoms::init_static(rt);
    init_core(rt, true);
}

/// The core of a realm: its REALM record, core intrinsics, global object, lexical record and
/// `%Symbol.prototype%` (all symbols for the `first` realm, which also makes the realm key),
/// written into the current-realm roots; then the record names the global object and the
/// global object holds the record. GC points; each new cell is in a root first.
fn init_core(rt: &Runtime, first: bool) {
    // The REALM record first: every function made from here on stores it (OBJECT_LAYOUT 3).
    let realm = new_realm_record(rt);
    rt.set_root(Root::Realm, realm);
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
    object::record_set(rt, realm, realmf::GLOBAL_OBJECT, global);
    object::record_set(rt, realm, realmf::GLOBAL_LEXICAL, lexical);
    if first {
        symbol::install(rt);
        // GC point: the description, then the key; the key goes into its root at once.
        let desc = atoms::intern_str(rt, "#.realm");
        let key = symbol::new_private(rt, desc);
        rt.set_root(Root::RealmKey, key);
    } else {
        symbol::install_prototype(rt);
    }
    // GC point (the global's properties): `global` and `realm` are roots.
    own_define(rt, global, rt.root(Root::RealmKey), realm, 0);
}

/// Installs the builtin tables into the current realm (made by [`init`] or [`create`]): every
/// table of the isolate (`builtins::install`: the VM table, then the embedder's, so every
/// module of `ep9_builtins::MODULES` too), then the `[[Prototype]]` of `%AggregateError%`, then
/// the REALM record's intrinsics list. The one install function of every realm (H22-Q3; a lazy
/// install, M1 per M3D-5, changes only this). GC points.
pub(crate) fn install(cx: &mut Cx) -> Result<(), String> {
    builtins::install(cx)?;
    aggregate_error_prototype(cx.rt());
    fill_intrinsics(cx.rt());
    Ok(())
}

/// §20.5.7.2: the `[[Prototype]]` of `%AggregateError%` is `%Error%` (a table gives a
/// builtin's function object `%Function.prototype%`). Runs after `builtins::install`, when a
/// table made `%AggregateError%` (`ep9_builtins` promise.rs, P4-PROMISE).
fn aggregate_error_prototype(rt: &Runtime) {
    let c = builtins::intrinsic(rt, IntrinsicId::AggregateError);
    let error = builtins::intrinsic(rt, IntrinsicId::Error);
    if object::is_object(c) && object::is_object(error) {
        let ok = property::set_prototype_of(rt, c, error);
        debug_assert!(ok, "%AggregateError% is extensible");
    }
}

/// A new REALM record: every field undefined, and an intrinsics list of
/// `IntrinsicId::ALL.len()` undefined entries. GC point.
pub(crate) fn new_realm_record(rt: &Runtime) -> u64 {
    // GC points: the list, then the record; the list lives in a Rust local (CSS) until the
    // record holds it.
    let list = object::new_list(rt, IntrinsicId::ALL.len(), UNDEFINED);
    let realm = object::new_record(rt, kinds::REALM);
    object::record_set(rt, realm, realmf::INTRINSICS, list);
    realm
}

/// Copies every intrinsic of the realm (`builtins::intrinsic`; undefined for one nothing made)
/// into the intrinsics list of the `Root::Realm` record, at index `IntrinsicId`. Runs after
/// `builtins::install`. Allocates nothing.
pub(crate) fn fill_intrinsics(rt: &Runtime) {
    let realm = rt.root(Root::Realm);
    let list = object::record_get(realm, realmf::INTRINSICS);
    for id in IntrinsicId::ALL {
        object::list_set(rt, list, id as usize, builtins::intrinsic(rt, id));
    }
}

/// Stores `v` as the intrinsic `id` in the current REALM record (`builtins::set_intrinsic`
/// keeps the record a copy of the roots). Not a GC point.
pub(crate) fn record_intrinsic(rt: &Runtime, id: IntrinsicId, v: u64) {
    let realm = rt.root(Root::Realm);
    if is_realm(realm) {
        let list = object::record_get(realm, realmf::INTRINSICS);
        object::list_set(rt, list, id as usize, v);
    }
}

/// True for a REALM record.
fn is_realm(v: u64) -> bool {
    kinds::is_cell_bits(v) && kinds::cell_kind(v) == kinds::REALM.get()
}

// ---- more realms, and the switch -----------------------------------------------------------

/// CreateRealm, SetRealmGlobalObject and SetDefaultGlobalBindings (§9.3.1-§9.3.3) with every
/// builtin table of the isolate: a new realm with its own intrinsics and global object.
/// Returns its REALM record; the current realm stays current. GC point.
///
/// The new realm is made in the current-realm roots (so `init_core` and `install` are the
/// first realm's code): they are blanked, filled by the new realm, then the caller's realm is
/// switched back in from its record (the record is a copy of the roots, and the saved record
/// lives in a Rust local meanwhile). The new realm's builtin function objects go into the
/// builtin install list (`builtins::begin_realm_install`).
pub(crate) fn create(cx: &mut Cx) -> conv::Res<u64> {
    let rt = cx.rt();
    let saved = cx.realm;
    debug_assert!(saved == rt.root(Root::Realm));
    // GC point: the install list; the current realm is still in its roots.
    builtins::begin_realm_install(rt);
    rt.clear_realm_roots();
    builtins::clear_intrinsic_slots(rt);
    symbol::set_prototype(rt, UNDEFINED);
    init_core(rt, false);
    let realm = rt.root(Root::Realm);
    cx.realm = realm;
    cx.global_this = rt.global();
    let r = install(cx);
    builtins::end_realm_install(rt);
    switch(cx, saved);
    match r {
        Ok(()) => Ok(realm),
        Err(m) => {
            let msg = format!("ep9 internal error: realm install failed: {m}");
            Err(error::throw_error(cx, ErrorKind::Error, &msg))
        }
    }
}

/// Makes `realm` the current realm (V8-A10): writes its global object, lexical record, every
/// intrinsic of its intrinsics list and the record itself into the current-realm roots, the
/// builtin intrinsic slots and `%Symbol.prototype%`, empties the function shape caches
/// (`shape::clear_realm_caches`), and sets `Cx::realm` and `Cx::global_this`. About 68
/// stores; no allocation, no GC point.
#[cold]
#[inline(never)]
pub(crate) fn switch(cx: &mut Cx, realm: u64) {
    debug_assert!(
        is_realm(realm),
        "switch to a value that is not a REALM record"
    );
    let rt = cx.rt();
    let list = object::record_get(realm, realmf::INTRINSICS);
    for id in IntrinsicId::ALL {
        builtins::load_intrinsic(rt, id, object::list_get(list, id as usize));
    }
    rt.set_root(
        Root::Lexical,
        object::record_get(realm, realmf::GLOBAL_LEXICAL),
    );
    rt.set_root(Root::Realm, realm);
    crate::shape::clear_realm_caches(rt);
    cx.realm = realm;
    cx.global_this = object::record_get(realm, realmf::GLOBAL_OBJECT);
    debug_assert!(cx.global_this == rt.global());
}

/// Enters the realm of the function `f` if it is not the current one: returns the realm to
/// give back to [`leave`] (`None`: no switch). One load and one compare on the same-realm
/// path. Not a GC point.
#[inline(always)]
pub(crate) fn enter_realm_of(cx: &mut Cx, f: u64) -> Option<u64> {
    let callee = object::function_realm(f);
    if callee == cx.realm {
        return None;
    }
    let saved = cx.realm;
    switch(cx, callee);
    Some(saved)
}

/// Switches back to the realm that [`enter_realm_of`] gave (after the call returned or
/// threw). Not a GC point.
#[inline(always)]
pub(crate) fn leave(cx: &mut Cx, saved: Option<u64>) {
    if let Some(r) = saved {
        switch(cx, r);
    }
}

/// The intrinsic `id` of `realm` (current or not): undefined if no table made it.
pub(crate) fn intrinsic_in(rt: &Runtime, realm: u64, id: IntrinsicId) -> u64 {
    if realm == rt.root(Root::Realm) {
        return builtins::intrinsic(rt, id);
    }
    let list = object::record_get(realm, realmf::INTRINSICS);
    object::list_get(list, id as usize)
}

/// GetFunctionRealm(obj) (§7.3.22): the `[[Realm]]` of a function, the realm of a bound
/// function's target (step 3), else the current realm (step 5; v0 has no Proxy). Not a GC
/// point.
pub(crate) fn get_function_realm(rt: &Runtime, mut obj: u64) -> u64 {
    loop {
        if kinds::is_function(obj) {
            return object::function_realm(obj);
        }
        if kinds::is_bound(obj) {
            obj = crate::bound::target(obj);
            continue;
        }
        return rt.root(Root::Realm);
    }
}

/// The REALM record of the realm whose global object is `global`, if it is one. Not a GC
/// point.
pub(crate) fn of_global(rt: &Runtime, global: u64) -> Option<u64> {
    if !object::is_object(global) {
        return None;
    }
    own_get(global, rt.root(Root::RealmKey))
        .map(|(v, _)| v)
        .filter(|&v| is_realm(v))
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
            || is_lexical(s.mode) && has_restricted_global_property(cx, global, name)?;
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
        if !can_declare_global_function(cx, global, name)? {
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
fn has_restricted_global_property(cx: &mut Cx, global: u64, name: u64) -> conv::Res<bool> {
    let desc = property::get_own_property(cx, global, name)?;
    Ok(desc.is_some_and(|d| d.configurable == Some(false)))
}

/// CanDeclareGlobalFunction (§9.1.1.4.16).
fn can_declare_global_function(cx: &mut Cx, global: u64, name: u64) -> conv::Res<bool> {
    Ok(match property::get_own_property(cx, global, name)? {
        // 3. No property: IsExtensible(globalObject).
        None => property::is_extensible(global),
        // 4. A configurable property can be replaced.
        Some(d) if d.configurable == Some(true) => true,
        // 5. A writable, enumerable data property keeps its attributes.
        Some(d) => !d.is_accessor() && d.writable == Some(true) && d.enumerable == Some(true),
    })
}

/// CanDeclareGlobalVar (§9.1.1.4.15).
fn can_declare_global_var(cx: &mut Cx, global: u64, name: u64) -> bool {
    property::has_own_property(cx, global, name) || property::is_extensible(global)
}
