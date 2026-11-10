//! The unit-load API (H20 part 2, M7-P4 A2-LOADER; plan v12 §7.3): the native part of an AOT
//! unit (H35) next to its bytecode. The embedder runs the H35 S4 checks first
//! (`ep9_unit::check`, steps 1-4) and decodes the function table; these methods are steps
//! 5-8 (`unit_loader.rs`).
//!
//! ```text
//! ep9_unit::check(bytes, host) == Load
//!   bytecode section --decode, verify--> VerifiedUnit
//!   text section, function table (C-17) --> functions: (index, digest, variants)
//! Cx::run_script_with_unit_code(unit, file, text, host cpu, functions)            safe
//!   LoadedUnit::load (strings interned once, H19) -> unit_loader::load (code sets,
//!   select(); never installs: each pick is "deferred") -> function 0 runs (as run_script)
//!   -> (the counts by reason, the completion value or the throw)
//! unsafe Cx::run_script_installing_unit_code(unit, file, text, unwind, rw, slot, host cpu,
//!                                             functions)
//!   the same, and if a function picked a unit object: the RW template checked, the shared
//!   mapped text (one per process), its unwind data registered (O3), the isolate's RW copy in
//!   unit-data slot `slot` (Cx::unit_data names the directory), step 8 installs the picks;
//!   the caller's # Safety is the H35 S4 check
//! ```
//!
//! Why two methods (route MC-w34-2, M7-P5 A2-RW; DL P5-DL-5): `ep9_vm` cannot run the H35 S4
//! checks itself (`ep9_unit` depends on `ep9_vm`, and the engine build id is made by
//! `ep9_unit/build.rs`), and matching header bytes would not make code bytes safe. So the safe
//! method never maps embedder bytes as code, and only the `unsafe fn` installs: its caller
//! states that `ep9_unit::check` gave `Load` for these bytes (H35:18, "a unit is trusted code,
//! like a native addon").
//!
//! A variant tuple is `(kind, lowering, entry, len, cpu_required, fuse ids)`: the dispatch kind
//! byte (3 AotBaseline, 4 AotOpt), the lowering class byte (1 hybrid, 2 jitless; 0 jit is
//! skipped), the code's offset and length in `text`, the H35-Q5 CPU bits and the static fuse
//! ids (H31). The safe method installs no unit code in a dispatch slot ("deferred"), so every
//! function runs its bytecode.

use ep9_bytecode::VerifiedUnit;
use ep9_value::JsResult;

use super::{Cx, Local};
use crate::frame::{HOLE, UNDEFINED};
use crate::unit::{FunctionInfo, LoadedUnit};
use crate::unit_loader::{self, FunctionCode, VariantCode};
use crate::{call, object};

impl<'s> Cx<'s> {
    /// The digest that binds the native code of function `function` of `unit` to its bytecode
    /// (the H35 function-table digest field; route MC-w31-8): what a unit writer stores and
    /// what [`Cx::run_script_with_unit_code`] compares. `None` if `unit` has no such function.
    /// The value is fixed for one engine build (H35 S4 step 3 ties a unit to its build).
    pub fn unit_function_digest(&self, unit: &VerifiedUnit, function: u32) -> Option<[u8; 16]> {
        unit_loader::unit_function_digest(unit, function)
    }

