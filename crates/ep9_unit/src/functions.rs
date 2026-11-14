//! The function table (H35 S2: the start of the `rodata` section; doc 13 C-17): one entry per
//! function that the unit lists, each with its native variants. An entry may have no variant:
//! the function then runs its bytecode. `select()` (doc 13 3.2) picks among the variants that
//! the isolate accepts; the loader skips one whose lowering is not accepted, whose CPU bits the
//! host lacks or whose fuse already popped, and counts the skip by reason (doc 13 5.1).
//!
//! ```text
//!  table    := magic "EPFT", u32 version (1), u32 entries, u32 variants, entry*, variant*
//!  entry    := 32 B   0 function u32 (the CompiledUnit function index; strictly ascending)
//!                     4 first_variant u32 (the variants of the entries before it)
//!                     8 variant_count u16   10 flags u16 = 0   12 reserved u32 = 0
//!                    16 digest [16] (the function digest that REBASE keys on, C-21)
//!  variant  := 48 B   0 kind u8 (3 aot-baseline, 4 aot-opt: the dispatch slot kind)
//!                     1 lowering u8 (1 hybrid, 2 jitless; 0 jit and 3+ are skipped, doc 13 1.4)
//!                     2 tag u8 (the writer's variant tag: region or sibling, CPU level)
//!                     3 flags u8 = 0
//!                     4 entry u32, 8 text_len u32 (the code in the text section)
//!                    12 meta u32, 16 meta_len u32 (its H29 format-1 table in rodata;
//!                       meta = NO_META: no deopt point)
//!                    20 fuse_refs u32 (rodata offset of fuse_count u32 fuse ids; 0 with no fuse id)
//!                    24 fuse_count u16
//!                    26 reserved u16 = 0   28 osr u32 = 0 (OSR entries: later)
//!                    32 cpu_required u64 (H35-Q5 bits)   40 resume u32 = 0 (resume map: later)
//!                    44 reserved u32 = 0
//! ```
//!
//! Every field is an offset, a size, an index or a code, never an address (H12, zero text
//! relocations). The form is canonical: one table has one encoding.

use crate::header::{le_u32, le_u64};

/// The magic of a function table.
pub const FT_MAGIC: [u8; 4] = *b"EPFT";
/// The function-table form version (inside container version 4; the same since version 2).
pub const FT_VERSION: u32 = 1;
/// The table header: magic, version, entry count, variant count.
pub const FT_HEADER_SIZE: usize = 16;
/// One entry.
pub const FT_ENTRY_SIZE: usize = 32;
/// One variant.
pub const FT_VARIANT_SIZE: usize = 48;
/// `meta` of a variant with no deopt metadata.
pub const NO_META: u32 = u32::MAX;

/// The kind of a variant: the dispatch slot kind its loader installs (H14; doc 13 3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VariantKind {
    /// `DispatchKind::AotBaseline` (3).
    AotBaseline,
    /// `DispatchKind::AotOpt` (4).
    AotOpt,
}

impl VariantKind {
    /// The slot kind byte.
    pub const fn code(self) -> u8 {
        match self {
            Self::AotBaseline => 3,
            Self::AotOpt => 4,
        }
    }

    /// The kind of slot kind byte `c`.
    pub const fn from_code(c: u8) -> Option<VariantKind> {
        match c {
            3 => Some(Self::AotBaseline),
            4 => Some(Self::AotOpt),
            _ => None,
        }
    }
}

/// The lowering class byte of a code object (doc 13 1.4; the H32 list, append-only).
pub mod lowering {
    /// In-memory JIT code: never stored in a unit, so a unit variant with it is skipped.
    pub const JIT: u8 = 0;
    /// Hybrid units: DEOPT, then re-tier (the H32 hybrid column).
    pub const HYBRID: u8 = 1;
    /// Jitless units: SLOWPATH, budget, PIN (the H32 jitless column).
    pub const JITLESS: u8 = 2;
}

