//! The T0 interpreter (H15): one DSL over `ep9_bytecode::for_each_opcode!`, three expansions.
//!
//! ```text
//! ep9_bytecode::for_each_opcode!  (names, bytes, operand kinds)          ops.rs: one body per opcode
//!        │                                                                   fast: Flow {Next, Jump, Cold}
//!        ├── expand_threaded![return, "sib"]   (b) sibling calls              cold: Done {Next, Jump}
//!        ├── expand_threaded![become, "become"] (c) nightly `become`
//!        ├── expand_match![]                    (a) loop { match }
//!        └── expand_scaled![]                   Wide / ExtraWide instructions (out of line)
//! ```
//!
//! | Rule (H15) | Where |
//! |---|---|
//! | Handler = `unsafe fn(&mut Cx, pc, fp, acc) -> u64`, ≤ 4 register-wide args; x86-64: `extern "sysv64-unwind"` on every OS (`extern "sysv64"` under `panic = "abort"`) | [`Handler`], `dsl::AtMostFourRegArgs`, `dsl::handler!` |
//! | `Cx` holds the 256-entry dispatch table at offset 0 | [`Cx`] const assert |
//! | Every runtime call is in a cold block; the fast handler reaches it by a tail jump | `ops.rs` `cold` bodies, `<op>_cold` handlers |
//! | Symbols `ep9h__<exp>__<tail\|exit>__<Op>`, match loop `ep9loop__match__run` | `dsl.rs` (read by `cargo xtask disasm-check`) |
//! | Debug builds use the match loop; release builds use sibling calls on every target | [`Dispatch::DEFAULT`] |
//!
//! Handlers return the accumulator bits to the host only through the VM-internal `Exit`
//! opcode (byte 0xFF, never in verified bytecode). 0 means "exception pending".

pub(crate) mod dsl;
pub(crate) mod ops;
pub(crate) mod slow;

use core::sync::atomic::AtomicU32;

use ep9_bytecode::Opcode;

use crate::frame::{HOLE, UNDEFINED, native_ptr, slot};
use crate::runtime::Runtime;
use crate::unit::FunctionInfo;

/// A handler: `(cx, pc, fp, acc) -> result` (H15). The result is the accumulator bits the run
/// returns (0 = exception pending). x86-64 handlers use the SysV ABI on every OS (see
/// `dsl::handler!`); other targets use the Rust ABI.
#[cfg(all(target_arch = "x86_64", panic = "unwind"))]
pub(crate) type Handler =
    unsafe extern "sysv64-unwind" fn(&mut Cx, *const u8, *mut u64, u64) -> u64;
/// A handler: `(cx, pc, fp, acc) -> result` (H15), x86-64 with `panic = "abort"`: plain
/// `sysv64`, because a call to an `-unwind` function from abort code needs an abort landing
/// pad, and that pad stops the sibling call (see `dsl::handler!`).
#[cfg(all(target_arch = "x86_64", not(panic = "unwind")))]
pub(crate) type Handler = unsafe extern "sysv64" fn(&mut Cx, *const u8, *mut u64, u64) -> u64;
/// A handler: `(cx, pc, fp, acc) -> result` (H15). The result is the accumulator bits the run
/// returns (0 = exception pending).
#[cfg(not(target_arch = "x86_64"))]
pub(crate) type Handler = unsafe fn(&mut Cx, *const u8, *mut u64, u64) -> u64;

/// VM-internal opcode: leave the dispatch and return the accumulator to Rust.
pub(crate) const OP_EXIT: u8 = 0xFF;
/// Spike B placement (b), feature `spike-native` only (P4-PLACEMENT-B): the VM stub byte whose
/// handler tail-enters the (b) code of the top frame's function (`native::ENTER_B_STUB`).
#[cfg(feature = "spike-native")]
pub(crate) const OP_ENTER_B: u8 = 0xFD;
/// VM-internal opcode: the `[[Construct]]` result check of a construct stub frame.
pub(crate) const OP_CONSTRUCT_RETURN: u8 = 0xFE;

