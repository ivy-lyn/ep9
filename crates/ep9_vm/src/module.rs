//! Modules (§16.2; owner M3-P2 MODULE, then M3-P3 IMPORT-TLA): Source Text Module Records
//! (the MODULE cell, kind 76, `object::modf`), the module map of a realm, loading, Link,
//! Evaluate, ResolveExport, and module namespace exotic objects (kind 26, `object::nsf`,
//! §10.4.6) with their internal methods.
//!
//! ```text
//! Cx::run_module(unit, name, resolve, load)        (isolate.rs: the loader API)
//!   with_host: the hooks are the isolate's module host for the run         HOSTS
//!   record of `name` (the realm's module map) or a new one          new_module
//!   load_graph: every [[RequestedModules]] entry, depth first       resolve(referrer, specifier)
//!     -> a name; the map, or load(name) -> a unit -> a new record   load(name)
//!   link: InnerModuleLinking (§16.2.1.5.1.1), InitializeEnvironment (§16.2.1.6.4)
//!   evaluate: Evaluate (§16.2.1.5.3): InnerModuleEvaluation; an error is cached in
//!     [[EvaluationError]]; the result is a promise ([[TopLevelCapability]])
//!   a microtask checkpoint (empty JS stack only), host in place: top-level await, import() in jobs
//!
//! import(s, o) (id 43, slow/module.rs) -> dynamic_import            the host of the run
//!   resolve + load (HostLoadImportedModule) -> load_graph -> a load promise
//!   job: link, evaluate -> job: GetModuleNamespace -> the import() promise
//!
//! exotic.rs hooks (get, own_property, own_attrs) ──────┐
//! property.rs hooks (set, define, delete, own keys) ───┴──> the namespace hook fns below
//! LdaModuleVariable / StaModuleVariable (ops.rs cold) ────> interp/slow/module.rs
//! ```
//!
//! # Records and environments
//!
//! | MODULE field | Value |
//! |---|---|
//! | `STATUS` | int32 [`status`]: new, unlinked, linking, linked, evaluating, evaluating-async, evaluated; and aborted (not in the spec: a termination stopped its Evaluate, or its pending async evaluation) |
//! | `ENVIRONMENT` | the module environment: a CONTEXT with the slots of [`mod@env`] (not on any context chain) |
//! | `NAMESPACE` | the namespace object, made on first use ([`get_namespace`]) |
//! | `UNIT` | native word: the module's `LoadedUnit` (its module records, ABI 5 section 5) |
//! | `REQUESTED` | ELEMENTS: the specifier of each ModuleRequest (an atom) |
//! | `LOADED` | ELEMENTS: the MODULE of each request (undefined until loaded) |
//! | `EVALUATION_ERROR` | the cached error of Evaluate; the hole for none |
//! | `DFS_INDEX`, `DFS_ANCESTOR_INDEX`, `CYCLE_ROOT` | Tarjan state of Link and Evaluate |
//! | `HAS_TLA` | `[[HasTLA]]`: true for a module whose body holds an await point (ABI 5 `has_tla`) |
//! | `ASYNC_EVALUATION_ORDER` | `[[AsyncEvaluationOrder]]`: undefined (unset), int32 >= 0 (the order), or [`ORDER_DONE`] |
//! | `TOP_LEVEL_CAPABILITY` | `[[TopLevelCapability]]`: the promise of Evaluate (the VM settles it; no resolve functions are made), or undefined |
//! | `ASYNC_PARENT_MODULES` | `[[AsyncParentModules]]`: an ELEMENTS list `[n, m1 .. mn]`, or undefined |
//! | `PENDING_ASYNC_DEPENDENCIES` | `[[PendingAsyncDependencies]]` (int32) |
//! | `HOST_DEFINED` | the record's `[[Realm]]`: the REALM whose module map holds it ([`realm_of`]) |
//!
//! A module binding that other modules can see (an export or an import) is a *cell*: the
//! front end numbers exported bindings `1, 2, ...` and imports `-1, -2, ...`
//! (`LdaModuleVariable cell, depth`). Each cell is a box (an ELEMENTS of length 1) in the
//! environment, made with the record; the hole in a box is a binding in its TDZ. A named
//! import shares the box of the binding it resolves to (a live binding); a namespace import
//! gets its own box with the namespace object.
//!
//! The entry function of the module unit (function 0) is resumable (M3-P2-MODULE-F0, the V8
//! form; the front end's `bytecodegen/module.rs` shows its code). Link makes its closure and a
//! generator object for it (`env::EVALUATE`) and resumes that from its start:
//! InitializeEnvironment's own part runs (the module's functions, `var` cells undefined,
//! TDZ holes) up to suspend id [`RUN`]. Evaluate resumes it once more for the statements.
//! The VM gives the generator object as the receiver (the entry function may not read
//! new.target, the ABI 5 verifier's entry-frame rule); the code makes `this` undefined.
//!
//! ```text
//! initialize_environment   imports bound; f = closure(function 0, context = MODULE)
//!                          g = GENERATOR(f, receiver g, context MODULE, state 0)
//!                          resume(g): table[0] -> declarations -> SuspendGenerator id 1
//! inner_evaluate           resume(g): table[1] -> ResumeGenerator -> statements -> Return
//! ```
//!
//! The closure context is the MODULE cell itself, so a function of the module reaches its
//! record by walking the context chain to its end (`module_of_context`). Other context
//! walkers stop at a value that is not a CONTEXT.
//!
//! Each realm has a module map (`realmf::HOST_DEFINED`: the realm's host data, where HTML
//! keeps its module map): the name the host gave a module -> its record. A name loads once
//! per realm.
//!
//! # Top-level await (M3-P3 IMPORT-TLA)
//!
//! Evaluate follows §16.2.1.5.3 with the async steps: a module with `[[HasTLA]]`, or with a
//! dependency that is still evaluating-async, gets an `[[AsyncEvaluationOrder]]`; such a
//! module runs through ExecuteAsyncModule (§16.2.1.5.3.2), and its parents wait in
//! `[[AsyncParentModules]]` until AsyncModuleExecutionFulfilled or ...Rejected
//! (§16.2.1.5.3.4-5) runs them or caches the error. The function 0 of a `[[HasTLA]]`
//! module is an async function body after suspend id [`RUN`] (the front end's module doc):
//! ExecuteModule with a capability makes the generator object an async function object (mode
//! `MODE_ASYNC_FUNCTION`, the capability's promise in `genf::ASYNC`) and resumes it, so its
//! awaits are the async function awaits (`interp/slow/async_fn.rs`). The lists that live
//! across a GC point are heap cells or scratch roots (plan rule 8).
//!
//! # The module host and `import()`
//!
//! `import()` needs the hooks of a host. [`with_host`] makes a [`Host`] the module host of
//! the isolate for the length of a call (a thread-local list, `HOSTS`); `Cx::run_module` does
//! it for its run, its loads and its checkpoint. With no host, `import()` throws the v0
//! not-implemented Error (as eval with no compiler hooks throws). `Isolate::with_module_host`
//! does it for an embedder's scripts, calls and checkpoints (isolate.rs).
//!
//! The referrer of `import()` is GetActiveScriptOrModule ([`active_script_or_module`]): the
//! module or script whose code runs, or for eval code and the functions it makes, the
//! ScriptOrModule when that code loaded ([`EvalReferrer`]). The module goes into the module
//! map of the referrer's realm, not of the realm that runs the call.

use core::ptr::NonNull;
use std::cell::RefCell;

use ep9_bytecode::{CodeKind, ExportImportName, ImportName, ModuleRecords, VerifiedUnit};
use ep9_heap::{RootStrength, Slot, Tracer};
use ep9_value::{JsResult, Throw};

use crate::abi::{self, Args, JsObject, JsValue, Local, PromiseState};
use crate::builtins::{IntrinsicId, intrinsic};
use crate::compiler::{CompileFailure, CompileResult};
use crate::conv::Res;
use crate::descriptor::Desc;
use crate::error::{FrameRef, not_implemented, throw_error};
use crate::exotic::probe;
use crate::frame::{
    FALSE, HOLE, NULL, TRUE, UNDEFINED, int32_bits, int32_of, native_of, native_ptr,
};
use crate::interp::Cx;
use crate::key::{Probe, is_symbol_key, key_eq};
use crate::kinds as kind;
use crate::object::{self, attr, genf, init, modf, nsf, realmf};
use crate::runtime::{ErrorKind, Root, Runtime, val};
use crate::unit::{FunctionInfo, LoadedUnit};
use crate::{atoms, conv, generator, jobs, property, string, symbol};

/// `[[Status]]` values (§16.2.1.5, Table 45), as int32.
pub(crate) mod status {
    pub const NEW: i32 = 0;
    pub const UNLINKED: i32 = 1;
    pub const LINKING: i32 = 2;
    pub const LINKED: i32 = 3;
    pub const EVALUATING: i32 = 4;
    pub const EVALUATING_ASYNC: i32 = 5;
    pub const EVALUATED: i32 = 6;
    /// Not a spec status: a termination (`TerminateHandle`, H23) stopped the module's
    /// Evaluate, or came while its async evaluation was pending (the termination drops the
    /// jobs that would settle it; `abort_async_modules`). The isolate is used again after a
    /// termination, so a later Evaluate of the module, or of a module that imports it, throws
    /// an Error; it never takes the module as evaluated (its body did not finish). Link
    /// takes it as evaluated.
    pub const ABORTED: i32 = 7;
}

/// The slots of a module environment (`modf::ENVIRONMENT`, a CONTEXT).
pub(crate) mod env {
    /// The name the host gave the module (an atom).
    pub const NAME: u32 = 0;
    /// The generator object of the unit's function 0 (made at InitializeEnvironment; it
    /// waits at suspend id [`super::RUN`] until Evaluate).
    pub const EVALUATE: u32 = 1;
    /// `import.meta` (`[[ImportMeta]]`), undefined until the first `GetImportMeta`.
    pub const IMPORT_META: u32 = 2;
    /// The first cell box ([`cell_slot`](super::cell_slot)).
    pub const CELLS: u32 = 3;
}

/// The suspend id at which function 0 waits between InitializeEnvironment and Evaluate (the
/// front end's `bytecodegen/module.rs` `RUN`): its jump table entry 1 starts the statements.
const RUN: i32 = 1;

/// `[[AsyncEvaluationOrder]]` done (§16.2.1.5: the module's async evaluation finished).
const ORDER_DONE: i32 = -1;

std::thread_local! {
    /// IncrementModuleAsyncEvaluationCount (§16.2.1.5.3.6): the next `[[AsyncEvaluationOrder]]`.
    /// Only the order of two values matters, so one counter per thread serves every isolate
    /// on it.
    static ASYNC_EVALUATION_COUNT: core::cell::Cell<i32> = const { core::cell::Cell::new(0) };
}

/// The environment slot of the box of module cell `cell` (not 0): exports and imports
/// alternate, so no count is needed.
fn cell_slot(cell: i32) -> u32 {
    let n = cell.unsigned_abs() - 1;
    env::CELLS + 2 * n + u32::from(cell < 0)
}

/// A new MODULE record: `[[EvaluationError]]` is the hole (none, H17); every other field is
/// undefined ([`new_module`] fills them). GC point.
pub(crate) fn new_module_record(rt: &Runtime) -> u64 {
    let m = object::new_record(rt, kind::MODULE);
    // No GC point between the allocation and this store.
    object::record_set(rt, m, object::modf::EVALUATION_ERROR, crate::frame::HOLE);
    m
}

/// A new MODULE_NAMESPACE object for `module` (a MODULE cell) with the export list
/// `exports`: prototype null, its in-object slot undefined. [`get_namespace`] makes the
/// namespaces of modules: it adds @@toStringTag and makes the object non-extensible. GC point.
pub(crate) fn new_namespace(rt: &Runtime, module: u64, exports: u64) -> u64 {
    let c = object::new_js_object(rt, kind::MODULE_NAMESPACE, NULL, nsf::CAPACITY);
    init(c, nsf::MODULE, module);
    init(c, nsf::EXPORTS, exports);
    c.to_value().to_bits()
}

