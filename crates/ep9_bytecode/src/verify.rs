//! The bytecode verifier (H2). It runs on every unit before the VM sees it: on builder output
//! and on every deserialized unit.
//!
//! A [`VerifiedUnit`] guarantees (the interpreter may read without checks):
//!
//! | Rule | Detail |
//! |---|---|
//! | Decoding | Every instruction decodes, inside its function; no undefined, `Illegal` or reserved opcode |
//! | Prefixes | A prefix precedes an opcode with a scalable operand; never two; never a larger scale than the operands need |
//! | Registers | Local `r<i>`: `i < register_count`. Parameter `a<k>`: `k < parameter_count`. Lists, pairs and triples: locals only, inside the frame; an empty list is `r0, 0` |
//! | Constants | Index in range; the kind matches the operand (`Const(..)` parameter) |
//! | Feedback | Slot index in range; the slot kind matches the operand (`Slot(..)` parameter) |
//! | Flags | `Flag8`/`Flag16` use only defined bits; `TypeOfLiteral` and `Runtime` ids are defined; `CallRuntime` passes the arity |
//! | Jumps | Target is an instruction start in the same function. Only `JumpLoop` goes backward (or to itself); all other jumps and every jump-table entry go forward |
//! | Fall-through | The last instruction is a terminator, so execution never runs off the end |
//! | Contexts | `CreateFunctionContext`/`CreateEvalContext` slot count equals the scope's slot count |
//! | Handlers | Ranges on instruction starts, nested or disjoint, ordered (start asc, end desc); target an instruction start at or after the range end, so a throw only goes forward and every loop inside a function passes a `JumpLoop` (an interrupt poll); context register is a local |
//! | Positions | Bytecode offsets strictly increase and are instruction starts; source offsets <= source length |
//! | Entry frame | Function 0 only, kind = the unit's code kind, 0 parameters, no arguments object, rest parameter or new.target. Every other function is not an entry kind and is a closure target only of its parent |
//! | Tree | Function ids are a preorder of the parent links; scopes point only to earlier scopes |
//! | Reader rules | Strings are canonical; scope and function flags use only defined bits. The reader checks the same, so a verified unit loads back |
//!
//! Not checked (VM duty, ADR H02 open questions): context slot and depth against the actual
//! context chain (only a unit-wide bound is checked), module cell indices, register
//! initialization.

use crate::decode::{DecodeError, Instruction, Instructions};
use crate::feedback::{RuntimeId, TypeOfLiteral};
use crate::opcodes::{Opcode, OpcodeFlags};
use crate::operand::{ConstOperand, OperandKind, OperandParam, OperandScale, Register};
use crate::sha256::sha256;
use crate::unit::{
    CodeKind, CompiledUnit, Constant, Function, FunctionFlags, FunctionKind, VerifiedUnit,
};

/// Largest `register_count` of a function.
pub const MAX_REGISTERS: u32 = 1 << 24;
/// Largest `parameter_count` of a function (V8 also allows 65534 formal parameters).
pub const MAX_PARAMETERS: u32 = 65534;