/// One native variant of a function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Variant {
    /// The slot kind.
    pub kind: VariantKind,
    /// The lowering class ([`lowering`]); a value the loader does not accept is a skip, not
    /// an error.
    pub lowering: u8,
    /// The writer's variant tag.
    pub tag: u8,
    /// Offset of the code in the text section.
    pub entry: u32,
    /// Length of the code.
    pub text_len: u32,
    /// Offset of the H29 format-1 deopt table in rodata, or [`NO_META`].
    pub meta: u32,
    /// Length of that table (0 with [`NO_META`]).
    pub meta_len: u32,
    /// Offset in rodata of the fuse-id list (u32 each).
    pub fuse_refs: u32,
    /// Number of fuse ids.
    pub fuse_count: u16,
    /// The CPU bits the code needs (H35-Q5).
    pub cpu_required: u64,
}

/// One function of the table.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// The CompiledUnit function index.
    pub function: u32,
    /// The function digest (C-21: its own bytecode and text, positions relative to its start).
    pub digest: [u8; 16],
    /// Its native variants; none: the function runs its bytecode.
    pub variants: Vec<Variant>,
}

/// Why a function table is refused (the unit then gives BYTECODE: its native code cannot be
/// found; the loader's outcome, not the header check's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadTable {
    /// Shorter than its counts say, or longer.
    Size,
    /// Wrong magic or version.
    Magic,
    /// Entries not in strictly ascending function order.
    Order,
    /// An entry's variant range does not follow the one before it.
    VariantRange,
    /// An unknown variant kind.
    Kind,
    /// A nonzero reserved, flags, osr or resume field.
    Reserved,
    /// A code range outside the text section.
    Text,
    /// A metadata or fuse-list range outside rodata.
    Rodata,
}

/// The byte size of a table with `entries` entries and `variants` variants.
pub const fn table_size(entries: usize, variants: usize) -> usize {
    FT_HEADER_SIZE + entries * FT_ENTRY_SIZE + variants * FT_VARIANT_SIZE
}

fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// The bytes of a table of `entries` (sorted by function by the caller).
pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let nv: usize = entries.iter().map(|e| e.variants.len()).sum();
    let mut b = Vec::with_capacity(table_size(entries.len(), nv));
    b.extend_from_slice(&FT_MAGIC);
    put_u32(&mut b, FT_VERSION);
    put_u32(&mut b, entries.len() as u32);
    put_u32(&mut b, nv as u32);
    let mut first = 0u32;
    for e in entries {
        put_u32(&mut b, e.function);
        put_u32(&mut b, first);
        b.extend_from_slice(&(e.variants.len() as u16).to_le_bytes());
        b.extend_from_slice(&[0; 6]);
        b.extend_from_slice(&e.digest);
        first += e.variants.len() as u32;
    }
    for v in entries.iter().flat_map(|e| &e.variants) {
        b.extend_from_slice(&[v.kind.code(), v.lowering, v.tag, 0]);
        for x in [v.entry, v.text_len, v.meta, v.meta_len, v.fuse_refs] {
            put_u32(&mut b, x);
        }
        b.extend_from_slice(&v.fuse_count.to_le_bytes());
        b.extend_from_slice(&[0; 6]);
        b.extend_from_slice(&v.cpu_required.to_le_bytes());
        b.extend_from_slice(&[0; 8]);
    }
    b
}

