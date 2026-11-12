//! The `.o` form of a unit (H35 S1; H35-Q3 filled in M8-P3; route MC-w34-5): one relocatable
//! object per unit container, written by hand for ELF, Mach-O and COFF (no object crate: no new
//! dependency, M8-PLAN-D29). The static linker puts the unit text into the binary's own
//! executable pages and its unwind entries into the binary's own unwind tables, so a binary
//! that may not make pages executable at run time (hardened-runtime macOS without the JIT
//! entitlement, iOS, Windows with ACG) runs the unit code. The embedder registers the record
//! (`ep9_platform::linked::register`, static registration), checks the container with
//! [`crate::check_linked`] and passes the linked text to the unsafe `Cx` unit-code entry.
//!
//! | Part | ELF (Linux) | Mach-O (Apple) | COFF (Windows x64) | Content |
//! |---|---|---|---|---|
//! | text | `.text.ep9u` (AX, 16) | `__TEXT,__ep9u_text` (pure instructions, 16) | `.text$ep9u` (code, RX, 16) | the container's text section, unchanged; no relocation |
//! | unit | `.rodata.ep9u` (A, 16) | `__TEXT,__ep9u_unit` (16) | `.rdata$ep9u` (R, 16) | the whole container, unchanged (`check` reads it; the text bytes are in it too) |
//! | record | `.data.rel.ro.ep9u` (WA, 8) | `__DATA,__ep9u_rec` (8) | `.rdata$ep9ur` (R, 8) | `ep9_platform::linked::LinkedUnit` (48 bytes): 2 absolute addresses (unit, text), the only data relocations |
//! | unwind | `.eh_frame` (`pc_begin` pcrel sdata4; PC32 / PREL32) | `__TEXT,__eh_frame` (pcrel 8 bytes; arm64 SUBTRACTOR + UNSIGNED, x86-64 none, as LLVM writes it) | `.pdata` (the RUNTIME_FUNCTION entries; ADDR32NB; the UNWIND_INFO bytes stay in the text) | the container's unwind section (form 1, `ep9_vm` `unit_loader.rs`), rebased on the text |
//! | symbol | the record symbol, global (ELF `<name>`, Mach-O `_<name>`, COFF `<name>`) | | | |
//! | Mach-O text symbols | | a local symbol at each FDE start (`ltmp_ep9u_f<k>`); no `MH_SUBSECTIONS_VIA_SYMBOLS` | | ld64 splits a section into atoms at its symbols and keeps one FDE per atom: with no symbol at an FDE start it drops that FDE (JIT review M8-P3 F1); without the flag the text stays one atom |
//!
//! ELF also gets an empty `.note.GNU-stack` (no executable stack). The target comes from the
//! header (`target_arch`, `target_abi`); Windows on AArch64 has no form (no tier-1 target).
//! The text section of the object has zero relocations (H35 S5; the tests check it).

use core::fmt;

use crate::check::unit_section;
use crate::codes::{SectionKind, target_abi, target_arch};
use crate::header::UnitHeader;

/// The object file format of a unit's target ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjFormat {
    /// ELF64 relocatable (Linux).
    Elf,
    /// Mach-O 64-bit object (Apple).
    MachO,
    /// COFF object (Windows x64).
    Coff,
}

/// Why [`write_object`] wrote nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjError {
    /// The bytes do not pass the H35 S4 steps 1 and 2 (layout, content hash).
    NotAUnit,
    /// The unit has no text section, or no unwind section for it.
    NoText,
    /// The header names a target with no `.o` form.
    Target {
        /// `target_arch`.
        arch: u8,
        /// `target_abi`.
        abi: u8,
    },
    /// The unwind section is not form 1 of the target, or names code outside the text.
    Unwind(&'static str),
    /// The symbol name is not `[A-Za-z_][A-Za-z0-9_]*`.
    Symbol,
}

impl fmt::Display for ObjError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjError::NotAUnit => f.write_str("not a unit (layout or content hash)"),
            ObjError::NoText => f.write_str("no text section, or no unwind section"),
            ObjError::Target { arch, abi } => {
                write!(f, "no .o form for target_arch {arch}, target_abi {abi}")
            }
            ObjError::Unwind(m) => write!(f, "unwind section: {m}"),
            ObjError::Symbol => f.write_str("bad symbol name"),
        }
    }
}

impl std::error::Error for ObjError {}