/// What the verifier found wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyErrorKind {
    /// The unit has no function.
    NoFunctions,
    /// A module unit is not strict, or a strict function has a sloppy child, or the entry's
    /// strictness differs from the header.
    Strictness,
    /// The embedded source does not match the header's length or hash.
    Source,
    /// A function kind breaks the entry rule (function 0 = the code kind; others not entry).
    EntryKind,
    /// The entry function breaks a frame rule.
    EntryFrame(&'static str),
    /// Parent links are not a preorder tree.
    Tree,
    /// A string id is out of range.
    StringId(u32),
    /// String `n` is UTF-16 but fits Latin-1 (not canonical).
    NonCanonicalString(u32),
    /// Scope `n` sets undefined flag bits.
    ScopeFlags(u32),
    /// A scope id is out of range, or a scope's outer scope is not earlier.
    ScopeId(u32),
    /// Function flags set undefined bits or do not fit the function kind.
    FunctionFlags,
    /// Source range out of order or past the source length.
    SourceRange,
    /// Too many registers or parameters, or `length > parameter_count`.
    Limits,
    /// The function has no bytecode.
    EmptyBytecode,
    /// Bytes do not decode.
    Decode(DecodeError),
    /// A prefix is larger than the operands need.
    NonMinimalScale,
    /// `Illegal` or a reserved opcode.
    ReservedOpcode(Opcode),
    /// Operand `n` is an invalid register, list, pair or triple.
    Register(usize),
    /// Operand `n` is an invalid constant index or names the wrong constant kind.
    ConstantOperand(usize),
    /// Operand `n` is an invalid feedback slot or names the wrong slot kind.
    FeedbackSlot(usize),
    /// Operand `n` sets undefined flag bits or names an undefined enum value.
    FlagValue(usize),
    /// `CallRuntime` argument count differs from the runtime function's arity.
    RuntimeArity,
    /// Operand `n` is a jump to a bad target or in a bad direction.
    Jump(usize),
    /// Entry `n` of a jump table goes to a bad target.
    JumpTable(usize),
    /// Operand `n` is a context slot or depth out of range, or a context slot count differs
    /// from its scope.
    Context(usize),
    /// The last instruction can fall through.
    FallsOffEnd,
    /// Constant pool entry is invalid.
    Constant(u32, &'static str),
    /// Handler table entry is invalid.
    Handler(u32, &'static str),
    /// Position table entry is invalid.
    Position(u32, &'static str),
}

/// A verifier error with its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    /// Function id, if the error is inside a function.
    pub function: Option<u32>,
    /// Bytecode offset, if the error is at an instruction.
    pub offset: Option<u32>,
    /// What is wrong.
    pub kind: VerifyErrorKind,
}

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(func) = self.function {
            write!(f, "f{func}: ")?;
        }
        if let Some(off) = self.offset {
            write!(f, "@{off}: ")?;
        }
        write!(f, "{:?}", self.kind)
    }
}

impl std::error::Error for VerifyError {}

fn unit_err<T>(kind: VerifyErrorKind) -> Result<T, VerifyError> {
    Err(VerifyError {
        function: None,
        offset: None,
        kind,
    })
}

/// Verifies `unit` and wraps it as a [`VerifiedUnit`].
pub fn verify(unit: CompiledUnit) -> Result<VerifiedUnit, VerifyError> {
    verify_ref(&unit)?;
    Ok(VerifiedUnit(unit))
}

/// Verifies `unit` in place.
pub fn verify_ref(unit: &CompiledUnit) -> Result<(), VerifyError> {
    let h = &unit.header;
    if unit.functions.is_empty() {
        return unit_err(VerifyErrorKind::NoFunctions);
    }
    if h.code_kind == CodeKind::Module && !h.strict {
        return unit_err(VerifyErrorKind::Strictness);
    }
    if let Some(src) = &unit.source
        && (src.len() != h.source_len as usize || sha256(src.as_bytes()) != h.source_hash)
    {
        return unit_err(VerifyErrorKind::Source);
    }
    if let Some(i) = unit.strings.iter().position(|s| !s.is_canonical()) {
        return unit_err(VerifyErrorKind::NonCanonicalString(i as u32));
    }
    // Scopes: defined flags, names valid, outer scope earlier.
    for (i, s) in unit.scopes.iter().enumerate() {
        if !s.flags.is_defined() {
            return unit_err(VerifyErrorKind::ScopeFlags(i as u32));
        }
        if let Some(o) = s.outer
            && o.index() >= i
        {
            return unit_err(VerifyErrorKind::ScopeId(o.0));
        }
        for slot in &s.slots {
            if slot.name.index() >= unit.strings.len() {
                return unit_err(VerifyErrorKind::StringId(slot.name.0));
            }
        }
    }
    let max_ctx_slots = unit.scopes.iter().map(|s| s.slots.len()).max().unwrap_or(0) as u64;
    // Function tree: preorder of the parent links. `chain` is the ancestor chain of the
    // previous function, root first.
    let mut chain: Vec<u32> = Vec::new();
    for (i, f) in unit.functions.iter().enumerate() {
        let i = i as u32;
        let ferr = |kind| {
            Err(VerifyError {
                function: Some(i),
                offset: None,
                kind,
            })
        };
        match (i, f.parent) {
            (0, None) => {}
            (0, Some(_)) | (_, None) => return ferr(VerifyErrorKind::Tree),
            (_, Some(p)) => {
                // The parent must be on the chain of the previous function (or be it).
                while chain.last().is_some_and(|&top| top != p.0) {
                    chain.pop();
                }
                if chain.is_empty() {
                    return ferr(VerifyErrorKind::Tree);
                }
                let parent = &unit.functions[p.index()];
                if parent.flags.contains(FunctionFlags::STRICT)
                    && !f.flags.contains(FunctionFlags::STRICT)
                {
                    return ferr(VerifyErrorKind::Strictness);
                }
            }
        }
        chain.push(i);
        if i == 0 {
            if f.kind != h.code_kind.entry_function_kind() {
                return ferr(VerifyErrorKind::EntryKind);
            }
            if f.flags.contains(FunctionFlags::STRICT) != h.strict {
                return ferr(VerifyErrorKind::Strictness);
            }
            if f.parameter_count != 0 {
                return ferr(VerifyErrorKind::EntryFrame("entry function has parameters"));
            }
        } else if f.kind.is_entry() {
            return ferr(VerifyErrorKind::EntryKind);
        }
    }
    // Function bodies, after the whole tree is known to be valid.
    for (i, f) in unit.functions.iter().enumerate() {
        verify_function(unit, i as u32, f, max_ctx_slots)?;
    }
    Ok(())
}