// ---- record fields ----------------------------------------------------------------------------

fn status_of(m: u64) -> i32 {
    let s = object::record_get(m, modf::STATUS);
    if s == UNDEFINED {
        status::NEW
    } else {
        int32_of(s)
    }
}

fn set_status(rt: &Runtime, m: u64, s: i32) {
    object::record_set(rt, m, modf::STATUS, int32_bits(s));
}

fn get_i32(m: u64, field: usize) -> i32 {
    int32_of(object::record_get(m, field))
}

fn set_i32(rt: &Runtime, m: u64, field: usize, v: i32) {
    object::record_set(rt, m, field, int32_bits(v));
}

/// The module's unit.
fn unit_of<'a>(m: u64) -> &'a LoadedUnit {
    // SAFETY: `modf::UNIT` holds the native word of a boxed LoadedUnit (`new_module`), and the
    // isolate keeps every unit until it drops (`Runtime::units` only grows).
    unsafe { &*native_ptr::<LoadedUnit>(object::record_get(m, modf::UNIT)) }
}

/// The module records of the module's unit (ABI 5 section 5).
fn records_of<'a>(m: u64) -> &'a ModuleRecords {
    unit_of(m)
        .unit
        .module_records
        .as_ref()
        .expect("a module unit has module records (verifier)")
}

fn environment(m: u64) -> u64 {
    object::record_get(m, modf::ENVIRONMENT)
}

/// The record that request `r` of `m` loaded (undefined before loading).
fn loaded(m: u64, r: u32) -> u64 {
    object::list_get(object::record_get(m, modf::LOADED), r as usize)
}

/// `[[Realm]]` of `m`: the realm whose module map holds it (`modf::HOST_DEFINED`). Its
/// requests load into that map, and Link makes its functions in that realm.
fn realm_of(m: u64) -> u64 {
    object::record_get(m, modf::HOST_DEFINED)
}

/// The name the host gave `m` (an atom).
fn name_of(m: u64) -> u64 {
    object::context_get(environment(m), env::NAME).unwrap_or(UNDEFINED)
}

/// The box of cell `cell` of `m`.
fn box_of(m: u64, cell: i32) -> Option<u64> {
    let b = object::context_get(environment(m), cell_slot(cell))?;
    (kind::cell_kind(b) == kind::ELEMENTS.get()).then_some(b)
}

/// The value of cell `cell` of the module `m` (the hole: the binding is in its TDZ); `None`
/// for a cell the module does not have.
pub(crate) fn cell_get(m: u64, cell: i32) -> Option<u64> {
    box_of(m, cell).map(|b| object::list_get(b, 0))
}

/// Stores `v` into cell `cell` (an export) of `m`; false for a cell the module does not have.
pub(crate) fn cell_set(rt: &Runtime, m: u64, cell: i32, v: u64) -> bool {
    match box_of(m, cell) {
        Some(b) if cell > 0 => {
            object::list_set(rt, b, 0, v);
            true
        }
        _ => false,
    }
}

/// The MODULE record at the end of the context chain from `c` (`d` hops first: the
/// `LdaModuleVariable` depth); `None` if the chain does not end at one (code that is not
/// module code).
pub(crate) fn module_of_context(mut c: u64, d: u32) -> Option<u64> {
    for _ in 0..d {
        if !kind::is_context(c) {
            break;
        }
        c = object::context_previous(c);
    }
    while kind::is_context(c) {
        c = object::context_previous(c);
    }
    (kind::is_cell_bits(c) && c != 0 && kind::cell_kind(c) == kind::MODULE.get()).then_some(c)
}

/// The module cell that the local name `name` (an atom) binds in `c`, if `c` is a MODULE
/// record: (the record, an import's cell < 0 or an exported binding's cell > 0). For direct
/// eval code (`interp/slow/lookup.rs`): its name walk ends at the MODULE record, the outer
/// environment of the module's top-level scope (§16.2.1.6.4; the cells are the bindings of
/// the module Environment Record that other modules see). `None` for another name: a
/// binding that no other module sees is a context slot, which the walk finds before. Not a
/// GC point.
pub(crate) fn cell_of_name(c: u64, name: u64) -> Option<(u64, i32)> {
    if !(kind::is_cell_bits(c) && c != 0 && kind::cell_kind(c) == kind::MODULE.get()) {
        return None;
    }
    let lu = unit_of(c);
    let recs = records_of(c);
    let import = recs
        .import_entries
        .iter()
        .find(|e| lu.string(e.local_name.index()) == name)
        .map(|e| e.cell);
    let cell = import.or_else(|| {
        recs.local_export_entries
            .iter()
            .find(|e| lu.string(e.local_name.index()) == name)
            .map(|e| e.cell)
    })?;
    Some((c, cell))
}

/// The `import.meta` object of `m` (§13.3.12.1): made once, an ordinary object with a null
/// prototype and no properties (no host hook adds any yet). GC point.
pub(crate) fn import_meta(rt: &Runtime, m: u64) -> u64 {
    let env = environment(m);
    match object::context_get(env, env::IMPORT_META) {
        Some(o) if object::is_object(o) => o,
        _ => {
            let o = object::new_object(rt, NULL);
            object::context_set(rt, env, env::IMPORT_META, o);
            o
        }
    }
}

// ---- the module map of a realm ----------------------------------------------------------------
//
// `realmf::HOST_DEFINED`: an ELEMENTS list, entry 0 = the used count n, then n pairs
// (name atom, MODULE record).

fn map_get(realm: u64, name: u64) -> Option<u64> {
    let list = object::record_get(realm, realmf::HOST_DEFINED);
    if !kind::is_cell_bits(list) || list == 0 || kind::cell_kind(list) != kind::ELEMENTS.get() {
        return None;
    }
    let n = int32_of(object::list_get(list, 0)) as usize;
    (0..n)
        .find(|&i| object::list_get(list, 1 + 2 * i) == name)
        .map(|i| object::list_get(list, 2 + 2 * i))
}

/// Adds `name -> m` to the realm's map. GC point (the list grows): `realm`, `name` and `m`
/// live in Rust locals.
fn map_add(rt: &Runtime, realm: u64, name: u64, m: u64) {
    let mut list = object::record_get(realm, realmf::HOST_DEFINED);
    let n = if list == UNDEFINED {
        0
    } else {
        int32_of(object::list_get(list, 0)) as usize
    };
    if list == UNDEFINED || 1 + 2 * (n + 1) > object::list_len(list) {
        let bigger = object::new_list(rt, 1 + 2 * (2 * n + 4), UNDEFINED);
        if list != UNDEFINED {
            for i in 1..1 + 2 * n {
                object::list_set(rt, bigger, i, object::list_get(list, i));
            }
        }
        object::record_set(rt, realm, realmf::HOST_DEFINED, bigger);
        list = bigger;
    }
    object::list_set(rt, list, 1 + 2 * n, name);
    object::list_set(rt, list, 2 + 2 * n, m);
    object::list_set(rt, list, 0, int32_bits((n + 1) as i32));
}

// ---- loading ----------------------------------------------------------------------------------

/// Loads `unit` into the isolate (a new entry of `Runtime::units`, never freed), as
/// `Cx::run_script` does. GC point.
fn load_unit<'a>(rt: &Runtime, unit: VerifiedUnit, file: &str) -> &'a LoadedUnit {
    let index = u32::try_from(rt.units.borrow().len()).expect("fewer than 2^32 units");
    let lu = LoadedUnit::load(rt, unit, file.to_string(), index);
    let p: *const LoadedUnit = &*lu;
    rt.units.borrow_mut().push(lu);
    // SAFETY: the box does not move and lives until the isolate drops.
    unsafe { &*p }
}

/// A new module record for the module unit `lu` named `name` (an atom), status new
/// (ParseModule's record, §16.2.1.6.1): its requests, and the environment with a box in the
/// hole for every cell. GC point: `name` lives in a Rust local, the unit is rooted.
fn new_module(rt: &Runtime, lu: &LoadedUnit, name: u64) -> u64 {
    let recs = lu
        .unit
        .module_records
        .as_ref()
        .expect("a module unit has module records (verifier)");
    let exports = recs
        .local_export_entries
        .iter()
        .map(|e| e.cell)
        .filter(|&c| c > 0)
        .map(i32::unsigned_abs)
        .max()
        .unwrap_or(0);
    let imports = recs
        .import_entries
        .iter()
        .map(|e| e.cell)
        .chain(recs.local_export_entries.iter().map(|e| e.cell))
        .filter(|&c| c < 0)
        .map(i32::unsigned_abs)
        .max()
        .unwrap_or(0);
    let m = new_module_record(rt);
    set_status(rt, m, status::NEW);
    let tla = if recs.has_tla { TRUE } else { FALSE };
    object::record_set(rt, m, modf::HAS_TLA, tla);
    object::record_set(rt, m, modf::UNIT, native_of(core::ptr::from_ref(lu)));
    let n = recs.requested_modules.len();
    let requested = object::new_list(rt, n, UNDEFINED);
    for (i, r) in recs.requested_modules.iter().enumerate() {
        object::list_set(rt, requested, i, lu.string(r.specifier.index()));
    }
    object::record_set(rt, m, modf::REQUESTED, requested);
    let loaded = object::new_list(rt, n, UNDEFINED);
    object::record_set(rt, m, modf::LOADED, loaded);
    let len = env::CELLS as usize + 2 * exports.max(imports) as usize;
    let e = object::new_context(rt, UNDEFINED, &raw const lu.context_scopes[0], len);
    object::context_set(rt, e, env::NAME, name);
    object::record_set(rt, m, modf::ENVIRONMENT, e);
    for c in 1..=exports {
        let b = object::new_list(rt, 1, HOLE);
        object::context_set(rt, e, cell_slot(c as i32), b);
    }
    for c in 1..=imports {
        let b = object::new_list(rt, 1, HOLE);
        object::context_set(rt, e, cell_slot(-(c as i32)), b);
    }
    m
}

/// The import attributes of request `r` of `m` (HostGetSupportedImportAttributes: `type`
/// only). `type: "json"` (JSON modules) is not built; another `type` is a TypeError, another
/// key a SyntaxError (§16.2.1.3.1, AllImportAttributesSupported).
fn check_attributes(cx: &mut Cx, m: u64, r: u32) -> Res<()> {
    let lu = unit_of(m);
    let req = &records_of(m).requested_modules[r as usize];
    let attributes: Vec<(String, String)> = req
        .attributes
        .iter()
        .map(|&(k, v)| {
            (
                string::to_rust_string(lu.string(k.index())),
                string::to_rust_string(lu.string(v.index())),
            )
        })
        .collect();
    check_attribute_list(cx, &attributes)
}