/// The return pc of every frame entered from Rust.
pub(crate) static EXIT_STUB: [u8; 1] = [OP_EXIT];
/// The return pc of a constructor frame called from JS.
pub(crate) static CONSTRUCT_RETURN_STUB: [u8; 1] = [OP_CONSTRUCT_RETURN];

const _: () = {
    assert!(Opcode::from_u8(OP_EXIT).is_none());
    assert!(Opcode::from_u8(OP_CONSTRUCT_RETURN).is_none());
};

/// Interrupt budget: one unit per loop back edge and per function entry (04 §2).
pub(crate) const BUDGET: i64 = 10_000;

/// The dispatch expansion an isolate runs (H15). All of them are built; the choice is a
/// runtime value so tests can compare them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dispatch {
    /// (a) `loop { match }`.
    Match,
    /// (b) one handler per opcode, each ends in `return table[op](..)`.
    Sibling,
    /// (c) the same with `become` (nightly, feature `become`).
    #[cfg(feature = "become")]
    Become,
}

impl Dispatch {
    /// The release expansion (H15 ADR, T0 result of 2026-10-21): sibling calls on every target.
    /// On x86-64 the T0 match loop has one dispatch jump (LLVM does not copy it into the
    /// arms), and sibling was faster on every sampled CPU model
    /// (docs/measurements/2026-10-21-t0-dispatch-x64.md).
    pub const RELEASE: Dispatch = Dispatch::Sibling;

    /// Debug builds use the match loop on every target (debug sibling calls are real calls).
    pub const DEFAULT: Dispatch = if cfg!(debug_assertions) {
        Dispatch::Match
    } else {
        Dispatch::RELEASE
    };

    /// Every expansion this build has.
    pub const ALL: &'static [Dispatch] = &[
        Dispatch::Match,
        Dispatch::Sibling,
        #[cfg(feature = "become")]
        Dispatch::Become,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Dispatch::Match => "match",
            Dispatch::Sibling => "sibling",
            #[cfg(feature = "become")]
            Dispatch::Become => "become",
        }
    }

    pub(crate) fn table(self) -> &'static [Handler; 256] {
        match self {
            Dispatch::Match | Dispatch::Sibling => &sibling::TABLE,
            #[cfg(feature = "become")]
            Dispatch::Become => &tail::TABLE,
        }
    }
}

/// Interpreter context (H15): the dispatch table at offset 0, then the fields handlers read.
/// It holds no GC-visible state (see `runtime.rs`).
///
/// The field list is a native-code contract: `crate::layout` exports the offsets native code
/// reads (`CX_*`). M5-P0 (C-18) removed `proto_epoch` and made the three native fields
/// unconditional, so the layout is the same in every build; the list is frozen for month 5.
#[repr(C)]
pub(crate) struct Cx {
    pub table: [Handler; 256],
    pub rt: *const Runtime,
    /// Frames must end at or below this slot (VM stack cap).
    pub stack_limit: *mut u64,
    /// Interrupt budget; a poll runs when it reaches 0.
    pub budget: i64,
    /// The global object (a copy of a realm root; cells never move).
    pub global_this: u64,
    /// Results of runtime calls that switch frames: the next fp and pc.
    pub out_fp: *mut u64,
    pub out_pc: *const u8,
    pub dispatch: Dispatch,
    /// Calls of a function before T0 makes its feedback vector (H16; 0 = at once, `u32::MAX`
    /// = never). From `IsolateConfig::feedback_threshold`; see `feedback.rs`.
    pub feedback_threshold: u32,
    // ---- appended in M2-P0 (no field above moves; the poll fields are fixed for month 2) ----
    /// The isolate's interrupt word (`Runtime::terminate`, bits in [`interrupt`]), read at
    /// the interrupt poll (`slow::interrupt`). Other threads set bits (`TerminateHandle`).
    pub interrupt: *const AtomicU32,
    /// The lowest native stack address native code may reach (`Runtime::native_limit`,
    /// copied so a JIT prologue compares one `Cx` field). Unconditional since M5-P0 (C-18):
    /// before, it existed with feature `spike-native` only, so the `Cx` layout depended on
    /// the features.
    pub native_limit: usize,
    /// The runtime functions native code calls (`native::RUNTIME_TABLE` with feature
    /// `spike-native`; null in a build without it). Unconditional since M5-P0 (C-18).
    pub runtime_table: *const crate::layout::RuntimeTable,
    // ---- appended in M3-P0 (no field above moves; the list is frozen for month 3) ----
    /// The current realm: a REALM cell (a copy of the `Root::Realm` root, like
    /// `global_this`; OBJECT_LAYOUT 3). M3-P1 REALMS switches it on a cross-realm call.
    pub realm: u64,
    /// The isolate's unit-data directory (SP-UNIT, H35-Q9, the unit-data register); 0 =
    /// none. Unconditional since M5-P0 (C-18); only spike-native code sets it today.
    pub unit_data: usize,
}

