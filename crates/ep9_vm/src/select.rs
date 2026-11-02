//! `select()` (owner M6-P0 MODES6; doc 13 §3.2 and C-15, MODES-D13, MODES-D14): the one rule
//! that picks the code object a function's dispatch slot caches, from its code set, the
//! execution policy and the session. A pure function with no caller yet, tested with fake code
//! objects and no codegen (`tests/select.rs`: the T1-T11 table, the append-only lists, JIT and
//! hybrid tier-up, PIN). The first caller comes with T1 (MODES-D6).
//!
//! ```text
//!  select(code set, policy, session) -> Pick       (slot := select(..) after every event)
//!
//!  if session.connected and set.breakpoint:          Interp(debug copy)            H24-Q3
//!  for kind in policy.install (rank order):
//!    for lowering in policy.accept (rank order):     one filter for every object   MODES-D13
//!      for obj in set.objects (in order):
//!        obj.kind == kind, obj.lowering == lowering, obj.state == Active           M-R5
//!        not (obj.kind == AotOpt and set.pinned)                                    MODES-D14
//!        every dep known and intact                                                 §3.5, H31
//!        obj.cpu bits all in session.cpu
//!          -> Code(index of obj)
//!  Interp(debug copy if session.connected and set.debug_copy, else normal)
//! ```
//!
//! | Append-only list (doc 13 §5.4) | A value this build does not know |
//! |---|---|
//! | dispatch kind bytes (H14, C-03; [`DispatchKind::from_u8`]) | no install rank names it: never picked |
//! | lowering classes (H32, C-07; [`lowering`]) | no accept list names it: never picked |
//! | static fuses (H31, `fuse::STATIC`) and dynamic descriptor kinds (H31, `fuse::descriptor`) | the dependency is not intact: the object is skipped |
//!
//! The candidate kinds GenInterp (5) and Lazy (6) of doc 13 §1.3 join the install ranks when
//! they are built (C-03); until then the jitless rank goes AotOpt > AotBaseline > Interp.
//! Counter sessions (doc 13 §3.2, MC-w15-17) are not part of this rule yet.
//!
//! This is the selector, so the rule 40 policy grep exempts it (plan v11 rule 40).

// No caller until T1 (MODES-D6); the tests use every item.
#![cfg_attr(not(test), allow(dead_code))]

use crate::dispatch::DispatchKind;
use crate::exec::Preset;
use crate::fuse;

/// The lowering classes of a code object (doc 13 §1.4; H32, C-07). Append-only: a new class
/// is a fill, and its fill row names every preset accept list that takes it.
pub(crate) mod lowering {
    /// In-memory JIT code (T1, T2, the hybrid re-tier). Never stored in a unit.
    pub(crate) const JIT: u8 = 0;
    /// Hybrid unit code (DEOPT, then re-tier).
    pub(crate) const HYBRID: u8 = 1;
    /// Jitless unit code (SLOWPATH, budget, PIN).
    pub(crate) const JITLESS: u8 = 2;

    /// True for a class this build knows. The others are refused until appended.
    pub(crate) const fn known(l: u8) -> bool {
        l <= JITLESS
    }
}

/// The state of a code object (doc 13 §3.1). PIN is a function state, not an object state
/// (MODES-D14). Not named `State`: `cargo xtask gc-lint` reads a type name without its module,
/// and the `State` of `ep9_builtins` `json/stringify.rs` holds a `Local`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectState {
    Active,
    Invalidated,
}

/// One InvalidatingFuse dependency of a code object, with the per-isolate fuse state the
/// caller read (doc 13 §3.5). A GuardFuse read is not a dependency: its code stays Active and
/// takes the slow path after a pop (T6, jitless).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dep {
    /// A builtin fuse by stable index (`fuse::STATIC`).
    Static { index: u32, intact: bool },
    /// A dynamic fuse by descriptor kind (`fuse::descriptor`).
    Dynamic { kind: u8, intact: bool },
}

impl Dep {
    /// True if this build knows the fuse and it is intact. An unknown index or descriptor kind
    /// is refused by the registry, so the variant that names it is skipped (H31 Freeze table).
    pub(crate) fn holds(self) -> bool {
        match self {
            Dep::Static { index, intact } => (index as usize) < fuse::STATIC.len() && intact,
            Dep::Dynamic { kind, intact } => fuse::descriptor::known(kind) && intact,
        }
    }
}

