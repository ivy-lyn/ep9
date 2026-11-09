//! Instruction decoding: bounds-checked, for the verifier, the disassembler, tools and tests.
//!
//! The interpreter reads verified bytecode without these checks. It uses the same layout:
//!
//! ```text
//! [prefix]? opcode operand0 operand1 ...
//!   prefix: Wide (0x01) -> scalable operands 2 bytes; ExtraWide (0x02) -> 4 bytes
//!   operand i starts at opcode + OpcodeInfo::operand_offset(i, scale)
//!   instruction offset (jump base, handler and position offsets) = offset of the prefix if any
//! ```

use crate::opcodes::{MAX_OPERANDS, OPCODE_COUNT, OPCODES, Opcode, OpcodeFlags};
use crate::operand::{OperandKind, OperandScale, Register};

/// The operand places of one opcode at one scale ([`PLACES`]): the byte offset of each operand
/// from the first byte of the instruction (its prefix included) and the instruction size
/// (prefix included). 0 for an opcode with no scalable operand at a prefixed scale (no such
/// instruction decodes).
#[derive(Clone, Copy)]
struct Places {
    at: [u8; MAX_OPERANDS],
    size: u8,
}

/// [`Places`] of every opcode at each scale ([`OperandScale::index`]), computed from the opcode
/// table at compile time (M7-P4 TSPASS2). [`decode_at`] and [`Instruction::operand`] read
/// them, so a decode sums no operand sizes and an operand read walks no earlier operand.
static PLACES: [[Places; 3]; OPCODE_COUNT] = {
    let none = Places {
        at: [0; MAX_OPERANDS],
        size: 0,
    };
    let mut t = [[none; 3]; OPCODE_COUNT];
    let scales = [
        OperandScale::Single,
        OperandScale::Wide,
        OperandScale::ExtraWide,
    ];
    let mut op = 0;
    while op < OPCODE_COUNT {
        let info = &OPCODES[op];
        let mut k = 0;
        while k < 3 {
            let scale = scales[k];
            if k == 0 || info.has_scalable_operands() {
                let prefix = if k == 0 { 0 } else { 1 };
                let mut i = 0;
                while i < info.operands.len() {
                    let at = prefix + info.operand_offset(i, scale);
                    assert!(at < 256);
                    t[op][k].at[i] = at as u8;
                    i += 1;
                }
                let size = info.size(scale);
                assert!(size < 256);
                t[op][k].size = size as u8;
            }
            k += 1;
        }
        op += 1;
    }
    t
};

/// The [`Places`] of `op` at `scale`.
#[inline(always)]
fn places(op: Opcode, scale: OperandScale) -> &'static Places {
    &PLACES[op as usize][scale.index()]
}

/// Why bytes do not decode as an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The offset is at or past the end of the code.
    OutOfBounds,
    /// The byte is not a defined opcode.
    UndefinedOpcode(u8),
    /// A prefix is the last byte, or is followed by another prefix.
    BadPrefix,
    /// A prefix precedes an opcode that has no scalable operand.
    PrefixWithoutScalableOperand(Opcode),
    /// The operands run past the end of the code.
    Truncated(Opcode),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfBounds => f.write_str("offset out of bounds"),
            Self::UndefinedOpcode(b) => write!(f, "undefined opcode byte 0x{b:02x}"),
            Self::BadPrefix => f.write_str("prefix at the end or before another prefix"),
            Self::PrefixWithoutScalableOperand(op) => {
                write!(f, "prefix before {op}, which has no scalable operand")
            }
            Self::Truncated(op) => write!(f, "{op} runs past the end of the code"),
        }
    }
}

/// One decoded instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction<'a> {
    /// Offset of the first byte (the prefix, if any).
    pub offset: u32,
    /// The opcode (never a prefix opcode).
    pub opcode: Opcode,
    /// The operand scale.
    pub scale: OperandScale,
    /// All bytes of the instruction, prefix included.
    pub bytes: &'a [u8],
}

