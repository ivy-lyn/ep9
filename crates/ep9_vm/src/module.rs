//! Modules (§16.2; owner M3-P2 MODULE, then M3-P3 IMPORT-TLA): Source Text Module Records
//! (the MODULE cell, kind 76, `object::modf`), the module map of a realm, loading, Link,
//! Evaluate, ResolveExport, and module namespace exotic objects (kind 26, `object::nsf`,
//! §10.4.6) with their internal methods.
//!
//! ```text
//! Cx::run_module(unit, name, resolve, load)        (isolate.rs: the loader API)
//!   record of `name` (the realm's module map) or a new one          new_module
//!   load_graph: every [[RequestedModules]] entry, depth first       resolve(referrer, specifier)
//!     -> a name; the map, or load(name) -> a unit -> a new record   load(name)
//!   link: InnerModuleLinking (§16.2.1.5.1.1), InitializeEnvironment (§16.2.1.6.4)
//!   evaluate: InnerModuleEvaluation (§16.2.1.5.3.1); an error is cached in [[EvaluationError]]
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
//! | `STATUS` | int32 [`status`]: new, unlinked, linking, linked, evaluating, evaluating-async, evaluated; and aborted (not in the spec: a termination stopped its Evaluate) |
//! | `ENVIRONMENT` | the module environment: a CONTEXT with the slots of [`env`] (not on any context chain) |
//! | `NAMESPACE` | the namespace object, made on first use ([`get_namespace`]) |
//! | `UNIT` | native word: the module's `LoadedUnit` (its module records, ABI 5 section 5) |
//! | `REQUESTED` | ELEMENTS: the specifier of each ModuleRequest (an atom) |
//! | `LOADED` | ELEMENTS: the MODULE of each request (undefined until loaded) |
//! | `EVALUATION_ERROR` | the cached error of Evaluate; the hole for none |
//! | `DFS_INDEX`, `DFS_ANCESTOR_INDEX`, `CYCLE_ROOT` | Tarjan state of Link and Evaluate |
//! | `HAS_TLA` | false (the front end rejects top-level `await` until M3-P3) |
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

use ep9_bytecode::{CodeKind, ExportImportName, ImportName, ModuleRecords, VerifiedUnit};

use crate::compiler::{CompileFailure, CompileResult};
use crate::conv::Res;
use crate::descriptor::Desc;
use crate::error::{not_implemented, throw_error};
use crate::exotic::probe;
use crate::frame::{HOLE, NULL, UNDEFINED, int32_bits, int32_of, native_of, native_ptr};
use crate::interp::Cx;
use crate::key::{Probe, is_symbol_key, key_eq};
use crate::kinds as kind;
use crate::object::{self, attr, genf, init, modf, nsf, realmf};
use crate::runtime::{ErrorKind, Runtime, val};
use crate::unit::{FunctionInfo, LoadedUnit};
use crate::{atoms, generator, property, string, symbol};

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
    /// Evaluate. The isolate is used again after a termination, so a later Evaluate of the
    /// module, or of a module that imports it, throws an Error; it never takes the module as
    /// evaluated (its body did not finish). Link takes it as evaluated.
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
    /// The first cell box ([`cell_slot`]).
    pub const CELLS: u32 = 3;
}

/// The suspend id at which function 0 waits between InitializeEnvironment and Evaluate (the
/// front end's `bytecodegen/module.rs` `RUN`): its jump table entry 1 starts the statements.
const RUN: i32 = 1;

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
/// `Isolate::run_script` does. GC point.
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
    object::record_set(rt, m, modf::HAS_TLA, crate::frame::FALSE);
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
    for &(k, _) in &req.attributes {
        let key = string::to_rust_string(lu.string(k.index()));
        if key != "type" {
            let msg = format!("Import attribute \"{key}\" is not supported");
            return Err(throw_error(cx, ErrorKind::SyntaxError, &msg));
        }
    }
    if let Some(&(_, v)) = req.attributes.first() {
        let value = string::to_rust_string(lu.string(v.index()));
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

/// The record of the module named `name` in the current realm, made from `unit` if the map
/// has none. GC point.
fn module_for_unit(cx: &mut Cx, unit: VerifiedUnit, name: &str) -> Res<u64> {
    if unit.header.code_kind != CodeKind::Module {
        let msg = format!("{name} is not a module unit");
        return Err(throw_error(cx, ErrorKind::TypeError, &msg));
    }
    let rt = cx.rt();
    let key = atoms::intern_str(rt, name);
    if let Some(m) = map_get(cx.realm, key) {
        return Ok(m);
    }
    let lu = load_unit(rt, unit, name);
    let m = new_module(rt, lu, key);
    map_add(rt, cx.realm, key, m);
    Ok(m)
}

/// The host hooks of one load: `resolve(referrer, specifier)` gives the name of the module
/// (an `Err` text is thrown as an Error); `load(name)` compiles it.
pub(crate) struct Host<'h> {
    pub resolve: &'h mut dyn FnMut(&str, &str) -> Result<String, String>,
    pub load: &'h mut dyn FnMut(&str) -> CompileResult,
}

