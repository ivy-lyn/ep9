//! Interpreter state and the runner for each expansion.

use crate::ops::{BUDGET, HEADER};
use crate::program::Verified;

/// A handler: `(cx, pc, fp, acc) -> result`. 4 register arguments (H15): Cx carries the
/// dispatch table at offset 0; pc, frame base and accumulator stay in registers.
#[cfg(not(feature = "neg-five-args"))]
pub type Handler = unsafe fn(&mut Cx, *const u8, *mut i64, i64) -> i64;

/// Negative test: a 5th argument. The DSL static check rejects this type.
#[cfg(feature = "neg-five-args")]
pub type Handler = unsafe fn(&mut Cx, *const u8, *mut i64, i64, u64) -> i64;

pub const ERR_ILLEGAL: u32 = 1;
pub const ERR_STACK_OVERFLOW: u32 = 2;

/// VM stack size in slots (8 bytes each).
pub const STACK_SLOTS: usize = 1 << 16;

/// Per-function data the Call handler reads.
#[repr(C)]
pub struct FuncEntry {
    pub code: *const u8,
    pub nregs: usize,
}

/// Interpreter context. `table` must stay at offset 0 (H15: fixed offset).
#[repr(C)]
pub struct Cx {
    pub table: [Handler; 256],
    pub code: *const u8,
    pub consts: *const i64,
    pub funcs: *const FuncEntry,
    /// One past the last VM stack slot.
    pub stack_limit: *const i64,
    /// The throw stub (a Halt in the entry function).
    pub throw_pc: *const u8,
    pub budget: i64,
    pub interrupts: u64,
    pub slow_paths: u64,
    /// Dispatch count; only the counting match loop updates it.
    pub dispatches: u64,
    pub error: u32,
}

const _: () = assert!(core::mem::offset_of!(Cx, table) == 0);

/// (a) match loop, plus a copy that counts dispatches. Debug builds use it on every target.
pub mod match_loop {
    use crate::dsl::{Flow, expand_match};
    crate::ops::for_each_op!(expand_match[run]);
    crate::ops::for_each_op!(expand_match[run_counting, count]);
}

/// (b) sibling calls: `return table[op](..)`.
pub mod sibling {
    use crate::dsl::{Flow, expand_threaded};
    crate::ops::for_each_op!(expand_threaded[return, "sib"]);
}

/// (c) nightly `become`.
#[cfg(feature = "become")]
pub mod tail {
    use crate::dsl::{Flow, expand_threaded};
    crate::ops::for_each_op!(expand_threaded[become, "become"]);
}

/// The expansion to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Match,
    MatchCounting,
    Sibling,
    #[cfg(feature = "become")]
    Become,
}

impl Variant {
    /// The release expansion: sibling calls on every target (H15 ADR,
    /// docs/adr/H15-interpreter-dispatch.md, T0 result of 2026-10-21). From the spike's first
    /// result until then it was the match loop on x86-64, because LLVM threads this spike's
    /// 20-opcode loop there. The real T0 loop is not threaded, and sibling calls were faster on
    /// every sampled x86-64 model (docs/measurements/2026-10-21-t0-dispatch-x64.md).
    pub const RELEASE: Variant = Variant::Sibling;

    pub const ALL: &[Variant] = &[
        Variant::Match,
        Variant::Sibling,
        #[cfg(feature = "become")]
        Variant::Become,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Match => "match",
            Variant::MatchCounting => "match-counting",
            Variant::Sibling => "sibling",
            #[cfg(feature = "become")]
            Variant::Become => "become",
        }
    }

    pub fn parse(s: &str) -> Option<Variant> {
        match s {
            "match" => Some(Variant::Match),
            "match-counting" => Some(Variant::MatchCounting),
            "sibling" => Some(Variant::Sibling),
            #[cfg(feature = "become")]
            "become" => Some(Variant::Become),
            _ => None,
        }
    }
}

/// What a run gives back. All variants and the C baseline must agree on every field
/// except `dispatches`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub result: i64,
    pub interrupts: u64,
    pub slow_paths: u64,
    pub error: u32,
    pub dispatches: u64,
}