    /// Runs the script `unit` as [`Cx::run_script`] does, after the unit loader read its
    /// native part: `text` (the unit's text section) and `functions` (its function table: per
    /// function its index in `unit`, its digest and its variants; see the module doc). The
    /// loader maps the text into the process code range, gives each listed function a code
    /// set and calls `select()` with the isolate's policy and a session with the CPU bits
    /// `host_cpu`. Returns the load counts by reason, `(name, count)` in a fixed order (H35
    /// S5), and the run's result. A function whose variants are all skipped, or that is not
    /// listed, runs its bytecode. This safe method never installs unit code (each pick counts
    /// as "deferred"), so every function runs its bytecode: the installing form is the unsafe
    /// [`Cx::run_script_installing_unit_code`] (route MC-w34-2).
    // The H35 function-table entry as plain data: no new pub type (H20 part 2: Cx methods only).
    #[allow(clippy::type_complexity)]
    pub fn run_script_with_unit_code(
        &mut self,
        unit: VerifiedUnit,
        file: &str,
        text: &[u8],
        host_cpu: u64,
        functions: &[(u32, [u8; 16], &[(u8, u8, u32, u32, u64, &[u32])])],
    ) -> (Vec<(&'static str, u32)>, JsResult<Local<'s>>) {
        self.run_unit(unit, file, text, None, host_cpu, functions)
    }

    /// [`Cx::run_script_with_unit_code`], and, if a function picked a unit object, steps 5, 7
    /// and 8 (H35 S4): the loader maps `text` once per process (a later load of the same text
    /// and unwind bytes shares it), registers `unwind` (the unit's unwind section in the form
    /// of the target: `crates/ep9_vm/src/unit_loader.rs` module doc), makes the isolate's RW
    /// copy of `rw` (the unit's RW template, form 1: link cells, IC cells, counters; empty for
    /// a unit with no RW data) in unit-data slot `slot` (the header's `unit_data_slot`,
    /// H35-Q9) and installs each
    /// AotBaseline pick with no fuse dependency in its function's dispatch slot, so a call of
    /// that function runs its unit code. A pick that cannot be installed (AotOpt, a fuse
    /// dependency, a function kind with no native entry yet) counts as "not-installable", a
    /// text that does not map as "no-text", unwind data that is empty or does not register as
    /// "no-unwind", an RW template that is not form 1 or names a link out of bounds as "link",
    /// and a slot of 64 or more or one that another unit of this isolate holds as "slot"; each
    /// runs its bytecode.
    ///
    /// # Safety
    /// The caller ran the H35 S4 checks, steps 1-4, on the unit bytes that `unit`, `text` and
    /// `unwind` and `functions` come from, against this engine build: `ep9_unit::check(bytes,
    /// &HostInfo::current())` gave `Load`, so (1) the prefix and the section table are sound,
    /// (2) the content hash (and a required signature) matches, (3) the engine build id and the
    /// bytecode ABI are this build's, and (4) every version, the target and the CPU features
    /// match. `unit` is the unit's bytecode section, `text` its text section, `unwind` its
    /// unwind section, `rw` its RW template section, `slot` its header's unit-data slot and
    /// `functions` its function table, unchanged. So the unit is one that this engine build wrote, and each
    /// variant's code is the native code of its function (H35:18: a unit is trusted code; a
    /// unit of another build never gets here, because step 3 sends it to its source). And
    /// `host_cpu` has no bit that this CPU lacks (the CPU bits of `HostInfo::current()`,
    /// H35-Q5): `select()` picks a variant by these bits, so a variant that needs a feature this
    /// CPU does not have would install and run.
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub unsafe fn run_script_installing_unit_code(
        &mut self,
        unit: VerifiedUnit,
        file: &str,
        text: &[u8],
        unwind: &[u8],
        rw: &[u8],
        slot: u8,
        host_cpu: u64,
        functions: &[(u32, [u8; 16], &[(u8, u8, u32, u32, u64, &[u32])])],
    ) -> (Vec<(&'static str, u32)>, JsResult<Local<'s>>) {
        let native = (unwind, rw, slot);
        self.run_unit(unit, file, text, Some(native), host_cpu, functions)
    }

    /// The body of both unit-code methods; steps 5-8 only with `native` (the unwind section,
    /// the RW template and the slot; only the unsafe method passes it; its caller contract
    /// stands for the install contract).
    #[allow(clippy::type_complexity)]
    fn run_unit(
        &mut self,
        unit: VerifiedUnit,
        file: &str,
        text: &[u8],
        native: Option<(&[u8], &[u8], u8)>,
        host_cpu: u64,
        functions: &[(u32, [u8; 16], &[(u8, u8, u32, u32, u64, &[u32])])],
    ) -> (Vec<(&'static str, u32)>, JsResult<Local<'s>>) {
        let functions: Vec<FunctionCode> = functions
            .iter()
            .map(|&(function, digest, variants)| FunctionCode {
                function,
                digest,
                variants: variants
                    .iter()
                    .map(|&(kind, lowering, entry, len, cpu, fuses)| VariantCode {
                        kind,
                        lowering,
                        entry,
                        len,
                        cpu,
                        fuses: fuses.to_vec(),
                    })
                    .collect(),
            })
            .collect();
        let cx = self.inner();
        let rt = cx.rt();
        let module = unit.header.code_kind == ep9_bytecode::CodeKind::Module;
        let index = u32::try_from(rt.units.borrow().len()).expect("fewer than 2^32 units");
        let lu = LoadedUnit::load(rt, unit, file.to_string(), index);
        let policy = unit_loader::isolate_policy(rt);
        let report = match native {
            None => unit_loader::load(rt, &lu, text, &functions, policy.as_ref(), host_cpu),
            // SAFETY: only `run_script_installing_unit_code` passes `native`; its caller ran the
            // H35 S4 checks (steps 1-4) on these bytes against this build, and `host_cpu` has no
            // bit that this CPU lacks (the CPU bits of `HostInfo::current()`, H35-Q5) (its
            // `# Safety`), which is the contract of `load_and_install`.
            Some((unwind, rw, slot)) => unsafe {
                unit_loader::load_and_install(
                    rt,
                    &lu,
                    text,
                    unwind,
                    rw,
                    slot,
                    &functions,
                    policy.as_ref(),
                    host_cpu,
                )
            },
        };
        if let Some(directory) = report.unit_data {
            // Unit code loads its unit-data register from this directory (H35-Q9).
            cx.unit_data = directory;
        }
        let entry: *const FunctionInfo = &lu.infos[0];
        rt.units.borrow_mut().push(lu);
        let f = object::new_function(rt, entry, UNDEFINED);
        let this = if module { UNDEFINED } else { rt.global() };
        let r = call::call_function(cx, f, this, &[])
            .map(|b| Self::local(if b == HOLE { UNDEFINED } else { b }));
        (report.rows(), r)
    }
}
