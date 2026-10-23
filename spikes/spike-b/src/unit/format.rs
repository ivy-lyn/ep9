//! The spike's unit writer and reader: an `ep9_unit` container (header v0, H35 S6) with the
//! sections SP-Unit fills. The A2 writer does not exist yet; this one writes only what the
//! spike loads.
//!
//! ```text
//! header (256 B, ep9_unit::UnitHeader) | section table | sections, 8-aligned, in kind order:
//!   text        the functions, each at a 16-aligned offset (entry = offset 0 of its code)
//!   rodata      "SPUR" 1, slot, arch, function table: kind, text offset, length, counter,
//!               first IC cell, IC cells, bytecode digest (SHA-256), name
//!   rw_template "SPUW" 1, the RW layout (ext, links, ics, counters), one record per external
//!               reference and per link cell
//!   bytecode    the CompiledUnit bytes (ep9_bytecode, H2)
//!   source      the source text (H5)
//!   unwind      "SPUX" 1, an .eh_frame image whose FDEs start at text offsets (pc_begin is a
//!               text offset; the loader adds the text address to its own copy), then the Win64
//!               UNWIND_INFO of each x86-64 function (RUNTIME_FUNCTION fields are offsets)
//! content hash: SHA-256 of the whole unit with the hash field set to 0 (no signature section)
//! ```
//!
//! Nothing in any section is a relocation of the text: the text bytes in the file are the
//! bytes that run.

use std::ops::Range;

use ep9_bytecode::sha256::sha256;
use ep9_platform::unwind::cfi::EhFrameBuilder;
use ep9_unit::{
    HostInfo, SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET, SectionEntry, SectionKind, UnitHeader,
    offsets, target_arch,
};
use ep9_vm::IntrinsicId;

use super::emit::{Assigned, RwLayout};
use crate::emit::{Arch, Finalize, FunctionCode};

/// The deopt-metadata version the spike's units and its loader use. `ep9_vm` has
/// `DEOPT_META_VERSION` 0 ("no native form yet", H35 S6), so the real host gives BYTECODE for
/// every unit with a text section. Spike code has no deopt point, so the spike loads its own
/// units against a host with this version (a decision row for the orchestrator).
pub const SPIKE_DEOPT_META: u32 = 1;

/// What the spike's loader compares a unit with: [`HostInfo::current`] with
/// [`SPIKE_DEOPT_META`].
pub fn spike_host() -> HostInfo {
    let mut h = HostInfo::current();
    h.versions.deopt_meta = SPIKE_DEOPT_META;
    h
}

/// The `ep9_unit` target-arch code of `arch`.
pub fn arch_code(arch: Arch) -> u8 {
    match arch {
        Arch::Aarch64 => target_arch::AARCH64,
        Arch::X86_64 => target_arch::X86_64,
    }
}

/// The kind of a function in the unit text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnKind {
    /// A JS function (placement (b) entry; installed in an Opt slot).
    Js = 0,
    /// The unwind self-test probe.
    Probe = 1,
    /// The cell reader.
    Cell = 2,
}

impl FnKind {
    fn from_u32(v: u32) -> Option<FnKind> {
        match v {
            0 => Some(FnKind::Js),
            1 => Some(FnKind::Probe),
            2 => Some(FnKind::Cell),
            _ => None,
        }
    }
}

/// A row of the function table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnEntry {
    pub kind: FnKind,
    pub name: String,
    /// Its code in the text section.
    pub text: Range<u32>,
    pub counter: u32,
    pub ic_base: u32,
    pub ic_count: u32,
    /// [`function_digest`] of the bytecode it was compiled from (zeros if not a JS function).
    pub digest: [u8; 32],
}

/// The digest that ties native code to the bytecode it was made from: SHA-256 over the
/// parameter, register and handler counts and the bytecode.
pub fn function_digest(params: u32, registers: u32, handlers: u32, bytecode: &[u8]) -> [u8; 32] {
    let mut b = Vec::with_capacity(12 + bytecode.len());
    b.extend_from_slice(&params.to_le_bytes());
    b.extend_from_slice(&registers.to_le_bytes());
    b.extend_from_slice(&handlers.to_le_bytes());
    b.extend_from_slice(bytecode);
    sha256(&b)
}

/// A link cell: what it resolves to at load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// The intrinsic with this stable index (`ep9_vm::IntrinsicId::ALL`, H13) of the current
    /// realm.
    Intrinsic(u16),
    /// A unit constant: a new string with this text, made once per isolate.
    Str(String),
}

