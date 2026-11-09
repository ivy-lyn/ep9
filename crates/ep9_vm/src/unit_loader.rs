//! The A2 unit loader (plan v12 §7.3, M7-P4; doc 13 §3.3 event T1 "unit load", C-17; H35 S4
//! steps 5-8 for the native part of one loaded unit). It is the first caller of
//! [`select()`](crate::select::select) (doc 13 C-15; MODES-D6): `select()` stays pure, and the
//! loader gives it the code set, the policy and the session.
//!
//! ```text
//!  load(rt, loaded unit, text, functions, policy, host cpu)          (a bytecode unit: LoadedUnit)
//!   5  text --unit_text::UnitText::map--> RX in the process code range (H07:52), one per unit
//!   per function entry (the H35 function table, C-17):
//!      the function index names a function of the unit ------ no ------> skip "function"
//!      no unit code yet (FunctionInfo::code_set empty) ------ no ------> skip "twice"
//!      entry digest == unit_function_digest (MC-w31-8) ------ no ------> skip "digest"
//!      per variant: kind AotBaseline | AotOpt --------------- no ------> skip "kind"
//!                   entry..entry+len inside the text -------- no ------> skip "range"
//!                   lowering != jit (MC-p0-cx-1) ------------ no ------> skip "jit-lowering"
//!      6-7 fuse refs -> Dep::Static { index, intact now }  (a popped fuse: never picked)
//!      the code set -> FunctionInfo::code_set (once; no Value inside: rule 8)
//!      select(code set, the isolate's policy, session) -> Pick ---------> "picked" | "bytecode"
//!   8  install the picks: not in M7-P4 (INSTALL = false) -----------> "deferred"
//! ```
//!
//! | Rule | Where |
//! |---|---|
//! | `select()` stays pure: the loader builds its inputs and stores its result | [`load`] |
//! | A unit variant with lowering `jit` is skipped under every preset, before `select()`: `jit` is never stored in a unit, so `jit` in the hybrid accept list admits only runtime code (doc 13 §1.4; MODES-D13 keeps one filter in `select()`, so the skip is the loader's; route MC-p0-cx-1) | [`load`], [`Reason::JitLowering`] |
//! | The loader keeps no counter session and `select()` reads none; the session model is T1 tier-manager work at M5 (V12-D31; route MC-p0-cx-2) | [`load`]: `Session { connected: false, cpu }` |
//! | The policy is the isolate's install rank and accept list ([`isolate_policy`]): the private `ExecOptions` field that the preset constructors set (`exec.rs`; route MC-w24-3, M7-P4 integration fixer), so the loader keeps no preset table of its own | [`isolate_policy`] |
//! | Unit load keeps one intern table: every unit string is an atom through `atoms::intern_*` in `LoadedUnit::load` (H19 "one atom per text"); the loader adds no string table (route MC-w25-9) | [`load`] reads no string |
//! | Native code binds to its bytecode by digest: a function whose bytecode differs (another BytecodeGen switch, a changed source) runs its bytecode. So every switch that changes bytecode or emitted code is in the header `codegen_flags` hash, or unit functions keep the default bytecode; REGOPT has no off switch outside tests (route MC-w31-8) | [`unit_function_digest`], [`Reason::Digest`] |
//! | No native code on the JS call path until MC-w30-2 (A2-RW, M7-P5): T0 sends a non-Interp slot cold (`interp/ops.rs` `call_fast`) and `slow::call` answers not_implemented for it, so a JS call of an installed function would throw. Step 8 runs only from the in-crate tests ([`install_picked`]) | [`INSTALL`] |
//! | A run with no unit pays nothing: no T0 path reads the code set; a function info gets one empty `OnceCell` word (routes MC-w22-O5, MC-w29-7) | `FunctionInfo::code_set` |
//! | The object model reads no execution policy, also with a unit loaded (doc 13 M-R9; route A2-MR9): the loader writes only `code_set` and (step 8) the dispatch slot and the code registry. This file reads the policy, as `select.rs` does: the rule-40 grep exempts both (plan v12 rule 40) | the tests |
//!
//! Unwind data of unit text: the H35 `unwind` section has no format yet, so this loader
//! registers none (`UnitCode::_text` keeps only the pages). A test registers the records of
//! its stub with `ep9_platform::unwind` in a build without `jit` (`tests/unit_loader.rs`;
//! route MC-w33-2.2).