fn flags_fit_kind(f: &Function) -> bool {
    let generator = f.flags.contains(FunctionFlags::GENERATOR);
    let is_async = f.flags.contains(FunctionFlags::ASYNC);
    let gen_ok = matches!(f.kind, FunctionKind::Normal | FunctionKind::Method);
    let async_ok = matches!(
        f.kind,
        FunctionKind::Normal | FunctionKind::Method | FunctionKind::Arrow | FunctionKind::Module
    );
    (!generator || gen_ok) && (!is_async || async_ok)
}

struct Ctx<'a> {
    unit: &'a CompiledUnit,
    func: &'a Function,
    id: u32,
}

impl Ctx<'_> {
    fn err<T>(&self, offset: Option<u32>, kind: VerifyErrorKind) -> Result<T, VerifyError> {
        Err(VerifyError {
            function: Some(self.id),
            offset,
            kind,
        })
    }

    fn register_ok(&self, r: Register) -> bool {
        match (r.local_index(), r.parameter_index()) {
            (Some(i), _) => i < self.func.register_count,
            (_, Some(k)) => k < self.func.parameter_count,
            _ => true, // receiver
        }
    }

    /// A run of `n` locals from `first`.
    fn locals_ok(&self, first: i64, n: u64) -> bool {
        first >= 0 && (first as u64) + n <= u64::from(self.func.register_count)
    }
}