/// Bits of the interrupt word (`Cx::interrupt`). Bit 0 keeps its meaning; new bits are
/// appended. `slow::interrupt` reads them at every poll.
pub(crate) mod interrupt {
    /// Terminate the running script (`TerminateHandle::terminate`); the poll clears it.
    pub const TERMINATE: u32 = 1;
    /// Run the queued embedder callbacks (`InterruptHandle::request`,
    /// `Runtime::interrupt_requests`); the poll clears it, then runs them (M2-P3).
    pub const REQUEST: u32 = 2;
}

const _: () = assert!(core::mem::offset_of!(Cx, table) == 0);

impl Cx {
    /// The runtime of this isolate. The `Runtime` box outlives the `Cx` box (Isolate drop
    /// order) and is only ever shared, so any lifetime the caller picks is fine while the
    /// isolate lives.
    #[inline(always)]
    pub fn rt<'r>(&self) -> &'r Runtime {
        // SAFETY: see above; `rt` was set from the live Box<Runtime> at isolate creation.
        unsafe { &*self.rt }
    }
}

/// Runs the dispatch from `pc` in frame `fp` until an `Exit`. Returns the accumulator bits,
/// or 0 if an exception is pending.
///
/// An entry at the first instruction counts as a call of the function for its feedback
/// vector (`feedback.rs`), as the `Call*` paths do for calls from JS.
///
/// # Safety
/// `fp` is the live top frame for the function whose verified code holds `pc`, and every
/// Value the caller needs after this call is in a frame, a root or a Rust local.
pub(crate) unsafe fn run(cx: &mut Cx, pc: *const u8, fp: *mut u64) -> u64 {
    // SAFETY: header slot 4 of the live frame `fp` holds its FunctionInfo native word.
    let fi = unsafe { &*native_ptr::<FunctionInfo>(*fp.offset(slot::FUNCTION_INFO)) };
    if fi.feedback.get().is_null() && pc == fi.code {
        slow::count_entry(cx, fp, pc);
    }
    // SAFETY: caller contract; each expansion takes this state.
    unsafe {
        match cx.dispatch {
            Dispatch::Match => match_loop::run(cx, pc, fp, UNDEFINED),
            _ => {
                let h = cx.table[*pc as usize];
                h(cx, pc, fp, UNDEFINED)
            }
        }
    }
}

/// The `[[Construct]]` result check of a construct stub frame `fp` (§10.2.2 steps 10-12):
/// an object result stays, anything else becomes the receiver the stub holds. Returns the
/// caller's (pc, fp, accumulator).
///
/// # Safety
/// `fp` is a live construct stub frame.
#[inline(always)]
pub(crate) unsafe fn construct_return(fp: *mut u64, acc: u64) -> (*const u8, *mut u64, u64) {
    // SAFETY: header and receiver slots of the live stub frame (caller contract).
    let (this, ret, caller) = unsafe {
        (
            *fp.offset(slot::RECEIVER),
            *fp.offset(slot::RETURN_PC),
            *fp.offset(slot::CALLER_FP),
        )
    };
    let acc = if crate::object::is_object(acc) {
        acc
    } else if this == HOLE {
        // v0: a derived constructor returned a non-object without its `this` check
        // (BytecodeGen emits that check); give undefined, never the hole.
        UNDEFINED
    } else {
        this
    };
    (native_ptr(ret), native_ptr(caller), acc)
}