use core::hash::{Hash, Hasher};
use core::ops::Range;
use std::rc::Rc;

use ep9_bytecode::VerifiedUnit;
use ep9_bytecode::sha256::Sha256;
use ep9_platform::CodeRange;
use ep9_platform::unit_text::UnitText;

use crate::dispatch::DispatchKind;
use crate::fuse::{self, FuseRef};
use crate::native::install_code;
use crate::runtime::Runtime;
use crate::select::{CodeObject, CodeSet, Dep, ObjectState, Pick, Policy, Session, lowering};
use crate::unit::LoadedUnit;

/// Step 8 (install the picks into the dispatch slots) is off until the T0 call and construct
/// paths can enter unit code (route MC-w30-2, A2-RW, M7-P5). The tests run it by
/// [`install_picked`].
const INSTALL: bool = false;

/// One native variant of a unit function, as the H35 function table lists it (C-17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VariantCode {
    /// The dispatch kind byte (3 AotBaseline, 4 AotOpt).
    pub kind: u8,
    /// The lowering class byte (`select::lowering`).
    pub lowering: u8,
    /// The offset of the code in the text.
    pub entry: u32,
    /// The length of the code.
    pub len: u32,
    /// The CPU bits that the code needs (H35-Q5).
    pub cpu: u64,
    /// The static fuse ids that the code depends on (H31, `fuse::STATIC`).
    pub fuses: Vec<u32>,
}

/// One function of the table: its index in the unit, the digest of the bytecode its code was
/// made from, its variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FunctionCode {
    pub function: u32,
    pub digest: [u8; 16],
    pub variants: Vec<VariantCode>,
}

/// What the loader keeps for a function with unit code (`FunctionInfo::code_set`). Rust memory
/// only: no Value, so no GC root (rule 8).
pub(crate) struct UnitCode {
    /// The code set that `select()` read (doc 13 §3.1).
    pub set: CodeSet,
    /// The code of each object of `set`, in the unit text.
    pub code: Box<[Range<usize>]>,
    /// What `select()` picked at load.
    pub pick: Pick,
    /// Keeps the unit text mapped while a function of the unit holds it (until the isolate
    /// drops, `Runtime::units` only grows).
    pub _text: Rc<UnitText<'static>>,
}

/// Why a function or a variant gets no unit code, and the other load counts (H35 S5: every
/// outcome other than a full load is counted by reason). Append-only: the report lists them in
/// this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// The text could not be mapped: every listed function runs its bytecode.
    NoText,
    /// The entry names no function of the unit.
    Function,
    /// The function has unit code already (a second entry, or a second load).
    Twice,
    /// The function's bytecode is not the bytecode the code was made from (MC-w31-8).
    Digest,
    /// A variant kind that is not AotBaseline or AotOpt.
    Kind,
    /// A variant code range that is empty or not inside the text.
    Range,
    /// A variant with lowering `jit` (MC-p0-cx-1).
    JitLowering,
    /// `select()` picked the bytecode: no variant the isolate accepts.
    Bytecode,
    /// `select()` picked a unit code object.
    Picked,
    /// A pick that step 8 does not install yet (MC-w30-2).
    Deferred,
    /// A pick that step 8 installed.
    Installed,
    /// A pick that step 8 cannot install: AotOpt (no optimizing-tier entry yet), a fuse
    /// dependency (no fuse-pop path re-selects yet), or an `install_code` refusal.
    NotInstallable,
}

impl Reason {
    /// Every reason, in report order.
    pub(crate) const ALL: [Reason; 12] = [
        Reason::NoText,
        Reason::Function,
        Reason::Twice,
        Reason::Digest,
        Reason::Kind,
        Reason::Range,
        Reason::JitLowering,
        Reason::Bytecode,
        Reason::Picked,
        Reason::Deferred,
        Reason::Installed,
        Reason::NotInstallable,
    ];

