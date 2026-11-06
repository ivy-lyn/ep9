//! The SIZE-REPORT model (doc 13 C-17, the K-A3 report; MODES-CHECK MC-w28-8.1): the bytes
//! a unit holds under each of 3 coverage policies, with the deopt metadata bytes beside the
//! text bytes.
//!
//! ```text
//!  policy   which functions get a native variant
//!  all      every function of the unit (all reachable)
//!  static   a static heuristic: a function with a loop, or one of 64 or more instructions
//!  pgo      the PGO set: the functions with a fact record in a training run's FactSnapshot
//!
//!  bytes    function table (exact, C-17 form) + text (est.) + deopt metadata (est., H29
//!           format 1) + the bytecode section (exact; every function keeps its bytecode)
//! ```
//!
//! Until a unit compiler exists (A2-LOADER and T2), text and metadata are estimates from Spike
//! C: am3 (crypto) is 82 bytecode instructions -> 2816 B of AArch64 code and 1448 B of format-1
//! metadata in 37 records (H29-Q1 fill (4) of 2026-11-02; spike-c `am3:` report line). A
//! function that is not covered runs its bytecode and costs no table entry (GraalVM Native
//! Image keeps cold methods as bytecode; MC-w15-20).

use crate::functions::{FT_ENTRY_SIZE, FT_HEADER_SIZE, FT_VARIANT_SIZE};

/// The Spike C sample the estimates scale from: bytecode instructions, code bytes, metadata
/// bytes (am3, mac a64, release, live facts).
pub const SAMPLE: (u64, u64, u64) = (82, 2816, 1448);

/// One function of a unit, as the report sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionStats {
    /// The CompiledUnit function index.
    pub index: u32,
    /// Bytecode instructions.
    pub instructions: u32,
    /// Bytecode bytes.
    pub bytecode_bytes: u32,
    /// It has a `JumpLoop` (a loop).
    pub has_loop: bool,
    /// It has a fact record in the training FactSnapshot.
    pub in_profile: bool,
}

/// A coverage policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Every function.
    All,
    /// A loop, or 64 or more instructions.
    Static,
    /// The functions with a fact record.
    Pgo,
}

impl Policy {
    /// The 3 policies, in report order.
    pub const ALL: [Policy; 3] = [Policy::All, Policy::Static, Policy::Pgo];

    /// The name in the report.
    pub const fn name(self) -> &'static str {
        match self {
            Policy::All => "all",
            Policy::Static => "static",
            Policy::Pgo => "pgo",
        }
    }

    /// Whether `f` gets a native variant under this policy.
    pub fn covers(self, f: &FunctionStats) -> bool {
        match self {
            Policy::All => true,
            Policy::Static => f.has_loop || f.instructions >= 64,
            Policy::Pgo => f.in_profile,
        }
    }
}

/// The bytes of one unit under one policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicySize {
    /// The policy.
    pub policy: Policy,
    /// The functions with a native variant.
    pub functions: u32,
    /// Their bytecode instructions.
    pub instructions: u64,
    /// The function table (exact: one entry and one variant per covered function).
    pub table_bytes: u64,
    /// Text (est.).
    pub text_bytes: u64,
    /// Deopt metadata (est.).
    pub meta_bytes: u64,
}

impl PolicySize {
    /// Native bytes: table, text and metadata.
    pub fn native_bytes(&self) -> u64 {
        self.table_bytes + self.text_bytes + self.meta_bytes
    }
}

/// The size of a unit of `fns` under `policy`.
pub fn size(fns: &[FunctionStats], policy: Policy) -> PolicySize {
    let covered: Vec<&FunctionStats> = fns.iter().filter(|f| policy.covers(f)).collect();
    let n = covered.len() as u64;
    let insns: u64 = covered.iter().map(|f| u64::from(f.instructions)).sum();
    let (si, st, sm) = SAMPLE;
    PolicySize {
        policy,
        functions: n as u32,
        instructions: insns,
        table_bytes: if n == 0 {
            0
        } else {
            (FT_HEADER_SIZE as u64) + n * (FT_ENTRY_SIZE + FT_VARIANT_SIZE) as u64
        },
        text_bytes: insns * st / si,
        meta_bytes: insns * sm / si,
    }
}
