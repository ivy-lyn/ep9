//! The A2 unit loader (plan v12 §7.3, M7-P4; doc 13 §3.3 event T1 "unit load", C-17; H35 S4
//! steps 5-8 for the native part of one loaded unit). It is the first caller of
//! [`select()`](crate::select::select) (doc 13 C-15; MODES-D6): `select()` stays pure, and the
//! loader gives it the code set, the policy and the session.
//!
//! ```text
//!  load(rt, loaded unit, text, functions, policy, host cpu)          (a bytecode unit: LoadedUnit)
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
//!   `load` never installs and maps nothing: each pick ------------------> "deferred"
//!   `load_and_install` (the caller ran the H35 S4 checks, MC-w34-2), only if a function picked
//!   a unit object (MC-w34-4):
//!   5  the process text map, keyed by SHA-256 of the text and its unwind data: a live entry
//!      is shared; else text --unit_text::UnitText::map--> RX in the process code range
//!      (H07:52) -- fails ----------------------------------------------> each pick "no-text"
//!   7  the unwind data of the text registered with the system unwinder, once per mapped
//!      text, before any of its code can run (JIT review O3) -- fails --> each pick "no-unwind"
//!   8  install the picks (`install_picked`) -----------------------------> "installed" |
//!                                                                          "not-installable"
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
//! | Step 8 runs only for a caller that ran the H35 S4 checks (steps 1-4) on the unit bytes: [`load_and_install`] is an `unsafe fn` (the unsafe `Cx` method of `abi/unit_code.rs`), and [`load`] (the safe `Cx` method) never installs, so safe Rust cannot run embedder bytes as code (route MC-w34-2, M7-P5 A2-RW; H35:18 "a unit is trusted code") | [`load`], [`load_and_install`] |
//! | A run with no unit pays nothing: no T0 path reads the code set; a function info gets one empty `OnceCell` word (routes MC-w22-O5, MC-w29-7) | `FunctionInfo::code_set` |
//! | The object model reads no execution policy, also with a unit loaded (doc 13 M-R9; route A2-MR9): the loader writes only `code_set` and (step 8) the dispatch slot and the code registry. This file reads the policy, as `select.rs` does: the rule-40 grep exempts both (plan v12 rule 40) | the tests |
//!
//! **The process text map** (H35 S2: text is per process; routes A2-UNIT-SHARE and MC-w34-4,
//! M7-P5 A2-RW). Unit text and JIT code share the one process code range (H07:52), so the
//! loader maps a text only when a function picked a unit object (the JIT preset picks none),
//! and it maps one text once per process: the key is SHA-256 over a tag, the text and its
//! unwind data, so a reuse is checked by its bytes, not by a name (V8 9d9ccafe3b). The map
//! holds only weak entries: a text goes back to the range when the last function of the last
//! isolate that uses it drops, so its size is bounded by the live texts. A text that is larger
//! than the whole range is refused before its bytes are hashed.
//!
//! **Unwind data** (H35 S2 "unwind/debug"; form 1, M7-P5 A2-RW; JIT review O3: no unit code
//! on the JS call path before its unwind data is registered). The unwind argument is the
//! unit's unwind section in the form of the target. Unix: an `.eh_frame` image (CIEs and FDEs
//! as `ep9_platform::unwind::cfi::EhFrameBuilder` writes them, 8-byte `pc_begin` and
//! `pc_range`) whose `pc_begin` values are text offsets; the loader adds the text address in
//! its copy, checks that each FDE lies inside the text, and registers it in the process mode
//! (`unwind::choose_mode`: a jitless build runs the self-test on the built-in probe in unit
//! text). Windows x64: RUNTIME_FUNCTION entries, 12 bytes each (begin, end, UNWIND_INFO; u32,
//! little-endian), every offset into the text (the UNWIND_INFO bytes are part of the text).
//! Empty unwind data installs nothing ("no-unwind").

use core::hash::{Hash, Hasher};
use core::ops::Range;
use std::io;
use std::sync::{Arc, Mutex, Weak};

use ep9_bytecode::VerifiedUnit;
use ep9_bytecode::sha256::Sha256;
use ep9_platform::CodeRange;
use ep9_platform::unit_text::UnitText;

