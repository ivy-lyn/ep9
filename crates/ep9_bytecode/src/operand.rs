//! Operand kinds, operand scales and the typed values that operands carry.
//!
//! Encoding model (copied from V8 Ignition `bytecode-operands.h`, see ADR H02):
//! - A *scalable* operand is 1 byte at scale [`OperandScale::Single`], 2 bytes after a `Wide`
//!   prefix and 4 bytes after an `ExtraWide` prefix. All operands of one instruction share the
//!   scale.
//! - A *fixed* operand ([`OperandKind::Flag8`], [`OperandKind::Flag16`], [`OperandKind::Runtime`],
//!   [`OperandKind::TypeOfLiteral`]) has the same size at every scale.
//! - All multi-byte operands are little endian. Signed kinds are sign extended.

use crate::feedback::FeedbackSlotKind;

/// The width of the scalable operands of one instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum OperandScale {
    /// No prefix: 1 byte per scalable operand.
    Single = 1,
    /// `Wide` prefix: 2 bytes per scalable operand.
    Wide = 2,
    /// `ExtraWide` prefix: 4 bytes per scalable operand.
    ExtraWide = 4,
}

impl OperandScale {
    /// All scales, smallest first.
    pub const ALL: [OperandScale; 3] = [Self::Single, Self::Wide, Self::ExtraWide];

    /// Bytes per scalable operand.
    pub const fn bytes(self) -> usize {
        self as usize
    }

    /// Index 0, 1 or 2, for per-scale tables.
    pub const fn index(self) -> usize {
        match self {
            Self::Single => 0,
            Self::Wide => 1,
            Self::ExtraWide => 2,
        }
    }

    /// The smallest scale that holds the unsigned value `v`.
    pub const fn for_unsigned(v: u32) -> Self {
        if v <= u8::MAX as u32 {
            Self::Single
        } else if v <= u16::MAX as u32 {
            Self::Wide
        } else {
            Self::ExtraWide
        }
    }

    /// The smallest scale that holds the signed value `v`.
    pub const fn for_signed(v: i32) -> Self {
        if v >= i8::MIN as i32 && v <= i8::MAX as i32 {
            Self::Single
        } else if v >= i16::MIN as i32 && v <= i16::MAX as i32 {
            Self::Wide
        } else {
            Self::ExtraWide
        }
    }
}

/// What one encoded operand field means. The verifier and the disassembler dispatch on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OperandKind {
    /// A register the instruction reads. Signed, scalable. See [`Register`].
    Reg = 0,
    /// A register the instruction writes.
    RegOut = 1,
    /// A register the instruction reads and writes.
    RegInOut = 2,
    /// The first register of a list the instruction reads. The next operand is its
    /// [`OperandKind::RegCount`]. Lists hold locals only.
    RegList = 3,
    /// The first register of a list the instruction writes. The next operand is its count.
    RegOutList = 4,
    /// Two consecutive locals the instruction reads.
    RegPair = 5,
    /// Three consecutive locals the instruction writes.
    RegOutTriple = 6,
    /// The length of the register list in the previous operand. Unsigned, scalable.
    RegCount = 7,
    /// Signed immediate. Scalable.
    Imm = 8,
    /// Unsigned immediate. Scalable.
    UImm = 9,
    /// Index into the function's constant pool. Unsigned, scalable. The operand parameter
    /// says which constant kinds are allowed.
    Const = 10,
    /// Index into the function's feedback slot list. Unsigned, scalable. The operand parameter
    /// says which slot kind it must have.
    Slot = 11,
    /// Variable slot in a context. Unsigned, scalable.
    CtxSlot = 12,
    /// Number of context hops outward from a context. Unsigned, scalable.
    CtxDepth = 13,
    /// Forward jump: target = instruction start + offset, offset > 0. Unsigned, scalable.
    JumpFwd = 14,
    /// Backward jump (only `JumpLoop`): target = instruction start - offset. Unsigned, scalable.
    JumpBack = 15,
    /// 8-bit flags. Fixed size. The operand parameter is the mask of defined bits.
    Flag8 = 16,
    /// 16-bit flags. Fixed size. The operand parameter is the mask of defined bits.
    Flag16 = 17,
    /// A [`crate::RuntimeId`]. Fixed 2 bytes.
    Runtime = 18,
    /// A [`crate::TypeOfLiteral`]. Fixed 1 byte.
    TypeOfLiteral = 19,
}

impl OperandKind {
    /// True if the size depends on the operand scale.
    pub const fn is_scalable(self) -> bool {
        !matches!(
            self,
            Self::Flag8 | Self::Flag16 | Self::Runtime | Self::TypeOfLiteral
        )
    }