/// A fake or real code object as `select()` sees it (doc 13 §3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeObject {
    /// The dispatch kind byte (H14). A byte [`DispatchKind::from_u8`] does not know is never
    /// picked.
    pub(crate) kind: u8,
    /// The lowering class ([`lowering`]).
    pub(crate) lowering: u8,
    pub(crate) state: ObjectState,
    /// The CPU feature bits the code needs; each must be in [`Session::cpu`].
    pub(crate) cpu: u64,
    /// Its InvalidatingFuse dependencies.
    pub(crate) deps: Vec<Dep>,
}

impl CodeObject {
    /// An Active object of `kind` and `lowering` with no CPU need and no dependency.
    pub(crate) fn new(kind: DispatchKind, lowering: u8) -> CodeObject {
        CodeObject {
            kind: kind as u8,
            lowering,
            state: ObjectState::Active,
            cpu: 0,
            deps: Vec::new(),
        }
    }
}

/// A function's code set (doc 13 §3.1): its compiled objects, its Interp variants and the PIN
/// state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CodeSet {
    pub(crate) objects: Vec<CodeObject>,
    /// The function has an Interp debug copy (H24-Q3).
    pub(crate) debug_copy: bool,
    /// The function has a breakpoint (T11).
    pub(crate) breakpoint: bool,
    /// PIN (MODES-D14): no AotOpt variant is picked, also ones installed later.
    pub(crate) pinned: bool,
}

/// The parts of `ExecPolicy` that `select()` reads (doc 13 §1.2): the install rank and the
/// accept list, both in rank order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    pub(crate) install: &'static [DispatchKind],
    pub(crate) accept: &'static [u8],
}

impl Policy {
    /// JIT: Opt > Baseline > Interp; accepts {`jit`}.
    pub(crate) const JIT: Policy = Policy {
        install: &[
            DispatchKind::Opt,
            DispatchKind::Baseline,
            DispatchKind::Interp,
        ],
        accept: &[lowering::JIT],
    };
    /// jitless: AotOpt > AotBaseline > Interp (GenInterp joins when built); accepts
    /// {`jitless`}.
    pub(crate) const JITLESS: Policy = Policy {
        install: &[
            DispatchKind::AotOpt,
            DispatchKind::AotBaseline,
            DispatchKind::Interp,
        ],
        accept: &[lowering::JITLESS],
    };
    /// hybrid: Opt > Baseline > AotOpt > AotBaseline > Interp; accepts {`jit`, `hybrid`,
    /// `jitless`} (MODES-D13).
    pub(crate) const HYBRID: Policy = Policy {
        install: &[
            DispatchKind::Opt,
            DispatchKind::Baseline,
            DispatchKind::AotOpt,
            DispatchKind::AotBaseline,
            DispatchKind::Interp,
        ],
        accept: &[lowering::JIT, lowering::HYBRID, lowering::JITLESS],
    };

    /// The policy of a preset this build has (`exec::Preset`). The hybrid preset is not built
    /// yet; a preset this build does not know has none.
    pub(crate) fn of(p: Preset) -> Option<Policy> {
        if p == Preset::JIT {
            Some(Policy::JIT)
        } else if p == Preset::JITLESS {
            Some(Policy::JITLESS)
        } else {
            None
        }
    }
}

/// The session state `select()` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Session {
    /// A debugger session is connected (T10, T11).
    pub(crate) connected: bool,
    /// The CPU feature bits of this host.
    pub(crate) cpu: u64,
}

/// What the slot runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// The object at this index of [`CodeSet::objects`].
    Code(usize),
    /// The normal Interp variant (bytecode).
    Interp,
    /// The Interp debug copy (H24-Q3).
    InterpDebug,
}

/// The rule of doc 13 §3.2 (see the module doc). Pure: it reads its arguments only.
pub(crate) fn select(set: &CodeSet, policy: &Policy, session: &Session) -> Pick {
    if session.connected && set.breakpoint {
        return Pick::InterpDebug;
    }
    for &kind in policy.install {
        for &low in policy.accept {
            let found = set.objects.iter().position(|o| {
                o.kind == kind as u8
                    && o.lowering == low
                    && o.state == ObjectState::Active
                    && !(kind == DispatchKind::AotOpt && set.pinned)
                    && o.deps.iter().all(|d| d.holds())
                    && o.cpu & !session.cpu == 0
            });
            if let Some(i) = found {
                return Pick::Code(i);
            }
        }
    }
    if session.connected && set.debug_copy {
        Pick::InterpDebug
    } else {
        Pick::Interp
    }
}
