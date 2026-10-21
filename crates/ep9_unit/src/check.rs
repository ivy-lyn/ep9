//! The load check, H35 S4 steps 1-4, as a pure function of the unit bytes and the host.
//!
//! ```text
//!  check(bytes, host)
//!    1 header (prefix, reserved, flags), total_size, section table, required sections,
//!      signed flag == signature section ------------------------------ fail -> LOAD ERROR
//!    2 SHA-256 (content_hash and signature bytes zeroed) == content_hash - fail -> LOAD ERROR
//!    3 engine_build_id, bytecode_abi ----------------------------------- differ -> SOURCE
//!    4 value_encoding .. builtin_abi, target, heap_page_log2, cpu_required
//!      (a required bit the host lacks); a text section while a native
//!      contract is at version 0 -------------------------------------- differ -> BYTECODE
//!    -> LOAD
//! ```
//!
//! Steps 5-8 (map, link, register, install) are the A2 loader.

use core::fmt;
use core::ops::Range;

use ep9_bytecode::sha256::{Digest, Sha256};

use crate::codes::{Contract, SectionKind, cpu};
use crate::header::{
    BadUnit, HEADER_SIZE, SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET, SectionEntry, UnitHeader,
    header_flags, offsets,
};
use crate::host::HostInfo;

/// What the caller does with a unit (H35 S4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Every check passed: use every section.
    Load,
    /// Run the unit's bytecode; drop native code (and the hints on a fact-schema mismatch).
    Bytecode,
    /// Compile the unit's source section with its source flags; drop everything else.
    Source,
    /// Clean load error: no section is used.
    LoadError,
}

/// The outcome and its reason. S5: every outcome other than [`Verdict::Load`] is counted by
/// reason and prints one warning line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every check passed.
    Load,
    /// Step 1 or 2 failed.
    LoadError(BadUnit),
    /// Step 3: another engine build.
    BuildId,
    /// Step 3 (bytecode ABI) or step 4 (every other contract): another version.
    Version {
        /// The contract.
        contract: Contract,
        /// The unit's version.
        unit: u32,
        /// The host's version.
        host: u32,
    },
    /// Step 4: another target, or a host whose target is unknown.
    Target {
        /// The unit's (arch, abi) codes.
        unit: (u8, u8),
        /// The host's (arch, abi) codes.
        host: (u8, u8),
    },
    /// Step 4: another heap page size.
    PageSize {
        /// The unit's log2 page size.
        unit: u8,
        /// The host's log2 page size.
        host: u8,
    },
    /// Step 4: required CPU features the host lacks (unknown bits included).
    Cpu {
        /// The missing bits.
        missing: u64,
    },
    /// Step 4: the unit has a text section, but this contract has no native form yet.
    NoNativeForm(Contract),
}

impl Verdict {
    /// The S4 outcome.
    pub fn outcome(&self) -> Outcome {
        match self {
            Self::Load => Outcome::Load,
            Self::LoadError(_) => Outcome::LoadError,
            Self::BuildId => Outcome::Source,
            Self::Version {
                contract: Contract::BytecodeAbi,
                ..
            } => Outcome::Source,
            Self::Version { .. }
            | Self::Target { .. }
            | Self::PageSize { .. }
            | Self::Cpu { .. }
            | Self::NoNativeForm(_) => Outcome::Bytecode,
        }
    }

    /// Whether the hints section is dropped. Only on a fact-schema mismatch: SOURCE and LOAD
    /// ERROR use no section of the unit at all.
    pub fn drops_hints(&self) -> bool {
        matches!(
            self,
            Self::Version {
                contract: Contract::FactSchema,
                ..
            }
        )
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load => f.write_str("loaded"),
            Self::LoadError(e) => write!(f, "load error: {e}"),
            Self::BuildId => f.write_str("source: built for another engine build"),
            Self::Version {
                contract,
                unit,
                host,
            } => write!(
                f,
                "{}: {} version {unit}, this engine has {host}",
                if *contract == Contract::BytecodeAbi {
                    "source"
                } else {
                    "bytecode"
                },
                contract.name()
            ),
            Self::Target { unit, host } => write!(
                f,
                "bytecode: target {}/{}, this engine is {}/{}",
                unit.0, unit.1, host.0, host.1
            ),
            Self::PageSize { unit, host } => {
                write!(f, "bytecode: heap page 2^{unit}, this engine has 2^{host}")
            }
            Self::Cpu { missing } => {
                write!(f, "bytecode: CPU features {missing:#x} missing")
            }
            Self::NoNativeForm(c) => write!(
                f,
                "bytecode: native code, but {} has no native form yet (version 0)",
                c.name()
            ),
        }
    }
}

/// The S4 outcome of `bytes` on `host`.
pub fn check(bytes: &[u8], host: &HostInfo) -> Outcome {
    verdict(bytes, host).outcome()
}

/// [`check`] with the reason.
pub fn verdict(bytes: &[u8], host: &HostInfo) -> Verdict {
    match verify(bytes) {
        Ok((h, sections)) => compare(&h, &sections, host),
        Err(e) => Verdict::LoadError(e),
    }
}

