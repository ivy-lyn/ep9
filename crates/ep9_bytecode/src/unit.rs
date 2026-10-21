//! `CompiledUnit` v0: the GC-free, serializable compile result (H1, H2). It is the code-cache
//! format and the bytecode section of an AOT unit (H35).
//!
//! ```text
//! CompiledUnit
//!   header     code kind, strict, TS flags (H3), engine build id, source hash + length
//!   strings    unit string table (Latin-1 / UTF-16), referenced by StringId
//!   scopes     scope descriptors (context layout, names), referenced by ScopeId
//!   functions  index = stable function id (preorder); function 0 is the entry function
//!     each: kind, flags, name, parent, outer scope, source range, parameter/register counts,
//!           bytecode, constant pool, feedback slot kinds, handler table, position table
//!   source     optional decoded source text (always present in AOT units, H5)
//!   hints      optional hint section (format owned by ep9_facts, H37)
//! ```

use crate::feedback::FeedbackSlotKind;
use crate::operand::{ConstIndex, FunctionId, Register, ScopeId, StringId};
use crate::sha256::Digest;
use crate::string::UnitString;

/// Bytecode ABI version (H2). Any change to the opcode table, the operand encoding, the
/// serialized layout or a verifier rule that accepts or rejects other bytes bumps it. The test
/// `tests/abi.rs` ties each version to the SHA-256 of the opcode table text, the runtime table
/// text and the tiny unit.
///
/// | Version | Change |
/// |---|---|
/// | 1 | First ABI (opcode list, CompiledUnit v0) |
/// | 2 | Runtime ids 6-18 for BytecodeGen v0; `DefineClass` takes 2 arguments; a handler target is at or after its range end (added before v2 was released, so no bump) |
/// | 3 | Generator opcodes 0x99-0x9b valid in generator and async functions (flag `Generator`, no longer `Reserved`); `CallRuntimeForPair` 0x9d with operand kind `RegOutPair`; `DefineKeyedOwnProperty` flag bit 1 (private add); runtime ids 19-28 and the `pair` mark in the runtime table |
/// | 4 | Runtime ids 29-31: a dynamic reference resolved before the right-hand side (`ResolveLookupSlot`, `LoadLookupSlotResolved`, `StoreLookupSlotResolved`; M2-P2 review fix L1) |
pub const ABI_VERSION: u32 = 4;

/// The ECMA-262 code kind of the unit's entry function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CodeKind {
    /// Global code (`Script` goal).
    Script = 0,
    /// Module code (`Module` goal); always strict.
    Module = 1,
    /// Eval code (direct or indirect eval).
    Eval = 2,
}

impl CodeKind {
    /// The kind with value `v`.
    pub const fn from_u8(v: u8) -> Option<CodeKind> {
        match v {
            0 => Some(Self::Script),
            1 => Some(Self::Module),
            2 => Some(Self::Eval),
            _ => None,
        }
    }

    /// Name in the disassembly.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Module => "module",
            Self::Eval => "eval",
        }
    }

    /// The function kind of the entry function for this code kind.
    pub const fn entry_function_kind(self) -> FunctionKind {
        match self {
            Self::Script => FunctionKind::Script,
            Self::Module => FunctionKind::Module,
            Self::Eval => FunctionKind::Eval,
        }
    }
}

/// Per-module TypeScript flags (H3, D20). Present only for TypeScript source.
///
/// D20: follow tsconfig; without a tsconfig, `use_define_for_class_fields` is true
/// (`[[Define]]`, as node type stripping does) and the decorator flags are false.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TsFlags {
    /// `useDefineForClassFields`: class fields use `[[Define]]` (true) or `[[Set]]` (false).
    pub use_define_for_class_fields: bool,
    /// `experimentalDecorators`: legacy decorators.
    pub experimental_decorators: bool,
    /// `emitDecoratorMetadata`.
    pub emit_decorator_metadata: bool,
}