    /// The report name.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Reason::NoText => "no-text",
            Reason::Function => "function",
            Reason::Twice => "twice",
            Reason::Digest => "digest",
            Reason::Kind => "kind",
            Reason::Range => "range",
            Reason::JitLowering => "jit-lowering",
            Reason::Bytecode => "bytecode",
            Reason::Picked => "picked",
            Reason::Deferred => "deferred",
            Reason::Installed => "installed",
            Reason::NotInstallable => "not-installable",
        }
    }
}

/// The counts of one load, per [`Reason`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    counts: [u32; Reason::ALL.len()],
}

impl Report {
    fn add(&mut self, r: Reason, n: u32) {
        self.counts[r as usize] += n;
    }

    /// The count of `r`.
    pub(crate) fn count(&self, r: Reason) -> u32 {
        self.counts[r as usize]
    }

    /// (name, count) for every reason, in [`Reason::ALL`] order.
    pub(crate) fn rows(&self) -> Vec<(&'static str, u32)> {
        Reason::ALL
            .iter()
            .map(|&r| (r.name(), self.count(r)))
            .collect()
    }
}

/// Feeds a derived `Hash` stream into SHA-256: the stream of one build is fixed, and a unit
/// loads only into the engine build that wrote it (H35 S4 step 3, the build id).
struct ShaHasher(Sha256);

impl Hasher for ShaHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        0
    }
}

/// The digest that binds native code to the bytecode it was made from (MC-w31-8; the H35
/// function-table digest field): the first 16 bytes of SHA-256 over a tag, the function's
/// fields (bytecode, counts, constants, handlers, feedback slots, positions; the derived
/// `Hash` of `ep9_bytecode::Function`) and the unit's string table (a String constant names a
/// unit string by index). `None` if `function` is not a function of `unit`. The unit writer
/// computes the same value (`Cx::unit_function_digest`); the REBASE key of doc 13 C-21 may
/// refine it before the H35 freeze (A3).
pub(crate) fn unit_function_digest(unit: &VerifiedUnit, function: u32) -> Option<[u8; 16]> {
    let f = unit.functions.get(function as usize)?;
    let mut h = ShaHasher(Sha256::new());
    h.write(b"ep9 unit function digest v1\0");
    f.hash(&mut h);
    unit.strings.hash(&mut h);
    let d = h.0.finish();
    let mut out = [0u8; 16];
    out.copy_from_slice(&d[..16]);
    Some(out)
}

/// The install rank and accept list of the isolate of `rt` (doc 13 §1.2), or `None` for a
/// preset this build has no policy for: the `ExecOptions` field that the preset constructors
/// set (`exec.rs`; route MC-w24-3), so the loader keeps no preset table of its own.
pub(crate) fn isolate_policy(rt: &Runtime) -> Option<Policy> {
    rt.exec.get().policy()
}