/// Reads a table at the start of `rodata` (it may go on with other rodata) and checks each
/// range against the text section (`text_len` bytes) and rodata. Returns the entries and the
/// table's size.
pub fn decode(rodata: &[u8], text_len: usize) -> Result<(Vec<Entry>, usize), BadTable> {
    if rodata.len() < FT_HEADER_SIZE {
        return Err(BadTable::Size);
    }
    if rodata[..4] != FT_MAGIC || le_u32(rodata, 4) != FT_VERSION {
        return Err(BadTable::Magic);
    }
    let (ne, nv) = (le_u32(rodata, 8) as usize, le_u32(rodata, 12) as usize);
    let size = ne
        .checked_mul(FT_ENTRY_SIZE)
        .and_then(|a| {
            nv.checked_mul(FT_VARIANT_SIZE)
                .and_then(|b| a.checked_add(b))
        })
        .and_then(|x| x.checked_add(FT_HEADER_SIZE))
        .ok_or(BadTable::Size)?;
    if rodata.len() < size {
        return Err(BadTable::Size);
    }
    let in_rodata = |at: u32, len: u64| u64::from(at) + len <= rodata.len() as u64;
    let mut entries: Vec<Entry> = Vec::with_capacity(ne);
    let mut first = 0usize;
    for i in 0..ne {
        let e = &rodata[FT_HEADER_SIZE + i * FT_ENTRY_SIZE..][..FT_ENTRY_SIZE];
        let function = le_u32(e, 0);
        if entries.last().is_some_and(|p| p.function >= function) {
            return Err(BadTable::Order);
        }
        let count = usize::from(u16::from_le_bytes([e[8], e[9]]));
        if le_u32(e, 4) as usize != first || first + count > nv {
            return Err(BadTable::VariantRange);
        }
        if e[10..16].iter().any(|&x| x != 0) {
            return Err(BadTable::Reserved);
        }
        let mut variants = Vec::with_capacity(count);
        for k in first..first + count {
            let at = FT_HEADER_SIZE + ne * FT_ENTRY_SIZE + k * FT_VARIANT_SIZE;
            let v = &rodata[at..at + FT_VARIANT_SIZE];
            let kind = VariantKind::from_code(v[0]).ok_or(BadTable::Kind)?;
            let reserved =
                v[3] != 0 || v[26..32].iter().any(|&x| x != 0) || v[40..].iter().any(|&x| x != 0);
            if reserved {
                return Err(BadTable::Reserved);
            }
            let var = Variant {
                kind,
                lowering: v[1],
                tag: v[2],
                entry: le_u32(v, 4),
                text_len: le_u32(v, 8),
                meta: le_u32(v, 12),
                meta_len: le_u32(v, 16),
                fuse_refs: le_u32(v, 20),
                fuse_count: u16::from_le_bytes([v[24], v[25]]),
                cpu_required: le_u64(v, 32),
            };
            if var.text_len == 0 || u64::from(var.entry) + u64::from(var.text_len) > text_len as u64
            {
                return Err(BadTable::Text);
            }
            let meta_ok = if var.meta == NO_META {
                var.meta_len == 0
            } else {
                in_rodata(var.meta, var.meta_len.into())
            };
            if !meta_ok || !in_rodata(var.fuse_refs, u64::from(var.fuse_count) * 4) {
                return Err(BadTable::Rodata);
            }
            // No fuse id: `fuse_refs` is 0, so one table has one encoding (M7-P1 JIT
            // reviewer F6: any offset in rodata decoded the same).
            if var.fuse_count == 0 && var.fuse_refs != 0 {
                return Err(BadTable::Reserved);
            }
            variants.push(var);
        }
        let mut digest = [0; 16];
        digest.copy_from_slice(&e[16..32]);
        entries.push(Entry {
            function,
            digest,
            variants,
        });
        first += count;
    }
    if first != nv {
        return Err(BadTable::VariantRange);
    }
    Ok((entries, size))
}

/// The variants of `entry` that an isolate whose preset accepts the lowering classes `accept`
/// (in rank order) and whose CPU has `cpu` may run, in table order (doc 13 1.4 loader rule).
pub fn accepted<'a>(
    entry: &'a Entry,
    accept: &'a [u8],
    cpu: u64,
) -> impl Iterator<Item = &'a Variant> {
    entry.variants.iter().filter(move |v| {
        v.lowering != lowering::JIT
            && accept.contains(&v.lowering)
            && v.cpu_required & !(cpu & crate::cpu::DEFINED) == 0
    })
}