/// The checks of [`check_attributes`] on a list of (key, value) pairs, sorted by key: a key
/// other than `type` is a SyntaxError; `type: "json"` the not-implemented Error; another
/// type a TypeError.
fn check_attribute_list(cx: &mut Cx, attributes: &[(String, String)]) -> Res<()> {
    for (key, _) in attributes {
        if key != "type" {
            let msg = format!("Import attribute \"{key}\" is not supported");
            return Err(throw_error(cx, ErrorKind::SyntaxError, &msg));
        }
    }
    if let Some((_, value)) = attributes.first() {
        if value == "json" {
            return Err(not_implemented(cx, "JSON modules"));
        }
        let msg = format!("Import attribute type \"{value}\" is not supported");
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    Ok(())
}

/// The error a failed `load` hook throws (as eval code's compile failures, `abi/script.rs`).
fn throw_load_failure(cx: &mut Cx, name: &str, f: CompileFailure) -> ep9_value::Throw {
    match f {
        CompileFailure::Syntax(m) => throw_error(cx, ErrorKind::SyntaxError, &m),
        CompileFailure::Range(m) => throw_error(cx, ErrorKind::RangeError, &m),
        CompileFailure::Unsupported(what) => not_implemented(cx, &format!("unsupported: {what}")),
        CompileFailure::Internal(m) => throw_error(
            cx,
            ErrorKind::Error,
            &format!("ep9 internal error: module {name}: {m}"),
        ),
    }
}

/// The record of the module named `name` in the module map of `realm`, made from `unit` if
/// the map has none (its `[[Realm]]` is `realm`). GC point.
fn module_for_unit(cx: &mut Cx, realm: u64, unit: VerifiedUnit, name: &str) -> Res<u64> {
    if unit.header.code_kind != CodeKind::Module {
        let msg = format!("{name} is not a module unit");
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let rt = cx.rt();
    let key = atoms::intern_str(rt, name);
    if let Some(m) = map_get(realm, key) {
        return Ok(m);
    }
    let lu = load_unit(rt, unit, name);
    let m = new_module(rt, lu, key);
    object::record_set(rt, m, modf::HOST_DEFINED, realm);
    map_add(rt, realm, key, m);
    Ok(m)
}

/// The host hooks of one load: `resolve(referrer, specifier)` gives the name of the module
/// (an `Err` text is thrown as an Error); `load(name)` compiles it.
pub(crate) struct Host<'h> {
    pub resolve: &'h mut dyn FnMut(&str, &str) -> Result<String, String>,
    pub load: &'h mut dyn FnMut(&str) -> CompileResult,
}

std::thread_local! {
    /// The module hosts in use on this thread, innermost last: (the isolate's key,
    /// [`host_key`]; its host). [`with_host`] adds an entry for the length of one call and
    /// removes it after, also on a panic, so an entry never outlives its host. The key is
    /// `Runtime::id`, which no other isolate gets, also not one made after this one drops
    /// at the same address (code in `Isolate::with_module_host`'s call can drop its isolate).
    /// Holds no Value.
    static HOSTS: RefCell<Vec<(u64, *mut Host<'static>)>> = const { RefCell::new(Vec::new()) };
}

/// The key of `rt`'s isolate in `HOSTS`: `Runtime::id`.
pub(crate) fn host_key(rt: &Runtime) -> u64 {
    rt.id
}

/// Runs `f` with `host` as the module host of the isolate with key `key` ([`host_key`]): the
/// hooks of its loads and of `import()` while `f` runs. Calls nest: the innermost host of an
/// isolate is the one in use. `Cx::run_module` uses it for its run, and
/// `Isolate::with_module_host` (isolate.rs) for an embedder's scripts and checkpoints.
pub(crate) fn with_host<R>(key: u64, host: &mut Host<'_>, f: impl FnOnce() -> R) -> R {
    /// Removes the entry of its key that it added (the innermost one) when it drops.
    struct Pop(u64);
    impl Drop for Pop {
        fn drop(&mut self) {
            HOSTS.with_borrow_mut(|h| {
                if let Some(i) = h.iter().rposition(|e| e.0 == self.0) {
                    h.remove(i);
                }
            });
        }
    }
    let p: *mut Host<'_> = host;
    // The entry is removed (by `Pop`) before the borrow of `host` ends; only `call_host`
    // reads it, while it is in the list.
    HOSTS.with_borrow_mut(|h| h.push((key, p.cast::<Host<'static>>())));
    let _pop = Pop(key);
    f()
}

/// Calls `f` with the innermost module host of the isolate with key `key` ([`host_key`]);
/// `None` if it has none.
pub(crate) fn call_host<R>(key: u64, f: impl FnOnce(&mut Host<'_>) -> R) -> Option<R> {
    let p = HOSTS.with_borrow(|h| h.iter().rev().find(|e| e.0 == key).map(|e| e.1))?;
    // SAFETY: `p` is the host of a `with_host` call that is still running on this thread
    // (its entry is in the list), so the host lives. Nothing else uses it now: `with_host`
    // gave its `&mut` to the list, and the hooks get no `Cx`, so they cannot reach the list.
    Some(f(unsafe { &mut *p }))
}

/// True if `rt`'s isolate has a module host now.
pub(crate) fn has_host(rt: &Runtime) -> bool {
    let key = host_key(rt);
    HOSTS.with_borrow(|h| h.iter().any(|e| e.0 == key))
}

/// `resolve(referrer, specifier)` of the host of `cx`'s isolate: the module's name, or the
/// thrown Error.
fn host_resolve(cx: &mut Cx, referrer: &str, specifier: &str) -> Res<String> {
    match call_host(host_key(cx.rt()), |h| (h.resolve)(referrer, specifier)) {
        Some(Ok(name)) => Ok(name),
        Some(Err(msg)) => Err(throw_error(cx, ErrorKind::Error, &msg)),
        None => Err(not_implemented(cx, "modules without a module host")),
    }
}

/// The record of the module named `name` in the module map of `realm` (the referrer's
/// realm, so one referrer and name give one module, HostLoadImportedModule §16.2.1.10, also
/// when code of another realm asks): the map's, or a new one from the host's `load(name)`
/// (the thrown error of a failed load). GC point.
fn host_module(cx: &mut Cx, realm: u64, name: &str) -> Res<u64> {
    let key = atoms::intern_str(cx.rt(), name);
    if let Some(t) = map_get(realm, key) {
        return Ok(t);
    }
    let unit = match call_host(host_key(cx.rt()), |h| (h.load)(name)) {
        Some(Ok(unit)) => unit,
        Some(Err(f)) => return Err(throw_load_failure(cx, name, f)),
        None => return Err(not_implemented(cx, "modules without a module host")),
    };
    module_for_unit(cx, realm, unit, name)
}

/// LoadRequestedModules (§16.2.1.5.1) with a synchronous host (the module host of the
/// isolate, [`with_host`]): every module that `root` needs, transitively, gets a record in
/// the realm's map and its `[[LoadedModules]]` entries. GC point.
///
/// InnerModuleLoading (§16.2.1.5.1.1) step 2: every reachable module that is still `new` is
/// walked, also one that is in the map already. A failed load leaves such records: a module
/// whose requests did not all load. A module that is not `new` has a fully loaded graph
/// (it became `unlinked` only with the whole graph of its load), so the walk stops there.
/// A module is marked `unlinked` when its own requests are loaded, which is also the
/// visited mark of the walk; if the load fails, every module marked by this call is `new`
/// again (ContinueModuleLoading marks the visited modules only when the whole graph loaded).
/// These records stay in the realm's map as retryable `new` records, and no failed completion
/// is kept (H22-Q3). A later load calls `resolve` again for each request that has no module
/// yet (an empty `[[LoadedModules]]` entry), and `load` only when the realm's map has no
/// module of that name. The same error comes again only if the host gives it again
/// (HostLoadImportedModule, §16.2.1.10, needs the same result again only for a load that
/// succeeded).
fn load_graph(cx: &mut Cx, root: u64) -> Res<()> {
    let rt = cx.rt();
    // The modules this call walks (a module can be in it twice before it is walked).
    let work = rt.heap.scratch_roots();
    work.push(val(root));
    let mut next = 0;
    let r = (|| -> Res<()> {
        while next < work.len() {
            let m = work.get(next).to_bits();
            next += 1;
            if status_of(m) != status::NEW {
                continue;
            }
            let referrer = string::to_rust_string(name_of(m));
            let n = object::list_len(object::record_get(m, modf::REQUESTED));
            for r in 0..n as u32 {
                let mut target = loaded(m, r);
                if target == UNDEFINED {
                    check_attributes(cx, m, r)?;
                    let spec = object::list_get(object::record_get(m, modf::REQUESTED), r as usize);
                    let spec = string::to_rust_string(spec);
                    let name = host_resolve(cx, &referrer, &spec)?;
                    target = host_module(cx, realm_of(m), &name)?;
                    object::list_set(rt, object::record_get(m, modf::LOADED), r as usize, target);
                }
                if status_of(target) == status::NEW {
                    work.push(val(target));
                }
            }
            set_status(rt, m, status::UNLINKED);
        }
        Ok(())
    })();
    if r.is_err() {
        for i in 0..next {
            set_status(rt, work.get(i).to_bits(), status::NEW);
        }
    }
    r
}

// ---- ResolveExport and GetExportedNames (no GC point) ----------------------------------------
//
// Both walks recurse once per (module, name) pair or module, which a shallow graph can make
// deep (`export { a as b } from` around a cycle of `export *`). Each level checks the native
// stack and gives `TooDeep`, which allocates nothing; the caller throws the RangeError after
// the walk's vectors are gone (H10: no Value in a Rust vector across a GC point).

/// The native stack is too short for a ResolveExport or GetExportedNames walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TooDeep;

/// `Err(TooDeep)` once the native stack passes the isolate's limit. Not a GC point.
fn walk_stack_ok(rt: &Runtime) -> Result<(), TooDeep> {
    if rt.native_stack_ok() {
        Ok(())
    } else {
        Err(TooDeep)
    }
}

/// The RangeError of a walk that was [`TooDeep`]. GC point.
fn too_deep(cx: &mut Cx) -> ep9_value::Throw {
    throw_error(
        cx,
        ErrorKind::RangeError,
        "Maximum call stack size exceeded",
    )
}

/// A ResolvedBinding Record (§16.2.1.6.3), or null, or `ambiguous`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolution {
    None,
    Ambiguous,
    /// `{ [[Module]], [[BindingName]] }`: the binding is cell `cell` of the module.
    Binding(u64, i32),
    /// `{ [[Module]], [[BindingName]]: namespace }`.
    Namespace(u64),
}

/// True for the name "default". Not a GC point.
fn is_default(name: u64) -> bool {
    string::equals_text(name, string::JsStr::Latin1(b"default"))
}

/// `m.ResolveExport(name, resolveSet)` (§16.2.1.6.3). `name` is an atom; module names are unit
/// atoms, so names compare by bits. Not a GC point: `set` holds cells only while nothing
/// allocates. [`TooDeep`] when the native stack runs out.
fn resolve_export(
    rt: &Runtime,
    m: u64,
    name: u64,
    set: &mut Vec<(ModuleCell, NameAtom)>,
) -> Result<Resolution, TooDeep> {
    walk_stack_ok(rt)?;
    if m == UNDEFINED {
        return Ok(Resolution::None);
    }
    let entry = (ModuleCell(m), NameAtom(name));
    if set.contains(&entry) {
        // A circular import request.
        return Ok(Resolution::None);
    }
    set.push(entry);
    let lu = unit_of(m);
    let recs = records_of(m);
    for e in &recs.local_export_entries {
        if lu.string(e.export_name.index()) == name {
            // A re-export of an imported namespace (`import * as ns; export { ns }`) is
            // the namespace binding of that module: ParseModule (§16.2.1.7.1 step 10 in the
            // ES2027 draft) rewrites the entry as an indirect export of the imported
            // module with [[ImportName]] namespace, and ResolveExport gives { that module,
            // namespace }. So two modules that re-export it agree, and `export *` of both
            // is not ambiguous (test262 ambiguous-export-bindings/namespace-unambiguous-*).
            // ES2025 and node v24 kept it a local export (ambiguous): ep9 follows the
            // current draft (G1: ES2026, ES2027 so far). The front end keeps it a local
            // export entry of the import's cell (ABI 5), so the rewrite is here.
            if e.cell < 0
                && let Some(ie) = recs.import_entries.iter().find(|ie| ie.cell == e.cell)
                && ie.import_name == ImportName::Namespace
            {
                return Ok(Resolution::Namespace(loaded(m, ie.module_request)));
            }
            return Ok(Resolution::Binding(m, e.cell));
        }
    }
    for e in &recs.indirect_export_entries {
        if lu.string(e.export_name.index()) == name {
            let target = loaded(m, e.module_request);
            return match e.import_name {
                ExportImportName::All => Ok(Resolution::Namespace(target)),
                ExportImportName::Name(n) => resolve_export(rt, target, lu.string(n.index()), set),
            };
        }
    }
    if is_default(name) {
        // A default export cannot come from `export *`.
        return Ok(Resolution::None);
    }
    let mut star = Resolution::None;
    for &r in &recs.star_export_entries {
        match resolve_export(rt, loaded(m, r), name, set)? {
            Resolution::Ambiguous => return Ok(Resolution::Ambiguous),
            Resolution::None => {}
            res => {
                if star == Resolution::None {
                    star = res;
                } else if star != res {
                    return Ok(Resolution::Ambiguous);
                }
            }
        }
    }
    Ok(star)
}

/// `m.GetExportedNames(exportStarSet)` (§16.2.1.6.2) into `out` (atoms). Not a GC point.
/// [`TooDeep`] when the native stack runs out.
fn exported_names(
    rt: &Runtime,
    m: u64,
    set: &mut Vec<ModuleCell>,
    out: &mut Vec<NameAtom>,
) -> Result<(), TooDeep> {
    walk_stack_ok(rt)?;
    if m == UNDEFINED || set.contains(&ModuleCell(m)) {
        return Ok(());
    }
    set.push(ModuleCell(m));
    let lu = unit_of(m);
    let recs = records_of(m);
    let start = out.len();
    for e in &recs.local_export_entries {
        out.push(NameAtom(lu.string(e.export_name.index())));
    }
    for e in &recs.indirect_export_entries {
        out.push(NameAtom(lu.string(e.export_name.index())));
    }
    let mut star_names = Vec::new();
    for &r in &recs.star_export_entries {
        star_names.clear();
        exported_names(rt, loaded(m, r), set, &mut star_names)?;
        for n in &star_names {
            if !is_default(n.0) && !out[start..].contains(n) {
                out.push(*n);
            }
        }
    }
    Ok(())
}

/// An export name: a unit string atom, rooted by its unit. Held in Rust vectors only between
/// GC points (rule 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NameAtom(u64);

/// A MODULE cell, rooted by the realm's module map. Held in Rust vectors only between GC
/// points (rule 8: ResolveExport's resolveSet, GetExportedNames' exportStarSet and
/// GatherAvailableAncestors' lists).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModuleCell(pub(crate) u64);

/// The names of the namespace of `m` (§16.2.1.10 GetModuleNamespace step 3: the exported
/// names whose ResolveExport is a binding), sorted by code units (§10.4.6.12 step 7), each
/// with its binding. Not a GC point. [`TooDeep`] when the native stack runs out.
fn namespace_entries(rt: &Runtime, m: u64) -> Result<Vec<(NameAtom, Resolution)>, TooDeep> {
    let mut names = Vec::new();
    exported_names(rt, m, &mut Vec::new(), &mut names)?;
    let mut out = Vec::with_capacity(names.len());
    for n in names {
        match resolve_export(rt, m, n.0, &mut Vec::new())? {
            Resolution::None | Resolution::Ambiguous => {}
            res => out.push((n, res)),
        }
    }
    out.sort_by(|a, b| string::compare(a.0.0, b.0.0));
    out.dedup_by(|a, b| a.0 == b.0);
    Ok(out)
}

// ---- namespaces -------------------------------------------------------------------------------

/// GetModuleNamespace(m) (§16.2.1.10): made once, then kept in `[[Namespace]]`. The export
/// list (`nsf::EXPORTS`) holds the n sorted names, then n bindings: the box of the binding,
/// or the MODULE whose namespace the name is (`export * as ns`). A RangeError when the
/// native stack is too short for the walk. GC point.
pub(crate) fn get_namespace(cx: &mut Cx, m: u64) -> Res<u64> {
    let rt = cx.rt();
    let ns = object::record_get(m, modf::NAMESPACE);
    if ns != UNDEFINED {
        return Ok(ns);
    }
    let Ok(entries) = namespace_entries(rt, m) else {
        return Err(too_deep(cx));
    };
    let n = entries.len();
    drop(entries);
    let list = object::new_list(rt, 2 * n, UNDEFINED);
    let tag = atoms::intern_str(rt, "Module");
    // No GC point from here to the fill's end: the names and bindings are read again (the
    // same walk from the same frame as the first one, so it passes the stack check too).
    let Ok(entries) = namespace_entries(rt, m) else {
        return Err(too_deep(cx));
    };
    for (i, (name, res)) in entries.into_iter().enumerate() {
        let binding = match res {
            Resolution::Binding(t, cell) => box_of(t, cell).unwrap_or(UNDEFINED),
            Resolution::Namespace(t) => t,
            Resolution::None | Resolution::Ambiguous => UNDEFINED,
        };
        object::list_set(rt, list, i, name.0);
        object::list_set(rt, list, n + i, binding);
    }
    let ns = new_namespace(rt, m, list);
    let key = symbol::well_known(rt, symbol::WellKnownSymbol::ToStringTag);
    if kind::is_symbol(key) {
        object::own_define(rt, ns, key, tag, 0);
    }
    property::prevent_extensions(rt, ns);
    object::record_set(rt, m, modf::NAMESPACE, ns);
    Ok(ns)
}

/// The index of the export `key` of the namespace `o`, if `key` is a string that it exports.
fn export_index(o: u64, key: u64, how: Probe) -> Option<usize> {
    if is_symbol_key(key) {
        return None;
    }
    let list = object::rd(object::cell(o), nsf::EXPORTS);
    if !kind::is_cell_bits(list) || list == 0 || kind::cell_kind(list) != kind::ELEMENTS.get() {
        return None;
    }
    let n = object::list_len(list) / 2;
    (0..n).find(|&i| {
        let name = object::list_get(list, i);
        match how {
            Probe::Bits => name == key,
            Probe::Content => key_eq(name, key),
        }
    })
}

/// The value of export `i` of the namespace `o` (§10.4.6.8 steps 3-14): a ReferenceError for
/// a binding in its TDZ. GC point (a nested namespace is made on first use).
fn export_value(cx: &mut Cx, o: u64, i: usize) -> Res<u64> {
    let list = object::rd(object::cell(o), nsf::EXPORTS);
    let n = object::list_len(list) / 2;
    let b = object::list_get(list, n + i);
    if kind::is_cell_bits(b) && b != 0 && kind::cell_kind(b) == kind::MODULE.get() {
        return get_namespace(cx, b);
    }
    if !(kind::is_cell_bits(b) && b != 0 && kind::cell_kind(b) == kind::ELEMENTS.get()) {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: a namespace export without a binding",
        ));
    }
    let v = object::list_get(b, 0);
    if v == HOLE {
        let name = string::to_rust_string(object::list_get(list, i));
        let msg = format!("Cannot access '{name}' before initialization");
        return Err(throw_error(cx, ErrorKind::ReferenceError, &msg));
    }
    Ok(v)
}

/// The attributes of an export: writable, enumerable, not configurable (§10.4.6.5 step 6).
const EXPORT_ATTRS: i32 = attr::WRITABLE | attr::ENUMERABLE;

// ---- namespace exotic hooks (`exotic.rs`): `None` = no exotic own property ----

/// `[[Get]]` of an export (§10.4.6.8): `Some(Err)` for a binding in its TDZ (a
/// ReferenceError). A symbol, or a string that is not exported: `None` (OrdinaryGet finds
/// @@toStringTag, or nothing: the prototype is null).
pub(crate) fn get(cx: &mut Cx, o: u64, key: u64, how: Probe) -> Option<Res<u64>> {
    probe::note("module get");
    let i = export_index(o, key, how)?;
    Some(export_value(cx, o, i))
}

/// `[[GetOwnProperty]]` of an export (§10.4.6.5): (value, attribute bits); `Some(Err)` for a
/// binding in its TDZ (step 4 reads it).
pub(crate) fn own_property(cx: &mut Cx, o: u64, key: u64, how: Probe) -> Option<Res<(u64, i32)>> {
    probe::note("module own_property");
    let i = export_index(o, key, how)?;
    Some(export_value(cx, o, i).map(|v| (v, EXPORT_ATTRS)))
}

/// The attributes of an export, without reading its binding (HasProperty, §10.4.6.7). Not a
/// GC point.
pub(crate) fn own_attrs(_rt: &Runtime, o: u64, key: u64, how: Probe) -> Option<i32> {
    probe::note("module own_attrs");
    export_index(o, key, how).map(|_| EXPORT_ATTRS)
}

// ---- namespace internal-method hooks (`property.rs`): `None` = the ordinary method ----

/// `[[Set]]` (§10.4.6.9): always false, also when the namespace is a prototype of the
/// receiver (M3-P0-CXVM2-MODULE: the spec, JSC and Brimstone answer false; node v24 and
/// QuickJS store on the receiver).
pub(crate) fn set(_cx: &mut Cx, _o: u64, _key: u64, _v: u64, _receiver: u64) -> Option<Res<bool>> {
    probe::note("module set");
    Some(Ok(false))
}

/// `[[DefineOwnProperty]]` (§10.4.6.6): a symbol key is OrdinaryDefineOwnProperty; an
/// export accepts only a descriptor that changes nothing.
pub(crate) fn define_own_property(cx: &mut Cx, o: u64, key: u64, desc: Desc) -> Option<Res<bool>> {
    probe::note("module define_own_property");
    if is_symbol_key(key) {
        return None;
    }
    let how = crate::key::probe(key);
    let Some(i) = export_index(o, key, how) else {
        return Some(Ok(false));
    };
    let current = match export_value(cx, o, i) {
        Ok(v) => v,
        Err(e) => return Some(Err(e)),
    };
    let ok = desc.configurable != Some(true)
        && desc.enumerable != Some(false)
        && !desc.is_accessor()
        && desc.writable != Some(false)
        && desc.value.is_none_or(|v| property::same_value(v, current));
    Some(Ok(ok))
}

/// `[[Delete]]` (§10.4.6.10): a symbol key is OrdinaryDelete; an export cannot be deleted;
/// any other string gives true.
pub(crate) fn delete(_cx: &mut Cx, o: u64, key: u64) -> Option<Res<bool>> {
    probe::note("module delete");
    if is_symbol_key(key) {
        return None;
    }
    let how = crate::key::probe(key);
    Some(Ok(export_index(o, key, how).is_none()))
}

/// `[[OwnPropertyKeys]]` (§10.4.6.11) as a new array: the exports in their sorted order, then
/// the symbol keys (OrdinaryOwnPropertyKeys: @@toStringTag). GC point.
pub(crate) fn own_property_keys(cx: &mut Cx, o: u64) -> Option<u64> {
    probe::note("module own_property_keys");
    let rt = cx.rt();
    let keys = property::ordinary_own_property_keys(cx, o);
    let list = object::rd(object::cell(o), nsf::EXPORTS);
    let n = if kind::is_cell_bits(list) && list != 0 {
        object::list_len(list) / 2
    } else {
        0
    };
    let out = object::new_array(rt, 0);
    let mut len = 0;
    for i in 0..n {
        object::array_set(rt, out, len, object::list_get(list, i));
        len += 1;
    }
    for j in 0..object::array_len(keys) {
        if let Some(k) = object::array_get(keys, j)
            && is_symbol_key(k)
        {
            object::array_set(rt, out, len, k);
            len += 1;
        }
    }
    Some(out)
}

// ---- Link (§16.2.1.5.1) -----------------------------------------------------------------------

/// A stack of MODULE cells in a scratch root list (its first `len` entries).
struct Stack<'r> {
    roots: ep9_heap::ScratchRoots<'r>,
    len: usize,
}

impl Stack<'_> {
    fn push(&mut self, m: u64) {
        if self.len < self.roots.len() {
            self.roots.set(self.len, val(m));
        } else {
            self.roots.push(val(m));
        }
        self.len += 1;
    }

    fn pop(&mut self) -> u64 {
        self.len -= 1;
        self.roots.get(self.len).to_bits()
    }

    fn get(&self, i: usize) -> u64 {
        self.roots.get(i).to_bits()
    }
}

/// The specifier of request `r` of `m`, for messages.
fn specifier_text(m: u64, r: u32) -> String {
    string::to_rust_string(object::list_get(
        object::record_get(m, modf::REQUESTED),
        r as usize,
    ))
}

/// The SyntaxError of an import or indirect export that does not resolve.
fn unresolved(cx: &mut Cx, m: u64, r: u32, name: u64, res: Resolution) -> ep9_value::Throw {
    let spec = specifier_text(m, r);
    let name = string::to_rust_string(name);
    let msg = if res == Resolution::Ambiguous {
        format!("The requested module '{spec}' contains conflicting star exports for name '{name}'")
    } else {
        format!("The requested module '{spec}' does not provide an export named '{name}'")
    };
    throw_error(cx, ErrorKind::SyntaxError, &msg)
}

/// InitializeEnvironment (§16.2.1.6.4): checks the indirect exports and binds the imports
/// (steps 1-20), then runs the first part of the unit's function 0 (steps 21-24: the `var`
/// cells, the TDZ holes, the functions) up to its suspend at [`RUN`]. GC point; runs no user
/// code (an interrupt or a full stack can throw).
fn initialize_environment(cx: &mut Cx, m: u64) -> Res<()> {
    let rt = cx.rt();
    let lu = unit_of(m);
    let recs = records_of(m);
    for e in &recs.indirect_export_entries {
        let name = lu.string(e.export_name.index());
        let Ok(res) = resolve_export(rt, m, name, &mut Vec::new()) else {
            return Err(too_deep(cx));
        };
        if matches!(res, Resolution::None | Resolution::Ambiguous) {
            return Err(unresolved(cx, m, e.module_request, name, res));
        }
    }
    for ie in &recs.import_entries {
        let target = loaded(m, ie.module_request);
        let res = match ie.import_name {
            ImportName::Namespace => Resolution::Namespace(target),
            ImportName::Name(n) => {
                let name = lu.string(n.index());
                let Ok(res) = resolve_export(rt, target, name, &mut Vec::new()) else {
                    return Err(too_deep(cx));
                };
                if matches!(res, Resolution::None | Resolution::Ambiguous) {
                    return Err(unresolved(cx, m, ie.module_request, name, res));
                }
                res
            }
        };
        match res {
            Resolution::Namespace(t) => {
                let ns = get_namespace(cx, t)?;
                let b = box_of(m, ie.cell).expect("an import cell has a box");
                object::list_set(rt, b, 0, ns);
            }
            Resolution::Binding(t, cell) => {
                let b = box_of(t, cell).expect("an export cell has a box");
                object::context_set(rt, environment(m), cell_slot(ie.cell), b);
            }
            Resolution::None | Resolution::Ambiguous => unreachable!("checked above"),
        }
    }
    let entry: *const FunctionInfo = &unit_of(m).infos[0];
    // SAFETY: the unit lives until the isolate drops (`unit_of`).
    let fi = unsafe { &*entry };
    let not_resumable = |cx: &mut Cx| {
        throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: module function 0 is not resumable",
        )
    };
    if !fi.is_async_function() {
        // A unit from before M3-P2-MODULE-F0: its body would run here.
        return Err(not_resumable(cx));
    }
    // GC points: the closure, then the generator; the values live in Rust locals (CSS).
    let f = object::new_function(rt, entry, m);
    let n = 1 + fi.parameter_count as usize + fi.register_count as usize;
    let g = object::new_generator(rt, NULL, f, UNDEFINED, m, genf::MODE_SYNC, n);
    object::gen_set(rt, g, genf::RECEIVER, g);
    object::gen_set(rt, g, genf::STATE, int32_bits(0));
    object::context_set(rt, environment(m), env::EVALUATE, g);
    generator::resume(cx, g, genf::RESUME_NEXT, UNDEFINED, "next")?;
    if object::gen_get_i32(g, genf::STATE) != RUN {
        return Err(not_resumable(cx));
    }
    Ok(())
}