/// What a fast body does next.
pub(crate) enum Flow {
    Next,
    Jump(*const u8),
    /// Continue in this opcode's cold block (its own handler in (b) and (c)).
    Cold,
    /// As `Next`, after a feedback record (`profile::Rec`; T0 profile, H16-Q2). In (b) the
    /// handler tail-calls `dsl::record_next`, which records and dispatches, so the record
    /// code is not in the handler: on x86-64 a handler that records inline saves registers on
    /// entry, also on its paths that record nothing (M4-P1 PROFILE-X64). (a), (c) and the
    /// prefixed step record inline.
    Record(crate::profile::Rec),
    /// As `Record`, for a comparison whose record needs the operand pair
    /// (`profile::Pair`); (b) tail-calls `dsl::record_pair_next`.
    RecordPair(crate::profile::Pair),
    /// As `Jump(t)` after `call_fast` pushed a callee frame (`fp` is the new frame), for a
    /// call in the sampled window: `profile::record_call_pushed` with the caller's Call slot.
    /// (b) tail-calls `dsl::record_call_next`.
    JumpRecordCall(*const u8, crate::interp::dsl::FSlot),
}

impl Flow {
    /// Runs the record of `Record` and `RecordPair` inline and gives `Next` for them; other
    /// values pass through. For the match loop (a), the debug expansion, and the prefixed
    /// steps (`step::<S>`): out of line in debug builds, so the ~150 arms that call it add no
    /// record locals to the one frame of the loop or of a step (their sizes bound the native
    /// stack a JS call chain or a host re-entry needs, `tests::interp`).
    #[cfg_attr(debug_assertions, inline(never))]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub(crate) fn recorded(self, st: &St<'_>) -> Flow {
        match self {
            Flow::Record(r) => {
                crate::profile::record(st, r);
                Flow::Next
            }
            Flow::RecordPair(p) => {
                crate::profile::record_pair(st, p);
                Flow::Next
            }
            Flow::JumpRecordCall(t, s) => {
                crate::profile::record_call_pushed(st, s);
                Flow::Jump(t)
            }
            f => f,
        }
    }
}

/// What a cold body (or a fast body without a cold block) does next.
pub(crate) enum Done {
    Next,
    Jump(*const u8),
}

/// Handler state as one struct for the bodies. LLVM keeps it in registers: bodies are
/// `inline(always)` and never pass `&mut St` to an out-of-line function (that would make it
/// escape and break the sibling call, H15).
pub(crate) struct St<'a> {
    pub cx: &'a mut Cx,
    /// Start of the current instruction (its prefix, if any).
    pub pc: *const u8,
    pub fp: *mut u64,
    pub acc: u64,
    /// Start of the next instruction.
    pub next: *const u8,
}