impl TsFlags {
    /// The D20 default without a tsconfig.
    pub const DEFAULT: TsFlags = TsFlags {
        use_define_for_class_fields: true,
        experimental_decorators: false,
        emit_decorator_metadata: false,
    };
}

impl Default for TsFlags {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Unit header fields that the serialized header carries (the content hash is computed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnitHeader {
    /// Code kind of the entry function.
    pub code_kind: CodeKind,
    /// Top-level strictness. Always true for modules.
    pub strict: bool,
    /// `Some` for TypeScript source (H3). Must be `None` for JavaScript.
    pub typescript: Option<TsFlags>,
    /// Engine build id. Opaque; not covered by the content hash.
    pub engine_build_id: [u8; 16],
    /// SHA-256 of the decoded source text (UTF-8 bytes).
    pub source_hash: Digest,
    /// Length of the decoded source text in bytes.
    pub source_len: u32,
}

/// Function kind (the code that BytecodeGen compiled).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FunctionKind {
    /// `function` declaration or expression.
    Normal = 0,
    /// Arrow function.
    Arrow = 1,
    /// Object or class method.
    Method = 2,
    /// Getter.
    Getter = 3,
    /// Setter.
    Setter = 4,
    /// Base class constructor.
    ClassConstructor = 5,
    /// Derived class constructor.
    DerivedConstructor = 6,
    /// Synthetic function that runs class field initializers.
    ClassFieldsInitializer = 7,
    /// Script top level (entry function).
    Script = 8,
    /// Module top level (entry function).
    Module = 9,
    /// Eval code top level (entry function).
    Eval = 10,
}

impl FunctionKind {
    /// The kind with value `v`.
    pub const fn from_u8(v: u8) -> Option<FunctionKind> {
        Some(match v {
            0 => Self::Normal,
            1 => Self::Arrow,
            2 => Self::Method,
            3 => Self::Getter,
            4 => Self::Setter,
            5 => Self::ClassConstructor,
            6 => Self::DerivedConstructor,
            7 => Self::ClassFieldsInitializer,
            8 => Self::Script,
            9 => Self::Module,
            10 => Self::Eval,
            _ => return None,
        })
    }

    /// True for the kinds the host enters directly (function 0 only).
    pub const fn is_entry(self) -> bool {
        matches!(self, Self::Script | Self::Module | Self::Eval)
    }

    /// Name in the disassembly.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Arrow => "arrow",
            Self::Method => "method",
            Self::Getter => "getter",
            Self::Setter => "setter",
            Self::ClassConstructor => "class-constructor",
            Self::DerivedConstructor => "derived-constructor",
            Self::ClassFieldsInitializer => "class-fields-initializer",
            Self::Script => "script",
            Self::Module => "module",
            Self::Eval => "eval",
        }
    }
}

/// Function flag bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct FunctionFlags(pub u8);

impl FunctionFlags {
    /// Strict mode code.
    pub const STRICT: FunctionFlags = FunctionFlags(1 << 0);
    /// Generator function.
    pub const GENERATOR: FunctionFlags = FunctionFlags(1 << 1);
    /// Async function.
    pub const ASYNC: FunctionFlags = FunctionFlags(1 << 2);
    /// Simple parameter list (no defaults, rest or destructuring).
    pub const SIMPLE_PARAMETERS: FunctionFlags = FunctionFlags(1 << 3);
    /// All defined bits.
    pub const ALL: u8 = 0x0f;

    /// True if only defined bits are set (the reader and the verifier both require it).
    pub const fn is_defined(self) -> bool {
        self.0 & !Self::ALL == 0
    }

    /// True if every bit of `other` is set.
    pub const fn contains(self, other: FunctionFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Union.
    pub const fn with(self, other: FunctionFlags) -> FunctionFlags {
        FunctionFlags(self.0 | other.0)
    }
}

impl core::ops::BitOr for FunctionFlags {
    type Output = FunctionFlags;