/// The RangeError of a module graph too deep for the native stack.
fn check_stack(cx: &mut Cx) -> Res<()> {
    if cx.rt().native_stack_ok() {
        Ok(())
    } else {
        Err(throw_error(
            cx,
            ErrorKind::RangeError,
            "Maximum call stack size exceeded",
        ))
    }
}

/// InnerModuleLinking (§16.2.1.5.1.1).
fn inner_link(cx: &mut Cx, m: u64, stack: &mut Stack<'_>, index: i32) -> Res<i32> {
    check_stack(cx)?;
    let rt = cx.rt();
    match status_of(m) {
        status::LINKING
        | status::LINKED
        | status::EVALUATING
        | status::EVALUATING_ASYNC
        | status::EVALUATED
        | status::ABORTED => return Ok(index),
        status::UNLINKED => {}
        _ => {
            return Err(throw_error(
                cx,
                ErrorKind::Error,
                "ep9 internal error: a module is linked before it is loaded",
            ));
        }
    }
    set_status(rt, m, status::LINKING);
    set_i32(rt, m, modf::DFS_INDEX, index);
    set_i32(rt, m, modf::DFS_ANCESTOR_INDEX, index);
    let mut index = index + 1;
    stack.push(m);
    let n = object::list_len(object::record_get(m, modf::LOADED));
    for r in 0..n as u32 {
        let required = loaded(m, r);
        index = inner_link(cx, required, stack, index)?;
        if status_of(required) == status::LINKING {
            let a = get_i32(m, modf::DFS_ANCESTOR_INDEX)
                .min(get_i32(required, modf::DFS_ANCESTOR_INDEX));
            set_i32(rt, m, modf::DFS_ANCESTOR_INDEX, a);
        }
    }
    initialize_environment(cx, m)?;
    if get_i32(m, modf::DFS_ANCESTOR_INDEX) == get_i32(m, modf::DFS_INDEX) {
        loop {
            let done = stack.pop();
            set_status(rt, done, status::LINKED);
            if done == m {
                break;
            }
        }
    }
    Ok(index)
}