/// Where each section is, indexed by kind id.
struct Sections([Option<Range<usize>>; 10]);

impl Sections {
    fn get(&self, k: SectionKind) -> Option<Range<usize>> {
        self.0[k as usize].clone()
    }
}

/// Steps 1 and 2.
fn verify(bytes: &[u8]) -> Result<(UnitHeader, Sections), BadUnit> {
    let h = UnitHeader::decode(bytes)?;
    if h.total_size != bytes.len() as u64 {
        return Err(BadUnit::TotalSize);
    }
    let sections = read_table(bytes, &h)?;
    for k in SectionKind::ALL {
        if k.required() && sections.get(k).is_none() {
            return Err(BadUnit::MissingSection(k));
        }
    }
    let signed = h.header_flags & header_flags::SIGNED != 0;
    let signature = sections.get(SectionKind::Signature);
    if signed != signature.is_some() {
        return Err(BadUnit::Signature);
    }
    if content_hash(bytes, signature) != h.content_hash {
        return Err(BadUnit::Hash);
    }
    Ok((h, sections))
}

/// Reads the section table and checks the canonical layout: kinds strictly ascending, each
/// section at the aligned end of the one before (the table end for the first), zero padding,
/// and the last section ending at the end of the input.
fn read_table(bytes: &[u8], h: &UnitHeader) -> Result<Sections, BadUnit> {
    let bad = |what| Err(BadUnit::SectionTable(what));
    let count = h.section_count as usize;
    let table_end = SECTION_TABLE_OFFSET + count * SECTION_ENTRY_SIZE;
    if bytes.len() < table_end {
        return bad("truncated table");
    }
    let mut sections = Sections(Default::default());
    let mut end = table_end;
    let mut prev_kind = 0u32;
    for i in 0..count {
        let at = SECTION_TABLE_OFFSET + i * SECTION_ENTRY_SIZE;
        let e = SectionEntry::decode(&bytes[at..at + SECTION_ENTRY_SIZE])?;
        if e.kind as u32 <= prev_kind {
            return bad("section kinds not ascending");
        }
        prev_kind = e.kind as u32;
        let align = 1usize << e.kind.align_log2();
        let start = end.next_multiple_of(align);
        if e.offset != start as u64 {
            return bad("section offset");
        }
        if start > bytes.len() || e.size > (bytes.len() - start) as u64 {
            return bad("section out of bounds");
        }
        if bytes[end..start].iter().any(|&b| b != 0) {
            return bad("padding not zero");
        }
        end = start + e.size as usize;
        sections.0[e.kind as usize] = Some(start..end);
    }
    if end != bytes.len() {
        return bad("bytes after the last section");
    }
    Ok(sections)
}

/// SHA-256 of `bytes` with the content-hash field and the signature section set to zero.
fn content_hash(bytes: &[u8], signature: Option<Range<usize>>) -> Digest {
    const ZEROS: [u8; 64] = [0; 64];
    let mut s = Sha256::new();
    let hash_end = offsets::CONTENT_HASH + 32;
    s.update(&bytes[..offsets::CONTENT_HASH]);
    s.update(&ZEROS[..32]);
    let rest = match signature {
        Some(r) => {
            debug_assert!(r.start >= HEADER_SIZE);
            s.update(&bytes[hash_end..r.start]);
            let mut n = r.len();
            while n > 0 {
                let k = n.min(ZEROS.len());
                s.update(&ZEROS[..k]);
                n -= k;
            }
            r.end
        }
        None => hash_end,
    };
    s.update(&bytes[rest..]);
    s.finish()
}

/// Steps 3 and 4.
fn compare(h: &UnitHeader, sections: &Sections, host: &HostInfo) -> Verdict {
    if h.engine_build_id != host.engine_build_id {
        return Verdict::BuildId;
    }
    // Contract::ALL starts with the bytecode ABI (step 3); the rest is step 4.
    for c in Contract::ALL {
        let (unit, host) = (h.versions.get(c), host.versions.get(c));
        if unit != host {
            return Verdict::Version {
                contract: c,
                unit,
                host,
            };
        }
    }
    let unit_target = (h.target_arch, h.target_abi);
    let host_target = (host.target_arch, host.target_abi);
    if host.target_arch == 0 || host.target_abi == 0 || unit_target != host_target {
        return Verdict::Target {
            unit: unit_target,
            host: host_target,
        };
    }
    if h.heap_page_log2 != host.heap_page_log2 {
        return Verdict::PageSize {
            unit: h.heap_page_log2,
            host: host.heap_page_log2,
        };
    }
    let missing = h.cpu_required & !(host.cpu_features & cpu::DEFINED);
    if missing != 0 {
        return Verdict::Cpu { missing };
    }
    if sections.get(SectionKind::Text).is_some()
        && let Some(c) = Contract::ALL
            .into_iter()
            .find(|&c| c.is_native() && h.versions.get(c) == 0)
    {
        return Verdict::NoNativeForm(c);
    }
    Verdict::Load
}