/// A prepared run: the VM stack and the function table. `exec` runs the interpreter only,
/// so a timer around it measures dispatch and handlers, not setup.
pub struct Prepared<'p> {
    prog: &'p Verified,
    stack: Vec<i64>,
    funcs: Vec<FuncEntry>,
}

impl<'p> Prepared<'p> {
    pub fn new(prog: &'p Verified) -> Self {
        let p = prog.program();
        let base = p.code.as_ptr();
        let funcs = p
            .funcs
            .iter()
            // In bounds: the verifier checked offset < code.len().
            .map(|f| FuncEntry {
                code: base.wrapping_add(f.offset as usize),
                nregs: f.nregs as usize,
            })
            .collect();
        Prepared {
            prog,
            stack: vec![0; STACK_SLOTS],
            funcs,
        }
    }

    pub fn exec(&mut self, variant: Variant) -> Outcome {
        self.exec_timed(variant).0
    }

    /// Like `exec`, and also returns the time of the interpreter run alone, in ns.
    pub fn exec_timed(&mut self, variant: Variant) -> (Outcome, u128) {
        let p = self.prog.program();
        let table = match variant {
            Variant::Match | Variant::MatchCounting => sibling::TABLE,
            Variant::Sibling => sibling::TABLE,
            #[cfg(feature = "become")]
            Variant::Become => tail::TABLE,
        };
        self.stack.fill(0);
        let stack = self.stack.as_mut_ptr();
        let code = p.code.as_ptr();
        let entry = &p.funcs[0];
        // SAFETY: STACK_SLOTS > HEADER + entry.nregs (verifier), so fp and the header slots
        // are inside the stack. code offsets were checked by the verifier.
        let (fp, pc, throw_pc, limit) = unsafe {
            let fp = stack.add(HEADER);
            *fp.sub(1) = entry.nregs as i64;
            (
                fp,
                code.add(entry.offset as usize),
                code.add(p.throw_off as usize),
                stack.add(STACK_SLOTS) as *const i64,
            )
        };
        let mut cx = Cx {
            table,
            code,
            consts: p.consts.as_ptr(),
            funcs: self.funcs.as_ptr(),
            stack_limit: limit,
            throw_pc,
            budget: BUDGET,
            interrupts: 0,
            slow_paths: 0,
            dispatches: 0,
            error: 0,
        };
        let t = std::time::Instant::now();
        // SAFETY: the program is verified, and pc, fp and cx describe it (see above).
        let result = unsafe {
            match variant {
                Variant::Match => match_loop::run(&mut cx, pc, fp, 0),
                Variant::MatchCounting => match_loop::run_counting(&mut cx, pc, fp, 0),
                Variant::Sibling => start(&mut cx, pc, fp),
                #[cfg(feature = "become")]
                Variant::Become => start(&mut cx, pc, fp),
            }
        };
        let ns = t.elapsed().as_nanos();
        let o = Outcome {
            result,
            interrupts: cx.interrupts,
            slow_paths: cx.slow_paths,
            error: cx.error,
            dispatches: cx.dispatches,
        };
        (o, ns)
    }
}

/// Calls the handler for the first opcode. Every later dispatch happens inside the handlers.
///
/// # Safety
/// As for the handlers: `cx`, `pc` and `fp` must come from `Prepared::exec`.
unsafe fn start(cx: &mut Cx, pc: *const u8, fp: *mut i64) -> i64 {
    // SAFETY: pc points at the entry opcode of verified code (caller contract).
    let h = cx.table[unsafe { *pc } as usize];
    // SAFETY: the table holds handlers of one expansion; the state is valid (caller contract).
    #[cfg(not(feature = "neg-five-args"))]
    let r = unsafe { h(cx, pc, fp, 0) };
    // SAFETY: as above.
    #[cfg(feature = "neg-five-args")]
    let r = unsafe { h(cx, pc, fp, 0, 0) };
    r
}

/// Runs a whole program once.
pub fn run(prog: &Verified, variant: Variant) -> Outcome {
    Prepared::new(prog).exec(variant)
}