fn verify_function(
    unit: &CompiledUnit,
    id: u32,
    f: &Function,
    max_ctx_slots: u64,
) -> Result<(), VerifyError> {
    let cx = Ctx { unit, func: f, id };
    // Metadata.
    if let Some(n) = f.name
        && n.index() >= unit.strings.len()
    {
        return cx.err(None, VerifyErrorKind::StringId(n.0));
    }
    if let Some(s) = f.outer_scope
        && s.index() >= unit.scopes.len()
    {
        return cx.err(None, VerifyErrorKind::ScopeId(s.0));
    }
    if f.source_start > f.source_end || f.source_end > unit.header.source_len {
        return cx.err(None, VerifyErrorKind::SourceRange);
    }
    if !f.flags.is_defined() || !flags_fit_kind(f) {
        return cx.err(None, VerifyErrorKind::FunctionFlags);
    }
    if f.register_count > MAX_REGISTERS
        || f.parameter_count > MAX_PARAMETERS
        || f.length > f.parameter_count
    {
        return cx.err(None, VerifyErrorKind::Limits);
    }
    if f.bytecode.is_empty() {
        return cx.err(None, VerifyErrorKind::EmptyBytecode);
    }
    if u32::try_from(f.bytecode.len()).is_err() {
        return cx.err(None, VerifyErrorKind::Limits);
    }
    verify_constants(&cx)?;

    // Pass 1: decode, mark instruction starts, check prefixes and opcodes.
    let len = f.bytecode.len();
    let mut starts = vec![false; len];
    let mut insns: Vec<Instruction<'_>> = Vec::new();
    for r in Instructions::new(&f.bytecode) {
        let insn = match r {
            Ok(i) => i,
            Err((at, e)) => return cx.err(Some(at), VerifyErrorKind::Decode(e)),
        };
        let info = insn.opcode.info();
        if info.flags.contains(OpcodeFlags::RESERVED) {
            return cx.err(
                Some(insn.offset),
                VerifyErrorKind::ReservedOpcode(insn.opcode),
            );
        }
        if insn.scale != OperandScale::Single && required_scale(&insn) < insn.scale {
            return cx.err(Some(insn.offset), VerifyErrorKind::NonMinimalScale);
        }
        if id == 0
            && matches!(
                insn.opcode,
                Opcode::CreateMappedArguments
                    | Opcode::CreateUnmappedArguments
                    | Opcode::CreateRestParameter
                    | Opcode::LdaNewTarget
            )
        {
            return cx.err(
                Some(insn.offset),
                VerifyErrorKind::EntryFrame("entry function uses its arguments or new.target"),
            );
        }
        starts[insn.offset as usize] = true;
        insns.push(insn);
    }
    let last = insns
        .last()
        .expect("non-empty bytecode decodes to an instruction");
    if !last.opcode.info().flags.contains(OpcodeFlags::TERMINATOR) {
        return cx.err(Some(last.offset), VerifyErrorKind::FallsOffEnd);
    }
    let is_start = |t: i64| t >= 0 && (t as usize) < len && starts[t as usize];

    // Pass 2: operands.
    for insn in &insns {
        let at = Some(insn.offset);
        let info = insn.opcode.info();
        for (k, op) in info.operands.iter().enumerate() {
            let v = insn.operand(k);
            let ok = match op.kind {
                OperandKind::Reg | OperandKind::RegOut | OperandKind::RegInOut => {
                    cx.register_ok(Register::from_operand(v as i32))
                }
                OperandKind::RegList | OperandKind::RegOutList => {
                    let n = insn.operand(k + 1) as u64;
                    if n == 0 { v == 0 } else { cx.locals_ok(v, n) }
                }
                OperandKind::RegPair => cx.locals_ok(v, 2),
                OperandKind::RegOutTriple => cx.locals_ok(v, 3),
                OperandKind::RegCount => true, // checked with its list
                OperandKind::Imm | OperandKind::UImm => true,
                OperandKind::Const => {
                    let OperandParam::Const(want) = op.param else {
                        unreachable!("opcode table: Const without parameter")
                    };
                    if !const_matches(f, v, want) {
                        return cx.err(at, VerifyErrorKind::ConstantOperand(k));
                    }
                    if want == ConstOperand::JumpTable {
                        let Constant::JumpTable(t) = &f.constants[v as usize] else {
                            unreachable!()
                        };
                        for (j, &e) in t.iter().enumerate() {
                            if e == 0 || !is_start(i64::from(insn.offset) + i64::from(e)) {
                                return cx.err(at, VerifyErrorKind::JumpTable(j));
                            }
                        }
                    }
                    true
                }
                OperandKind::Slot => {
                    let OperandParam::Slot(want) = op.param else {
                        unreachable!("opcode table: Slot without parameter")
                    };
                    if f.feedback_slots.get(v as usize) != Some(&want) {
                        return cx.err(at, VerifyErrorKind::FeedbackSlot(k));
                    }
                    true
                }
                OperandKind::CtxSlot => {
                    if (v as u64) >= max_ctx_slots {
                        return cx.err(at, VerifyErrorKind::Context(k));
                    }
                    true
                }
                OperandKind::CtxDepth => {
                    if (v as u64) > unit.scopes.len() as u64 {
                        return cx.err(at, VerifyErrorKind::Context(k));
                    }
                    true
                }
                OperandKind::JumpFwd => {
                    if v <= 0 || !is_start(i64::from(insn.offset) + v) {
                        return cx.err(at, VerifyErrorKind::Jump(k));
                    }
                    true
                }
                OperandKind::JumpBack => {
                    if !is_start(i64::from(insn.offset) - v) {
                        return cx.err(at, VerifyErrorKind::Jump(k));
                    }
                    true
                }
                OperandKind::Flag8 | OperandKind::Flag16 => {
                    let OperandParam::FlagMask(mask) = op.param else {
                        unreachable!("opcode table: flag without mask")
                    };
                    if v as u64 & !u64::from(mask) != 0 {
                        return cx.err(at, VerifyErrorKind::FlagValue(k));
                    }
                    true
                }
                OperandKind::Runtime => {
                    let Some(id) = RuntimeId::from_u16(v as u16) else {
                        return cx.err(at, VerifyErrorKind::FlagValue(k));
                    };
                    // CallRuntime(id, args, argc): argc is operand k + 2.
                    if let Some(arity) = id.info().arity
                        && insn.operand(k + 2) != i64::from(arity)
                    {
                        return cx.err(at, VerifyErrorKind::RuntimeArity);
                    }
                    true
                }
                OperandKind::TypeOfLiteral => TypeOfLiteral::from_u8(v as u8).is_some(),
            };
            if !ok {
                let kind = if op.kind.is_register() {
                    VerifyErrorKind::Register(k)
                } else {
                    VerifyErrorKind::FlagValue(k)
                };
                return cx.err(at, kind);
            }
        }
        // Opcode-specific rules.
        match insn.opcode {
            Opcode::CreateFunctionContext | Opcode::CreateEvalContext => {
                let Constant::Scope(s) = f.constants[insn.operand(0) as usize] else {
                    unreachable!("checked above")
                };
                let slots = unit.scopes[s.index()].slots.len() as i64;
                if insn.operand(1) != slots {
                    return cx.err(at, VerifyErrorKind::Context(1));
                }
            }
            Opcode::JumpLoop if insn.operand(1) < 0 => {
                return cx.err(at, VerifyErrorKind::FlagValue(1));
            }
            _ => {}
        }
    }
    verify_handlers(&cx, &starts)?;
    verify_positions(&cx, &starts)?;
    Ok(())
}