/// Link() (§16.2.1.5.1): on an error, every module on the stack is unlinked again. GC point.
fn link(cx: &mut Cx, root: u64) -> Res<()> {
    let rt = cx.rt();
    let mut stack = Stack {
        roots: rt.heap.scratch_roots(),
        len: 0,
    };
    match inner_link(cx, root, &mut stack, 0) {
        Ok(_) => Ok(()),
        Err(e) => {
            for i in 0..stack.len {
                let m = stack.get(i);
                set_status(rt, m, status::UNLINKED);
            }
            Err(e)
        }
    }
}

// ---- Evaluate (§16.2.1.5.3) -------------------------------------------------------------------

/// Throws the cached `[[EvaluationError]]` of `m`, if it has one.
fn rethrow_cached(cx: &mut Cx, m: u64) -> Res<()> {
    let e = object::record_get(m, modf::EVALUATION_ERROR);
    if e == HOLE {
        return Ok(());
    }
    cx.rt().throw_value(e);
    Err(ep9_value::Throw)
}

/// `[[HasTLA]]` of `m`.
fn has_tla(m: u64) -> bool {
    object::record_get(m, modf::HAS_TLA) == TRUE
}

/// `[[AsyncEvaluationOrder]]` of `m` is not unset: the module took the async path of
/// InnerModuleEvaluation (ES2024 `[[AsyncEvaluation]]` true).
fn is_async_evaluation(m: u64) -> bool {
    object::record_get(m, modf::ASYNC_EVALUATION_ORDER) != UNDEFINED
}

/// `[[AsyncEvaluationOrder]]` of `m` is an integer: its async evaluation has not finished,
/// so an importer must wait for it (InnerModuleEvaluation step 12.c.v). Not [`ORDER_DONE`]:
/// a module that finished does not hold a new importer (test262
/// `top-level-await/module-graphs-does-not-hang.js`).
fn is_async_pending(m: u64) -> bool {
    is_async_evaluation(m) && async_order(m) != ORDER_DONE
}

/// The `[[AsyncEvaluationOrder]]` of `m` (an int32 order, or [`ORDER_DONE`]).
fn async_order(m: u64) -> i32 {
    get_i32(m, modf::ASYNC_EVALUATION_ORDER)
}

/// The count-prefixed list in `field` of `m` (`[n, v1 .. vn]`, or undefined): its n values.
fn list_count(m: u64, field: usize) -> usize {
    let list = object::record_get(m, field);
    if list == UNDEFINED {
        0
    } else {
        int32_of(object::list_get(list, 0)) as usize
    }
}

/// Value `i` of the count-prefixed list in `field` of `m`.
fn list_item(m: u64, field: usize, i: usize) -> u64 {
    object::list_get(object::record_get(m, field), 1 + i)
}

/// Appends `v` to the count-prefixed list in `field` of `m`. GC point (the list grows): `m`
/// and `v` live in Rust locals.
pub(crate) fn list_append(rt: &Runtime, m: u64, field: usize, v: u64) {
    let mut list = object::record_get(m, field);
    let n = list_count(m, field);
    if list == UNDEFINED || 1 + n + 1 > object::list_len(list) {
        let bigger = object::new_list(rt, 1 + 2 * n + 2, UNDEFINED);
        if list != UNDEFINED {
            for i in 1..=n {
                object::list_set(rt, bigger, i, object::list_get(list, i));
            }
        }
        object::record_set(rt, m, field, bigger);
        list = bigger;
    }
    object::list_set(rt, list, 1 + n, v);
    object::list_set(rt, list, 0, int32_bits((n + 1) as i32));
}

/// `%Promise%.prototype` of the current realm, or the v0 not-implemented Error without
/// `%Promise%` (an embedder with no ep9_builtins tables).
fn promise_prototype(cx: &mut Cx) -> Res<u64> {
    let rt = cx.rt();
    if !kind::is_object(intrinsic(rt, IntrinsicId::Promise)) {
        return Err(not_implemented(cx, "modules without %Promise%"));
    }
    Ok(intrinsic(rt, IntrinsicId::PromisePrototype))
}

/// A new pending promise of the current realm (NewPromiseCapability(%Promise%): the VM
/// settles it, so no resolve functions are made). GC point.
fn new_promise(cx: &mut Cx) -> Res<u64> {
    let proto = promise_prototype(cx)?;
    Ok(abi::Cx::new(cx)
        .new_promise(Local::<JsValue>::from_bits(proto))
        .bits())
}

/// Fulfills the promise `p` with undefined if it is still pending.
fn fulfill_undefined(cx: &mut Cx, p: u64) {
    let mut acx = abi::Cx::new(cx);
    let p = Local::<JsObject>::from_bits(p);
    if acx.promise_state(p).ok() == Some(PromiseState::Pending) {
        acx.fulfill_promise(p, Local::undefined());
    }
}

/// Rejects the promise `p` with `reason` if it is still pending.
fn reject_pending(cx: &mut Cx, p: u64, reason: u64) {
    let mut acx = abi::Cx::new(cx);
    let p = Local::<JsObject>::from_bits(p);
    if acx.promise_state(p).ok() == Some(PromiseState::Pending) {
        acx.reject_promise(p, Local::<JsValue>::from_bits(reason));
    }
}

/// Takes the exception of the throw `t` (the thrown value); `Err(t)` again while the isolate
/// terminates (a termination is never caught).
fn catch(cx: &mut Cx, t: Throw) -> Res<u64> {
    abi::Cx::new(cx).catch_exception(t).map(|v| v.bits())
}

/// The generator object of the unit's function 0 of `m`, waiting at suspend id [`RUN`].
fn evaluate_generator(cx: &mut Cx, m: u64) -> Res<u64> {
    let g = object::context_get(environment(m), env::EVALUATE).unwrap_or(UNDEFINED);
    if !generator::is_generator(g) || object::gen_get_i32(g, genf::STATE) != RUN {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: a module is evaluated before its environment is made",
        ));
    }
    Ok(g)
}

/// ExecuteModule() (§16.2.1.6.5) with no capability: the rest of the unit's function 0,
/// `this` undefined. GC point (the module runs).
fn execute_module(cx: &mut Cx, m: u64) -> Res<()> {
    let g = evaluate_generator(cx, m)?;
    generator::resume(cx, g, genf::RESUME_NEXT, UNDEFINED, "next")?;
    Ok(())
}