/// Steps 5-7 and the pick for the native part of `lu` (see the module doc): maps `text`,
/// gives each listed function its code set and calls `select()` with `policy` and a session
/// with the host CPU bits `cpu`. Step 8 is off ([`INSTALL`]). Runs no JS and allocates no
/// heap cell. Returns the counts.
pub(crate) fn load(
    rt: &Runtime,
    lu: &LoadedUnit,
    text: &[u8],
    functions: &[FunctionCode],
    policy: Option<&Policy>,
    cpu: u64,
) -> Report {
    let mut r = Report::default();
    if functions.is_empty() {
        return r;
    }
    let mapped = CodeRange::process().map(|range| UnitText::map(range, text));
    let Ok(Ok(mapped)) = mapped else {
        r.add(Reason::NoText, functions.len() as u32);
        return r;
    };
    let text = Rc::new(mapped);
    let (base, len) = (text.base(), text.len() as u64);
    let session = Session {
        connected: false,
        cpu,
    };
    for f in functions {
        let Some(fi) = lu.infos.get(f.function as usize) else {
            r.add(Reason::Function, 1);
            continue;
        };
        if fi.code_set.get().is_some() {
            r.add(Reason::Twice, 1);
            continue;
        }
        if unit_function_digest(&lu.unit, f.function) != Some(f.digest) {
            r.add(Reason::Digest, 1);
            continue;
        }
        let mut set = CodeSet::default();
        let mut code = Vec::new();
        for v in &f.variants {
            let kind = match DispatchKind::from_u8(v.kind) {
                Some(k @ (DispatchKind::AotBaseline | DispatchKind::AotOpt)) => k,
                _ => {
                    r.add(Reason::Kind, 1);
                    continue;
                }
            };
            let end = u64::from(v.entry) + u64::from(v.len);
            if v.len == 0 || end > len {
                r.add(Reason::Range, 1);
                continue;
            }
            if v.lowering == lowering::JIT {
                r.add(Reason::JitLowering, 1);
                continue;
            }
            // Steps 6-7: a fuse that popped already, or one this build does not know, makes
            // the object a dependency that does not hold, so `select()` skips it.
            let deps = v
                .fuses
                .iter()
                .map(|&index| Dep::Static {
                    index,
                    intact: (index as usize) < fuse::STATIC.len()
                        && rt.fuses.intact(rt, FuseRef::Static(index), 0),
                })
                .collect();
            set.objects.push(CodeObject {
                kind: kind as u8,
                lowering: v.lowering,
                state: ObjectState::Active,
                cpu: v.cpu,
                deps,
            });
            code.push(base + v.entry as usize..base + end as usize);
        }
        let pick = match policy {
            Some(p) => crate::select::select(&set, p, &session),
            None => Pick::Interp,
        };
        r.add(
            match pick {
                Pick::Code(_) => Reason::Picked,
                Pick::Interp | Pick::InterpDebug => Reason::Bytecode,
            },
            1,
        );
        let unit_code = UnitCode {
            set,
            code: code.into_boxed_slice(),
            pick,
            _text: Rc::clone(&text),
        };
        // Empty (checked above), and nothing else writes it.
        let _ = fi.code_set.set(Box::new(unit_code));
    }
    if INSTALL {
        // SAFETY: never runs in M7-P4 (INSTALL is false); when it runs, the unit's own check
        // (H35 S4 steps 1-4, the digest above) stands for the code contract of
        // `install_picked`.
        let installed = unsafe { install_picked(rt, lu) };
        for reason in Reason::ALL {
            r.add(reason, installed.count(reason));
        }
    } else {
        r.add(Reason::Deferred, r.count(Reason::Picked));
    }
    r
}

/// Step 8: installs each AotBaseline pick of `lu` with no fuse dependency into its dispatch
/// slot and the code registry (`native::install_code`). Counts [`Reason::Installed`] and
/// [`Reason::NotInstallable`].
///
/// # Safety
/// The code of each picked object follows the `install_code` contract: the `NativeEntry`
/// ABI, it runs the bytecode of its function (the digest binds them), and it follows the
/// `RuntimeTable` rules. Its text lives as long as the isolate (`UnitCode::text`). JS can
/// call an installed function only after MC-w30-2; until then only Rust enters it
/// (`native::call_native`).
pub(crate) unsafe fn install_picked(rt: &Runtime, lu: &LoadedUnit) -> Report {
    let mut r = Report::default();
    for fi in lu.infos.iter() {
        let Some(uc) = fi.code_set.get() else {
            continue;
        };
        let Pick::Code(i) = uc.pick else {
            continue;
        };
        let obj = &uc.set.objects[i];
        let kind = DispatchKind::from_u8(obj.kind);
        if kind != Some(DispatchKind::AotBaseline) || !obj.deps.is_empty() {
            r.add(Reason::NotInstallable, 1);
            continue;
        }
        // SAFETY: the caller's contract; `uc.code[i]` lies in `uc.text`.
        match unsafe { install_code(rt, fi, DispatchKind::AotBaseline, uc.code[i].clone()) } {
            Ok(_) => r.add(Reason::Installed, 1),
            Err(_) => r.add(Reason::NotInstallable, 1),
        }
    }
    r
}

/// The code set of `fi`, if the loader gave it one.
#[cfg(test)]
pub(crate) fn code_of(fi: &crate::unit::FunctionInfo) -> Option<&UnitCode> {
    fi.code_set.get().map(|b| &**b)
}