    fn bitor(self, rhs: FunctionFlags) -> FunctionFlags {
        self.with(rhs)
    }
}

/// Scope kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ScopeKind {
    /// Script top level: slots list the script's declarations.
    Script = 0,
    /// Module top level.
    Module = 1,
    /// Eval code.
    Eval = 2,
    /// Function body and parameters.
    Function = 3,
    /// Block (`let`, `const`, `class` in a block).
    Block = 4,
    /// `catch` clause binding.
    Catch = 5,
    /// `with` statement.
    With = 6,
    /// Class body (class binding, private names, home object).
    Class = 7,
}

impl ScopeKind {
    /// The kind with value `v`.
    pub const fn from_u8(v: u8) -> Option<ScopeKind> {
        Some(match v {
            0 => Self::Script,
            1 => Self::Module,
            2 => Self::Eval,
            3 => Self::Function,
            4 => Self::Block,
            5 => Self::Catch,
            6 => Self::With,
            7 => Self::Class,
            _ => return None,
        })
    }

    /// Name in the disassembly.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Module => "module",
            Self::Eval => "eval",
            Self::Function => "function",
            Self::Block => "block",
            Self::Catch => "catch",
            Self::With => "with",
            Self::Class => "class",
        }
    }
}

/// Scope flag bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ScopeFlags(pub u8);

impl ScopeFlags {
    /// The scope allocates a context at run time.
    pub const NEEDS_CONTEXT: ScopeFlags = ScopeFlags(1 << 0);
    /// The scope contains a sloppy direct eval (dynamic lookups). It is set only on the var
    /// scope of the eval site. In a function var scope (not a `Script` scope), slot 0 is the
    /// [`EVAL_EXT_BINDING`](crate::EVAL_EXT_BINDING) binding (from ABI 3).
    pub const CALLS_SLOPPY_EVAL: ScopeFlags = ScopeFlags(1 << 1);
    /// Strict mode scope.
    pub const STRICT: ScopeFlags = ScopeFlags(1 << 2);
    /// All defined bits.
    pub const ALL: u8 = 0x07;

    /// True if only defined bits are set (the reader and the verifier both require it).
    pub const fn is_defined(self) -> bool {
        self.0 & !Self::ALL == 0
    }

    /// True if every bit of `other` is set.
    pub const fn contains(self, other: ScopeFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for ScopeFlags {
    type Output = ScopeFlags;

    fn bitor(self, rhs: ScopeFlags) -> ScopeFlags {
        ScopeFlags(self.0 | rhs.0)
    }
}

/// How a binding was declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VariableMode {
    /// `var` or a parameter.
    Var = 0,
    /// `let`.
    Let = 1,
    /// `const`.
    Const = 2,
    /// Hoisted function declaration.
    Function = 3,
    /// Class binding.
    Class = 4,
}

impl VariableMode {
    /// The mode with value `v`.
    pub const fn from_u8(v: u8) -> Option<VariableMode> {
        Some(match v {
            0 => Self::Var,
            1 => Self::Let,
            2 => Self::Const,
            3 => Self::Function,
            4 => Self::Class,
            _ => return None,
        })
    }

    /// Name in the disassembly.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Var => "var",
            Self::Let => "let",
            Self::Const => "const",
            Self::Function => "function",
            Self::Class => "class",
        }
    }
}

/// One variable of a scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeSlot {
    /// Variable name.
    pub name: StringId,
    /// Declaration kind.
    pub mode: VariableMode,
}

/// A scope descriptor (V8 `ScopeInfo` subset).
///
/// For function, block, catch, class and with scopes, `slots[i]` is context slot `i`. For
/// script, module and eval scopes, `slots` lists the declarations of that code.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Scope {
    /// Kind.
    pub kind: ScopeKind,
    /// Flags.
    pub flags: ScopeFlags,
    /// The enclosing scope. Always a smaller id than this scope.
    pub outer: Option<ScopeId>,
    /// Variables.
    pub slots: Vec<ScopeSlot>,
}

/// A number constant, stored as IEEE-754 bits. NaN is always `0x7ff8_0000_0000_0000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NumberBits(u64);

impl NumberBits {
    /// The canonical quiet NaN.
    pub const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;