/// The smallest scale that holds every scalable operand of `insn`.
fn required_scale(insn: &Instruction<'_>) -> OperandScale {
    let mut s = OperandScale::Single;
    for (k, op) in insn.opcode.info().operands.iter().enumerate() {
        if !op.kind.is_scalable() {
            continue;
        }
        let v = insn.operand(k);
        let need = if op.kind.is_signed() {
            OperandScale::for_signed(v as i32)
        } else {
            OperandScale::for_unsigned(v as u32)
        };
        s = s.max(need);
    }
    s
}

fn const_matches(f: &Function, index: i64, want: ConstOperand) -> bool {
    let Some(c) = usize::try_from(index).ok().and_then(|i| f.constants.get(i)) else {
        return false;
    };
    match want {
        ConstOperand::Value => matches!(
            c,
            Constant::Number(_) | Constant::String(_) | Constant::BigInt(_)
        ),
        ConstOperand::String => matches!(c, Constant::String(_)),
        ConstOperand::Function => matches!(c, Constant::Function(_)),
        ConstOperand::Scope => matches!(c, Constant::Scope(_)),
        ConstOperand::JumpTable => matches!(c, Constant::JumpTable(_)),
        ConstOperand::ArrayBoilerplate => matches!(c, Constant::ArrayBoilerplate(_)),
        ConstOperand::ObjectBoilerplate => matches!(c, Constant::ObjectBoilerplate(_)),
        ConstOperand::TemplateObject => matches!(c, Constant::TemplateObject(_)),
    }
}

