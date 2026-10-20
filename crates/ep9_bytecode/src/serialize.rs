//! Canonical serialization of a [`CompiledUnit`] and the validating reader.
//!
//! All integers are little endian. Every field has exactly one encoding (fixed-width integers,
//! minimal LEB128 in position tables, canonical strings, NaN and BigInts, zero reserved bytes,
//! sections back to back). So for any input that loads, writing the unit again gives the same
//! bytes. ADR H02 holds the byte tables.
//!
//! ```text
//! header (160 bytes)
//!   0  magic "\x89EP9BC\r\n"     8   abi_version u32       12  header_size u32 (160)
//!   16 total_size u32           20  unit flags u32        24  hash algorithm u8 (1 = SHA-256)
//!   25 reserved [7] = 0         32  content hash [32]     64  source hash [32]
//!   96 engine build id [16]    112  source_len u32       116  reserved u32 = 0
//!  120 section table: 5 x (offset u32, length u32): strings, scopes, functions, source, hints
//! body: the 5 sections, back to back, in that order, from offset 160 to total_size
//! ```
//!
//! Bytes 0..20 (magic, ABI version, header size, total size) are a frozen prefix: they keep
//! their place and meaning in every ABI version, so any later reader can name the version.
//!
//! Content hash = SHA-256 of the whole unit with the content-hash field (32..64) and the
//! engine-build-id field (96..112) set to zero. It excludes the build id (01-front-end §5).

use crate::feedback::FeedbackSlotKind;
use crate::operand::{ConstIndex, FunctionId, Register, ScopeId, StringId};
use crate::positions::{decode_positions, encode_positions};
use crate::sha256::{Digest, Sha256, sha256};
use crate::string::UnitString;
use crate::unit::{
    ABI_VERSION, BigIntConst, CodeKind, CompiledUnit, Constant, Function, FunctionFlags,
    FunctionKind, Handler, HandlerKind, NumberBits, Oddball, Scope, ScopeFlags, ScopeKind,
    ScopeSlot, TsFlags, UnitHeader, VariableMode, VerifiedUnit,
};
use crate::verify::{VerifyError, verify};

/// Magic bytes: a high byte (catches 7-bit transfers), "EP9BC", CR LF (catches line-ending
/// conversion), as in the PNG signature.
pub const MAGIC: [u8; 8] = [0x89, b'E', b'P', b'9', b'B', b'C', 0x0D, 0x0A];
/// Header size in bytes.
pub const HEADER_SIZE: usize = 160;
/// Hash algorithm id of SHA-256.
pub const HASH_SHA256: u8 = 1;
/// Number of sections.
pub const SECTION_COUNT: usize = 5;
/// Size of the frozen prefix (magic, ABI version, header size, total size).
pub const FROZEN_PREFIX_SIZE: usize = 20;

/// Byte offsets of the header fields.
pub mod offsets {
    /// Magic, 8 bytes.
    pub const MAGIC: usize = 0;
    /// Bytecode ABI version, u32.
    pub const ABI_VERSION: usize = 8;
    /// Header size, u32.
    pub const HEADER_SIZE: usize = 12;
    /// Total unit size, u32.
    pub const TOTAL_SIZE: usize = 16;
    /// Unit flags, u32.
    pub const FLAGS: usize = 20;
    /// Hash algorithm, u8.
    pub const HASH_ALGORITHM: usize = 24;
    /// Reserved, 7 bytes, zero.
    pub const RESERVED0: usize = 25;
    /// Content hash, 32 bytes.
    pub const CONTENT_HASH: usize = 32;
    /// Source hash, 32 bytes.
    pub const SOURCE_HASH: usize = 64;
    /// Engine build id, 16 bytes.
    pub const BUILD_ID: usize = 96;
    /// Source length, u32.
    pub const SOURCE_LEN: usize = 112;
    /// Reserved u32, zero.
    pub const RESERVED1: usize = 116;
    /// Section table: 5 x (offset u32, length u32).
    pub const SECTIONS: usize = 120;
}