use crate::dispatch::DispatchKind;
use crate::fuse::{self, FuseRef};
use crate::native::install_code;
use crate::runtime::Runtime;
use crate::select::{CodeObject, CodeSet, Dep, ObjectState, Pick, Policy, Session, lowering};
use crate::unit::{FunctionInfo, LoadedUnit};

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
    /// The code of each object of `set`: offsets in the unit text.
    pub code: Box<[Range<usize>]>,
    /// What `select()` picked at load.
    pub pick: Pick,
    /// The mapped text, when the load installs ([`load_and_install`]) and a function picked a
    /// unit object: it keeps the text mapped and its unwind data registered while a function
    /// of the unit holds it (until the isolate drops, `Runtime::units` only grows). `None`
    /// for [`load`], which maps nothing.
    pub text: Option<Arc<SharedText>>,
}

impl UnitCode {
    /// The address range of the code of object `i` of the set, if the text is mapped.
    pub(crate) fn code_at(&self, i: usize) -> Option<Range<usize>> {
        let base = self.text.as_ref()?.text.base();
        let c = self.code.get(i)?;
        Some(base + c.start..base + c.end)
    }
}

/// The unwind registration of the target (unix: `.eh_frame`; Windows: a function table).
#[cfg(unix)]
type Registration = ep9_platform::unwind::EhFrameRegistration;
/// The unwind registration of the target (unix: `.eh_frame`; Windows: a function table).
#[cfg(windows)]
type Registration = ep9_platform::unwind::FunctionTableRegistration;

/// One mapped unit text of the process (H35 S2), with its unwind registration: shared by
/// every function of every isolate that loaded the same text and unwind bytes (module doc).
pub(crate) struct SharedText {
    /// Declared first, so it drops (deregisters) before the text pages go back.
    _unwind: Registration,
    text: UnitText<'static>,
    key: [u8; 32],
}

// SAFETY: the text is written once, in `UnitText::map`, before the value exists, and never
// again; the value is only read (base, length) and dropped. The registration is `Send` and
// `Sync` itself. Dropping it on another thread is allowed by both unwinders, and the drop runs
// only when no function of any isolate holds the text, so no frame of it is on any stack.
unsafe impl Send for SharedText {}
// SAFETY: as above: no method writes through `&self`.
unsafe impl Sync for SharedText {}

/// The process text map (module doc): weak entries, pruned at each lookup.
static TEXTS: Mutex<Vec<Weak<SharedText>>> = Mutex::new(Vec::new());

/// The map key of a text and its unwind data: SHA-256 over a tag, the text length, the text
/// and the unwind bytes.
fn text_key(text: &[u8], unwind: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"ep9 unit text key v1\0");
    h.update(&(text.len() as u64).to_le_bytes());
    h.update(text);
    h.update(unwind);
    h.finish()
}

/// Steps 5 and 7 (module doc): the mapped text for `text` and `unwind`, shared if a live
/// entry has the same key, else mapped and registered now. `Err(NoText)` if the text does
/// not map (empty, larger than the process code range, no room); `Err(NoUnwind)` if the
/// unwind data is empty, does not lie inside the text or does not register.
fn shared_text(text: &[u8], unwind: &[u8]) -> Result<Arc<SharedText>, Reason> {
    let range = CodeRange::process().map_err(|_| Reason::NoText)?;
    if text.is_empty() || text.len() > range.size() {
        return Err(Reason::NoText);
    }
    let key = text_key(text, unwind);
    // One lock for the lookup and the map: two isolates that load one unit at once map it
    // once.
    let mut texts = TEXTS.lock().unwrap_or_else(|e| e.into_inner());
    texts.retain(|w| w.strong_count() > 0);
    if let Some(t) = texts
        .iter()
        .filter_map(Weak::upgrade)
        .find(|t| t.key == key)
    {
        return Ok(t);
    }
    if unwind.is_empty() {
        return Err(Reason::NoUnwind);
    }
    let mapped = UnitText::map(range, text).map_err(|_| Reason::NoText)?;
    let reg = register_unwind(&mapped, unwind).map_err(|_| Reason::NoUnwind)?;
    let t = Arc::new(SharedText {
        _unwind: reg,
        text: mapped,
        key,
    });
    texts.push(Arc::downgrade(&t));
    Ok(t)
}