/// The section names of the `.o` form in `format`: (text, unit, record, unwind), as the
/// module table lists them (Mach-O names as `segment,section`).
pub const fn section_names(format: ObjFormat) -> [&'static str; 4] {
    match format {
        ObjFormat::Elf => [
            ".text.ep9u",
            ".rodata.ep9u",
            ".data.rel.ro.ep9u",
            ".eh_frame",
        ],
        ObjFormat::MachO => [
            "__TEXT,__ep9u_text",
            "__TEXT,__ep9u_unit",
            "__DATA,__ep9u_rec",
            "__TEXT,__eh_frame",
        ],
        ObjFormat::Coff => [".text$ep9u", ".rdata$ep9u", ".rdata$ep9ur", ".pdata"],
    }
}

/// The object format and architecture of a unit header: `None` for a target with no form.
pub fn target_of(h: &UnitHeader) -> Option<(ObjFormat, u8)> {
    let format = match h.target_abi {
        target_abi::LINUX => ObjFormat::Elf,
        target_abi::APPLE => ObjFormat::MachO,
        target_abi::WINDOWS if h.target_arch == target_arch::X86_64 => ObjFormat::Coff,
        _ => return None,
    };
    matches!(h.target_arch, target_arch::X86_64 | target_arch::AARCH64)
        .then_some((format, h.target_arch))
}

/// The magic of the record (`ep9_platform::linked::LinkedUnit::MAGIC`).
pub const RECORD_MAGIC: [u8; 8] = *b"EP9ULNK\0";
/// The record version.
pub const RECORD_VERSION: u32 = 1;
/// The record size.
pub const RECORD_SIZE: usize = 48;
/// The offset of the container address in the record.
pub const RECORD_UNIT: usize = 16;
/// The offset of the linked-text address in the record.
pub const RECORD_TEXT: usize = 32;

/// The Apple OS of a Mach-O object: its `LC_BUILD_VERSION` platform and lowest OS version
/// (the Rust target's default, so the static linker of any build for that OS takes it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppleOs {
    /// macOS: 11.0 on AArch64, 10.12 on x86-64.
    MacOs,
    /// iOS: 10.0.
    Ios,
    /// The iOS simulator: 14.0 on AArch64, 10.0 on x86-64.
    IosSimulator,
}

impl AppleOs {
    /// (platform, minos as `xxxx.yy.zz` nibbles) for `arch`.
    const fn build_version(self, arch: u8) -> (u32, u32) {
        let a64 = arch == target_arch::AARCH64;
        match self {
            AppleOs::MacOs if a64 => (1, 11 << 16),
            AppleOs::MacOs => (1, (10 << 16) | (12 << 8)),
            AppleOs::Ios => (2, 10 << 16),
            AppleOs::IosSimulator if a64 => (7, 14 << 16),
            AppleOs::IosSimulator => (7, 10 << 16),
        }
    }
}

/// Writes the `.o` form of the container `unit` with the record symbol `symbol` (a Mach-O
/// object for macOS: [`write_object_for`] names another Apple OS).
pub fn write_object(unit: &[u8], symbol: &str) -> Result<Vec<u8>, ObjError> {
    write_object_for(unit, symbol, AppleOs::MacOs)
}

/// [`write_object`] with the Apple OS of a Mach-O object (unused for ELF and COFF).
pub fn write_object_for(unit: &[u8], symbol: &str, apple: AppleOs) -> Result<Vec<u8>, ObjError> {
    let ok_symbol = !symbol.is_empty()
        && symbol
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()));
    if !ok_symbol {
        return Err(ObjError::Symbol);
    }
    let h = UnitHeader::decode(unit).map_err(|_| ObjError::NotAUnit)?;
    unit_section(unit, SectionKind::Bytecode).ok_or(ObjError::NotAUnit)?;
    let text = unit_section(unit, SectionKind::Text).ok_or(ObjError::NoText)?;
    let unwind = unit_section(unit, SectionKind::Unwind).ok_or(ObjError::NoText)?;
    if text.is_empty() || unwind.is_empty() {
        return Err(ObjError::NoText);
    }
    let (format, arch) = target_of(&h).ok_or(ObjError::Target {
        arch: h.target_arch,
        abi: h.target_abi,
    })?;
    let mut rec = Vec::with_capacity(RECORD_SIZE);
    rec.extend_from_slice(&RECORD_MAGIC);
    rec.extend_from_slice(&RECORD_VERSION.to_le_bytes());
    rec.extend_from_slice(&0u32.to_le_bytes());
    rec.extend_from_slice(&0u64.to_le_bytes()); // unit address (relocated)
    rec.extend_from_slice(&(unit.len() as u64).to_le_bytes());
    rec.extend_from_slice(&0u64.to_le_bytes()); // text address (relocated)
    rec.extend_from_slice(&(text.len() as u64).to_le_bytes());
    let parts = Parts {
        arch,
        text,
        unit,
        rec,
        symbol,
        apple,
    };
    match format {
        ObjFormat::Elf => {
            let (eh, fields) = eh_frame(unwind, text.len(), EhForm::Sdata4)?;
            Ok(elf(&parts, &eh, &fields))
        }
        ObjFormat::MachO => {
            let (eh, fields) = eh_frame(unwind, text.len(), EhForm::Pcrel8)?;
            Ok(macho(&parts, eh, &fields))
        }
        ObjFormat::Coff => {
            pdata_check(unwind, text.len())?;
            Ok(coff(&parts, unwind))
        }
    }
}

