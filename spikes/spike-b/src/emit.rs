//! The emitter interface of Spike B: [`Emitter`] turns one function's bytecode into
//! position-independent machine code plus its unwind data ([`FunctionCode`]); [`Finalize`]
//! places a set of them: in the process code range for the JIT (placement (a),
//! `crate::jit::JitModule`), or in a unit's text (SP-Unit, P4).
//!
//! **Frozen at the P2 merge** (plan-v4 P2-SPIKE-B task 3, item \[I10\]): P3-SPIKEB-BENCH and
//! P4-SP-UNIT build on these types and do not change them. A change needs a decisions-log row.
//!
//! ```text
//! NativeTarget (ep9_vm)  ──> FunctionInput ──Emitter::emit──> FunctionCode (bytes, CFI, Win64)
//!                                                   │
//!            Vec<FunctionCode> ──Finalize::finalize─┴─> in memory: JitModule (code range,
//!                                                       unwind registration, install)
//!                                                   └─> unit text (SP-Unit)
//! ```
//!
//! | Rule | Why |
//! |---|---|
//! | `FunctionCode::code` has no absolute address and no relocation: branches are pc-relative inside the function, runtime functions are reached through `Cx::runtime_table` (the isolate register) | the same bytes run at any address: a JIT chunk or a unit mapped once per process (SP-Unit "text with 0 relocations") |
//! | The entry is offset 0 and has the `ep9_vm::NativeEntry` ABI | `install_native` writes the start address into the dispatch slot |
//! | Unwind data comes with the code, in both forms the targets need: DWARF CFI for one FDE ([`Unwind::cfi`], after the CIE of [`Arch::cie`]) and, for x86-64, a Win64 UNWIND_INFO ([`Unwind::win64`]) | a finalizer registers the form of its OS before the code runs (H28) |
//! | Registers: isolate x28 / r15, VM frame pointer x26 / r13, unit-data x27 / r14 reserved (not used by placement (a)) | M2-JIT-ABI; H35-Q9 |

use std::fmt;

use ep9_bytecode::Opcode;
use ep9_platform::unwind::cfi::{CfiProgram, Cie};

/// A target instruction set of the emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arch {
    /// AArch64, AAPCS64 (`extern "C"`).
    Aarch64,
    /// x86-64, System V (`extern "sysv64"` on every OS, Windows too).
    X86_64,
}

impl Arch {
    /// The architecture of this build, if the emitter has one for it.
    pub const HOST: Option<Arch> = if cfg!(target_arch = "aarch64") {
        Some(Arch::Aarch64)
    } else if cfg!(target_arch = "x86_64") {
        Some(Arch::X86_64)
    } else {
        None
    };

    /// The CIE that [`Unwind::cfi`] programs follow.
    pub fn cie(self) -> Cie {
        match self {
            Arch::Aarch64 => Cie::aarch64(),
            Arch::X86_64 => Cie::x86_64(),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Arch::Aarch64 => "aarch64",
            Arch::X86_64 => "x86-64",
        }
    }
}

/// What an emitter compiles: one function's verified bytecode and frame facts (from
/// `ep9_vm::native_target`).
#[derive(Clone, Copy, Debug)]
pub struct FunctionInput<'a> {
    /// The function's name, for reports (may be empty).
    pub name: &'a str,
    /// Its verified bytecode. Runtime calls pass offsets into it.
    pub bytecode: &'a [u8],
    pub parameter_count: u32,
    pub register_count: u32,
    /// Entries of its handler table.
    pub handler_count: u32,
    /// An opaque key the caller uses to match the result to its function (the JIT uses the
    /// FunctionInfo address, `NativeTarget::info`).
    pub key: usize,
}

/// Why an emitter did not compile a function. The function then stays in T0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A reachable instruction the emitter has no code for.
    Opcode { offset: u32, opcode: Opcode },
    /// A reachable `Wide` or `ExtraWide` prefix (T0 runs prefixed operands in its cold path).
    Prefix { offset: u32, opcode: Opcode },
    /// The function has a handler table.
    Handlers(u32),
    /// The bytecode did not decode, or a jump leaves it (verified bytecode never does this).
    Decode { offset: u32, detail: String },
    /// An operand or offset the emitter cannot encode.
    Limit(String),
    /// The emitter has no code generator for this architecture.
    Arch,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Opcode { offset, opcode } => {
                write!(
                    f,
                    "opcode {} at offset {offset} has no native code",
                    opcode.info().name
                )
            }
            Refusal::Prefix { offset, opcode } => {
                write!(f, "prefixed {} at offset {offset}", opcode.info().name)
            }
            Refusal::Handlers(n) => write!(f, "{n} handler table entries"),
            Refusal::Decode { offset, detail } => write!(f, "bad bytecode at {offset}: {detail}"),
            Refusal::Limit(m) => write!(f, "limit: {m}"),
            Refusal::Arch => f.write_str("no code generator for this architecture"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Unwind data of one function, in the forms the targets need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unwind {
    /// DWARF call frame instructions of one FDE that covers the whole function, after the
    /// initial rules of [`Arch::cie`]. Unix finalizers put it in an `.eh_frame` section.
    pub cfi: CfiProgram,
    /// The Win64 UNWIND_INFO bytes (x86-64 only; `None` for AArch64). Windows finalizers put
    /// them next to the code (4-byte aligned) and register a RUNTIME_FUNCTION.
    pub win64: Option<Vec<u8>>,
}

/// One compiled function: position-independent code, entry at offset 0.
#[derive(Clone, Debug)]
pub struct FunctionCode {
    pub name: String,
    pub arch: Arch,
    /// [`FunctionInput::key`].
    pub key: usize,
    /// Machine code; no absolute address, no relocation.
    pub code: Vec<u8>,
    pub unwind: Unwind,
    /// The reachable bytecode instructions that got code, in offset order (`(offset,
    /// opcode)`). Unreachable bytecode gets none.
    pub ops: Vec<(u32, Opcode)>,
    /// Native offset of the first machine instruction of each entry of `ops` (same order).
    pub native_offsets: Vec<u32>,
}

impl FunctionCode {
    /// The opcode names of `ops`, space separated (gate.sh prints it).
    pub fn ops_line(&self) -> String {
        let names: Vec<&str> = self.ops.iter().map(|(_, o)| o.info().name).collect();
        names.join(" ")
    }
}

/// Compiles functions for one architecture.
pub trait Emitter {
    /// The architecture of the code this emitter makes.
    fn arch(&self) -> Arch;

    /// Compiles one function, or refuses it (it then stays in T0).
    fn emit(&mut self, input: &FunctionInput<'_>) -> Result<FunctionCode, Refusal>;
}

/// Places compiled functions where they run, with their unwind data registered before any of
/// them can run.
pub trait Finalize {
    /// The placed code (for the JIT: the live code memory and its registrations).
    type Output;
    type Error;

    /// Places `functions` (all of one [`Arch`]).
    fn finalize(self, functions: Vec<FunctionCode>) -> Result<Self::Output, Self::Error>;
}