    /// The constant for `v`. Any NaN becomes the canonical NaN; -0 stays -0.
    pub fn from_f64(v: f64) -> NumberBits {
        if v.is_nan() {
            NumberBits(Self::CANONICAL_NAN)
        } else {
            NumberBits(v.to_bits())
        }
    }

    /// The constant with these bits, if they are canonical.
    pub fn from_bits(bits: u64) -> Option<NumberBits> {
        let v = f64::from_bits(bits);
        if v.is_nan() && bits != Self::CANONICAL_NAN {
            None
        } else {
            Some(NumberBits(bits))
        }
    }

    /// The bits.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// The value.
    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

/// A BigInt constant: sign and magnitude, 64-bit limbs, least significant first.
///
/// Canonical: no most-significant zero limb; zero has no limbs and is not negative.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BigIntConst {
    /// True if negative.
    pub negative: bool,
    /// Magnitude limbs, least significant first.
    pub limbs: Box<[u64]>,
}

impl BigIntConst {
    /// The canonical form of (negative, limbs): strips zero high limbs, zero is non-negative.
    pub fn new(negative: bool, limbs: &[u64]) -> BigIntConst {
        let mut n = limbs.len();
        while n > 0 && limbs[n - 1] == 0 {
            n -= 1;
        }
        BigIntConst {
            negative: negative && n > 0,
            limbs: limbs[..n].into(),
        }
    }

    /// True if the representation is canonical.
    pub fn is_canonical(&self) -> bool {
        match self.limbs.last() {
            None => !self.negative,
            Some(&top) => top != 0,
        }
    }
}

/// A constant that only boilerplates use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Oddball {
    /// `undefined`
    Undefined = 0,
    /// `null`
    Null = 1,
    /// `true`
    True = 2,
    /// `false`
    False = 3,
    /// An array hole (`[1, , 3]`).
    Hole = 4,
}

impl Oddball {
    /// The oddball with value `v`.
    pub const fn from_u8(v: u8) -> Option<Oddball> {
        Some(match v {
            0 => Self::Undefined,
            1 => Self::Null,
            2 => Self::True,
            3 => Self::False,
            4 => Self::Hole,
            _ => return None,
        })
    }

    /// Name in the disassembly.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Null => "null",
            Self::True => "true",
            Self::False => "false",
            Self::Hole => "hole",
        }
    }
}

/// One constant pool entry.
///
/// Boilerplate entries refer to other entries of the same pool by [`ConstIndex`]. Each such
/// reference must be smaller than the entry's own index, so the pool has no cycles.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constant {
    /// A Number.
    Number(NumberBits),
    /// A String from the unit string table.
    String(StringId),
    /// A BigInt.
    BigInt(BigIntConst),
    /// A nested function: a direct child of the function that owns the pool.
    Function(FunctionId),
    /// A scope descriptor.
    Scope(ScopeId),
    /// A jump table: forward offsets from the start of the switch instruction that uses it.
    JumpTable(Box<[u32]>),
    /// Array literal elements. `None` = computed by code after creation.
    ArrayBoilerplate(Box<[Option<ConstIndex>]>),
    /// Object literal properties: (key, value). Key is a String or Number entry. `None` value
    /// = computed by code after creation.
    ObjectBoilerplate(Box<[(ConstIndex, Option<ConstIndex>)]>),
    /// Tagged template strings: (raw, cooked). `None` cooked = undefined (invalid escape).
    TemplateObject(Box<[(StringId, Option<StringId>)]>),
    /// undefined, null, true, false or hole (boilerplate values only).
    Oddball(Oddball),
}