/// Registers the unix unwind data `unwind` of the mapped `text` (module doc, form 1).
#[cfg(unix)]
fn register_unwind(text: &UnitText<'static>, unwind: &[u8]) -> io::Result<Registration> {
    use ep9_platform::unwind::cfi::EhFrameSection;
    let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidData, m);
    let s = EhFrameSection::from_bytes(unwind)?;
    let mut b = unwind.to_vec();
    let word = |b: &[u8], at: usize| -> io::Result<u64> {
        let w = b.get(at..at + 8).ok_or_else(|| bad("an FDE ends early"))?;
        Ok(u64::from_le_bytes(w.try_into().expect("8 bytes")))
    };
    let (base, len) = (text.base() as u64, text.len() as u64);
    let mut fdes = 0;
    for (at, is_cie) in s.records()? {
        if is_cie {
            if !cie_is_absptr(&b[at..]) {
                return Err(bad("a CIE that is not the zR absptr form"));
            }
            continue;
        }
        // FDE: length (4), CIE pointer (4), pc_begin (8), pc_range (8); `records` refused the
        // 64-bit length form.
        let n = u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"));
        if n < 20 {
            return Err(bad("an FDE shorter than its address fields"));
        }
        let (off, n) = (word(&b, at + 8)?, word(&b, at + 16)?);
        if n == 0 || off.checked_add(n).is_none_or(|end| end > len) {
            return Err(bad("an FDE that does not lie inside the text"));
        }
        b[at + 8..at + 16].copy_from_slice(&(base + off).to_le_bytes());
        fdes += 1;
    }
    if fdes == 0 {
        return Err(bad("no FDE"));
    }
    let section = EhFrameSection::from_bytes(&b)?;
    ep9_platform::unwind::choose_mode()?;
    // SAFETY: each FDE describes code of this text (the unit writer made both; the caller of
    // `load_and_install` vouches for the unit); the text stays mapped and unchanged while the
    // registration lives (`SharedText` drops the registration first); the chunk is new, so
    // no other registration overlaps its span.
    unsafe { Registration::register(section) }
}

/// True if the CIE record `cie` (from its length field) is the form that
/// `cfi::EhFrameBuilder` writes: version 1, augmentation "zR", and the FDE pointer encoding
/// `DW_EH_PE_absptr` (0), so each FDE's `pc_begin` and `pc_range` are 8-byte values.
#[cfg(unix)]
fn cie_is_absptr(cie: &[u8]) -> bool {
    // length (4), CIE id (4), version, "zR\0", code align (uleb), data align (sleb), return
    // address register (1 byte in version 1), augmentation length (uleb), pointer encoding.
    let Some(rest) = cie.get(8..) else {
        return false;
    };
    if !rest.starts_with(&[1, b'z', b'R', 0]) {
        return false;
    }
    let mut i = 4;
    for _ in 0..2 {
        // The two LEB128 numbers: skip their bytes.
        while rest.get(i).is_some_and(|&b| b & 0x80 != 0) {
            i += 1;
        }
        i += 1;
    }
    i += 1; // the return address register
    // The augmentation length (one byte for "zR": 1) and the encoding byte.
    rest.get(i) == Some(&1) && rest.get(i + 1) == Some(&0)
}

/// Registers the Windows unwind data `unwind` of the mapped `text` (module doc, form 1).
#[cfg(windows)]
fn register_unwind(text: &UnitText<'static>, unwind: &[u8]) -> io::Result<Registration> {
    use ep9_platform::unwind::win64::RuntimeFunction;
    let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidData, m);
    if unwind.is_empty() || !unwind.len().is_multiple_of(12) {
        return Err(bad("the function table is not a list of 12-byte entries"));
    }
    let u = |c: &[u8]| u32::from_le_bytes(c.try_into().expect("4 bytes"));
    let len = text.len() as u64;
    let entries: Box<[RuntimeFunction]> = unwind
        .chunks_exact(12)
        .map(|e| RuntimeFunction {
            begin: u(&e[0..4]),
            end: u(&e[4..8]),
            unwind_info: u(&e[8..12]),
        })
        .collect();
    if entries
        .iter()
        .any(|e| u64::from(e.end) > len || u64::from(e.unwind_info) + 4 > len)
    {
        return Err(bad("a function table entry leaves the text"));
    }
    // SAFETY: each UNWIND_INFO describes its function in this text (the unit writer made
    // both); text and UNWIND_INFO stay mapped and unchanged while the registration lives
    // (`SharedText` drops it first); the chunk is new, so no other table covers its pages.
    // `register` checks the order, the bounds and the alignment of the entries.
    unsafe { Registration::register(text.base()..text.pages().end, entries) }
}