impl<'a> Instruction<'a> {
    /// Size in bytes, prefix included.
    pub fn size(&self) -> u32 {
        self.bytes.len() as u32
    }

    /// Offset of the next instruction.
    pub fn next_offset(&self) -> u32 {
        self.offset + self.size()
    }

    /// Number of operands.
    pub fn operand_count(&self) -> usize {
        self.opcode.info().operands.len()
    }

    /// Kind of operand `i`.
    pub fn operand_kind(&self, i: usize) -> OperandKind {
        self.opcode.info().operands[i].kind
    }

    /// Raw value of operand `i`: sign extended to i64 for signed kinds, zero extended else.
    /// The operand's offset comes from a table made at compile time (M7-P4 TSPASS2: no walk
    /// over the earlier operands).
    pub fn operand(&self, i: usize) -> i64 {
        let kind = self.opcode.info().operands[i].kind;
        let at = usize::from(places(self.opcode, self.scale).at[i]);
        read_operand(&self.bytes[at..], kind, self.scale)
    }

    /// Operand `i` as an unsigned value. Panics if the kind is signed.
    pub fn unsigned(&self, i: usize) -> u32 {
        assert!(!self.operand_kind(i).is_signed(), "operand {i} is signed");
        self.operand(i) as u32
    }

    /// Operand `i` as a signed immediate. Panics if the kind is unsigned.
    pub fn signed(&self, i: usize) -> i32 {
        assert!(self.operand_kind(i).is_signed(), "operand {i} is unsigned");
        self.operand(i) as i32
    }

    /// Operand `i` as a register. Panics if it is not a register kind.
    pub fn register(&self, i: usize) -> Register {
        assert!(
            self.operand_kind(i).is_register(),
            "operand {i} is not a register"
        );
        Register::from_operand(self.operand(i) as i32)
    }

    /// The jump target of a jump instruction (`None` for other opcodes). A target outside
    /// `0..=u32::MAX` gives `None`.
    pub fn jump_target(&self) -> Option<u32> {
        if !self.opcode.info().flags.contains(OpcodeFlags::JUMP) {
            return None;
        }
        // The jump operand is always operand 0 (a unit test of the opcode table checks it).
        let off = self.operand(0);
        let target = match self.operand_kind(0) {
            OperandKind::JumpFwd => i64::from(self.offset) + off,
            _ => i64::from(self.offset) - off,
        };
        u32::try_from(target).ok()
    }
}

/// Reads one operand of `kind` at the start of `bytes`. `bytes` must hold enough bytes.
#[inline]
pub fn read_operand(bytes: &[u8], kind: OperandKind, scale: OperandScale) -> i64 {
    let size = kind.size(scale);
    let signed = kind.is_signed();
    match size {
        1 => {
            if signed {
                i64::from(bytes[0] as i8)
            } else {
                i64::from(bytes[0])
            }
        }
        2 => {
            let b = [bytes[0], bytes[1]];
            if signed {
                i64::from(i16::from_le_bytes(b))
            } else {
                i64::from(u16::from_le_bytes(b))
            }
        }
        _ => {
            let b = [bytes[0], bytes[1], bytes[2], bytes[3]];
            if signed {
                i64::from(i32::from_le_bytes(b))
            } else {
                i64::from(u32::from_le_bytes(b))
            }
        }
    }
}