/// LoadRequestedModules (§16.2.1.5.1) with a synchronous host: every module that `root`
/// needs, transitively, gets a record in the realm's map and its `[[LoadedModules]]` entries.
/// GC point.
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
fn load_graph(cx: &mut Cx, root: u64, host: &mut Host<'_>) -> Res<()> {
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
                    let name = match (host.resolve)(&referrer, &spec) {
                        Ok(name) => name,
                        Err(msg) => return Err(throw_error(cx, ErrorKind::Error, &msg)),
                    };
                    let key = atoms::intern_str(rt, &name);
                    target = match map_get(cx.realm, key) {
                        Some(t) => t,
                        None => {
                            let unit =
                                (host.load)(&name).map_err(|f| throw_load_failure(cx, &name, f))?;
                            module_for_unit(cx, unit, &name)?
                        }
                    };
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
/// points (rule 8: ResolveExport's resolveSet and GetExportedNames' exportStarSet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModuleCell(u64);

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
    out.sort_by(|a, b| string::str_view(a.0.0).cmp(&string::str_view(b.0.0)));
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

/// InnerModuleEvaluation (§16.2.1.5.3.1) of a module with no top-level `await`.
fn inner_evaluate(cx: &mut Cx, m: u64, stack: &mut Stack<'_>, index: i32) -> Res<i32> {
    check_stack(cx)?;
    let rt = cx.rt();
    match status_of(m) {
        status::EVALUATED => {
            rethrow_cached(cx, m)?;
            return Ok(index);
        }
        status::EVALUATING | status::EVALUATING_ASYNC => return Ok(index),
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
    }
    // ExecuteModule (§16.2.1.6.5): the rest of the unit's function 0, `this` undefined.
    let g = object::context_get(environment(m), env::EVALUATE).unwrap_or(UNDEFINED);
    if !generator::is_generator(g) || object::gen_get_i32(g, genf::STATE) != RUN {
        return Err(throw_error(
            cx,
            ErrorKind::Error,
            "ep9 internal error: a module is evaluated before its environment is made",
        ));
    }
    generator::resume(cx, g, genf::RESUME_NEXT, UNDEFINED, "next")?;
    if get_i32(m, modf::DFS_ANCESTOR_INDEX) == get_i32(m, modf::DFS_INDEX) {
        loop {
            let done = stack.pop();
            set_status(rt, done, status::EVALUATED);
            object::record_set(rt, done, modf::CYCLE_ROOT, m);
            if done == m {
                break;
            }
        }
    }
    Ok(index)
}

/// Evaluate() (§16.2.1.5.3) without top-level `await`: on an error, every module on the
/// stack is evaluated with that error cached (a later import of one of them throws it
/// again). A termination makes every module on the stack [`status::ABORTED`] (no allocation
/// while it unwinds): a later Evaluate that reaches one of them throws an Error, and the
/// modules on that Evaluate's stack cache it. GC point (the modules run).
fn evaluate(cx: &mut Cx, root: u64) -> Res<()> {
    let rt = cx.rt();
    let mut stack = Stack {
        roots: rt.heap.scratch_roots(),
        len: 0,
    };
    match inner_evaluate(cx, root, &mut stack, 0) {
        Ok(_) => Ok(()),
        Err(e) => {
            if rt.terminating.get() {
                for i in 0..stack.len {
                    set_status(rt, stack.get(i), status::ABORTED);
                }
            } else {
                let error = rt.pending();
                for i in 0..stack.len {
                    let m = stack.get(i);
                    set_status(rt, m, status::EVALUATED);
                    object::record_set(rt, m, modf::EVALUATION_ERROR, error);
                }
            }
            Err(e)
        }
    }
}

/// Loads, links and evaluates the module unit `unit` named `name` in the current realm
/// (the body of `Cx::run_module`, isolate.rs). If the realm's map already has `name`, that
/// record runs and `unit` is dropped; a module that ran before does not run again (its cached
/// error is thrown again). The result is undefined. GC point.
pub(crate) fn run_module(
    cx: &mut Cx,
    unit: VerifiedUnit,
    name: &str,
    host: &mut Host<'_>,
) -> Res<u64> {
    let m = module_for_unit(cx, unit, name)?;
    load_graph(cx, m, host)?;
    link(cx, m)?;
    evaluate(cx, m)?;
    Ok(UNDEFINED)
}