impl Constant {
    /// The serialized tag byte.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Number(_) => 0,
            Self::String(_) => 1,
            Self::BigInt(_) => 2,
            Self::Function(_) => 3,
            Self::Scope(_) => 4,
            Self::JumpTable(_) => 5,
            Self::ArrayBoilerplate(_) => 6,
            Self::ObjectBoilerplate(_) => 7,
            Self::TemplateObject(_) => 8,
            Self::Oddball(_) => 9,
        }
    }

    /// Name in the disassembly.
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Number(_) => "Number",
            Self::String(_) => "String",
            Self::BigInt(_) => "BigInt",
            Self::Function(_) => "Function",
            Self::Scope(_) => "Scope",
            Self::JumpTable(_) => "JumpTable",
            Self::ArrayBoilerplate(_) => "ArrayBoilerplate",
            Self::ObjectBoilerplate(_) => "ObjectBoilerplate",
            Self::TemplateObject(_) => "TemplateObject",
            Self::Oddball(_) => "Oddball",
        }
    }

    /// True if this constant can be a boilerplate element or property value.
    pub const fn is_boilerplate_value(&self) -> bool {
        matches!(
            self,
            Self::Number(_)
                | Self::String(_)
                | Self::BigInt(_)
                | Self::Oddball(_)
                | Self::ArrayBoilerplate(_)
                | Self::ObjectBoilerplate(_)
        )
    }
}

/// Exception-handler catch prediction (V8 `HandlerTable::CatchPrediction`), for the debugger's
/// "pause on uncaught exceptions".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HandlerKind {
    /// A `catch` block catches the exception.
    Caught = 0,
    /// A `finally` block rethrows it.
    Uncaught = 1,
    /// The handler rejects a promise.
    Promise = 2,
    /// The handler belongs to an `await`.
    AsyncAwait = 3,
}

impl HandlerKind {
    /// The kind with value `v`.
    pub const fn from_u8(v: u8) -> Option<HandlerKind> {
        Some(match v {
            0 => Self::Caught,
            1 => Self::Uncaught,
            2 => Self::Promise,
            3 => Self::AsyncAwait,
            _ => return None,
        })
    }

    /// Name in the disassembly.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Caught => "caught",
            Self::Uncaught => "uncaught",
            Self::Promise => "promise",
            Self::AsyncAwait => "async-await",
        }
    }
}

/// One exception handler: a throw at an offset in `start..end` jumps to `target`, with the
/// exception in the accumulator and the current context restored from `context`.
///
/// Table order: by `start` ascending, then `end` descending; ranges are nested or disjoint.
/// So the innermost handler for an offset is the last entry whose range holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handler {
    /// First covered instruction offset.
    pub start: u32,
    /// End of the covered range (exclusive): an instruction start or the code length.
    pub end: u32,
    /// Handler entry: an instruction start.
    pub target: u32,
    /// Local register that holds the context to restore.
    pub context: Register,
    /// Catch prediction.
    pub kind: HandlerKind,
}

/// One source position entry: the instruction at `bytecode_offset` (and the ones after it, up
/// to the next entry) came from the source text at byte `source_offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PositionEntry {
    /// Instruction start. Entries are in strictly increasing offset order.
    pub bytecode_offset: u32,
    /// UTF-8 byte offset in the decoded source text.
    pub source_offset: u32,
    /// True for a statement position (breakpoints, stepping); false for an expression.
    pub is_statement: bool,
}

/// One compiled function.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Function {
    /// Kind.
    pub kind: FunctionKind,
    /// Flags.
    pub flags: FunctionFlags,
    /// Name (`None` for anonymous functions).
    pub name: Option<StringId>,
    /// Enclosing function. `None` only for function 0.
    pub parent: Option<FunctionId>,
    /// The scope in which a closure of this function is created (H4: outer ScopeInfo).
    pub outer_scope: Option<ScopeId>,
    /// Source range, UTF-8 byte offsets `start..end` in the decoded source.
    pub source_start: u32,
    /// End of the source range (exclusive).
    pub source_end: u32,
    /// Formal parameter registers `a0..`, receiver not counted.
    pub parameter_count: u32,
    /// The `length` property value.
    pub length: u32,
    /// Local registers `r0..`.
    pub register_count: u32,
    /// Bytecode.
    pub bytecode: Vec<u8>,
    /// Constant pool.
    pub constants: Vec<Constant>,
    /// Feedback slot kinds; slot `i` has kind `feedback_slots[i]`.
    pub feedback_slots: Vec<FeedbackSlotKind>,
    /// Exception handler table.
    pub handlers: Vec<Handler>,
    /// Source position table.
    pub positions: Vec<PositionEntry>,
}