fn verify_constants(cx: &Ctx<'_>) -> Result<(), VerifyError> {
    let unit = cx.unit;
    let f = cx.func;
    let string_ok = |s: crate::StringId| s.index() < unit.strings.len();
    for (i, c) in f.constants.iter().enumerate() {
        let bad = |what| cx.err(None, VerifyErrorKind::Constant(i as u32, what));
        // An earlier entry that may be a boilerplate value.
        let value_ok =
            |k: crate::ConstIndex| k.index() < i && f.constants[k.index()].is_boilerplate_value();
        match c {
            Constant::Number(n) => {
                if crate::NumberBits::from_bits(n.bits()).is_none() {
                    return bad("non-canonical NaN");
                }
            }
            Constant::String(s) => {
                if !string_ok(*s) {
                    return bad("string id");
                }
            }
            Constant::BigInt(b) => {
                if !b.is_canonical() {
                    return bad("non-canonical BigInt");
                }
            }
            Constant::Function(id) => {
                let ok = unit
                    .functions
                    .get(id.index())
                    .is_some_and(|child| child.parent == Some(crate::FunctionId(cx.id)));
                if !ok {
                    return bad("function is not a direct child");
                }
            }
            Constant::Scope(s) => {
                if s.index() >= unit.scopes.len() {
                    return bad("scope id");
                }
            }
            Constant::JumpTable(t) => {
                if t.is_empty() {
                    return bad("empty jump table");
                }
            }
            Constant::ArrayBoilerplate(e) => {
                if !e.iter().flatten().all(|&k| value_ok(k)) {
                    return bad("array element");
                }
            }
            Constant::ObjectBoilerplate(p) => {
                for &(k, v) in p.iter() {
                    let key_ok = k.index() < i
                        && matches!(
                            f.constants[k.index()],
                            Constant::String(_) | Constant::Number(_)
                        );
                    if !key_ok || !v.is_none_or(value_ok) {
                        return bad("object property");
                    }
                }
            }
            Constant::TemplateObject(p) => {
                if p.is_empty()
                    || !p
                        .iter()
                        .all(|&(raw, cooked)| string_ok(raw) && cooked.is_none_or(string_ok))
                {
                    return bad("template strings");
                }
            }
            Constant::Oddball(_) => {}
        }
    }
    Ok(())
}

fn verify_handlers(cx: &Ctx<'_>, starts: &[bool]) -> Result<(), VerifyError> {
    let f = cx.func;
    let len = starts.len() as u32;
    let start_or_end = |o: u32| o == len || (o < len && starts[o as usize]);
    let mut open: Vec<(u32, u32)> = Vec::new();
    let mut prev: Option<(u32, u32)> = None;
    for (j, h) in f.handlers.iter().enumerate() {
        let bad = |what| cx.err(None, VerifyErrorKind::Handler(j as u32, what));
        if h.start >= h.end || !start_or_end(h.start) || h.start == len || !start_or_end(h.end) {
            return bad("range");
        }
        if h.target >= len || !starts[h.target as usize] {
            return bad("target");
        }
        // A throw goes forward, as every jump but `JumpLoop` does (H02 F6). A target before
        // the range end is a back edge without an interrupt poll: a `Throw` loop that
        // termination never stops.
        if h.target < h.end {
            return bad("target before the range end");
        }
        if !h
            .context
            .local_index()
            .is_some_and(|i| i < f.register_count)
        {
            return bad("context register");
        }
        if let Some((ps, pe)) = prev
            && (h.start < ps || (h.start == ps && h.end > pe))
        {
            return bad("order");
        }
        prev = Some((h.start, h.end));
        while open.last().is_some_and(|&(_, e)| e <= h.start) {
            open.pop();
        }
        if let Some(&(_, e)) = open.last()
            && h.end > e
        {
            return bad("partial overlap");
        }
        open.push((h.start, h.end));
    }
    Ok(())
}

fn verify_positions(cx: &Ctx<'_>, starts: &[bool]) -> Result<(), VerifyError> {
    let mut prev: Option<u32> = None;
    for (j, p) in cx.func.positions.iter().enumerate() {
        let bad = |what| cx.err(None, VerifyErrorKind::Position(j as u32, what));
        if prev.is_some_and(|q| q >= p.bytecode_offset) {
            return bad("offsets not increasing");
        }
        prev = Some(p.bytecode_offset);
        if (p.bytecode_offset as usize) >= starts.len() || !starts[p.bytecode_offset as usize] {
            return bad("not an instruction start");
        }
        if p.source_offset > cx.unit.header.source_len {
            return bad("source offset past the end");
        }
    }
    Ok(())
}