    /// True if the encoded value is signed (sign extended on decode).
    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::Reg
                | Self::RegOut
                | Self::RegInOut
                | Self::RegList
                | Self::RegOutList
                | Self::RegPair
                | Self::RegOutTriple
                | Self::Imm
        )
    }

    /// True for the register kinds (not [`OperandKind::RegCount`]).
    pub const fn is_register(self) -> bool {
        matches!(
            self,
            Self::Reg
                | Self::RegOut
                | Self::RegInOut
                | Self::RegList
                | Self::RegOutList
                | Self::RegPair
                | Self::RegOutTriple
        )
    }

    /// Size in bytes at `scale`.
    pub const fn size(self, scale: OperandScale) -> usize {
        match self {
            Self::Flag8 | Self::TypeOfLiteral => 1,
            Self::Flag16 | Self::Runtime => 2,
            _ => scale.bytes(),
        }
    }

    /// Lower-camel name used in golden files.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Reg => "Reg",
            Self::RegOut => "RegOut",
            Self::RegInOut => "RegInOut",
            Self::RegList => "RegList",
            Self::RegOutList => "RegOutList",
            Self::RegPair => "RegPair",
            Self::RegOutTriple => "RegOutTriple",
            Self::RegCount => "RegCount",
            Self::Imm => "Imm",
            Self::UImm => "UImm",
            Self::Const => "Const",
            Self::Slot => "Slot",
            Self::CtxSlot => "CtxSlot",
            Self::CtxDepth => "CtxDepth",
            Self::JumpFwd => "JumpFwd",
            Self::JumpBack => "JumpBack",
            Self::Flag8 => "Flag8",
            Self::Flag16 => "Flag16",
            Self::Runtime => "Runtime",
            Self::TypeOfLiteral => "TypeOfLiteral",
        }
    }
}

/// Which constant kinds a [`OperandKind::Const`] operand accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ConstOperand {
    /// A JS value: Number, String or BigInt (`LdaConstant`).
    Value = 0,
    /// A String (property names, global names, RegExp patterns).
    String = 1,
    /// A nested function (`CreateClosure`).
    Function = 2,
    /// A scope descriptor (context creation).
    Scope = 3,
    /// A jump table (`SwitchOnSmi`, `SwitchOnGeneratorState`).
    JumpTable = 4,
    /// An array literal boilerplate.
    ArrayBoilerplate = 5,
    /// An object literal boilerplate.
    ObjectBoilerplate = 6,
    /// A tagged-template description.
    TemplateObject = 7,
}

impl ConstOperand {
    /// Name used in golden files.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Value => "Value",
            Self::String => "String",
            Self::Function => "Function",
            Self::Scope => "Scope",
            Self::JumpTable => "JumpTable",
            Self::ArrayBoilerplate => "ArrayBoilerplate",
            Self::ObjectBoilerplate => "ObjectBoilerplate",
            Self::TemplateObject => "TemplateObject",
        }
    }
}

/// The extra rule of one operand: allowed constant kinds, required slot kind, or flag mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperandParam {
    /// No extra rule.
    None,
    /// For [`OperandKind::Const`].
    Const(ConstOperand),
    /// For [`OperandKind::Slot`]: the slot must have this kind.
    Slot(FeedbackSlotKind),
    /// For [`OperandKind::Flag8`] and [`OperandKind::Flag16`]: the defined bits.
    FlagMask(u16),
}

/// Static facts about one encoded operand of an opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OperandInfo {
    /// The operand name in the opcode list (`object`, `slot`, ...).
    pub name: &'static str,
    /// The kind.
    pub kind: OperandKind,
    /// The extra rule.
    pub param: OperandParam,
}

/// A register name in the bytecode ABI.
///
/// | Encoded value `v` | Register | Printed |
/// |---|---|---|
/// | `v >= 0` | local register `r<v>` | `r0`, `r1`, ... |
/// | `v == -1` | the receiver (`this` slot) | `<this>` |
/// | `v <= -2` | parameter `a<k>` with `k = -2 - v` | `a0`, `a1`, ... |
///
/// The encoding names registers; it does not fix where the frame keeps them. The VM maps a
/// name to a frame slot (H14). A local is valid if `v < register_count`; a parameter if
/// `k < parameter_count`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Register(i32);

impl Register {
    /// The receiver register (`this` slot of the frame).
    pub const RECEIVER: Register = Register(-1);

    /// Local register `r<index>`.
    pub const fn local(index: u32) -> Register {
        assert!(index <= i32::MAX as u32, "register index out of range");
        Register(index as i32)
    }

    /// Parameter register `a<index>` (0-based, without the receiver).
    pub const fn parameter(index: u32) -> Register {
        assert!(
            index <= (i32::MAX - 2) as u32,
            "parameter index out of range"
        );
        Register(-2 - index as i32)
    }

    /// The register with the encoded operand value `v`.
    pub const fn from_operand(v: i32) -> Register {
        Register(v)
    }

    /// The encoded operand value.
    pub const fn to_operand(self) -> i32 {
        self.0
    }