/// Why a function or a variant gets no unit code, and the other load counts (H35 S5: every
/// outcome other than a full load is counted by reason). Append-only: the report lists them in
/// this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// A pick of an installing load whose text could not be mapped (empty, larger than the
    /// process code range, no room): it runs its bytecode.
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
    /// A pick of an installing load whose text has no unwind data, or unwind data that does
    /// not lie inside the text or does not register (JIT review O3): it runs its bytecode.
    NoUnwind,
}

impl Reason {
    /// Every reason, in report order.
    pub(crate) const ALL: [Reason; 13] = [
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
        Reason::NoUnwind,
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
            Reason::NoUnwind => "no-unwind",
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
/// `Hash` of `ep9_bytecode::Function`) and the [`strings_digest`] of the unit's string table
/// (a String constant names a unit string by index). `None` if `function` is not a function
/// of `unit`. The unit writer computes the same value (`Cx::unit_function_digest`); the
/// REBASE key of doc 13 C-21 may refine it before the H35 freeze (A3). Tag v2 since M7-P4
/// fixer r2: v1 hashed the whole string table into each function digest, so a load hashed it
/// once per listed function (Codex r1 a2-loader:digest-per-function).
pub(crate) fn unit_function_digest(unit: &VerifiedUnit, function: u32) -> Option<[u8; 16]> {
    let f = unit.functions.get(function as usize)?;
    Some(function_digest(f, &strings_digest(unit)))
}

/// SHA-256 over a tag and the string table of `unit` (the derived `Hash` of its strings): one
/// pass, shared by the digests of all its functions ([`function_digest`]).
fn strings_digest(unit: &VerifiedUnit) -> [u8; 32] {
    #[cfg(test)]
    STRING_TABLE_PASSES.with(|n| n.set(n.get() + 1));
    let mut h = ShaHasher(Sha256::new());
    h.write(b"ep9 unit strings digest v2\0");
    unit.strings.hash(&mut h);
    h.0.finish()
}

/// The digest of [`unit_function_digest`] for the function `f` of a unit whose
/// [`strings_digest`] is `strings`.
fn function_digest(f: &ep9_bytecode::Function, strings: &[u8; 32]) -> [u8; 16] {
    let mut h = ShaHasher(Sha256::new());
    h.write(b"ep9 unit function digest v2\0");
    f.hash(&mut h);
    h.write(strings);
    let d = h.0.finish();
    let mut out = [0u8; 16];
    out.copy_from_slice(&d[..16]);
    out
}

#[cfg(test)]
std::thread_local! {
    /// The passes over a unit's string table on this thread (the digest cost test).
    static STRING_TABLE_PASSES: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// The passes over a unit's string table on this thread so far.
#[cfg(test)]
pub(crate) fn string_table_passes() -> u64 {
    STRING_TABLE_PASSES.with(core::cell::Cell::get)
}

/// The install rank and accept list of the isolate of `rt` (doc 13 §1.2), or `None` for a
/// preset this build has no policy for: the `ExecOptions` field that the preset constructors
/// set (`exec.rs`; route MC-w24-3), so the loader keeps no preset table of its own.
pub(crate) fn isolate_policy(rt: &Runtime) -> Option<Policy> {
    rt.exec.get().policy()
}

/// The code sets and the picks for the native part of `lu` (see the module doc): gives each
/// listed function its code set and calls `select()` with `policy` and a session with the host
/// CPU bits `cpu`. Maps nothing and never installs (step 8): each pick counts as
/// [`Reason::Deferred`] (routes MC-w34-2, MC-w34-4). Runs no JS and allocates no heap cell.
/// Returns the counts.
pub(crate) fn load(
    rt: &Runtime,
    lu: &LoadedUnit,
    text: &[u8],
    functions: &[FunctionCode],
    policy: Option<&Policy>,
    cpu: u64,
) -> Report {
    load_inner(rt, lu, text, None, functions, policy, cpu)
}

/// [`load`], then, if a function picked a unit object, steps 5, 7 and 8: the shared mapped
/// text of `text` with its unwind data `unwind` registered (module doc), and
/// [`install_picked`].
///
/// # Safety
/// The caller ran the H35 S4 checks, steps 1-4, on the unit bytes that `lu`, `text`, `unwind`
/// and `functions` come from, against this engine build (the `# Safety` of the unsafe `Cx`
/// method in `abi/unit_code.rs`): so the unwind data describes the text, and each picked
/// variant follows the [`install_picked`] contract.
#[allow(dead_code)] // the unsafe `Cx` method calls it from the M7-P5 install commit
pub(crate) unsafe fn load_and_install(
    rt: &Runtime,
    lu: &LoadedUnit,
    text: &[u8],
    unwind: &[u8],
    functions: &[FunctionCode],
    policy: Option<&Policy>,
    cpu: u64,
) -> Report {
    load_inner(rt, lu, text, Some(unwind), functions, policy, cpu)
}

/// [`load`]; with `unwind` (only [`load_and_install`] passes it; its caller contract stands
/// for the `install_picked` contract) also steps 5, 7 and 8.
fn load_inner(
    rt: &Runtime,
    lu: &LoadedUnit,
    text: &[u8],
    unwind: Option<&[u8]>,
    functions: &[FunctionCode],
    policy: Option<&Policy>,
    cpu: u64,
) -> Report {
    let mut r = Report::default();
    if functions.is_empty() {
        return r;
    }
    let len = text.len() as u64;
    let session = Session {
        connected: false,
        cpu,
    };
    // One pass over the string table for the whole load (the digest of each function mixes
    // it in).
    let strings = strings_digest(&lu.unit);
    let mut sets: Vec<(&FunctionInfo, UnitCode)> = Vec::new();
    for f in functions {
        let Some(fi) = lu.infos.get(f.function as usize) else {
            r.add(Reason::Function, 1);
            continue;
        };
        if fi.code_set.get().is_some() || sets.iter().any(|(s, _)| core::ptr::eq(*s, fi)) {
            r.add(Reason::Twice, 1);
            continue;
        }
        let digest = lu
            .unit
            .functions
            .get(f.function as usize)
            .map(|func| function_digest(func, &strings));
        if digest != Some(f.digest) {
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
            code.push(v.entry as usize..end as usize);
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
            text: None,
        };
        sets.push((fi, unit_code));
    }
    let picked = r.count(Reason::Picked);
    let mut mapped = false;
    match unwind {
        None => r.add(Reason::Deferred, picked),
        // MC-w34-4: no pick, no map (the JIT preset picks no unit object).
        Some(_) if picked == 0 => {}
        Some(unwind) => match shared_text(text, unwind) {
            Ok(t) => {
                for (_, uc) in &mut sets {
                    uc.text = Some(Arc::clone(&t));
                }
                mapped = true;
            }
            Err(reason) => r.add(reason, picked),
        },
    }
    for (fi, uc) in sets {
        // Empty (checked above), and nothing else writes it.
        let _ = fi.code_set.set(Box::new(uc));
    }
    if mapped {
        // SAFETY: only `load_and_install` passes `unwind`; its caller ran the H35 S4 checks
        // (steps 1-4) on these bytes, and the digest above binds each code set to its bytecode:
        // that stands for the code contract of `install_picked`. The text is mapped, and its
        // unwind data is registered (O3).
        let installed = unsafe { install_picked(rt, lu) };
        for reason in Reason::ALL {
            r.add(reason, installed.count(reason));
        }
    }
    r
}

/// Step 8: installs each AotBaseline pick of `lu` with no fuse dependency and a mapped text
/// into its dispatch slot and the code registry (`native::install_code`). Counts
/// [`Reason::Installed`] and [`Reason::NotInstallable`]; a pick with no mapped text is not
/// counted (its load counted it).
///
/// # Safety
/// The code of each picked object follows the `install_code` contract: the `NativeEntry`
/// ABI, it runs the bytecode of its function (the digest binds them), and it follows the
/// `RuntimeTable` rules. Its text lives as long as the isolate (`UnitCode::text`), and its
/// unwind data is registered.
pub(crate) unsafe fn install_picked(rt: &Runtime, lu: &LoadedUnit) -> Report {
    let mut r = Report::default();
    for fi in lu.infos.iter() {
        let Some(uc) = fi.code_set.get() else {
            continue;
        };
        let Pick::Code(i) = uc.pick else {
            continue;
        };
        let Some(code) = uc.code_at(i) else {
            continue;
        };
        let obj = &uc.set.objects[i];
        let kind = DispatchKind::from_u8(obj.kind);
        if kind != Some(DispatchKind::AotBaseline) || !obj.deps.is_empty() {
            r.add(Reason::NotInstallable, 1);
            continue;
        }
        // SAFETY: the caller's contract; `code` lies in `uc.text`.
        match unsafe { install_code(rt, fi, DispatchKind::AotBaseline, code) } {
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