/// Unit flag bits (header offset 20).
pub mod flags {
    /// Bits 0-1: [`crate::CodeKind`].
    pub const CODE_KIND_MASK: u32 = 0x3;
    /// Bit 2: top-level strict.
    pub const STRICT: u32 = 1 << 2;
    /// Bit 8: TypeScript source (H3).
    pub const TYPESCRIPT: u32 = 1 << 8;
    /// Bit 9: `useDefineForClassFields` (H3). Only with [`TYPESCRIPT`].
    pub const TS_USE_DEFINE_FOR_CLASS_FIELDS: u32 = 1 << 9;
    /// Bit 10: `experimentalDecorators` (H3). Only with [`TYPESCRIPT`].
    pub const TS_EXPERIMENTAL_DECORATORS: u32 = 1 << 10;
    /// Bit 11: `emitDecoratorMetadata` (H3). Only with [`TYPESCRIPT`].
    pub const TS_EMIT_DECORATOR_METADATA: u32 = 1 << 11;
    /// Bits 12-15: reserved for later TS flags; zero in ABI v1.
    pub const TS_RESERVED: u32 = 0xf << 12;
    /// Bit 16: the source section holds the source text.
    pub const HAS_SOURCE: u32 = 1 << 16;
    /// Bit 17: the hints section is present.
    pub const HAS_HINTS: u32 = 1 << 17;
    /// Every defined bit.
    pub const DEFINED: u32 = CODE_KIND_MASK
        | STRICT
        | TYPESCRIPT
        | TS_USE_DEFINE_FOR_CLASS_FIELDS
        | TS_EXPERIMENTAL_DECORATORS
        | TS_EMIT_DECORATOR_METADATA
        | HAS_SOURCE
        | HAS_HINTS;
}

/// The sections, in body order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    /// The header.
    Header,
    /// String table.
    Strings,
    /// Scope descriptors.
    Scopes,
    /// Function table.
    Functions,
    /// Source text.
    Source,
    /// Hint section.
    Hints,
}

/// What the caller does with a unit that failed to load (H35 S4 outcomes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadOutcome {
    /// No byte is trusted: clean load error (a code cache drops the entry and recompiles).
    LoadError,
    /// Another bytecode ABI: compile the source instead.
    Source,
}

/// Why a unit did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The input is shorter than the header, or the magic is wrong.
    BadMagic,
    /// Another bytecode ABI version.
    AbiMismatch {
        /// The version in the input.
        found: u32,
    },
    /// A header field is invalid.
    BadHeader(&'static str),
    /// The content hash does not match.
    HashMismatch,
    /// A section is malformed.
    Malformed {
        /// The section.
        section: Section,
        /// What is wrong.
        what: &'static str,
    },
    /// The embedded source does not match the header's source hash or length.
    SourceMismatch,
    /// The unit is well formed but fails verification.
    Verify(VerifyError),
}