/// ExecuteAsyncModule(m) (§16.2.1.5.3.2): a new capability whose settlement runs
/// AsyncModuleExecutionFulfilled or ...Rejected, then ExecuteModule(capability): the body
/// runs as an async function body (AsyncBlockStart) up to its first `await`. `Err` only for
/// a termination; any other throw rejects the capability. GC point (the module runs).
fn execute_async_module(cx: &mut Cx, m: u64) -> Res<()> {
    let g = evaluate_generator(cx, m)?;
    let p = new_promise(cx)?;
    let mut acx = abi::Cx::new(cx);
    let data = Local::<JsValue>::from_bits(m);
    let on_fulfilled = acx.new_builtin_closure(async_module_fulfilled, "", 0, data);
    let on_rejected = acx.new_builtin_closure(async_module_rejected, "", 0, data);
    acx.perform_promise_then(
        Local::<JsObject>::from_bits(p),
        on_fulfilled,
        on_rejected,
        None,
    );
    // ExecuteModule(capability): the generator object becomes the async function object of
    // the body, with the capability's promise (`interp/slow/async_fn.rs` ids 33-35 use it).
    let rt = cx.rt();
    object::gen_set(rt, g, genf::MODE, int32_bits(genf::MODE_ASYNC_FUNCTION));
    object::gen_set(rt, g, genf::ASYNC, p);
    if let Err(t) = generator::resume_async_function(cx, g, genf::RESUME_NEXT, UNDEFINED) {
        // The frame could not be pushed (a full stack, an interrupt), or a throw left it.
        let e = catch(cx, t)?;
        generator::close(cx.rt(), g);
        reject_pending(cx, p, e);
    }
    Ok(())
}

/// The fulfilled closure of ExecuteAsyncModule (step 4): AsyncModuleExecutionFulfilled of
/// the closure's module.
fn async_module_fulfilled<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let m = cx.closure_data(args.callee()).bits();
    async_execution_fulfilled(cx.inner(), m)?;
    Ok(Local::undefined())
}

/// The rejected closure of ExecuteAsyncModule (step 6): AsyncModuleExecutionRejected of the
/// closure's module with the reason.
fn async_module_rejected<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let m = cx.closure_data(args.callee()).bits();
    let error = args.get(0).bits();
    async_execution_rejected(cx.inner(), m, error);
    Ok(Local::undefined())
}

/// Resolves `[[TopLevelCapability]]` of `m` with undefined, if it has one.
fn resolve_top_level(cx: &mut Cx, m: u64) {
    let p = object::record_get(m, modf::TOP_LEVEL_CAPABILITY);
    if p != UNDEFINED {
        fulfill_undefined(cx, p);
    }
}

/// GatherAvailableAncestors(m, execList) (§16.2.1.5.3.3): the async parents whose last
/// pending dependency `m` was, and (through a parent without `[[HasTLA]]`) their parents. A
/// work list instead of the recursion gives the same set: a module joins once, when its
/// count reaches 0. Not a GC point, so the lists may hold cells (plan rule 8; as
/// ResolveExport's resolveSet).
pub(crate) fn gather_available_ancestors(rt: &Runtime, m: u64) -> Vec<ModuleCell> {
    let mut found: Vec<ModuleCell> = Vec::new();
    let mut work = vec![ModuleCell(m)];
    while let Some(ModuleCell(x)) = work.pop() {
        for i in 0..list_count(x, modf::ASYNC_PARENT_MODULES) {
            let p = list_item(x, modf::ASYNC_PARENT_MODULES, i);
            let root = object::record_get(p, modf::CYCLE_ROOT);
            let root = if root == UNDEFINED { p } else { root };
            if found.contains(&ModuleCell(p))
                || status_of(p) == status::ABORTED
                || object::record_get(root, modf::EVALUATION_ERROR) != HOLE
            {
                continue;
            }
            let pending = get_i32(p, modf::PENDING_ASYNC_DEPENDENCIES) - 1;
            set_i32(rt, p, modf::PENDING_ASYNC_DEPENDENCIES, pending);
            if pending == 0 {
                found.push(ModuleCell(p));
                if !has_tla(p) {
                    work.push(ModuleCell(p));
                }
            }
        }
    }
    found
}

/// AsyncModuleExecutionFulfilled(m) (§16.2.1.5.3.4). `Err` only for a termination. GC point
/// (the waiting modules run).
fn async_execution_fulfilled(cx: &mut Cx, m: u64) -> Res<()> {
    let rt = cx.rt();
    if matches!(status_of(m), status::EVALUATED | status::ABORTED) {
        // Rejected already (an error of a module of its cycle came first), or a termination
        // made it aborted while its body waited (`abort_async_modules`).
        return Ok(());
    }
    if status_of(m) != status::EVALUATING_ASYNC {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: an async module settled while it is not evaluating",
        ));
    }
    set_i32(rt, m, modf::ASYNC_EVALUATION_ORDER, ORDER_DONE);
    set_status(rt, m, status::EVALUATED);
    resolve_top_level(cx, m);
    // Steps 8-11: the ancestors that can run now, by [[AsyncEvaluationOrder]].
    let mut found = gather_available_ancestors(rt, m);
    found.sort_by_key(|x| async_order(x.0));
    let exec = rt.heap.scratch_roots();
    for x in found {
        exec.push(val(x.0));
    }
    // Step 12.
    for i in 0..exec.len() {
        let x = exec.get(i).to_bits();
        if status_of(x) == status::EVALUATED {
            // An error of its cycle reached it first.
            continue;
        }
        if has_tla(x) {
            execute_async_module(cx, x)?;
            continue;
        }
        match execute_module(cx, x) {
            Err(t) => {
                let e = catch(cx, t)?;
                async_execution_rejected(cx, x, e);
            }
            Ok(()) => {
                set_i32(cx.rt(), x, modf::ASYNC_EVALUATION_ORDER, ORDER_DONE);
                set_status(cx.rt(), x, status::EVALUATED);
                resolve_top_level(cx, x);
            }
        }
    }
    Ok(())
}

/// Steps 5-9 of AsyncModuleExecutionRejected: `m` is evaluated with `error` cached, and its
/// `[[TopLevelCapability]]` (if any) is rejected. GC point (the rejection).
fn reject_module(cx: &mut Cx, m: u64, error: u64) {
    let rt = cx.rt();
    object::record_set(rt, m, modf::EVALUATION_ERROR, error);
    set_status(rt, m, status::EVALUATED);
    set_i32(rt, m, modf::ASYNC_EVALUATION_ORDER, ORDER_DONE);
    let cap = object::record_get(m, modf::TOP_LEVEL_CAPABILITY);
    if cap != UNDEFINED {
        reject_pending(cx, cap, error);
    }
}

/// AsyncModuleExecutionRejected(m, error) (§16.2.1.5.3.5, the current draft: the
/// capability is rejected before the parents, so the promises settle leaf to root; test262
/// `top-level-await/rejection-order.js`): `m` and its async parents, depth first, cache
/// `error`. An explicit stack instead of the recursion (a module graph can be deep). GC
/// point (the rejections).
fn async_execution_rejected(cx: &mut Cx, m: u64, error: u64) {
    if matches!(status_of(m), status::EVALUATED | status::ABORTED) {
        return;
    }
    reject_module(cx, m, error);
    // The modules of the walk and, beside them, the next parent index of each.
    let rt = cx.rt();
    let mut stack = Stack {
        roots: rt.heap.scratch_roots(),
        len: 0,
    };
    let mut next: Vec<usize> = Vec::new();
    stack.push(m);
    next.push(0);
    while stack.len > 0 {
        let top = stack.len - 1;
        let x = stack.get(top);
        let i = next[top];
        if i < list_count(x, modf::ASYNC_PARENT_MODULES) {
            next[top] = i + 1;
            let p = list_item(x, modf::ASYNC_PARENT_MODULES, i);
            if !matches!(status_of(p), status::EVALUATED | status::ABORTED) {
                reject_module(cx, p, error);
                stack.push(p);
                next.push(0);
            }
            continue;
        }
        stack.pop();
        next.pop();
    }
}

/// Adds `m` (its `[[AsyncEvaluationOrder]]` just set) to the isolate's list of async
/// modules (`JobState::async_modules`), which [`abort_async_modules`] reads at a termination.
/// When the list is full, the modules whose async evaluation is no longer pending leave it
/// first, so it holds about the pending ones. GC point (the list grows): `m` lives in a Rust
/// local.
fn note_async_module(rt: &Runtime, m: u64) {
    let mut list = rt.jobs.async_modules();
    let mut n = if list == UNDEFINED {
        0
    } else {
        int32_of(object::list_get(list, 0)) as usize
    };
    if list != UNDEFINED && 1 + n + 1 > object::list_len(list) {
        // Compact in place: keep the modules that can still be pending.
        let mut kept = 0;
        for i in 0..n {
            let x = object::list_get(list, 1 + i);
            if is_async_pending(x) && status_of(x) != status::ABORTED {
                object::list_set(rt, list, 1 + kept, x);
                kept += 1;
            }
        }
        for i in kept..n {
            object::list_set(rt, list, 1 + i, UNDEFINED);
        }
        n = kept;
        object::list_set(rt, list, 0, int32_bits(n as i32));
    }
    if list == UNDEFINED || 1 + n + 1 > object::list_len(list) {
        let bigger = object::new_list(rt, 1 + 2 * n + 4, UNDEFINED);
        if list != UNDEFINED {
            for i in 0..=n {
                object::list_set(rt, bigger, i, object::list_get(list, i));
            }
        }
        rt.jobs.set_async_modules(bigger);
        list = bigger;
    }
    object::list_set(rt, list, 1 + n, m);
    object::list_set(rt, list, 0, int32_bits((n + 1) as i32));
}

/// The end of a termination (isolate.rs `end_termination`, after the job queue is dropped):
/// each module whose async evaluation is still pending becomes [`status::ABORTED`], with no
/// `[[TopLevelCapability]]`, as Evaluate does for the modules on its stack. The jobs that
/// would settle it are gone, so a later Evaluate that reaches it throws the "was terminated"
/// Error, and its importers cache that Error (no run waits for it, and a later `import()` of
/// it rejects). A body that waits for a promise outside the dropped queue may still go on
/// later (as any async function after a termination), but its module stays aborted. Not a
/// GC point.
pub(crate) fn abort_async_modules(rt: &Runtime) {
    let list = rt.jobs.async_modules();
    if list == UNDEFINED {
        return;
    }
    for i in 0..int32_of(object::list_get(list, 0)) as usize {
        let m = object::list_get(list, 1 + i);
        if is_async_pending(m)
            && matches!(status_of(m), status::EVALUATING | status::EVALUATING_ASYNC)
        {
            set_status(rt, m, status::ABORTED);
            object::record_set(rt, m, modf::TOP_LEVEL_CAPABILITY, UNDEFINED);
        }
    }
    rt.jobs.set_async_modules(UNDEFINED);
}

/// InnerModuleEvaluation (§16.2.1.5.3.1).
fn inner_evaluate(cx: &mut Cx, m: u64, stack: &mut Stack<'_>, index: i32) -> Res<i32> {
    check_stack(cx)?;
    let rt = cx.rt();
    match status_of(m) {
        status::EVALUATED | status::EVALUATING_ASYNC => {
            rethrow_cached(cx, m)?;
            return Ok(index);
        }
        status::EVALUATING => return Ok(index),
        status::ABORTED => {
            let msg = format!(
                "the evaluation of module {} was terminated",
                string::to_rust_string(name_of(m))
            );
            return Err(throw_error(cx, ErrorKind::Error, &msg));
        }
        status::LINKED => {}
        _ => {
            return Err(throw_error(
                cx,
                ErrorKind::Error,
                "ep9 internal error: a module is evaluated before it is linked",
            ));
        }
    }
    set_status(rt, m, status::EVALUATING);
    set_i32(rt, m, modf::DFS_INDEX, index);
    set_i32(rt, m, modf::DFS_ANCESTOR_INDEX, index);
    set_i32(rt, m, modf::PENDING_ASYNC_DEPENDENCIES, 0);
    let mut index = index + 1;
    stack.push(m);
    let n = object::list_len(object::record_get(m, modf::LOADED));
    for r in 0..n as u32 {
        let mut required = loaded(m, r);
        index = inner_evaluate(cx, required, stack, index)?;
        if status_of(required) == status::EVALUATING {
            let a = get_i32(m, modf::DFS_ANCESTOR_INDEX)
                .min(get_i32(required, modf::DFS_ANCESTOR_INDEX));
            set_i32(rt, m, modf::DFS_ANCESTOR_INDEX, a);
        } else {
            let root = object::record_get(required, modf::CYCLE_ROOT);
            if root != UNDEFINED {
                required = root;
            }
            rethrow_cached(cx, required)?;
        }
        if is_async_pending(required) {
            let pending = get_i32(m, modf::PENDING_ASYNC_DEPENDENCIES) + 1;
            set_i32(rt, m, modf::PENDING_ASYNC_DEPENDENCIES, pending);
            list_append(rt, required, modf::ASYNC_PARENT_MODULES, m);
        }
    }
    let pending = get_i32(m, modf::PENDING_ASYNC_DEPENDENCIES);
    if pending > 0 || has_tla(m) {
        let order = ASYNC_EVALUATION_COUNT.with(|c| {
            let v = c.get();
            c.set(v.wrapping_add(1) & i32::MAX);
            v
        });
        set_i32(rt, m, modf::ASYNC_EVALUATION_ORDER, order);
        note_async_module(rt, m);
        if pending == 0 {
            execute_async_module(cx, m)?;
        }
    } else {
        execute_module(cx, m)?;
    }
    if get_i32(m, modf::DFS_ANCESTOR_INDEX) == get_i32(m, modf::DFS_INDEX) {
        loop {
            let done = stack.pop();
            let s = if is_async_evaluation(done) {
                status::EVALUATING_ASYNC
            } else {
                status::EVALUATED
            };
            set_status(rt, done, s);
            object::record_set(rt, done, modf::CYCLE_ROOT, m);
            if done == m {
                break;
            }
        }
    }
    Ok(index)
}