/// A compiled unit. Fields are public so that tools and tests can build or change any unit;
/// the VM runs only a [`VerifiedUnit`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompiledUnit {
    /// Header fields.
    pub header: UnitHeader,
    /// Unit string table.
    pub strings: Vec<UnitString>,
    /// Scope descriptors.
    pub scopes: Vec<Scope>,
    /// Functions; index = stable function id (preorder). Function 0 is the entry function.
    pub functions: Vec<Function>,
    /// Decoded source text, if embedded.
    pub source: Option<String>,
    /// Hint section bytes, if present (format owned by `ep9_facts`).
    pub hints: Option<Vec<u8>>,
}

impl CompiledUnit {
    /// String `id`. Panics if out of range.
    pub fn string(&self, id: StringId) -> &UnitString {
        &self.strings[id.index()]
    }

    /// Function `id`. Panics if out of range.
    pub fn function(&self, id: FunctionId) -> &Function {
        &self.functions[id.index()]
    }

    /// Scope `id`. Panics if out of range.
    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id.index()]
    }
}

/// A unit that passed [`crate::verify()`]. The VM accepts only this type; ADR H02 lists what it
/// may assume.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VerifiedUnit(pub(crate) CompiledUnit);

impl VerifiedUnit {
    /// The unit.
    pub fn unit(&self) -> &CompiledUnit {
        &self.0
    }

    /// The unit, for changes. The result is not verified any more.
    pub fn into_inner(self) -> CompiledUnit {
        self.0
    }
}

impl core::ops::Deref for VerifiedUnit {
    type Target = CompiledUnit;

    fn deref(&self) -> &CompiledUnit {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_bits_canonical_nan() {
        let a = NumberBits::from_f64(f64::NAN);
        let b = NumberBits::from_f64(f64::from_bits(0x7ff0_0000_0000_0001));
        assert_eq!(a, b);
        assert_eq!(a.bits(), NumberBits::CANONICAL_NAN);
        assert_ne!(NumberBits::from_f64(0.0), NumberBits::from_f64(-0.0));
        assert_eq!(NumberBits::from_bits(0x7ff0_0000_0000_0001), None);
        assert!(NumberBits::from_bits(NumberBits::CANONICAL_NAN).is_some());
    }

    #[test]
    fn bigint_canonical() {
        let z = BigIntConst::new(true, &[0, 0]);
        assert!(!z.negative);
        assert!(z.limbs.is_empty());
        assert!(z.is_canonical());
        let v = BigIntConst::new(true, &[5, 0]);
        assert_eq!(&*v.limbs, &[5]);
        assert!(v.is_canonical());
        assert!(
            !BigIntConst {
                negative: false,
                limbs: vec![1, 0].into()
            }
            .is_canonical()
        );
    }

    #[test]
    fn enum_values_round_trip() {
        for v in 0..=255u8 {
            if let Some(k) = FunctionKind::from_u8(v) {
                assert_eq!(k as u8, v);
            }
            if let Some(k) = ScopeKind::from_u8(v) {
                assert_eq!(k as u8, v);
            }
            if let Some(k) = VariableMode::from_u8(v) {
                assert_eq!(k as u8, v);
            }
            if let Some(k) = HandlerKind::from_u8(v) {
                assert_eq!(k as u8, v);
            }
            if let Some(k) = Oddball::from_u8(v) {
                assert_eq!(k as u8, v);
            }
            if let Some(k) = CodeKind::from_u8(v) {
                assert_eq!(k as u8, v);
            }
        }
        assert!(FunctionKind::Script.is_entry());
        assert!(!FunctionKind::Arrow.is_entry());
        assert_eq!(CodeKind::Eval.entry_function_kind(), FunctionKind::Eval);
    }
}
