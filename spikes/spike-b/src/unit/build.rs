//! The spike's AOT build: source -> CompiledUnit -> unit text (the [`UnitEmitter`]) -> unit
//! bytes (the [`UnitFinalizer`]). No isolate runs: the function inputs come from the
//! CompiledUnit.

use ep9_bytecode::{CompiledUnit, UnitString, unit_flags};
use ep9_unit::UnitHeader;

use super::emit::{EXT_RUNTIME_TABLE, RwLayout, UnitEmitter, call_sites};
use super::format::{
    EXT_KIND_RUNTIME_TABLE, FnKind, Link, RwTemplate, UnitFinalizer, arch_code, function_digest,
    spike_host,
};
use crate::emit::{Arch, Emitter, Finalize, FunctionInput};
use crate::harness::compile;

/// What a unit holds.
#[derive(Clone, Debug)]
pub struct UnitSpec {
    /// For reports.
    pub name: &'static str,
    pub source: String,
    /// Its slot in the unit-data directory (fixed at build time).
    pub slot: u32,
    /// The functions that get native code (global function declarations of `source`).
    pub native: Vec<&'static str>,
    pub links: Vec<Link>,
}

/// The emitter input of the function called `name` in `cu` (key = its function id).
pub fn function_input<'u>(
    cu: &'u CompiledUnit,
    name: &'u str,
) -> Result<FunctionInput<'u>, String> {
    let want = UnitString::new(name);
    let (id, f) = cu
        .functions
        .iter()
        .enumerate()
        .find(|(_, f)| f.name.is_some_and(|s| *cu.string(s) == want))
        .ok_or_else(|| format!("no function {name}"))?;
    Ok(FunctionInput {
        name,
        bytecode: &f.bytecode,
        parameter_count: f.parameter_count,
        register_count: f.register_count,
        handler_count: f.handlers.len() as u32,
        key: id,
    })
}

/// The unit bytes of `spec` with code for `arch`. The header matches [`spike_host`] except
/// for the target (`arch`).
pub fn build(spec: &UnitSpec, arch: Arch) -> Result<Vec<u8>, String> {
    let unit = compile(&spec.source)?;
    let cu: &CompiledUnit = &unit;
    let inputs = spec
        .native
        .iter()
        .map(|n| function_input(cu, n))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ics = 0;
    for i in &inputs {
        ics += call_sites(i).map_err(|r| format!("{}: {r}", i.name))?;
    }
    let layout = RwLayout {
        slot: spec.slot,
        ext: EXT_RUNTIME_TABLE + 1,
        links: spec.links.len() as u32,
        ics,
        counters: inputs.len() as u32,
    };
    let mut em = UnitEmitter::new(arch, layout).map_err(|r| r.to_string())?;
    let mut codes = Vec::new();
    let mut meta = Vec::new();
    for i in &inputs {
        codes.push(em.emit(i).map_err(|r| format!("{}: {r}", i.name))?);
        let digest = function_digest(
            i.parameter_count,
            i.register_count,
            i.handler_count,
            i.bytecode,
        );
        meta.push((FnKind::Js, digest, em.assigned().last().copied()));
    }
    codes.push(em.probe().map_err(|r| r.to_string())?);
    meta.push((FnKind::Probe, [0; 32], None));
    codes.push(em.cell_reader().map_err(|r| r.to_string())?);
    meta.push((FnKind::Cell, [0; 32], None));
    let host = spike_host();
    let mut flags = cu.header.code_kind as u32;
    if cu.header.strict {
        flags |= unit_flags::STRICT;
    }
    let header = UnitHeader {
        total_size: 0,
        section_count: 0,
        header_flags: 0,
        content_hash: [0; 32],
        engine_build_id: host.engine_build_id,
        versions: host.versions,
        target_arch: arch_code(arch),
        target_abi: host.target_abi,
        heap_page_log2: host.heap_page_log2,
        source_flags: flags,
        cpu_required: 0,
    };
    UnitFinalizer {
        header,
        source: spec.source.clone(),
        bytecode: cu.to_bytes(),
        rw: RwTemplate {
            layout,
            ext: vec![EXT_KIND_RUNTIME_TABLE],
            links: spec.links.clone(),
        },
        meta,
    }
    .finalize(codes)
}