/// Evaluate() (§16.2.1.5.3): the promise of the evaluation of `module`'s cycle root (made
/// once, `[[TopLevelCapability]]`; a module that ran before gives it again). On an error
/// of the synchronous part, every module on the stack is evaluated with that error cached
/// (a later import of one of them throws it again) and the promise is rejected; then an
/// `entry` call (`run_module`) gives `Err` with the error still pending, and another call
/// gives the rejected promise. An `entry` promise is marked handled: its host reads it. A
/// termination makes every module on the stack [`status::ABORTED`] (no allocation while it
/// unwinds) and leaves no capability: a later Evaluate that reaches one of them throws an
/// Error, and the modules on that Evaluate's stack cache it. A termination after a
/// top-level `await` (in a job) does the same for each module whose async evaluation is
/// pending ([`abort_async_modules`]). GC point (the modules run).
fn evaluate(cx: &mut Cx, module: u64, entry: bool) -> Res<u64> {
    let rt = cx.rt();
    let mut m = module;
    if matches!(status_of(m), status::EVALUATING_ASYNC | status::EVALUATED) {
        let root = object::record_get(m, modf::CYCLE_ROOT);
        if root != UNDEFINED {
            m = root;
        }
    }
    let cap = object::record_get(m, modf::TOP_LEVEL_CAPABILITY);
    if cap != UNDEFINED {
        return Ok(cap);
    }
    let p = new_promise(cx)?;
    if entry {
        object::slot_set(rt, p, jobs::promise::IS_HANDLED, TRUE);
    }
    object::record_set(rt, m, modf::TOP_LEVEL_CAPABILITY, p);
    let mut stack = Stack {
        roots: rt.heap.scratch_roots(),
        len: 0,
    };
    match inner_evaluate(cx, m, &mut stack, 0) {
        Ok(_) => {
            // Step 10.c: the module is evaluated (it ran now, or its async evaluation
            // finished before this call, `[[AsyncEvaluationOrder]]` done); else it is
            // evaluating-async and AsyncModuleExecutionFulfilled or ...Rejected settles `p`.
            if status_of(m) == status::EVALUATED {
                fulfill_undefined(cx, p);
            }
            Ok(p)
        }
        Err(t) => {
            if rt.terminating.get() {
                for i in 0..stack.len {
                    set_status(rt, stack.get(i), status::ABORTED);
                }
                object::record_set(rt, m, modf::TOP_LEVEL_CAPABILITY, UNDEFINED);
                return Err(t);
            }
            let error = rt.pending();
            for i in 0..stack.len {
                let x = stack.get(i);
                set_status(rt, x, status::EVALUATED);
                object::record_set(rt, x, modf::EVALUATION_ERROR, error);
            }
            drop(stack);
            if entry {
                // The error stays pending: the caller throws it with its throw site.
                reject_pending(cx, p, error);
                return Err(t);
            }
            let e = catch(cx, t)?;
            reject_pending(cx, p, e);
            Ok(p)
        }
    }
}

/// The result of an entry's Evaluate promise `p` (of module `m`): undefined, or the throw of
/// its rejection, after a microtask checkpoint, while the module host is still in place. So
/// the jobs of the run use the host: a top-level `await` goes on, and `import()` in a job
/// (`Promise.resolve().then(() => import(s))`), or in a module that `import()` evaluates in
/// a job, loads through the run's hooks. An Error if `p` is still pending after the
/// checkpoint. With JS on the stack (a host function that JS called runs the module), a
/// checkpoint cannot run: a settled `p` gives its result with no checkpoint (the jobs stay
/// queued for the caller's checkpoint), and a pending `p` gets the checkpoint's Error ("a
/// microtask checkpoint needs an empty JS stack"). GC point (jobs run).
fn settle_entry(cx: &mut Cx, m: u64, p: u64) -> Res<u64> {
    let js_on_stack = !cx.rt().stack().fp().is_null();
    let mut acx = abi::Cx::new(cx);
    let pl = Local::<JsObject>::from_bits(p);
    if !js_on_stack || acx.promise_state(pl)? == PromiseState::Pending {
        acx.perform_microtask_checkpoint()?;
    }
    match acx.promise_state(pl)? {
        PromiseState::Fulfilled => Ok(UNDEFINED),
        PromiseState::Rejected => {
            let reason = acx.promise_result(pl)?.bits();
            cx.rt().throw_value(reason);
            Err(Throw)
        }
        PromiseState::Pending => {
            let msg = format!(
                "the top-level await of module {} never settled",
                string::to_rust_string(name_of(m))
            );
            Err(throw_error(cx, ErrorKind::Error, &msg))
        }
    }
}

/// Loads, links and evaluates the module unit `unit` named `name` in the current realm
/// (the body of `Cx::run_module`, isolate.rs), with `host` as the isolate's module host
/// for the run ([`with_host`]). If the realm's map already has `name`, that record runs and
/// `unit` is dropped; a module that ran before does not run again (its cached error is
/// thrown again). After Evaluate, a microtask checkpoint runs with the host in place
/// ([`settle_entry`]; none with JS on the stack): a top-level `await` settles there, and
/// `import()` in jobs uses the host. The result is undefined. GC point.
pub(crate) fn run_module(
    cx: &mut Cx,
    unit: VerifiedUnit,
    name: &str,
    host: &mut Host<'_>,
) -> Res<u64> {
    with_host(host_key(cx.rt()), host, || {
        let m = module_for_unit(cx, cx.realm, unit, name)?;
        load_graph(cx, m)?;
        link(cx, m)?;
        let p = evaluate(cx, m, true)?;
        settle_entry(cx, m, p)
    })
}

// ---- import() (§13.3.10.1) ---------------------------------------------------------------------

/// The ScriptOrModule of eval code and of a `Function()` body (a `CodeKind::Eval` unit):
/// GetActiveScriptOrModule (§9.4.1) when the unit loaded (`LoadedUnit::script_or_module`,
/// set by `abi::Cx::load_entry`). A function made by that code keeps it as its
/// `[[ScriptOrModule]]` (OrdinaryFunctionCreate, §10.2.3), so its `import()` resolves from
/// there, not from the code that calls it later. Holds no Value (a realm of another realm
/// is in a weak root slot that the functions of the unit trace: [`ReferrerRealm::Other`]).
pub(crate) enum EvalReferrer {
    /// A null ScriptOrModule (no script or module code ran, for example `Function` called
    /// by a promise job): GetActiveScriptOrModule skips the frames of this code and goes on
    /// with their callers.
    Null,
    /// A script or a module.
    Of {
        /// The host's referrer name: a module's name or a script's file.
        name: String,
        /// The referrer's realm (its module map).
        realm: ReferrerRealm,
    },
}

/// The realm of an [`EvalReferrer`].
pub(crate) enum ReferrerRealm {
    /// The realm that loaded the eval code, so the `[[Realm]]` of its functions (the common
    /// case: no handle).
    Own,
    /// Another realm, in a [`ReferrerSlot`]: a function of the eval code keeps its
    /// ScriptOrModule, so that module's `[[Realm]]` and its module map, alive (C2-M1), and
    /// only as long as such a function lives (C3-M1).
    Other(ReferrerSlot),
    /// A referrer with no realm in reach (script code with no function frame): the current
    /// realm.
    Current,
}

/// GetActiveScriptOrModule (§9.4.1) as the host sees it: (the referrer name for `resolve`,
/// the realm whose module map holds the modules it loads; undefined for none: the caller
/// takes the current realm, as HostLoadImportedModule takes the current Realm Record for a
/// null referrer). A module (the end of `context`'s chain, or of the closure context of a
/// frame's function) gives its name and `[[Realm]]`; script code its file and its
/// function's realm; eval code its [`EvalReferrer`]. Host frames and eval code with a null
/// ScriptOrModule ([`EvalReferrer::Null`]) are passed. `None` if no frame has one. Not a GC
/// point.
fn active_script_or_module(fp: *mut u64, context: u64) -> Option<(String, u64)> {
    let of_module = |m: u64| (string::to_rust_string(name_of(m)), realm_of(m));
    if let Some(m) = module_of_context(context, 0) {
        return Some(of_module(m));
    }
    let mut f = FrameRef(fp);
    while !f.is_null() {
        if let Some(u) = f.info().loaded_unit() {
            if u.unit.header.code_kind != CodeKind::Eval {
                // SAFETY: `f` is a live frame; its header slot holds the callee (a Value).
                let callee = unsafe { *f.0.offset(crate::frame::slot::CALLEE) };
                if !kind::is_object(callee) || kind::cell_kind(callee) != kind::FUNCTION.get() {
                    return Some((u.file.clone(), UNDEFINED));
                }
                if let Some(m) = module_of_context(object::function_context(callee), 0) {
                    return Some(of_module(m));
                }
                return Some((u.file.clone(), object::function_realm(callee)));
            }
            if let Some(EvalReferrer::Of { name, realm }) = &u.script_or_module {
                let realm = match realm {
                    ReferrerRealm::Own => {
                        // SAFETY: as above.
                        let callee = unsafe { *f.0.offset(crate::frame::slot::CALLEE) };
                        if kind::is_object(callee)
                            && kind::cell_kind(callee) == kind::FUNCTION.get()
                        {
                            object::function_realm(callee)
                        } else {
                            UNDEFINED
                        }
                    }
                    // A frame of this unit holds a function of it (its callee), which keeps
                    // the realm: the slot is not cleared.
                    ReferrerRealm::Other(s) => s.bits(),
                    ReferrerRealm::Current => UNDEFINED,
                };
                return Some((name.clone(), realm));
            }
            // EvalReferrer::Null: a null ScriptOrModule; the caller's frame is next (C2-M3).
        }
        f = f.caller();
    }
    None
}

/// The [`EvalReferrer`] of eval code that loads now, with `context` (the caller's context
/// for direct eval code; undefined else) and the frames of the running code
/// (`abi::Cx::load_entry`). Not a GC point.
pub(crate) fn eval_referrer(rt: &Runtime, context: u64) -> EvalReferrer {
    match active_script_or_module(rt.stack().fp(), context) {
        Some((name, realm)) => EvalReferrer::Of {
            name,
            realm: if realm == UNDEFINED {
                ReferrerRealm::Current
            } else if realm == rt.root(Root::Realm) {
                ReferrerRealm::Own
            } else {
                ReferrerRealm::Other(ReferrerSlot::new(rt, realm))
            },
        },
        None => EvalReferrer::Null,
    }
}