/// The RW template: the layout and the records of the external references and link cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RwTemplate {
    pub layout: RwLayout,
    /// External references; 1 = the `RuntimeTable`.
    pub ext: Vec<u32>,
    pub links: Vec<Link>,
}

/// External reference kind 1: the `RuntimeTable`.
pub const EXT_KIND_RUNTIME_TABLE: u32 = 1;

/// One function's Win64 UNWIND_INFO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Win64Entry {
    pub text: Range<u32>,
    pub info: Vec<u8>,
}

/// The unwind section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnwindSection {
    /// `.eh_frame` with text-offset `pc_begin`s.
    pub eh_frame: Vec<u8>,
    pub win64: Vec<Win64Entry>,
}

// ---- byte writer and reader ------------------------------------------------------------------

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
        self
    }
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> R<'a> {
    fn new(b: &'a [u8]) -> R<'a> {
        R { b, at: 0 }
    }
    fn u32(&mut self) -> Result<u32, String> {
        let v = self
            .b
            .get(self.at..self.at + 4)
            .ok_or("section ends early")?;
        self.at += 4;
        Ok(u32::from_le_bytes(v.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        let v = self
            .b
            .get(self.at..self.at + n)
            .ok_or("section ends early")?;
        self.at = (self.at + n).next_multiple_of(4);
        Ok(v)
    }
    fn tag(&mut self, t: &[u8; 4]) -> Result<(), String> {
        let v = self.b.get(self.at..self.at + 4).ok_or("no section tag")?;
        if v != t {
            return Err(format!("bad section tag {v:?}"));
        }
        self.at += 4;
        if self.u32()? != 1 {
            return Err("section version is not 1".into());
        }
        Ok(())
    }
}

// ---- the finalizer ---------------------------------------------------------------------------

/// The [`Finalize`] that places compiled functions in unit text and writes the unit.
#[derive(Clone, Debug)]
pub struct UnitFinalizer {
    pub header: UnitHeader,
    pub source: String,
    /// The CompiledUnit bytes.
    pub bytecode: Vec<u8>,
    pub rw: RwTemplate,
    /// Per function given to `finalize` (same order): its kind, bytecode digest and the words
    /// the emitter gave it (`None` for the probe and the cell reader).
    pub meta: Vec<(FnKind, [u8; 32], Option<Assigned>)>,
}

/// Text alignment of each function.
pub const FN_ALIGN: usize = 16;

impl Finalize for UnitFinalizer {
    type Output = Vec<u8>;
    type Error = String;

    fn finalize(self, functions: Vec<FunctionCode>) -> Result<Vec<u8>, String> {
        if functions.len() != self.meta.len() || functions.is_empty() {
            return Err(format!(
                "{} functions, {} meta rows",
                functions.len(),
                self.meta.len()
            ));
        }
        let arch = functions[0].arch;
        if functions.iter().any(|f| f.arch != arch) {
            return Err("functions of two architectures".into());
        }
        if self.header.target_arch != arch_code(arch) {
            return Err("the header names another architecture".into());
        }
        // text
        let mut text = Vec::new();
        let mut entries = Vec::new();
        for (f, (kind, digest, a)) in functions.iter().zip(&self.meta) {
            while !text.len().is_multiple_of(FN_ALIGN) {
                text.push(0);
            }
            let start = text.len() as u32;
            text.extend_from_slice(&f.code);
            let a = a.unwrap_or(Assigned {
                key: usize::MAX,
                counter: u32::MAX,
                ic_base: 0,
                ic_count: 0,
            });
            entries.push(FnEntry {
                kind: *kind,
                name: f.name.clone(),
                text: start..text.len() as u32,
                counter: a.counter,
                ic_base: a.ic_base,
                ic_count: a.ic_count,
                digest: *digest,
            });
        }
        // rodata
        let mut ro = W::default();
        ro.0.extend_from_slice(b"SPUR");
        ro.u32(1)
            .u32(self.rw.layout.slot)
            .u32(u32::from(arch_code(arch)))
            .u32(entries.len() as u32);
        for e in &entries {
            ro.u32(e.kind as u32)
                .u32(e.text.start)
                .u32(e.text.end)
                .u32(e.counter)
                .u32(e.ic_base)
                .u32(e.ic_count)
                .bytes(&e.digest)
                .bytes(e.name.as_bytes());
        }
        // RW template
        let l = self.rw.layout;
        if self.rw.ext.len() != l.ext as usize || self.rw.links.len() != l.links as usize {
            return Err("the RW records do not match the layout".into());
        }
        let mut rw = W::default();
        rw.0.extend_from_slice(b"SPUW");
        rw.u32(1)
            .u32(l.slot)
            .u32(l.ext)
            .u32(l.links)
            .u32(l.ics)
            .u32(l.counters);
        for &e in &self.rw.ext {
            rw.u32(e);
        }
        for link in &self.rw.links {
            match link {
                Link::Intrinsic(i) => rw.u32(1).u32(u32::from(*i)),
                Link::Str(s) => rw.u32(2).bytes(s.as_bytes()),
            };
        }
        // unwind
        let mut eh = EhFrameBuilder::new(&arch.cie());
        for (f, e) in functions.iter().zip(&entries) {
            eh.add_fde(e.text.start as usize, f.code.len(), &f.unwind.cfi);
        }
        let eh = eh.finish();
        let mut uw = W::default();
        uw.0.extend_from_slice(b"SPUX");
        uw.u32(1).bytes(eh.bytes());
        let win: Vec<(&FnEntry, &Vec<u8>)> = entries
            .iter()
            .zip(&functions)
            .filter_map(|(e, f)| f.unwind.win64.as_ref().map(|w| (e, w)))
            .collect();
        if arch == Arch::X86_64 && win.len() != entries.len() {
            return Err("an x86-64 function has no UNWIND_INFO".into());
        }
        uw.u32(win.len() as u32);
        for (e, w) in win {
            uw.u32(e.text.start).u32(e.text.end).bytes(w);
        }
        let sections = vec![
            (SectionKind::Text, text),
            (SectionKind::Rodata, ro.0),
            (SectionKind::RwTemplate, rw.0),
            (SectionKind::Bytecode, self.bytecode),
            (SectionKind::Source, self.source.into_bytes()),
            (SectionKind::Unwind, uw.0),
        ];
        Ok(encode(self.header, &sections))
    }
}

/// The canonical container bytes of `sections` (kind order, 8-aligned, zero padding) with
/// the content hash written.
pub fn encode(header: UnitHeader, sections: &[(SectionKind, Vec<u8>)]) -> Vec<u8> {
    let mut sections = sections.to_vec();
    sections.sort_by_key(|(k, _)| *k);
    let table_end = SECTION_TABLE_OFFSET + sections.len() * SECTION_ENTRY_SIZE;
    let mut table = Vec::new();
    let mut body = Vec::new();
    let mut end = table_end;
    for (kind, data) in &sections {
        let align = 1usize << kind.align_log2();
        let start = end.next_multiple_of(align);
        body.resize(body.len() + (start - end), 0);
        table.push(SectionEntry {
            kind: *kind,
            offset: start as u64,
            size: data.len() as u64,
        });
        body.extend_from_slice(data);
        end = start + data.len();
    }
    let mut h = header;
    h.total_size = end as u64;
    h.section_count = sections.len() as u32;
    h.content_hash = [0; 32];
    let mut out = h.encode().to_vec();
    for e in &table {
        out.extend_from_slice(&e.encode());
    }
    out.extend_from_slice(&body);
    rehash(&mut out);
    out
}

/// Writes the content hash of `bytes` (a unit with no signature section) into its hash field.
pub fn rehash(bytes: &mut [u8]) {
    let r = offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32;
    bytes[r.clone()].fill(0);
    let h = sha256(bytes);
    bytes[r].copy_from_slice(&h);
}

// ---- the reader ------------------------------------------------------------------------------

/// The sections of a unit that passed `ep9_unit::check` (the check already validated the
/// header, the table and the hash).
#[derive(Clone, Debug)]
pub struct UnitView<'a> {
    pub header: UnitHeader,
    pub text: &'a [u8],
    pub slot: u32,
    pub arch: u8,
    pub functions: Vec<FnEntry>,
    pub rw: RwTemplate,
    pub bytecode: &'a [u8],
    pub source: &'a str,
    pub unwind: UnwindSection,
}

/// The byte range of section `kind`, if the unit has it.
pub fn section(bytes: &[u8], kind: SectionKind) -> Result<Option<&[u8]>, String> {
    let h = UnitHeader::decode(bytes).map_err(|e| e.to_string())?;
    for i in 0..h.section_count as usize {
        let at = SECTION_TABLE_OFFSET + i * SECTION_ENTRY_SIZE;
        let e = SectionEntry::decode(
            bytes
                .get(at..at + SECTION_ENTRY_SIZE)
                .ok_or("table runs past the end")?,
        )
        .map_err(|e| e.to_string())?;
        if e.kind == kind {
            let s = e.offset as usize;
            return Ok(Some(
                bytes
                    .get(s..s + e.size as usize)
                    .ok_or("section runs past the end")?,
            ));
        }
    }
    Ok(None)
}

impl<'a> UnitView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<UnitView<'a>, String> {
        let header = UnitHeader::decode(bytes).map_err(|e| e.to_string())?;
        let need = |k: SectionKind| -> Result<&'a [u8], String> {
            section(bytes, k)?.ok_or_else(|| format!("no {} section", k.name()))
        };
        let text = need(SectionKind::Text)?;
        let mut r = R::new(need(SectionKind::Rodata)?);
        r.tag(b"SPUR")?;
        let slot = r.u32()?;
        let arch = r.u32()? as u8;
        let n = r.u32()?;
        let mut functions = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let kind = FnKind::from_u32(r.u32()?).ok_or("bad function kind")?;
            let (start, end) = (r.u32()?, r.u32()?);
            let (counter, ic_base, ic_count) = (r.u32()?, r.u32()?, r.u32()?);
            let digest: [u8; 32] = r.bytes()?.try_into().map_err(|_| "bad digest")?;
            let name = String::from_utf8(r.bytes()?.to_vec()).map_err(|e| e.to_string())?;
            if start > end || end as usize > text.len() {
                return Err(format!("{name}: code outside the text"));
            }
            functions.push(FnEntry {
                kind,
                name,
                text: start..end,
                counter,
                ic_base,
                ic_count,
                digest,
            });
        }
        let mut r = R::new(need(SectionKind::RwTemplate)?);
        r.tag(b"SPUW")?;
        let layout = RwLayout {
            slot: r.u32()?,
            ext: r.u32()?,
            links: r.u32()?,
            ics: r.u32()?,
            counters: r.u32()?,
        };
        if layout.slot != slot {
            return Err("rodata and RW template name two slots".into());
        }
        // The loader resolves each external reference and link cell after the unit's script
        // ran, so every record it can fail on is checked here: a bad unit is a LOAD ERROR
        // before any section runs (H35 S3).
        let ext: Vec<u32> = (0..layout.ext).map(|_| r.u32()).collect::<Result<_, _>>()?;
        if let Some(k) = ext.iter().find(|&&k| k != EXT_KIND_RUNTIME_TABLE) {
            return Err(format!("unknown external reference kind {k}"));
        }
        let mut links = Vec::new();
        for _ in 0..layout.links {
            links.push(match r.u32()? {
                1 => {
                    let k = r.u32()?;
                    if k as usize >= IntrinsicId::ALL.len() {
                        return Err(format!("no intrinsic {k}"));
                    }
                    Link::Intrinsic(k as u16)
                }
                2 => Link::Str(String::from_utf8(r.bytes()?.to_vec()).map_err(|e| e.to_string())?),
                k => return Err(format!("bad link kind {k}")),
            });
        }
        let mut r = R::new(need(SectionKind::Unwind)?);
        r.tag(b"SPUX")?;
        let eh_frame = r.bytes()?.to_vec();
        let nw = r.u32()?;
        let mut win64 = Vec::new();
        for _ in 0..nw {
            let (s, e) = (r.u32()?, r.u32()?);
            win64.push(Win64Entry {
                text: s..e,
                info: r.bytes()?.to_vec(),
            });
        }
        let source = std::str::from_utf8(need(SectionKind::Source)?).map_err(|e| e.to_string())?;
        Ok(UnitView {
            header,
            text,
            slot,
            arch,
            functions,
            rw: RwTemplate { layout, ext, links },
            bytecode: need(SectionKind::Bytecode)?,
            source,
            unwind: UnwindSection { eh_frame, win64 },
        })
    }

    /// The function called `name` of kind `kind`.
    pub fn function(&self, kind: FnKind, name: &str) -> Option<&FnEntry> {
        self.functions
            .iter()
            .find(|f| f.kind == kind && (kind != FnKind::Js || f.name == name))
    }
}