impl LoadError {
    /// The H35 outcome for this error.
    pub fn outcome(&self) -> LoadOutcome {
        match self {
            Self::AbiMismatch { .. } => LoadOutcome::Source,
            _ => LoadOutcome::LoadError,
        }
    }
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a compiled unit (bad magic)"),
            Self::AbiMismatch { found } => {
                write!(f, "bytecode ABI version {found}, expected {ABI_VERSION}")
            }
            Self::BadHeader(what) => write!(f, "bad header: {what}"),
            Self::HashMismatch => f.write_str("content hash mismatch"),
            Self::Malformed { section, what } => write!(f, "malformed {section:?}: {what}"),
            Self::SourceMismatch => f.write_str("source text does not match its hash or length"),
            Self::Verify(e) => write!(f, "verification failed: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

// ---- Writer ------------------------------------------------------------------------------

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_len(out: &mut Vec<u8>, n: usize) {
    put_u32(out, u32::try_from(n).expect("count exceeds u32"));
}

fn put_opt(out: &mut Vec<u8>, v: Option<u32>) {
    put_u32(
        out,
        v.map_or(0, |v| v.checked_add(1).expect("id exceeds u32::MAX - 1")),
    );
}

fn write_strings(unit: &CompiledUnit) -> Vec<u8> {
    let mut out = Vec::new();
    put_len(&mut out, unit.strings.len());
    for s in &unit.strings {
        let len = u32::try_from(s.len()).expect("string too long");
        assert!(len < (1 << 31), "string too long");
        match s {
            UnitString::Latin1(b) => {
                put_u32(&mut out, len << 1);
                out.extend_from_slice(b);
            }
            UnitString::Utf16(u) => {
                put_u32(&mut out, (len << 1) | 1);
                for &c in u.iter() {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
    }
    out
}

fn write_scopes(unit: &CompiledUnit) -> Vec<u8> {
    let mut out = Vec::new();
    put_len(&mut out, unit.scopes.len());
    for s in &unit.scopes {
        out.push(s.kind as u8);
        out.push(s.flags.0);
        put_opt(&mut out, s.outer.map(|o| o.0));
        put_len(&mut out, s.slots.len());
        for slot in &s.slots {
            put_u32(&mut out, slot.name.0);
            out.push(slot.mode as u8);
        }
    }
    out
}

fn write_constant(out: &mut Vec<u8>, c: &Constant) {
    out.push(c.tag());
    match c {
        Constant::Number(n) => out.extend_from_slice(&n.bits().to_le_bytes()),
        Constant::String(s) => put_u32(out, s.0),
        Constant::BigInt(b) => {
            out.push(u8::from(b.negative));
            put_len(out, b.limbs.len());
            for &l in b.limbs.iter() {
                out.extend_from_slice(&l.to_le_bytes());
            }
        }
        Constant::Function(f) => put_u32(out, f.0),
        Constant::Scope(s) => put_u32(out, s.0),
        Constant::JumpTable(t) => {
            put_len(out, t.len());
            for &o in t.iter() {
                put_u32(out, o);
            }
        }
        Constant::ArrayBoilerplate(e) => {
            put_len(out, e.len());
            for v in e.iter() {
                put_opt(out, v.map(|k| k.0));
            }
        }
        Constant::ObjectBoilerplate(p) => {
            put_len(out, p.len());
            for (k, v) in p.iter() {
                put_u32(out, k.0);
                put_opt(out, v.map(|k| k.0));
            }
        }
        Constant::TemplateObject(p) => {
            put_len(out, p.len());
            for (raw, cooked) in p.iter() {
                put_u32(out, raw.0);
                put_opt(out, cooked.map(|s| s.0));
            }
        }
        Constant::Oddball(o) => out.push(*o as u8),
    }
}

fn write_function(out: &mut Vec<u8>, f: &Function) {
    out.push(f.kind as u8);
    out.push(f.flags.0);
    put_opt(out, f.name.map(|n| n.0));
    put_opt(out, f.parent.map(|p| p.0));
    put_opt(out, f.outer_scope.map(|s| s.0));
    put_u32(out, f.source_start);
    put_u32(out, f.source_end);
    put_u32(out, f.parameter_count);
    put_u32(out, f.length);
    put_u32(out, f.register_count);
    put_len(out, f.bytecode.len());
    out.extend_from_slice(&f.bytecode);
    put_len(out, f.constants.len());
    for c in &f.constants {
        write_constant(out, c);
    }
    put_len(out, f.feedback_slots.len());
    out.extend(f.feedback_slots.iter().map(|&k| k as u8));
    put_len(out, f.handlers.len());
    for h in &f.handlers {
        put_u32(out, h.start);
        put_u32(out, h.end);
        put_u32(out, h.target);
        out.extend_from_slice(&h.context.to_operand().to_le_bytes());
        out.push(h.kind as u8);
    }
    let pos = encode_positions_lossy(&f.positions);
    put_len(out, pos.len());
    out.extend_from_slice(&pos);
}

/// Encodes positions even when the offsets do not increase (an unverified unit). Such bytes
/// do not load back; the reader reports them.
fn encode_positions_lossy(entries: &[crate::unit::PositionEntry]) -> Vec<u8> {
    if entries
        .windows(2)
        .all(|w| w[0].bytecode_offset < w[1].bytecode_offset)
    {
        encode_positions(entries)
    } else {
        let mut sorted = entries.to_vec();
        sorted.sort_by_key(|e| e.bytecode_offset);
        let mut out = encode_positions(&sorted);
        out.push(0x80); // a truncated LEB128: never decodes
        out
    }
}

fn unit_flags(h: &UnitHeader, has_source: bool, has_hints: bool) -> u32 {
    let mut v = h.code_kind as u32;
    if h.strict {
        v |= flags::STRICT;
    }
    if let Some(ts) = h.typescript {
        v |= flags::TYPESCRIPT;
        if ts.use_define_for_class_fields {
            v |= flags::TS_USE_DEFINE_FOR_CLASS_FIELDS;
        }
        if ts.experimental_decorators {
            v |= flags::TS_EXPERIMENTAL_DECORATORS;
        }
        if ts.emit_decorator_metadata {
            v |= flags::TS_EMIT_DECORATOR_METADATA;
        }
    }
    if has_source {
        v |= flags::HAS_SOURCE;
    }
    if has_hints {
        v |= flags::HAS_HINTS;
    }
    v
}

/// The content hash of `bytes`, a serialized unit with a full header.
pub fn compute_content_hash(bytes: &[u8]) -> Digest {
    let mut h = Sha256::new();
    h.update(&bytes[..offsets::CONTENT_HASH]);
    h.update(&[0u8; 32]);
    h.update(&bytes[offsets::CONTENT_HASH + 32..offsets::BUILD_ID]);
    h.update(&[0u8; 16]);
    h.update(&bytes[offsets::BUILD_ID + 16..]);
    h.finish()
}

impl CompiledUnit {
    /// Serializes the unit in canonical form, content hash included.
    ///
    /// Panics if a count or size does not fit in u32. A unit that fails [`crate::verify`] may
    /// serialize to bytes that do not load.
    pub fn to_bytes(&self) -> Vec<u8> {
        let sections: [Vec<u8>; SECTION_COUNT] = [
            write_strings(self),
            write_scopes(self),
            {
                let mut out = Vec::new();
                put_len(&mut out, self.functions.len());
                for f in &self.functions {
                    write_function(&mut out, f);
                }
                out
            },
            self.source
                .as_deref()
                .map_or(Vec::new(), |s| s.as_bytes().to_vec()),
            self.hints.clone().unwrap_or_default(),
        ];
        let body_len: usize = sections.iter().map(Vec::len).sum();
        let total = u32::try_from(HEADER_SIZE + body_len).expect("unit larger than 4 GiB");
        let mut out = Vec::with_capacity(total as usize);
        out.extend_from_slice(&MAGIC);
        put_u32(&mut out, ABI_VERSION);
        put_u32(&mut out, HEADER_SIZE as u32);
        put_u32(&mut out, total);
        put_u32(
            &mut out,
            unit_flags(&self.header, self.source.is_some(), self.hints.is_some()),
        );
        out.push(HASH_SHA256);
        out.extend_from_slice(&[0; 7]);
        out.extend_from_slice(&[0; 32]); // content hash, filled below
        out.extend_from_slice(&self.header.source_hash);
        out.extend_from_slice(&self.header.engine_build_id);
        put_u32(&mut out, self.header.source_len);
        put_u32(&mut out, 0);
        let mut at = HEADER_SIZE as u32;
        for s in &sections {
            put_u32(&mut out, at);
            put_len(&mut out, s.len());
            at += s.len() as u32;
        }
        debug_assert_eq!(out.len(), HEADER_SIZE);
        for s in &sections {
            out.extend_from_slice(s);
        }
        let hash = compute_content_hash(&out);
        out[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].copy_from_slice(&hash);
        out
    }

    /// The content hash: SHA-256 of the canonical bytes with the hash and build-id fields
    /// zeroed. Equal for equal units on every target.
    pub fn content_hash(&self) -> Digest {
        let bytes = self.to_bytes();
        let mut h = [0u8; 32];
        h.copy_from_slice(&bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32]);
        h
    }

    /// Reads, checks and verifies a serialized unit.
    pub fn from_bytes(bytes: &[u8]) -> Result<VerifiedUnit, LoadError> {
        let unit = decode_unit(bytes)?;
        verify(unit).map_err(LoadError::Verify)
    }
}

// ---- Reader ------------------------------------------------------------------------------

/// The header fields a loader needs before it trusts the unit (H35 S4 steps 1 and 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeaderPeek {
    /// Bytecode ABI version.
    pub abi_version: u32,
    /// Total unit size in bytes.
    pub total_size: u32,
    /// Engine build id.
    pub engine_build_id: [u8; 16],
    /// Source hash.
    pub source_hash: Digest,
    /// Stored content hash.
    pub content_hash: Digest,
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads the frozen prefix and the hash fields. Does not check the hash.
pub fn peek_header(bytes: &[u8]) -> Result<HeaderPeek, LoadError> {
    if bytes.len() < FROZEN_PREFIX_SIZE || bytes[..8] != MAGIC {
        return Err(LoadError::BadMagic);
    }
    let abi_version = le_u32(bytes, offsets::ABI_VERSION);
    let total_size = le_u32(bytes, offsets::TOTAL_SIZE);
    if abi_version != ABI_VERSION {
        return Err(LoadError::AbiMismatch { found: abi_version });
    }
    if le_u32(bytes, offsets::HEADER_SIZE) as usize != HEADER_SIZE || bytes.len() < HEADER_SIZE {
        return Err(LoadError::BadHeader("header size"));
    }
    let mut peek = HeaderPeek {
        abi_version,
        total_size,
        engine_build_id: [0; 16],
        source_hash: [0; 32],
        content_hash: [0; 32],
    };
    peek.engine_build_id
        .copy_from_slice(&bytes[offsets::BUILD_ID..offsets::BUILD_ID + 16]);
    peek.source_hash
        .copy_from_slice(&bytes[offsets::SOURCE_HASH..offsets::SOURCE_HASH + 32]);
    peek.content_hash
        .copy_from_slice(&bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32]);
    Ok(peek)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    section: Section,
}

impl<'a> Reader<'a> {
    fn err<T>(&self, what: &'static str) -> Result<T, LoadError> {
        Err(LoadError::Malformed {
            section: self.section,
            what,
        })
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], LoadError> {
        match self.at.checked_add(n) {
            Some(end) if end <= self.b.len() => {
                let s = &self.b[self.at..end];
                self.at = end;
                Ok(s)
            }
            _ => self.err("truncated"),
        }
    }

    fn u8(&mut self) -> Result<u8, LoadError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, LoadError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn u64(&mut self) -> Result<u64, LoadError> {
        let s = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Ok(u64::from_le_bytes(a))
    }

    fn opt(&mut self) -> Result<Option<u32>, LoadError> {
        let v = self.u32()?;
        Ok(v.checked_sub(1))
    }

    /// A count whose items take at least `min_item` bytes each: checked against the bytes
    /// left, so a corrupt count cannot make a huge allocation.
    fn count(&mut self, min_item: usize) -> Result<usize, LoadError> {
        let n = self.u32()? as usize;
        let left = self.b.len() - self.at;
        if n.checked_mul(min_item.max(1))
            .is_none_or(|need| need > left)
        {
            return self.err("count larger than the section");
        }
        Ok(n)
    }

    fn done(&self) -> Result<(), LoadError> {
        if self.at == self.b.len() {
            Ok(())
        } else {
            self.err("trailing bytes")
        }
    }
}

fn read_strings(r: &mut Reader<'_>) -> Result<Vec<UnitString>, LoadError> {
    let n = r.count(4)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let head = r.u32()?;
        let len = (head >> 1) as usize;
        if head & 1 == 0 {
            out.push(UnitString::Latin1(r.take(len)?.into()));
        } else {
            let bytes = r.take(len.checked_mul(2).ok_or(LoadError::Malformed {
                section: Section::Strings,
                what: "length",
            })?)?;
            let units: Box<[u16]> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .collect();
            let s = UnitString::Utf16(units);
            if !s.is_canonical() {
                return r.err("UTF-16 string that fits Latin-1");
            }
            out.push(s);
        }
    }
    r.done()?;
    Ok(out)
}

fn read_scopes(r: &mut Reader<'_>) -> Result<Vec<Scope>, LoadError> {
    let n = r.count(10)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let Some(kind) = ScopeKind::from_u8(r.u8()?) else {
            return r.err("scope kind");
        };
        let flags = r.u8()?;
        if flags & !ScopeFlags::ALL != 0 {
            return r.err("scope flags");
        }
        let outer = r.opt()?.map(ScopeId);
        let slots_n = r.count(5)?;
        let mut slots = Vec::with_capacity(slots_n);
        for _ in 0..slots_n {
            let name = StringId(r.u32()?);
            let Some(mode) = VariableMode::from_u8(r.u8()?) else {
                return r.err("variable mode");
            };
            slots.push(ScopeSlot { name, mode });
        }
        out.push(Scope {
            kind,
            flags: ScopeFlags(flags),
            outer,
            slots,
        });
    }
    r.done()?;
    Ok(out)
}

fn read_constant(r: &mut Reader<'_>) -> Result<Constant, LoadError> {
    Ok(match r.u8()? {
        0 => match NumberBits::from_bits(r.u64()?) {
            Some(n) => Constant::Number(n),
            None => return r.err("non-canonical NaN"),
        },
        1 => Constant::String(StringId(r.u32()?)),
        2 => {
            let negative = match r.u8()? {
                0 => false,
                1 => true,
                _ => return r.err("BigInt sign"),
            };
            let n = r.count(8)?;
            let mut limbs = Vec::with_capacity(n);
            for _ in 0..n {
                limbs.push(r.u64()?);
            }
            let b = BigIntConst {
                negative,
                limbs: limbs.into(),
            };
            if !b.is_canonical() {
                return r.err("non-canonical BigInt");
            }
            Constant::BigInt(b)
        }
        3 => Constant::Function(FunctionId(r.u32()?)),
        4 => Constant::Scope(ScopeId(r.u32()?)),
        5 => {
            let n = r.count(4)?;
            let mut t = Vec::with_capacity(n);
            for _ in 0..n {
                t.push(r.u32()?);
            }
            Constant::JumpTable(t.into())
        }
        6 => {
            let n = r.count(4)?;
            let mut e = Vec::with_capacity(n);
            for _ in 0..n {
                e.push(r.opt()?.map(ConstIndex));
            }
            Constant::ArrayBoilerplate(e.into())
        }
        7 => {
            let n = r.count(8)?;
            let mut p = Vec::with_capacity(n);
            for _ in 0..n {
                let k = ConstIndex(r.u32()?);
                p.push((k, r.opt()?.map(ConstIndex)));
            }
            Constant::ObjectBoilerplate(p.into())
        }
        8 => {
            let n = r.count(8)?;
            let mut p = Vec::with_capacity(n);
            for _ in 0..n {
                let raw = StringId(r.u32()?);
                p.push((raw, r.opt()?.map(StringId)));
            }
            Constant::TemplateObject(p.into())
        }
        9 => match Oddball::from_u8(r.u8()?) {
            Some(o) => Constant::Oddball(o),
            None => return r.err("oddball value"),
        },
        _ => return r.err("constant tag"),
    })
}

/// Bytes of a function record without its variable parts.
const FUNCTION_FIXED_SIZE: usize = 2 + 8 * 4 + 5 * 4;

fn read_function(r: &mut Reader<'_>) -> Result<Function, LoadError> {
    let Some(kind) = FunctionKind::from_u8(r.u8()?) else {
        return r.err("function kind");
    };
    let flags = r.u8()?;
    if flags & !FunctionFlags::ALL != 0 {
        return r.err("function flags");
    }
    let name = r.opt()?.map(StringId);
    let parent = r.opt()?.map(FunctionId);
    let outer_scope = r.opt()?.map(ScopeId);
    let source_start = r.u32()?;
    let source_end = r.u32()?;
    let parameter_count = r.u32()?;
    let length = r.u32()?;
    let register_count = r.u32()?;
    let n = r.count(1)?;
    let bytecode = r.take(n)?.to_vec();
    let n = r.count(2)?;
    let mut constants = Vec::with_capacity(n);
    for _ in 0..n {
        constants.push(read_constant(r)?);
    }
    let n = r.count(1)?;
    let mut feedback_slots = Vec::with_capacity(n);
    for _ in 0..n {
        match FeedbackSlotKind::from_u8(r.u8()?) {
            Some(k) => feedback_slots.push(k),
            None => return r.err("feedback slot kind"),
        }
    }
    let n = r.count(17)?;
    let mut handlers = Vec::with_capacity(n);
    for _ in 0..n {
        let start = r.u32()?;
        let end = r.u32()?;
        let target = r.u32()?;
        let context = Register::from_operand(r.u32()? as i32);
        let Some(hk) = HandlerKind::from_u8(r.u8()?) else {
            return r.err("handler kind");
        };
        handlers.push(Handler {
            start,
            end,
            target,
            context,
            kind: hk,
        });
    }
    let n = r.count(1)?;
    let positions = match decode_positions(r.take(n)?) {
        Ok(p) => p,
        Err(_) => return r.err("position table"),
    };
    Ok(Function {
        kind,
        flags: FunctionFlags(flags),
        name,
        parent,
        outer_scope,
        source_start,
        source_end,
        parameter_count,
        length,
        register_count,
        bytecode,
        constants,
        feedback_slots,
        handlers,
        positions,
    })
}

fn read_functions(r: &mut Reader<'_>) -> Result<Vec<Function>, LoadError> {
    let n = r.count(FUNCTION_FIXED_SIZE)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(read_function(r)?);
    }
    r.done()?;
    Ok(out)
}