/// The realm of [`ReferrerRealm::Other`]: one Value slot in a box, a weak root range of the
/// heap. It is not a root: a FUNCTION cell of the unit marks it when the GC traces that cell
/// ([`trace_referrer_realm`], `object::trace_function`), so the realm lives exactly as long
/// as a function of the eval code (C3-M1). A frame of the unit's code holds such a function
/// (its callee slot, or the generator object of a resumed frame), and so does a suspended
/// generator or async function. After a GC in which no function of the unit was marked, the
/// weak range clears the slot to undefined (the realm died; the current realm is used, as
/// for no realm). The unit keeps the slot until the isolate drops; the range is never
/// removed.
///
/// ```text
/// FUNCTION cell --INFO--> FunctionInfo --unit--> LoadedUnit.script_or_module
///   (marked)                                        Other(ReferrerSlot) --GC trace--> REALM
/// ```
///
/// The slot is a raw pointer from `Box::leak`, not a `Box` field: the heap's range table
/// and this struct both reach it, and moving a `Box` would assert unique access (Miri).
pub(crate) struct ReferrerSlot(NonNull<[Slot]>);

impl ReferrerSlot {
    /// A slot that holds `realm` (a REALM cell that a live frame or context reaches now, so
    /// it stays marked until the unit's first function holds it). Not a GC point.
    fn new(rt: &Runtime, realm: u64) -> ReferrerSlot {
        let slots: Box<[Slot]> = Box::new([Slot::new(val(realm))]);
        let p = NonNull::from(Box::leak(slots));
        // SAFETY: `p` is valid until `drop` frees it. Its `LoadedUnit` lives until the Runtime
        // drops, which drops `heap` (and its range table) before `units` (as for the unit's
        // string and constant ranges), so the range is never read after the free. The slot
        // holds Value bits (never 0): the realm, or undefined after the weak range clears it.
        let _ = unsafe { rt.heap.add_root_range(p, RootStrength::Weak) };
        ReferrerSlot(p)
    }

    /// The slot.
    fn slot(&self) -> &Slot {
        // SAFETY: valid until `drop` (see `new`); Slot is atomic, so a shared reference is
        // fine while the GC clears it.
        unsafe { &self.0.as_ref()[0] }
    }

    /// The realm's bits; undefined after a GC in which no function of the unit was marked.
    fn bits(&self) -> u64 {
        self.slot().bits()
    }
}

impl Drop for ReferrerSlot {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `Box::leak` in `new` and is freed once, here; the heap
        // that read it dropped first (see `new`).
        drop(unsafe { Box::from_raw(self.0.as_ptr()) });
    }
}

/// Marks the referrer realm of the eval unit of `info` (the INFO word of a FUNCTION cell
/// that the GC traces), if the unit has a [`ReferrerRealm::Other`]. Called from the trace
/// function of FUNCTION cells: reads only Rust data that lives until the isolate drops, and
/// does not call into the heap.
#[inline]
pub(crate) fn trace_referrer_realm(info: &FunctionInfo, t: &mut Tracer<'_>) {
    if let Some(u) = info.loaded_unit()
        && let Some(EvalReferrer::Of {
            realm: ReferrerRealm::Other(s),
            ..
        }) = &u.script_or_module
    {
        t.visit(s.slot());
    }
}

/// The import attributes of `options` (EvaluateImportCall step 10, §13.3.10.1.1): the
/// (key, value) list of its `with` object, sorted by key. A TypeError for an options or
/// `with` value that is not an object, or a value that is not a string. The entries come
/// first (EnumerableOwnProperties: every getter runs, and its throw is the error), then the
/// value check. GC point; may run JS (getters).
fn import_attributes(cx: &mut Cx, options: u64) -> Res<Vec<(String, String)>> {
    let mut out = Vec::new();
    if options == UNDEFINED {
        return Ok(out);
    }
    if !kind::is_object(options) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "The second argument to import() must be an object",
        ));
    }
    let with = atoms::intern_str(cx.rt(), "with");
    let attributes = property::get(cx, options, with)?;
    if attributes == UNDEFINED {
        return Ok(out);
    }
    if !kind::is_object(attributes) {
        return Err(throw_error(
            cx,
            ErrorKind::TypeError,
            "The 'with' option of import() must be an object",
        ));
    }
    // 10.d.ii. EnumerableOwnProperties(attributes, key+value) (§7.3.23): the entries go to
    // scratch roots (key, value, key, value, ...), as getters run between them.
    let rt = cx.rt();
    let entries = rt.heap.scratch_roots();
    entries.push(val(attributes));
    let keys = property::own_property_keys(cx, attributes);
    entries.push(val(keys));
    for i in 0..object::array_len(keys) {
        let Some(k) = object::array_get(keys, i) else {
            continue;
        };
        if !kind::is_string(k) {
            continue;
        }
        let Some(desc) = property::get_own_property(cx, attributes, k)? else {
            continue;
        };
        if desc.enumerable != Some(true) {
            continue;
        }
        let v = property::get(cx, attributes, k)?;
        entries.push(val(k));
        entries.push(val(v));
    }
    // 10.d.iv. Each value must be a string.
    for i in (2..entries.len()).step_by(2) {
        let (k, v) = (entries.get(i).to_bits(), entries.get(i + 1).to_bits());
        if !kind::is_string(v) {
            let msg = format!(
                "Import attribute value must be a string (attribute \"{}\")",
                string::to_rust_string(k)
            );
            return Err(throw_error(cx, ErrorKind::TypeError, &msg));
        }
        out.push((string::to_rust_string(k), string::to_rust_string(v)));
    }
    out.sort_by(|a, b| {
        let (a, b) = (a.0.encode_utf16(), b.0.encode_utf16());
        a.cmp(b)
    });
    Ok(out)
}

/// Id 43: `import(specifier, options)` (§13.3.10.1.1 EvaluateImportCall steps 3-13, with
/// HostLoadImportedModule and ContinueDynamicImport, §16.2.1.10-11) in the frame `fp`: a new
/// promise. Every error after it exists rejects it. The host loads the module and its graph
/// at once (a synchronous host); Link and Evaluate run in a job, and the namespace fulfills
/// the promise in the job after Evaluate's promise settles. The referrer is the active
/// ScriptOrModule ([`active_script_or_module`]), and the module goes into its realm's map;
/// the promise is of the current realm. With no module host (no `Cx::run_module` or
/// `Isolate::with_module_host` call is in progress), the v0 not-implemented Error. GC point;
/// may run JS (ToString, the options' getters).
pub(crate) fn dynamic_import(
    cx: &mut Cx,
    fp: *mut u64,
    context: u64,
    specifier: u64,
    options: u64,
) -> Res<u64> {
    if !has_host(cx.rt()) {
        return Err(not_implemented(cx, "DynamicImport"));
    }
    // 1. GetActiveScriptOrModule: its name and realm (a null referrer: the current realm).
    let (referrer, realm) =
        active_script_or_module(fp, context).unwrap_or_else(|| (String::new(), UNDEFINED));
    let realm = if realm == UNDEFINED { cx.realm } else { realm };
    // 5. NewPromiseCapability(%Promise%) of the current realm.
    let p = new_promise(cx)?;
    match import_load(cx, &referrer, realm, specifier, options) {
        Ok(m) => {
            // ContinueDynamicImport steps 2-6 with the load promise of the graph.
            let load = new_promise(cx)?;
            let ok = load_graph(cx, m);
            match ok {
                Ok(()) => fulfill_undefined(cx, load),
                Err(t) => {
                    let e = catch(cx, t)?;
                    reject_pending(cx, load, e);
                }
            }
            let rt = cx.rt();
            let data = object::new_list(rt, 2, UNDEFINED);
            object::list_set(rt, data, 0, m);
            object::list_set(rt, data, 1, p);
            let mut acx = abi::Cx::new(cx);
            let data = Local::<JsValue>::from_bits(data);
            let on_loaded = acx.new_builtin_closure(import_link_and_evaluate, "", 0, data);
            let on_rejected =
                acx.new_builtin_closure(import_rejected, "", 1, Local::<JsValue>::from_bits(p));
            acx.perform_promise_then(
                Local::<JsObject>::from_bits(load),
                on_loaded,
                on_rejected,
                None,
            );
        }
        Err(t) => {
            let e = catch(cx, t)?;
            reject_pending(cx, p, e);
        }
    }
    Ok(p)
}

/// EvaluateImportCall steps 6-12 and HostLoadImportedModule (with FinishLoadingImportedModule)
/// for `import(specifier, options)` from `referrer`, whose modules are in the map of `realm`:
/// the module's record (its graph not loaded yet). GC point; may run JS.
fn import_load(cx: &mut Cx, referrer: &str, realm: u64, specifier: u64, options: u64) -> Res<u64> {
    // 6-7. ToString(specifier).
    let spec = conv::to_string(cx, specifier)?;
    let spec = string::to_rust_string(spec);
    // 8-11. The attributes. AllImportAttributesSupported false is a TypeError here (step
    // 11.c), not the SyntaxError of a static request (`check_attribute_list`); then the type.
    let attributes = import_attributes(cx, options)?;
    if let Some((key, value)) = attributes.iter().find(|(k, _)| k != "type") {
        let msg = format!("Import attribute \"{key}\" with value \"{value}\" is not supported");
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    check_attribute_list(cx, &attributes)?;
    // HostLoadImportedModule.
    let name = host_resolve(cx, referrer, &spec)?;
    host_module(cx, realm, &name)
}

/// The rejected closure of ContinueDynamicImport (steps 3-4, 6.b): rejects the import()
/// promise (the closure's data) with the reason.
fn import_rejected<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let p = cx.closure_data(args.callee()).bits();
    reject_pending(cx.inner(), p, args.get(0).bits());
    Ok(Local::undefined())
}

/// The linkAndEvaluate closure of ContinueDynamicImport (step 6): Link (a throw rejects),
/// then Evaluate, then the namespace in a job after Evaluate's promise settles. The data is
/// `[module, the import() promise]`. Link and Evaluate run in the module's `[[Realm]]` (the
/// job runs in the realm of the `import()` call, which can be another one).
fn import_link_and_evaluate<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let data = cx.closure_data(args.callee()).bits();
    let (m, p) = (object::list_get(data, 0), object::list_get(data, 1));
    let icx = cx.inner();
    let saved = icx.realm;
    let realm = realm_of(m);
    let switch = realm != saved && realm != UNDEFINED;
    if switch {
        crate::realm::switch(icx, realm);
    }
    // Err((true, _)): Link threw (a rejection); Err((false, _)): a termination in Evaluate.
    let r = match link(icx, m) {
        Ok(()) => evaluate(icx, m, false).map_err(|t| (false, t)),
        Err(t) => Err((true, t)),
    };
    if switch {
        crate::realm::switch(icx, saved);
    }
    let ep = match r {
        Ok(ep) => ep,
        Err((true, t)) => {
            let e = catch(icx, t)?;
            reject_pending(icx, p, e);
            return Ok(Local::undefined());
        }
        Err((false, t)) => return Err(t),
    };
    let on_fulfilled =
        cx.new_builtin_closure(import_evaluated, "", 0, Local::<JsValue>::from_bits(data));
    let on_rejected =
        cx.new_builtin_closure(import_rejected, "", 1, Local::<JsValue>::from_bits(p));
    cx.perform_promise_then(
        Local::<JsObject>::from_bits(ep),
        on_fulfilled,
        on_rejected,
        None,
    );
    Ok(Local::undefined())
}

/// The onFulfilled closure of linkAndEvaluate (step 6.d): resolves the import() promise
/// with the module's namespace (a namespace with a `then` export is a thenable). The data is
/// `[module, the import() promise]`.
fn import_evaluated<'s>(
    cx: &mut abi::Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let data = cx.closure_data(args.callee()).bits();
    let (m, p) = (object::list_get(data, 0), object::list_get(data, 1));
    let icx = cx.inner();
    let ns = match get_namespace(icx, m) {
        Ok(ns) => ns,
        Err(t) => {
            let e = catch(icx, t)?;
            reject_pending(icx, p, e);
            return Ok(Local::undefined());
        }
    };
    let pl = Local::<JsObject>::from_bits(p);
    if cx.promise_state(pl)? == PromiseState::Pending {
        cx.resolve_promise_once(pl, Local::<JsValue>::from_bits(ns))?;
    }
    Ok(Local::undefined())
}