impl<'a> St<'a> {
    /// # Safety
    /// `fp` is the live top frame of the function whose verified code holds `pc`, and `next`
    /// is the start of the instruction after `pc`.
    #[inline(always)]
    pub unsafe fn new(
        cx: &'a mut Cx,
        pc: *const u8,
        fp: *mut u64,
        acc: u64,
        next: *const u8,
    ) -> St<'a> {
        St {
            cx,
            pc,
            fp,
            acc,
            next,
        }
    }

    /// Register `r`.
    #[inline(always)]
    pub fn reg(&self, r: dsl::R) -> u64 {
        // SAFETY: `R` comes only from the operand decoder of the current verified
        // instruction, so it names a slot of the live frame `fp` (St contract).
        unsafe { *self.fp.offset(r.0) }
    }

    #[inline(always)]
    pub fn set_reg(&mut self, r: dsl::R, v: u64) {
        // SAFETY: as for `reg`.
        unsafe { *self.fp.offset(r.0) = v };
    }

    /// Register `i` of a register list (`i` below the list's count).
    #[inline(always)]
    pub fn list(&self, l: dsl::RList, i: u32) -> u64 {
        // SAFETY: the verifier checked that the whole list lies in the frame; callers pass
        // `i` below the count operand of the same instruction.
        unsafe { *self.fp.offset(l.0 + i as isize) }
    }

    /// Header slot `s` of the current frame.
    #[inline(always)]
    pub fn hdr(&self, s: isize) -> u64 {
        // SAFETY: header slots lie below `fp` of the live frame.
        unsafe { *self.fp.offset(s) }
    }

    #[inline(always)]
    pub fn set_hdr(&mut self, s: isize, v: u64) {
        // SAFETY: as for `hdr`.
        unsafe { *self.fp.offset(s) = v };
    }

    /// The FunctionInfo of the current frame.
    #[inline(always)]
    pub fn info(&self) -> &'a FunctionInfo {
        // SAFETY: header slot 4 of a live frame holds a FunctionInfo native word.
        unsafe { &*native_ptr::<FunctionInfo>(self.hdr(slot::FUNCTION_INFO)) }
    }

    #[inline(always)]
    pub fn rt(&self) -> &'a Runtime {
        self.cx.rt()
    }

    #[inline(always)]
    pub fn constant(&self, k: dsl::K) -> u64 {
        self.info().constant(k.0)
    }

    #[inline(always)]
    pub fn fwd(&self, off: dsl::Fwd) -> *const u8 {
        self.pc.wrapping_add(off.0 as usize)
    }

    #[inline(always)]
    pub fn back(&self, off: dsl::Back) -> *const u8 {
        self.pc.wrapping_sub(off.0 as usize)
    }

    /// Unwinds the pending exception (out of line) and continues at the handler, or at the EXIT
    /// stub with accumulator 0 when no handler of this run catches it.
    #[inline(always)]
    pub fn unwind(&mut self) -> Done {
        self.acc = slow::unwind(self.cx, self.fp, self.pc);
        self.fp = self.cx.out_fp;
        Done::Jump(self.cx.out_pc)
    }

    /// The result `r` of a runtime call that stays in this frame: the new accumulator, or 0
    /// for "exception pending" (then unwind).
    #[inline(always)]
    pub fn result(&mut self, r: u64) -> Done {
        if r == 0 {
            return self.unwind();
        }
        self.acc = r;
        Done::Next
    }

    /// Adopts the state of a runtime call that switched frames (`slow::call`): accumulator
    /// `acc`, frame `cx.out_fp`, pc `cx.out_pc`. Such calls set both even when they unwind.
    #[inline(always)]
    pub fn switch(&mut self, acc: u64) -> Done {
        self.acc = acc;
        self.fp = self.cx.out_fp;
        Done::Jump(self.cx.out_pc)
    }

    /// Runs one `Wide`/`ExtraWide` instruction out of line.
    #[inline(always)]
    pub fn scaled<S: dsl::Scale>(&mut self) -> Done {
        // SAFETY: `pc` points at a prefix of verified code in the live frame `fp`.
        let acc = unsafe { scaled::step::<S>(self.cx, self.pc, self.fp, self.acc) };
        self.acc = acc;
        self.fp = self.cx.out_fp;
        Done::Jump(self.cx.out_pc)
    }
}

/// (a) The match loop.
pub(crate) mod match_loop {
    use super::*;
    use crate::interp::dsl::expand_match;
    ep9_bytecode::for_each_opcode!(expand_match[]);
}

/// (b) Sibling calls.
pub(crate) mod sibling {
    use super::*;
    use crate::interp::dsl::expand_threaded;
    ep9_bytecode::for_each_opcode!(expand_threaded[return, "sib"]);
}

/// (c) `become` (nightly).
#[cfg(feature = "become")]
pub(crate) mod tail {
    use super::*;
    use crate::interp::dsl::expand_threaded;
    ep9_bytecode::for_each_opcode!(expand_threaded[become, "become"]);
}

/// Prefixed (`Wide`, `ExtraWide`) instructions: one out-of-line step per scale, shared by all
/// expansions. Prefixes are rare (more than 256 registers or constants, large immediates).
pub(crate) mod scaled {
    use super::*;
    use crate::interp::dsl::expand_scaled;
    ep9_bytecode::for_each_opcode!(expand_scaled[]);
}
