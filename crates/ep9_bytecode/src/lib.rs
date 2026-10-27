//! Bytecode ABI (version [`ABI_VERSION`]) and `CompiledUnit` v0 (H1, H2, H3 header fields). ADR:
//! `docs/adr/H02-bytecode-abi.md`, `docs/adr/H03-ts-runtime-semantics.md`.
//!
//! ```text
//!  BytecodeGen (ep9_frontend)                       interpreter (ep9_vm)
//!     FunctionBuilder: typed emitters, labels,         for_each_opcode!: handler DSL
//!     registers, constants, handlers, positions        OPCODES / OpcodeInfo: sizes, offsets
//!        | finish(): jump relaxation, encode            read_operand(): operand decoding
//!     UnitBuilder: strings, scopes, functions             ^
//!        | finish(): verify  ----------------------> VerifiedUnit
//!        v                                                ^
//!     CompiledUnit::to_bytes()  --- code cache / unit -> CompiledUnit::from_bytes()
//!                                   (SHA-256 content hash)      (hash + structure + verify)
//! ```
//!
//! - Register + accumulator machine, 1-byte opcodes, `Wide`/`ExtraWide` prefixes, feedback-slot
//!   operands, relative jumps, constant-pool indices only (no raw pointers, H13).
//! - Deterministic: the same builder calls give the same bytes and hash on every target.
//! - No unsafe code and no dependencies.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod builder;
mod decode;
mod disasm;
mod eval_context;
mod feedback;
mod opcodes;
mod operand;
mod positions;
mod serialize;
pub mod sha256;
mod string;
mod unit;
mod verify;

pub use builder::{
    BuildError, FunctionBuilder, FunctionCode, FunctionMeta, IntoOperand, Label, PendingOperand,
    RegisterMark, UnitBuilder, UnitOptions,
};
pub use decode::{DecodeError, Instruction, Instructions, decode_at, read_operand};
pub use disasm::{disassemble, disassemble_function, instruction_text};
pub use eval_context::{EVAL_EXT_BINDING, EvalContext, FunctionCtorKind};
pub use feedback::{
    FeedbackSlotKind, RUNTIME_FUNCTIONS, RuntimeId, RuntimeInfo, TypeOfLiteral, runtime_table_text,
};
pub use opcodes::{
    AccUse, MAX_INSTRUCTION_SIZE, MAX_OPERANDS, OPCODE_COUNT, OPCODES, Opcode, OpcodeFlags,
    OpcodeInfo, opcode_table_text,
};
pub use operand::{
    ConstIndex, ConstOperand, FeedbackSlot, FunctionId, OperandInfo, OperandKind, OperandParam,
    OperandScale, Register, RegisterList, ScopeId, StringId,
};
pub use positions::{
    LineColumn, PositionTableError, SourceLines, decode_positions, encode_positions, position_at,
};
pub use serialize::{
    FROZEN_PREFIX_SIZE, HASH_SHA256, HEADER_SIZE, HeaderPeek, LoadError, LoadOutcome, MAGIC,
    SECTION_COUNT, Section, compute_content_hash, decode_unit, flags as unit_flags,
    offsets as header_offsets, peek_header,
};
pub use string::UnitString;
pub use unit::{
    ABI_VERSION, BigIntConst, CodeKind, CompiledUnit, Constant, ExportImportName, Function,
    FunctionFlags, FunctionKind, Handler, HandlerKind, ImportEntry, ImportName,
    IndirectExportEntry, LocalExportEntry, MappedParameters, ModuleRecords, ModuleRequest,
    NumberBits, Oddball, PositionEntry, Scope, ScopeFlags, ScopeKind, ScopeSlot, TsFlags,
    UnitHeader, VariableMode, VerifiedUnit,
};
pub use verify::{MAX_PARAMETERS, MAX_REGISTERS, VerifyError, VerifyErrorKind, verify, verify_ref};