/// Decodes the instruction at `offset`.
pub fn decode_at(code: &[u8], offset: u32) -> Result<Instruction<'_>, DecodeError> {
    let start = offset as usize;
    let first = *code.get(start).ok_or(DecodeError::OutOfBounds)?;
    let op0 = Opcode::from_u8(first).ok_or(DecodeError::UndefinedOpcode(first))?;
    let (opcode, scale, op_at) = match op0.prefix_scale() {
        None => (op0, OperandScale::Single, start),
        Some(scale) => {
            let b = *code.get(start + 1).ok_or(DecodeError::BadPrefix)?;
            let op = Opcode::from_u8(b).ok_or(DecodeError::UndefinedOpcode(b))?;
            if op.prefix_scale().is_some() {
                return Err(DecodeError::BadPrefix);
            }
            if !op.info().has_scalable_operands() {
                return Err(DecodeError::PrefixWithoutScalableOperand(op));
            }
            (op, scale, start + 1)
        }
    };
    // The size from the table made at compile time (M7-P4 TSPASS2), prefix included; it is
    // `op_at + 1 + operands_size(scale) - start`.
    let end = start + usize::from(places(opcode, scale).size);
    debug_assert_eq!(end, op_at + 1 + opcode.info().operands_size(scale));
    if end > code.len() {
        return Err(DecodeError::Truncated(opcode));
    }
    Ok(Instruction {
        offset,
        opcode,
        scale,
        bytes: &code[start..end],
    })
}

/// Iterates over the instructions of one function's bytecode, in order.
#[derive(Clone, Debug)]
pub struct Instructions<'a> {
    code: &'a [u8],
    at: u32,
    failed: bool,
}

impl<'a> Instructions<'a> {
    /// An iterator from offset 0.
    pub fn new(code: &'a [u8]) -> Self {
        Self {
            code,
            at: 0,
            failed: false,
        }
    }
}

