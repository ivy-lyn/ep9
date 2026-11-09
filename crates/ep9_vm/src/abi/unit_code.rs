//! The unit-load API (H20 part 2, M7-P4 A2-LOADER; plan v12 §7.3): the native part of an AOT
//! unit (H35) next to its bytecode. The embedder runs the H35 S4 checks first
//! (`ep9_unit::check`, steps 1-4) and decodes the function table; these methods are steps
//! 5-8 (`unit_loader.rs`).
//!
//! ```text
//! ep9_unit::check(bytes, host) == Load
//!   bytecode section --decode, verify--> VerifiedUnit
//!   text section, function table (C-17) --> functions: (index, digest, variants)
//! Cx::run_script_with_unit_code(unit, file, text, host cpu, functions)
//!   LoadedUnit::load (strings interned once, H19) -> unit_loader::load (map, code sets,
//!   select()) -> function 0 runs (as Cx::run_script)
//!   -> (the counts by reason, the completion value or the throw)
//! ```
//!
//! A variant tuple is `(kind, lowering, entry, len, cpu_required, fuse ids)`: the dispatch kind
//! byte (3 AotBaseline, 4 AotOpt), the lowering class byte (1 hybrid, 2 jitless; 0 jit is
//! skipped), the code's offset and length in `text`, the H35-Q5 CPU bits and the static fuse
//! ids (H31). In M7-P4 no unit code is installed in a dispatch slot ("deferred"): the T0
//! call paths cannot enter it yet (route MC-w30-2), so every function runs its bytecode.

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
    /// listed, runs its bytecode; in M7-P4 every function does (no unit code is installed yet).
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
        let report = unit_loader::load(rt, &lu, text, &functions, policy.as_ref(), host_cpu);
        let entry: *const FunctionInfo = &lu.infos[0];
        rt.units.borrow_mut().push(lu);
        let f = object::new_function(rt, entry, UNDEFINED);
        let this = if module { UNDEFINED } else { rt.global() };
        let r = call::call_function(cx, f, this, &[])
            .map(|b| Self::local(if b == HOLE { UNDEFINED } else { b }));
        (report.rows(), r)
    }
}