    /// `Some(index)` for a local register.
    pub const fn local_index(self) -> Option<u32> {
        if self.0 >= 0 {
            Some(self.0 as u32)
        } else {
            None
        }
    }

    /// `Some(index)` for a parameter register (not the receiver).
    pub const fn parameter_index(self) -> Option<u32> {
        if self.0 <= -2 {
            Some((-2 - self.0) as u32)
        } else {
            None
        }
    }

    /// True for the receiver register.
    pub const fn is_receiver(self) -> bool {
        self.0 == -1
    }
}

impl core::fmt::Display for Register {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match (self.local_index(), self.parameter_index()) {
            (Some(i), _) => write!(f, "r{i}"),
            (_, Some(k)) => write!(f, "a{k}"),
            _ => f.write_str("<this>"),
        }
    }
}

/// A run of consecutive local registers: `r<first> .. r<first + len - 1>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegisterList {
    first: u32,
    len: u32,
}

impl RegisterList {
    /// The list `r<first>..` of `len` registers.
    pub const fn new(first: u32, len: u32) -> RegisterList {
        RegisterList { first, len }
    }

    /// The empty list. It encodes as `r0` with count 0.
    pub const EMPTY: RegisterList = RegisterList { first: 0, len: 0 };

    /// The first register. For an empty list this is `r0` by convention.
    pub const fn first(self) -> Register {
        if self.len == 0 {
            Register::local(0)
        } else {
            Register::local(self.first)
        }
    }

    /// Number of registers.
    pub const fn len(self) -> u32 {
        self.len
    }

    /// True if the list has no registers.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Register `i` of the list.
    pub const fn get(self, i: u32) -> Register {
        assert!(i < self.len, "register list index out of range");
        Register::local(self.first + i)
    }

    /// The sub-list that starts at `i`.
    pub const fn skip(self, i: u32) -> RegisterList {
        assert!(i <= self.len, "register list index out of range");
        RegisterList {
            first: self.first + i,
            len: self.len - i,
        }
    }
}

macro_rules! index_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            /// The index as `usize`.
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

index_type!(
    /// Index into a function's constant pool.
    ConstIndex,
    "k"
);
index_type!(
    /// Index into a function's feedback slot list.
    FeedbackSlot,
    "s"
);
index_type!(
    /// Stable function id: the preorder index of the function in the source tree (H2).
    FunctionId,
    "f"
);
index_type!(
    /// Index into the unit's string table.
    StringId,
    "str"
);
index_type!(
    /// Index into the unit's scope descriptor table.
    ScopeId,
    "scope"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_encoding() {
        assert_eq!(Register::local(0).to_operand(), 0);
        assert_eq!(Register::local(7).to_operand(), 7);
        assert_eq!(Register::RECEIVER.to_operand(), -1);
        assert_eq!(Register::parameter(0).to_operand(), -2);
        assert_eq!(Register::parameter(3).to_operand(), -5);
        assert_eq!(Register::parameter(3).parameter_index(), Some(3));
        assert_eq!(Register::RECEIVER.parameter_index(), None);
        assert_eq!(Register::local(4).local_index(), Some(4));
        assert_eq!(Register::from_operand(-1), Register::RECEIVER);
        assert_eq!(Register::local(12).to_string(), "r12");
        assert_eq!(Register::parameter(2).to_string(), "a2");
        assert_eq!(Register::RECEIVER.to_string(), "<this>");
    }

    #[test]
    fn scales() {
        assert_eq!(OperandScale::for_unsigned(255), OperandScale::Single);
        assert_eq!(OperandScale::for_unsigned(256), OperandScale::Wide);
        assert_eq!(OperandScale::for_unsigned(65535), OperandScale::Wide);
        assert_eq!(OperandScale::for_unsigned(65536), OperandScale::ExtraWide);
        assert_eq!(OperandScale::for_signed(-128), OperandScale::Single);
        assert_eq!(OperandScale::for_signed(127), OperandScale::Single);
        assert_eq!(OperandScale::for_signed(128), OperandScale::Wide);
        assert_eq!(OperandScale::for_signed(-129), OperandScale::Wide);
        assert_eq!(OperandScale::for_signed(-32768), OperandScale::Wide);
        assert_eq!(OperandScale::for_signed(-32769), OperandScale::ExtraWide);
        assert_eq!(OperandKind::Reg.size(OperandScale::ExtraWide), 4);
        assert_eq!(OperandKind::Flag8.size(OperandScale::ExtraWide), 1);
        assert_eq!(OperandKind::Runtime.size(OperandScale::Single), 2);
    }

    #[test]
    fn register_lists() {
        let l = RegisterList::new(3, 4);
        assert_eq!(l.first(), Register::local(3));
        assert_eq!(l.get(3), Register::local(6));
        assert_eq!(l.skip(1).first(), Register::local(4));
        assert_eq!(l.skip(4).len(), 0);
        assert_eq!(RegisterList::EMPTY.first(), Register::local(0));
    }
}