impl<'a> Iterator for Instructions<'a> {
    type Item = Result<Instruction<'a>, (u32, DecodeError)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.at as usize >= self.code.len() {
            return None;
        }
        match decode_at(self.code, self.at) {
            Ok(insn) => {
                self.at = insn.next_offset();
                Some(Ok(insn))
            }
            Err(e) => {
                self.failed = true;
                Some(Err((self.at, e)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_single_wide_extra_wide() {
        // LdaSmi -1
        let code = [Opcode::LdaSmi as u8, 0xff];
        let i = decode_at(&code, 0).unwrap();
        assert_eq!(i.opcode, Opcode::LdaSmi);
        assert_eq!(i.scale, OperandScale::Single);
        assert_eq!(i.signed(0), -1);
        assert_eq!(i.size(), 2);
        // Wide LdaSmi 1000
        let v = 1000i16.to_le_bytes();
        let code = [Opcode::Wide as u8, Opcode::LdaSmi as u8, v[0], v[1]];
        let i = decode_at(&code, 0).unwrap();
        assert_eq!(
            (i.scale, i.signed(0), i.size()),
            (OperandScale::Wide, 1000, 4)
        );
        // ExtraWide GetNamedProperty r70000, k1, s300000
        let mut code = vec![Opcode::ExtraWide as u8, Opcode::GetNamedProperty as u8];
        code.extend_from_slice(&70000i32.to_le_bytes());
        code.extend_from_slice(&1u32.to_le_bytes());
        code.extend_from_slice(&300000u32.to_le_bytes());
        let i = decode_at(&code, 0).unwrap();
        assert_eq!(i.register(0), Register::local(70000));
        assert_eq!(i.unsigned(1), 1);
        assert_eq!(i.unsigned(2), 300000);
        assert_eq!(i.size(), 14);
    }

    /// The table of operand places agrees with the opcode table: for every opcode and every
    /// scale it decodes at, each operand's offset is the prefix plus `operand_offset`, and the
    /// size is `OpcodeInfo::size`.
    #[test]
    fn places_match_the_opcode_table() {
        let scales = [
            OperandScale::Single,
            OperandScale::Wide,
            OperandScale::ExtraWide,
        ];
        let mut checked = 0;
        for info in OPCODES.iter() {
            for scale in scales {
                if scale != OperandScale::Single && !info.has_scalable_operands() {
                    assert_eq!(places(info.opcode, scale).size, 0, "{}", info.name);
                    continue;
                }
                let p = places(info.opcode, scale);
                let prefix = usize::from(scale != OperandScale::Single);
                for i in 0..info.operands.len() {
                    assert_eq!(
                        usize::from(p.at[i]),
                        prefix + info.operand_offset(i, scale),
                        "{} operand {i} at {scale:?}",
                        info.name
                    );
                }
                assert_eq!(usize::from(p.size), info.size(scale), "{}", info.name);
                checked += 1;
            }
        }
        assert!(checked > OPCODE_COUNT);
    }

    /// Every operand of every opcode at every scale it decodes at reads back the value that
    /// was encoded there (a distinct value per operand), through `decode_at` and `operand`.
    /// The two prefixes are not instructions of their own.
    #[test]
    fn every_operand_reads_back_at_every_scale() {
        for info in OPCODES.iter().filter(|i| i.opcode.prefix_scale().is_none()) {
            for (scale, prefix) in [
                (OperandScale::Single, None),
                (OperandScale::Wide, Some(Opcode::Wide)),
                (OperandScale::ExtraWide, Some(Opcode::ExtraWide)),
            ] {
                if prefix.is_some() && !info.has_scalable_operands() {
                    continue;
                }
                let mut code: Vec<u8> = prefix.map(|p| p as u8).into_iter().collect();
                code.push(info.opcode as u8);
                let mut want = Vec::new();
                for (i, o) in info.operands.iter().enumerate() {
                    let size = o.kind.size(scale);
                    // A value that differs per operand and per byte.
                    let raw: u32 = 0x8180_7f00 + (i as u32) * 0x0101_0101;
                    let b = raw.to_le_bytes();
                    code.extend_from_slice(&b[..size]);
                    let v = match (size, o.kind.is_signed()) {
                        (1, true) => i64::from(b[0] as i8),
                        (1, false) => i64::from(b[0]),
                        (2, true) => i64::from(i16::from_le_bytes([b[0], b[1]])),
                        (2, false) => i64::from(u16::from_le_bytes([b[0], b[1]])),
                        (_, true) => i64::from(raw as i32),
                        (_, false) => i64::from(raw),
                    };
                    want.push(v);
                }
                let insn = decode_at(&code, 0).unwrap();
                assert_eq!(
                    insn.size() as usize,
                    code.len(),
                    "{} at {scale:?}",
                    info.name
                );
                let got: Vec<i64> = (0..want.len()).map(|i| insn.operand(i)).collect();
                assert_eq!(got, want, "{} at {scale:?}", info.name);
            }
        }
    }

    #[test]
    fn decode_errors() {
        assert_eq!(decode_at(&[], 0), Err(DecodeError::OutOfBounds));
        assert_eq!(
            decode_at(&[0xff], 0),
            Err(DecodeError::UndefinedOpcode(0xff))
        );
        assert_eq!(
            decode_at(&[Opcode::Wide as u8], 0),
            Err(DecodeError::BadPrefix)
        );
        assert_eq!(
            decode_at(&[Opcode::Wide as u8, Opcode::ExtraWide as u8, 0], 0),
            Err(DecodeError::BadPrefix)
        );
        assert_eq!(
            decode_at(&[Opcode::Wide as u8, Opcode::Return as u8], 0),
            Err(DecodeError::PrefixWithoutScalableOperand(Opcode::Return))
        );
        assert_eq!(
            decode_at(&[Opcode::Wide as u8, Opcode::LdaSmi as u8, 0], 0),
            Err(DecodeError::Truncated(Opcode::LdaSmi))
        );
    }

    #[test]
    fn jump_targets() {
        // 0: Jump +3; 2: LdaZero; 3: JumpLoop -1 depth 0 slot 0
        let code = [
            Opcode::Jump as u8,
            3,
            Opcode::LdaZero as u8,
            Opcode::JumpLoop as u8,
            1,
            0,
            0,
        ];
        let insns: Vec<_> = Instructions::new(&code).map(|r| r.unwrap()).collect();
        assert_eq!(insns.len(), 3);
        assert_eq!(insns[0].jump_target(), Some(3));
        assert_eq!(insns[1].jump_target(), None);
        assert_eq!(insns[2].jump_target(), Some(2));
    }
}