/// The parts that every format writes.
struct Parts<'a> {
    arch: u8,
    text: &'a [u8],
    unit: &'a [u8],
    rec: Vec<u8>,
    symbol: &'a str,
    apple: AppleOs,
}

// ---------------------------------------------------------------------------------------------
// The unwind section.

/// A converted `.eh_frame`: the bytes and, per FDE, (its `pc_begin` field offset, the text
/// offset).
type EhFrame = (Vec<u8>, Vec<(usize, u64)>);

/// The `pc_begin` form of the converted `.eh_frame`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EhForm {
    /// `DW_EH_PE_pcrel | DW_EH_PE_sdata4` (0x1b): 4-byte `pc_begin` and `pc_range` (ELF, the
    /// form a PIE needs: no dynamic relocation in a read-only section).
    Sdata4,
    /// `DW_EH_PE_pcrel` (0x10): 8-byte `pc_begin` and `pc_range` (Mach-O, the LLVM form).
    Pcrel8,
}

fn uleb(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*at)?;
        *at += 1;
        if shift < 64 {
            v |= u64::from(byte & 0x7f) << shift;
        }
        shift += 7;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Converts the unix unwind section of form 1 (`zR` CIEs with `DW_EH_PE_absptr`, FDEs with
/// 8-byte `pc_begin` and `pc_range` that are text offsets, a zero terminator) into the
/// `.eh_frame` of an object: each CIE gets the `pcrel` encoding of `form`, each FDE its new
/// CIE pointer, and the terminator is dropped (the linker ends the output section).
/// Returns the bytes and, per FDE, (the offset of its `pc_begin` field, the text offset).
fn eh_frame(src: &[u8], text_len: usize, form: EhForm) -> Result<EhFrame, ObjError> {
    let bad = ObjError::Unwind;
    let mut out = Vec::new();
    let mut fields = Vec::new();
    let mut cies: Vec<(usize, usize)> = Vec::new(); // (old offset, new offset)
    let mut at = 0;
    loop {
        let len = u32_at(src, at).ok_or(bad("a record ends early"))? as usize;
        if len == 0 {
            break;
        }
        if len == 0xffff_ffff {
            return Err(bad("the 64-bit length form"));
        }
        let end = at.checked_add(4 + len).filter(|&e| e <= src.len());
        let end = end.ok_or(bad("a record runs past the section"))?;
        let id = u32_at(src, at + 4).ok_or(bad("a record ends early"))?;
        if id == 0 {
            // CIE: version 1, "zR", code and data alignment, return register (u8), augmentation
            // length 1, the FDE encoding (absptr).
            let mut p = at + 8;
            if src.get(p) != Some(&1) || src.get(p + 1..p + 4) != Some(b"zR\0") {
                return Err(bad("a CIE that is not version 1 with \"zR\""));
            }
            p += 4;
            uleb(src, &mut p).ok_or(bad("a CIE ends early"))?;
            uleb(src, &mut p).ok_or(bad("a CIE ends early"))?;
            p += 1;
            if uleb(src, &mut p) != Some(1) || src.get(p) != Some(&0) {
                return Err(bad("a CIE whose FDE encoding is not absptr"));
            }
            cies.push((at, out.len()));
            let enc_at = out.len() + (p - at);
            out.extend_from_slice(&src[at..end]);
            out[enc_at] = match form {
                EhForm::Sdata4 => 0x1b,
                EhForm::Pcrel8 => 0x10,
            };
        } else {
            let cie_old = (at + 4)
                .checked_sub(id as usize)
                .ok_or(bad("an FDE before its CIE"))?;
            let &(_, cie_new) = cies
                .iter()
                .find(|(o, _)| *o == cie_old)
                .ok_or(bad("an FDE whose CIE pointer names no CIE"))?;
            let begin = u64_at(src, at + 8).ok_or(bad("an FDE ends early"))?;
            let range = u64_at(src, at + 16).ok_or(bad("an FDE ends early"))?;
            if range == 0 || begin.checked_add(range).is_none_or(|e| e > text_len as u64) {
                return Err(bad("an FDE that does not lie inside the text"));
            }
            let rest = &src[at + 24..end];
            let start = out.len();
            match form {
                EhForm::Sdata4 => {
                    out.extend_from_slice(&((len - 8) as u32).to_le_bytes());
                    out.extend_from_slice(&((start + 4 - cie_new) as u32).to_le_bytes());
                    fields.push((out.len(), begin));
                    out.extend_from_slice(&0u32.to_le_bytes());
                    let range = u32::try_from(range).map_err(|_| bad("an FDE over 4 GiB"))?;
                    out.extend_from_slice(&range.to_le_bytes());
                }
                EhForm::Pcrel8 => {
                    out.extend_from_slice(&(len as u32).to_le_bytes());
                    out.extend_from_slice(&((start + 4 - cie_new) as u32).to_le_bytes());
                    fields.push((out.len(), begin));
                    out.extend_from_slice(&0u64.to_le_bytes());
                    out.extend_from_slice(&range.to_le_bytes());
                }
            }
            out.extend_from_slice(rest);
        }
        at = end;
    }
    if fields.is_empty() {
        return Err(bad("no FDE"));
    }
    Ok((out, fields))
}

/// The Windows unwind section of form 1: 12-byte RUNTIME_FUNCTION entries, each offset into
/// the text (begin < end <= text length; the UNWIND_INFO offset inside the text). A COFF
/// section header counts its relocations in 16 bits and each entry needs 3, so at most 21,845
/// entries (65,535 relocations; JIT review M8-P3 F4: no IMAGE_SCN_LNK_NRELOC_OVFL form).
fn pdata_check(src: &[u8], text_len: usize) -> Result<(), ObjError> {
    let bad = ObjError::Unwind;
    if src.is_empty() || !src.len().is_multiple_of(12) {
        return Err(bad("not a list of 12-byte RUNTIME_FUNCTION entries"));
    }
    if src.len() / 4 > usize::from(u16::MAX) {
        return Err(bad("over 65535 .pdata relocations"));
    }
    for e in src.as_chunks::<12>().0 {
        let w = |i: usize| u32::from_le_bytes(e[i..i + 4].try_into().expect("4 bytes")) as usize;
        if w(0) >= w(4) || w(4) > text_len || w(8) + 4 > text_len {
            return Err(bad("a RUNTIME_FUNCTION entry outside the text"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Writers.

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn pad_to(v: &mut Vec<u8>, align: usize) {
    v.resize(v.len().next_multiple_of(align), 0);
}
fn name16(v: &mut Vec<u8>, s: &str) {
    let mut n = [0u8; 16];
    n[..s.len()].copy_from_slice(s.as_bytes());
    v.extend_from_slice(&n);
}

/// ELF64 relocatable object (little-endian).
fn elf(p: &Parts<'_>, eh: &[u8], fields: &[(usize, u64)]) -> Vec<u8> {
    const SHT_PROGBITS: u32 = 1;
    const SHT_SYMTAB: u32 = 2;
    const SHT_STRTAB: u32 = 3;
    const SHT_RELA: u32 = 4;
    const SHT_X86_64_UNWIND: u32 = 0x7000_0001;
    const SHF_WRITE: u64 = 1;
    const SHF_ALLOC: u64 = 2;
    const SHF_EXECINSTR: u64 = 4;
    const SHF_INFO_LINK: u64 = 0x40;
    let x64 = p.arch == target_arch::X86_64;
    let (machine, abs64, prel32) = if x64 { (62, 1, 2) } else { (183, 257, 261) };
    // Symbols: 0 null, 1 the text section, 2 the unit section, 3 the record (global).
    const SYM_TEXT: u64 = 1;
    const SYM_UNIT: u64 = 2;
    let mut strtab = vec![0u8];
    let sym_name = strtab.len() as u32;
    strtab.extend_from_slice(p.symbol.as_bytes());
    strtab.push(0);
    let mut symtab = vec![0u8; 24];
    for shndx in [1u16, 2] {
        put32(&mut symtab, 0);
        symtab.push(3); // STB_LOCAL, STT_SECTION
        symtab.push(0);
        put16(&mut symtab, shndx);
        put64(&mut symtab, 0);
        put64(&mut symtab, 0);
    }
    put32(&mut symtab, sym_name);
    symtab.push(0x11); // STB_GLOBAL, STT_OBJECT
    symtab.push(0);
    put16(&mut symtab, 3);
    put64(&mut symtab, 0);
    put64(&mut symtab, RECORD_SIZE as u64);
    let rela = |v: &mut Vec<u8>, off: usize, sym: u64, ty: u64, addend: i64| {
        put64(v, off as u64);
        put64(v, (sym << 32) | ty);
        put64(v, addend as u64);
    };
    let mut rela_rec = Vec::new();
    rela(&mut rela_rec, RECORD_UNIT, SYM_UNIT, abs64, 0);
    rela(&mut rela_rec, RECORD_TEXT, SYM_TEXT, abs64, 0);
    let mut rela_eh = Vec::new();
    for &(off, begin) in fields {
        rela(&mut rela_eh, off, SYM_TEXT, prel32, begin as i64);
    }
    // (name, type, flags, data, align, link, info, entsize)
    let names = section_names(ObjFormat::Elf);
    let eh_type = if x64 { SHT_X86_64_UNWIND } else { SHT_PROGBITS };
    type Sec<'a> = (&'a str, u32, u64, &'a [u8], u64, u32, u32, u64);
    let secs: [Sec<'_>; 10] = [
        (
            names[0],
            SHT_PROGBITS,
            SHF_ALLOC | SHF_EXECINSTR,
            p.text,
            16,
            0,
            0,
            0,
        ),
        (names[1], SHT_PROGBITS, SHF_ALLOC, p.unit, 16, 0, 0, 0),
        (
            names[2],
            SHT_PROGBITS,
            SHF_ALLOC | SHF_WRITE,
            &p.rec,
            8,
            0,
            0,
            0,
        ),
        (
            ".rela.data.rel.ro.ep9u",
            SHT_RELA,
            SHF_INFO_LINK,
            &rela_rec,
            8,
            8,
            3,
            24,
        ),
        (names[3], eh_type, SHF_ALLOC, eh, 8, 0, 0, 0),
        (
            ".rela.eh_frame",
            SHT_RELA,
            SHF_INFO_LINK,
            &rela_eh,
            8,
            8,
            5,
            24,
        ),
        (".note.GNU-stack", SHT_PROGBITS, 0, &[], 1, 0, 0, 0),
        (".symtab", SHT_SYMTAB, 0, &symtab, 8, 9, 3, 24),
        (".strtab", SHT_STRTAB, 0, &strtab, 1, 0, 0, 0),
        (".shstrtab", SHT_STRTAB, 0, &[], 1, 0, 0, 0),
    ];
    let mut shstrtab = vec![0u8];
    let mut name_at = Vec::new();
    for s in &secs {
        name_at.push(shstrtab.len() as u32);
        shstrtab.extend_from_slice(s.0.as_bytes());
        shstrtab.push(0);
    }
    let mut out = vec![0u8; 64];
    let mut place = Vec::new();
    for (i, s) in secs.iter().enumerate() {
        let data = if i == secs.len() - 1 {
            &shstrtab[..]
        } else {
            s.3
        };
        pad_to(&mut out, s.4 as usize);
        place.push((out.len() as u64, data.len() as u64));
        out.extend_from_slice(data);
    }
    pad_to(&mut out, 8);
    let shoff = out.len() as u64;
    out.extend_from_slice(&[0u8; 64]);
    for (i, s) in secs.iter().enumerate() {
        put32(&mut out, name_at[i]);
        put32(&mut out, s.1);
        put64(&mut out, s.2);
        put64(&mut out, 0);
        put64(&mut out, place[i].0);
        put64(&mut out, place[i].1);
        put32(&mut out, s.5);
        put32(&mut out, s.6);
        put64(&mut out, s.4);
        put64(&mut out, s.7);
    }
    let shnum = secs.len() as u16 + 1;
    let mut hdr = Vec::with_capacity(64);
    hdr.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    hdr.extend_from_slice(&[0u8; 8]);
    put16(&mut hdr, 1); // ET_REL
    put16(&mut hdr, machine);
    put32(&mut hdr, 1);
    put64(&mut hdr, 0);
    put64(&mut hdr, 0);
    put64(&mut hdr, shoff);
    put32(&mut hdr, 0);
    put16(&mut hdr, 64);
    put16(&mut hdr, 0);
    put16(&mut hdr, 0);
    put16(&mut hdr, 64);
    put16(&mut hdr, shnum);
    put16(&mut hdr, shnum - 1);
    out[..64].copy_from_slice(&hdr);
    // The section header indices above are 1-based: index 0 is the null header.
    out
}

/// Mach-O 64-bit object (MH_OBJECT).
fn macho(p: &Parts<'_>, mut eh: Vec<u8>, fields: &[(usize, u64)]) -> Vec<u8> {
    let x64 = p.arch == target_arch::X86_64;
    let (cputype, cpusub, unsigned, subtractor) = if x64 {
        (0x0100_0007u32, 3u32, 0u32, 5u32)
    } else {
        (0x0100_000c, 0, 0, 1)
    };
    let names = section_names(ObjFormat::MachO);
    // (segment, section, data, align log2, flags); addresses laid out in this order.
    let mut addr = Vec::new();
    let mut at = 0u64;
    let datas: [(&[u8], u32); 4] = [(p.text, 4), (p.unit, 4), (&p.rec, 3), (&[], 3)];
    let sizes = [p.text.len(), p.unit.len(), p.rec.len(), eh.len()];
    for (i, (_, align)) in datas.iter().enumerate() {
        at = at.next_multiple_of(1 << align);
        addr.push(at);
        at += sizes[i] as u64;
    }
    // x86-64: the pcrel pc_begin values have no relocation (ld64 reads them as they are).
    // arm64: SUBTRACTOR (the __eh_frame start) + UNSIGNED (the text), the LLVM form.
    for &(off, begin) in fields {
        let v = if x64 {
            (addr[0] + begin).wrapping_sub(addr[3] + off as u64)
        } else {
            begin.wrapping_sub(off as u64)
        };
        eh[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
    // Symbols: 0 text, 1 unit, 2 eh_frame, then one per FDE start in the text (locals), then the
    // record (external). ld64 keeps one FDE per atom of a section; the FDE symbols make an
    // atom start at each FDE, so it keeps every FDE (JIT review M8-P3 F1). The header sets no
    // MH_SUBSECTIONS_VIA_SYMBOLS, so the text is not split and stays in one piece.
    let mut strtab = vec![0u8, 0];
    let mut syms = Vec::new();
    let locals = ["ltmp_ep9u_text", "ltmp_ep9u_unit", "ltmp_ep9u_eh"];
    for (i, n) in locals.iter().enumerate() {
        let sect = [1u8, 2, 4][i];
        syms.push((strtab.len() as u32, 0x0eu8, sect, addr[sect as usize - 1]));
        strtab.extend_from_slice(n.as_bytes());
        strtab.push(0);
    }
    for (k, &(_, begin)) in fields.iter().enumerate() {
        syms.push((strtab.len() as u32, 0x0eu8, 1, addr[0] + begin));
        strtab.extend_from_slice(format!("ltmp_ep9u_f{k}").as_bytes());
        strtab.push(0);
    }
    let nlocal = syms.len() as u32;
    syms.push((strtab.len() as u32, 0x0f, 3, addr[2]));
    strtab.push(b'_');
    strtab.extend_from_slice(p.symbol.as_bytes());
    strtab.push(0);
    pad_to(&mut strtab, 8);
    let reloc = |v: &mut Vec<u8>, off: usize, sym: u32, ty: u32| {
        put32(v, off as u32);
        put32(v, sym | (3 << 25) | (1 << 27) | (ty << 28)); // length 3 (8 bytes), extern
    };
    let mut rel_rec = Vec::new();
    reloc(&mut rel_rec, RECORD_UNIT, 1, unsigned);
    reloc(&mut rel_rec, RECORD_TEXT, 0, unsigned);
    let mut rel_eh = Vec::new();
    if !x64 {
        for &(off, _) in fields {
            reloc(&mut rel_eh, off, 2, subtractor);
            reloc(&mut rel_eh, off, 0, unsigned);
        }
    }
    let nsects = 4u32;
    let seg_cmd = 72 + 80 * nsects;
    let sizeofcmds = seg_cmd + 24 + 24 + 80;
    let header = 32u32;
    let data_off = (header + sizeofcmds) as u64;
    let data_off = data_off.next_multiple_of(16);
    let vmsize = at;
    // File layout: the sections at data_off + addr, then relocations, symbols, strings.
    let file_end = data_off + vmsize;
    let rel_rec_off = file_end.next_multiple_of(8);
    let rel_eh_off = rel_rec_off + rel_rec.len() as u64;
    let symoff = (rel_eh_off + rel_eh.len() as u64).next_multiple_of(8);
    let stroff = symoff + 16 * syms.len() as u64;
    let mut out = Vec::new();
    put32(&mut out, 0xfeed_facf);
    put32(&mut out, cputype);
    put32(&mut out, cpusub);
    put32(&mut out, 1); // MH_OBJECT
    put32(&mut out, 4);
    put32(&mut out, sizeofcmds);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, 0x19); // LC_SEGMENT_64
    put32(&mut out, seg_cmd);
    name16(&mut out, "");
    put64(&mut out, 0);
    put64(&mut out, vmsize);
    put64(&mut out, data_off);
    put64(&mut out, vmsize);
    put32(&mut out, 7);
    put32(&mut out, 7);
    put32(&mut out, nsects);
    put32(&mut out, 0);
    let flags = [
        0x8000_0400u32, // S_REGULAR, pure and some instructions
        0,
        0,
        0x6800_000b, // S_COALESCED, no TOC, strip static syms, live support
    ];
    let relocs = [
        (0u64, 0u32),
        (0, 0),
        (rel_rec_off, 2),
        (rel_eh_off, rel_eh.len() as u32 / 8),
    ];
    for i in 0..4 {
        let (seg, sect) = names[i].split_once(',').expect("segment,section");
        name16(&mut out, sect);
        name16(&mut out, seg);
        put64(&mut out, addr[i]);
        put64(&mut out, sizes[i] as u64);
        put32(&mut out, (data_off + addr[i]) as u32);
        put32(&mut out, datas[i].1);
        put32(
            &mut out,
            if relocs[i].1 == 0 {
                0
            } else {
                relocs[i].0 as u32
            },
        );
        put32(&mut out, relocs[i].1);
        put32(&mut out, flags[i]);
        put32(&mut out, 0);
        put32(&mut out, 0);
        put32(&mut out, 0);
    }
    let (platform, minos) = p.apple.build_version(p.arch);
    put32(&mut out, 0x32); // LC_BUILD_VERSION: platform, minos, sdk 0, no tools
    put32(&mut out, 24);
    put32(&mut out, platform);
    put32(&mut out, minos);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, 2); // LC_SYMTAB
    put32(&mut out, 24);
    put32(&mut out, symoff as u32);
    put32(&mut out, syms.len() as u32);
    put32(&mut out, stroff as u32);
    put32(&mut out, strtab.len() as u32);
    put32(&mut out, 0xb); // LC_DYSYMTAB
    put32(&mut out, 80);
    for v in [0u32, nlocal, nlocal, 1, nlocal + 1, 0] {
        put32(&mut out, v); // ilocalsym, nlocalsym, iextdefsym, nextdefsym, iundefsym, nundefsym
    }
    out.extend_from_slice(&[0u8; 80 - 32]);
    out.resize(data_off as usize, 0);
    for (i, d) in [p.text, p.unit, &p.rec[..], &eh[..]].iter().enumerate() {
        out.resize((data_off + addr[i]) as usize, 0);
        out.extend_from_slice(d);
    }
    out.resize(rel_rec_off as usize, 0);
    out.extend_from_slice(&rel_rec);
    out.extend_from_slice(&rel_eh);
    out.resize(symoff as usize, 0);
    for (strx, ty, sect, value) in syms {
        put32(&mut out, strx);
        out.push(ty);
        out.push(sect);
        put16(&mut out, 0);
        put64(&mut out, value);
    }
    out.extend_from_slice(&strtab);
    out
}

/// COFF object (x86-64).
fn coff(p: &Parts<'_>, pdata: &[u8]) -> Vec<u8> {
    const CODE: u32 = 0x20 | 0x2000_0000 | 0x4000_0000 | 0x0050_0000;
    const RDATA16: u32 = 0x40 | 0x4000_0000 | 0x0050_0000;
    const RDATA8: u32 = 0x40 | 0x4000_0000 | 0x0040_0000;
    const PDATA: u32 = 0x40 | 0x4000_0000 | 0x0030_0000;
    const ADDR64: u16 = 1;
    const ADDR32NB: u16 = 3;
    let names = section_names(ObjFormat::Coff);
    let mut strtab = Vec::new(); // without the 4-byte size
    // A name over 8 bytes goes to the string table: a section header names it "/<offset>", a
    // symbol record 4 zero bytes and the offset (the offsets count the 4-byte size field).
    let mut name = |n: &str| -> ([u8; 8], [u8; 8]) {
        let (mut sec, mut sym) = ([0u8; 8], [0u8; 8]);
        if n.len() <= 8 {
            sec[..n.len()].copy_from_slice(n.as_bytes());
            sym = sec;
        } else {
            let at = 4 + strtab.len();
            let s = format!("/{at}");
            sec[..s.len()].copy_from_slice(s.as_bytes());
            sym[4..].copy_from_slice(&(at as u32).to_le_bytes());
            strtab.extend_from_slice(n.as_bytes());
            strtab.push(0);
        }
        (sec, sym)
    };
    let sec_names: Vec<([u8; 8], [u8; 8])> = names.iter().map(|n| name(n)).collect();
    let rec_name = name(p.symbol).1;
    // Symbols: each section symbol (2 entries: the symbol and its aux record), then the
    // record symbol. Section symbol i is at index 2 * i.
    let mut rel_rec = Vec::new();
    for (off, sym) in [(RECORD_UNIT, 2u32), (RECORD_TEXT, 0)] {
        put32(&mut rel_rec, off as u32);
        put32(&mut rel_rec, sym);
        put16(&mut rel_rec, ADDR64);
    }
    let mut rel_pdata = Vec::new();
    for at in (0..pdata.len()).step_by(4) {
        put32(&mut rel_pdata, at as u32);
        put32(&mut rel_pdata, 0);
        put16(&mut rel_pdata, ADDR32NB);
    }
    let datas: [(&[u8], u32, &[u8]); 4] = [
        (p.text, CODE, &[]),
        (p.unit, RDATA16, &[]),
        (&p.rec, RDATA8, &rel_rec),
        (pdata, PDATA, &rel_pdata),
    ];
    let mut out = vec![0u8; 20 + 40 * 4];
    let mut raw = Vec::new();
    for (data, _, rel) in &datas {
        pad_to(&mut out, 16);
        let d = out.len();
        out.extend_from_slice(data);
        pad_to(&mut out, 4);
        let r = out.len();
        out.extend_from_slice(rel);
        raw.push((d as u32, r as u32));
    }
    pad_to(&mut out, 4);
    let symoff = out.len() as u32;
    for (i, (data, _, rel)) in datas.iter().enumerate() {
        out.extend_from_slice(&sec_names[i].1);
        put32(&mut out, 0);
        put16(&mut out, i as u16 + 1);
        put16(&mut out, 0);
        out.push(3); // IMAGE_SYM_CLASS_STATIC
        out.push(1);
        put32(&mut out, data.len() as u32);
        put16(&mut out, (rel.len() / 10) as u16);
        put16(&mut out, 0);
        put32(&mut out, 0);
        put16(&mut out, 0);
        out.push(0);
        out.extend_from_slice(&[0u8; 3]);
    }
    out.extend_from_slice(&rec_name);
    put32(&mut out, 0);
    put16(&mut out, 3);
    put16(&mut out, 0);
    out.push(2); // IMAGE_SYM_CLASS_EXTERNAL
    out.push(0);
    put32(&mut out, 4 + strtab.len() as u32);
    out.extend_from_slice(&strtab);
    let mut hdr = Vec::new();
    put16(&mut hdr, 0x8664);
    put16(&mut hdr, 4);
    put32(&mut hdr, 0);
    put32(&mut hdr, symoff);
    put32(&mut hdr, 4 * 2 + 1);
    put16(&mut hdr, 0);
    put16(&mut hdr, 0);
    for (i, (data, flags, rel)) in datas.iter().enumerate() {
        hdr.extend_from_slice(&sec_names[i].0);
        put32(&mut hdr, 0);
        put32(&mut hdr, 0);
        put32(&mut hdr, data.len() as u32);
        put32(&mut hdr, if data.is_empty() { 0 } else { raw[i].0 });
        put32(&mut hdr, if rel.is_empty() { 0 } else { raw[i].1 });
        put32(&mut hdr, 0);
        put16(&mut hdr, (rel.len() / 10) as u16);
        put16(&mut hdr, 0);
        put32(&mut hdr, *flags);
    }
    out[..hdr.len()].copy_from_slice(&hdr);
    out
}