/// Reads a serialized unit and checks its header, hash and structure. Does not run the
/// verifier: use [`CompiledUnit::from_bytes`] for that.
pub fn decode_unit(bytes: &[u8]) -> Result<CompiledUnit, LoadError> {
    // Step 1: the frozen prefix and the header layout.
    let peek = peek_header(bytes)?;
    if peek.total_size as usize != bytes.len() {
        return Err(LoadError::BadHeader("total size"));
    }
    if bytes[offsets::HASH_ALGORITHM] != HASH_SHA256 {
        return Err(LoadError::BadHeader("hash algorithm"));
    }
    if bytes[offsets::RESERVED0..offsets::CONTENT_HASH]
        .iter()
        .any(|&b| b != 0)
        || le_u32(bytes, offsets::RESERVED1) != 0
    {
        return Err(LoadError::BadHeader("reserved bytes"));
    }
    let mut ranges = [(0usize, 0usize); SECTION_COUNT];
    let mut at = HEADER_SIZE;
    for (i, range) in ranges.iter_mut().enumerate() {
        let off = le_u32(bytes, offsets::SECTIONS + 8 * i) as usize;
        let len = le_u32(bytes, offsets::SECTIONS + 8 * i + 4) as usize;
        if off != at || off.checked_add(len).is_none_or(|end| end > bytes.len()) {
            return Err(LoadError::BadHeader("section table"));
        }
        *range = (off, off + len);
        at = off + len;
    }
    if at != bytes.len() {
        return Err(LoadError::BadHeader("section table"));
    }
    // Step 2: the content hash.
    if compute_content_hash(bytes) != peek.content_hash {
        return Err(LoadError::HashMismatch);
    }
    // Header fields.
    let fl = le_u32(bytes, offsets::FLAGS);
    if fl & !flags::DEFINED != 0 {
        return Err(LoadError::BadHeader("unit flags"));
    }
    let code_kind = CodeKind::from_u8((fl & flags::CODE_KIND_MASK) as u8)
        .ok_or(LoadError::BadHeader("code kind"))?;
    let strict = fl & flags::STRICT != 0;
    let typescript = if fl & flags::TYPESCRIPT != 0 {
        Some(TsFlags {
            use_define_for_class_fields: fl & flags::TS_USE_DEFINE_FOR_CLASS_FIELDS != 0,
            experimental_decorators: fl & flags::TS_EXPERIMENTAL_DECORATORS != 0,
            emit_decorator_metadata: fl & flags::TS_EMIT_DECORATOR_METADATA != 0,
        })
    } else {
        if fl
            & (flags::TS_USE_DEFINE_FOR_CLASS_FIELDS
                | flags::TS_EXPERIMENTAL_DECORATORS
                | flags::TS_EMIT_DECORATOR_METADATA)
            != 0
        {
            return Err(LoadError::BadHeader("TS flags on a JavaScript unit"));
        }
        None
    };
    let header = UnitHeader {
        code_kind,
        strict,
        typescript,
        engine_build_id: peek.engine_build_id,
        source_hash: peek.source_hash,
        source_len: le_u32(bytes, offsets::SOURCE_LEN),
    };
    let section = |i: usize, s: Section| Reader {
        b: &bytes[ranges[i].0..ranges[i].1],
        at: 0,
        section: s,
    };
    let strings = read_strings(&mut section(0, Section::Strings))?;
    let scopes = read_scopes(&mut section(1, Section::Scopes))?;
    let functions = read_functions(&mut section(2, Section::Functions))?;
    let source_bytes = &bytes[ranges[3].0..ranges[3].1];
    let source = if fl & flags::HAS_SOURCE != 0 {
        let text = core::str::from_utf8(source_bytes).map_err(|_| LoadError::Malformed {
            section: Section::Source,
            what: "invalid UTF-8",
        })?;
        if text.len() != header.source_len as usize || sha256(text.as_bytes()) != header.source_hash
        {
            return Err(LoadError::SourceMismatch);
        }
        Some(text.to_owned())
    } else {
        if !source_bytes.is_empty() {
            return Err(LoadError::Malformed {
                section: Section::Source,
                what: "bytes without the HAS_SOURCE flag",
            });
        }
        None
    };
    let hint_bytes = &bytes[ranges[4].0..ranges[4].1];
    let hints = if fl & flags::HAS_HINTS != 0 {
        Some(hint_bytes.to_vec())
    } else {
        if !hint_bytes.is_empty() {
            return Err(LoadError::Malformed {
                section: Section::Hints,
                what: "bytes without the HAS_HINTS flag",
            });
        }
        None
    };
    Ok(CompiledUnit {
        header,
        strings,
        scopes,
        functions,
        source,
        hints,
    })
}
