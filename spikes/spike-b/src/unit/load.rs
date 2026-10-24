//! The spike's unit loader: H35 S4 steps 1-8 for the units of [`super::build`].
//!
//! ```text
//! load(bytes, host)
//!   1-4  ep9_unit::verdict(bytes, host)  LOAD ERROR -> nothing runs
//!                                        SOURCE     -> compile the source section, run it
//!                                        BYTECODE   -> run the bytecode section
//!        LOAD: parse the sections and plan the link (external-reference kinds, intrinsic
//!        indices); an error here is a LOAD ERROR, and nothing has run yet
//!   5    map the text once per process (the registry; keyed by the content hash):
//!        one chunk of the code range, the text bytes as they are in the file, the Win64
//!        UNWIND_INFO after them; a unit slot that another unit holds -> BYTECODE
//!        the first unit of the process: the unwind self-test on its probe (H28, unix)
//!   6    run the bytecode; copy the RW template for this isolate (cannot fail): external
//!        references and link cells from the plan, IC cells undefined, counters 0; link
//!        cells a strong root range, IC cells a weak one (Heap::add_root_range);
//!        cx.unit_data[slot] = the copy
//!   7    unwind registration (step 5, once per process); per isolate: the code registry
//!   8    install each JS function in an Opt slot (ep9_vm::install_native_b), if the digest
//!        of its bytecode equals the one in the function table; else it stays in T0
//! ```
//!
//! Text, its unwind registration and the registry entry live until the process ends. The RW
//! copies and the unit-data directory live in the isolate's [`UnitSession`], which drops the
//! isolate (and its root ranges) first.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use ep9_bytecode::CompiledUnit;
use ep9_heap::{RootId, RootStrength, Slot};
use ep9_platform::jit::WxMode;
use ep9_platform::{CodeRange, page_size};
use ep9_unit::{HostInfo, Outcome, SectionKind, Verdict};
use ep9_value::encoding::VALUE_UNDEFINED;
use ep9_vm::{IntrinsicId, NativeCx, RuntimeTable};

use super::emit::{RwLayout, SLOTS};
use super::format::{EXT_KIND_RUNTIME_TABLE, FnEntry, FnKind, Link, UnitView, section};
use crate::harness::{Config, RunOutput, Session, exception_text, take_output};

/// A unit's text, mapped once per process.
#[derive(Debug)]
pub struct Mapped {
    /// The unit's content hash.
    pub hash: [u8; 32],
    pub slot: u32,
    /// Its text (executable addresses).
    pub text: Range<usize>,
    pub functions: Vec<FnEntry>,
}

impl Mapped {
    /// The executable range of function `f`.
    pub fn code(&self, f: &FnEntry) -> Range<usize> {
        self.text.start + f.text.start as usize..self.text.start + f.text.end as usize
    }

    /// The first function of kind `kind` (and, for a JS function, called `name`).
    pub fn function(&self, kind: FnKind, name: &str) -> Option<&FnEntry> {
        self.functions
            .iter()
            .find(|f| f.kind == kind && (kind != FnKind::Js || f.name == name))
    }
}

struct Registry {
    maps: Vec<&'static Mapped>,
    slots: [Option<[u8; 32]>; SLOTS],
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    maps: Vec::new(),
    slots: [None; SLOTS],
});

/// The result of the unwind self-test on the first mapped unit's text (unix): the mode, or
/// why no mode worked.
static SELF_TEST: OnceLock<Result<String, String>> = OnceLock::new();

/// The unwind self-test result of this process, if a unit was mapped (unix; Windows has one
/// registration form and no self-test).
pub fn self_test_result() -> Option<Result<String, String>> {
    SELF_TEST.get().cloned()
}

/// Every unit mapped in this process.
pub fn mapped_units() -> Vec<&'static Mapped> {
    REGISTRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .maps
        .clone()
}

/// A context-synchronizing event on this thread, before it runs text that another thread
/// wrote: `isb` (AArch64), `cpuid` (x86-64, a serializing instruction). The writer flushed the
/// icache when it closed its write window; the registry lock orders the data.
pub fn context_sync() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: `isb` only flushes this core's pipeline; it touches no memory or register.
    unsafe {
        core::arch::asm!("isb", options(nostack, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    {
        // `cpuid` is serializing (Intel SDM vol. 3 8.1.3, cross-modifying code).
        #[allow(unused_unsafe)]
        // SAFETY: cpuid leaf 0 exists on every x86-64 CPU and has no side effect.
        let _ = unsafe { core::arch::x86_64::__cpuid(0) };
    }
}

/// Maps `view`'s text (content hash `hash`) for this process, or returns the mapping a load
/// made before. Fails if another unit holds the slot, or the unwind data does not register.
fn map(view: &UnitView<'_>, hash: [u8; 32]) -> Result<&'static Mapped, String> {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = reg.maps.iter().find(|m| m.hash == hash) {
        return Ok(m);
    }
    let slot = view.slot as usize;
    if slot >= SLOTS {
        return Err(format!("unit slot {slot} >= {SLOTS}"));
    }
    if reg.slots[slot].is_some_and(|h| h != hash) {
        return Err(format!(
            "unit slot {slot} is held by another unit in this process"
        ));
    }
    // Layout: the text as it is in the file, then (Windows) the UNWIND_INFO blobs, 4-aligned.
    let mut end = view.text.len();
    let mut info_at = Vec::new();
    if cfg!(windows) {
        for w in &view.unwind.win64 {
            let s = end.next_multiple_of(4);
            info_at.push(s);
            end = s + w.info.len();
        }
    }
    let size = end.max(1).next_multiple_of(page_size());
    let range = CodeRange::process().map_err(|e| e.to_string())?;
    // The chunk and its code memory live until the process ends: every isolate of the
    // process may run the text.
    let chunk = Box::leak(Box::new(range.alloc(size).map_err(|e| e.to_string())?));
    let mut mem = chunk.commit(WxMode::DEFAULT).map_err(|e| e.to_string())?;
    {
        let mut w = mem.open_write(0, end).map_err(|e| e.to_string())?;
        let b = w.bytes();
        b.fill(0);
        b[..view.text.len()].copy_from_slice(view.text);
        for (e, &at) in view.unwind.win64.iter().zip(&info_at) {
            b[at..at + e.info.len()].copy_from_slice(&e.info);
        }
        w.close().map_err(|e| e.to_string())?;
    }
    let base = mem.exec_base();
    // SAFETY: the chunk is committed and holds `end` written bytes from `base`; it is never
    // unmapped (leaked above) and never written again.
    let mapped = unsafe { std::slice::from_raw_parts(base as *const u8, view.text.len()) };
    if mapped != view.text {
        return Err("the mapped text differs from the unit's text section".into());
    }
    let text = base..base + view.text.len();
    register_unwind(view, text.clone(), base..base + size, &info_at)?;
    std::mem::forget(mem);
    reg.slots[slot] = Some(hash);
    let m: &'static Mapped = Box::leak(Box::new(Mapped {
        hash,
        slot: view.slot,
        text,
        functions: view.functions.clone(),
    }));
    reg.maps.push(m);
    Ok(m)
}

/// The unit's `.eh_frame` image with each FDE's `pc_begin` moved from a text offset to an
/// address (`text_base` + offset).
#[cfg(unix)]
pub fn placed_eh_frame(
    image: &[u8],
    text_base: usize,
) -> Result<ep9_platform::unwind::cfi::EhFrameSection, String> {
    use ep9_platform::unwind::cfi::EhFrameSection;
    let s = EhFrameSection::from_bytes(image).map_err(|e| e.to_string())?;
    let mut b = image.to_vec();
    for (at, is_cie) in s.records().map_err(|e| e.to_string())? {
        if !is_cie {
            // FDE: length (4), CIE pointer (4), pc_begin (8, DW_EH_PE_absptr).
            let p = at + 8;
            let off = u64::from_le_bytes(b[p..p + 8].try_into().unwrap());
            b[p..p + 8].copy_from_slice(&(off + text_base as u64).to_le_bytes());
        }
    }
    EhFrameSection::from_bytes(&b).map_err(|e| e.to_string())
}

/// Step 7, once per process: the unwind registration of the mapped text. On unix the first
/// unit of the process first runs the unwind self-test on its probe (H28: a jitless loader
/// chooses the registration mode on the first loaded unit's text).
#[cfg(unix)]
fn register_unwind(
    view: &UnitView<'_>,
    text: Range<usize>,
    _chunk: Range<usize>,
    _info_at: &[usize],
) -> Result<(), String> {
    use ep9_platform::unwind::{EhFrameRegistration, ProbeFn, SelfTestProbe, self_test};
    let section = placed_eh_frame(&view.unwind.eh_frame, text.start)?;
    let first = SELF_TEST.get_or_init(|| {
        let p = view
            .functions
            .iter()
            .find(|f| f.kind == FnKind::Probe)
            .ok_or("the unit has no probe")?;
        let code = text.start + p.text.start as usize..text.start + p.text.end as usize;
        // SAFETY: the probe is the platform probe's code (`ProbeFn`), mapped executable at
        // `code` and never unmapped.
        let entry = unsafe {
            std::mem::transmute::<*const (), ProbeFn>(std::ptr::with_exposed_provenance(code.start))
        };
        let probe = SelfTestProbe {
            entry,
            code,
            section: section.clone(),
        };
        // SAFETY: the probe follows `ProbeFn`; each FDE of the section describes the code the
        // unit emitter made at its text offset (the section puts the JS functions' FDEs before
        // the probe's); no registration covers this new chunk yet.
        unsafe { self_test(probe) }
            .map(|m| m.to_string())
            .map_err(|e| e.to_string())
    });
    if let Err(e) = first {
        return Err(format!(
            "unwind self-test failed, native code cannot run: {e}"
        ));
    }
    // SAFETY: each FDE describes code the unit emitter made; the text stays mapped and
    // unchanged for the process; the chunk is new, so no other registration overlaps it.
    let reg = unsafe { EhFrameRegistration::register(section) }.map_err(|e| e.to_string())?;
    std::mem::forget(reg);
    Ok(())
}

/// Step 7, once per process (Windows): one RUNTIME_FUNCTION per function, UNWIND_INFO after
/// the text in the same chunk.
#[cfg(windows)]
fn register_unwind(
    view: &UnitView<'_>,
    text: Range<usize>,
    chunk: Range<usize>,
    info_at: &[usize],
) -> Result<(), String> {
    use ep9_platform::unwind::FunctionTableRegistration;
    use ep9_platform::unwind::win64::RuntimeFunction;
    let off = (text.start - chunk.start) as u32;
    let entries: Box<[RuntimeFunction]> = view
        .unwind
        .win64
        .iter()
        .zip(info_at)
        .map(|(w, &i)| RuntimeFunction {
            begin: off + w.text.start,
            end: off + w.text.end,
            unwind_info: i as u32,
        })
        .collect();
    // SAFETY: each UNWIND_INFO describes the prologue the unit emitter made; code and
    // UNWIND_INFO stay mapped and unchanged for the process; the chunk is new.
    let reg = unsafe { FunctionTableRegistration::register(chunk, entries) }
        .map_err(|e| e.to_string())?;
    std::mem::forget(reg);
    let _ = SELF_TEST.set(Ok("windows: function table, no self-test".into()));
    Ok(())
}

// ---- per isolate -----------------------------------------------------------------------------

/// One isolate's RW copy of a unit.
#[derive(Debug)]
pub struct RwCopy {
    pub slot: u32,
    pub layout: RwLayout,
    words: Box<[AtomicU64]>,
    roots: Vec<RootId>,
}

impl RwCopy {
    /// Word `i` (raw bits).
    pub fn word(&self, i: usize) -> u64 {
        self.words[i].load(Relaxed)
    }

    /// The address of word 0 (the unit-data register's value for this isolate).
    pub fn base(&self) -> usize {
        self.words.as_ptr().addr()
    }

    /// Link cell `i` (a Value).
    pub fn link(&self, i: u32) -> u64 {
        self.word(self.layout.link_base() + i as usize)
    }

    /// IC cell `i` (a Value).
    pub fn ic(&self, i: u32) -> u64 {
        self.word(self.layout.ic_base() + i as usize)
    }

    /// Counter `i`.
    pub fn counter(&self, i: u32) -> u64 {
        self.word(self.layout.counter_base() + i as usize)
    }

    /// The root ranges registered for this copy.
    pub fn roots(&self) -> usize {
        self.roots.len()
    }
}

/// An isolate's unit state: the unit-data directory `Cx::unit_data` points at, and the RW
/// copies.
#[derive(Debug)]
pub struct UnitHost {
    dir: Box<[usize; SLOTS]>,
    rws: Vec<RwCopy>,
}

impl Default for UnitHost {
    fn default() -> Self {
        UnitHost {
            dir: Box::new([0; SLOTS]),
            rws: Vec::new(),
        }
    }
}

impl UnitHost {
    /// The RW copy of the unit in `slot`.
    pub fn rw(&self, slot: u32) -> Option<&RwCopy> {
        self.rws.iter().find(|r| r.slot == slot)
    }

    /// The directory address (what `Cx::unit_data` holds once a unit is linked).
    pub fn directory(&self) -> usize {
        self.dir.as_ptr().addr()
    }

    /// The directory word of `slot`.
    pub fn directory_word(&self, slot: u32) -> usize {
        self.dir[slot as usize]
    }
}

/// An isolate with `print` and `gc` (a [`Session`]) and its unit state. The isolate drops
/// first: its heap holds the root ranges over the RW copies, and its slots point at unit text,
/// which never unmaps.
pub struct UnitSession {
    pub s: Session,
    pub units: UnitHost,
}

impl UnitSession {
    /// A session under `config` with the threaded T0 dispatch (placement (b) code runs only
    /// there; the match loop runs the bytecode, plan-v6 D4).
    pub fn new(config: &Config) -> Result<UnitSession, String> {
        Ok(UnitSession {
            s: Session::new(&config.threaded())?,
            units: UnitHost::default(),
        })
    }
}

/// Load options for the tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOptions {
    /// The table the runtime-table external reference resolves to (default: the process
    /// table, `RuntimeTable::process`).
    pub runtime_table: Option<&'static RuntimeTable>,
}

/// What a load did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadKind {
    /// Native code: the text is mapped, the RW copy linked, these functions installed; the
    /// others (name, why) stay in T0.
    Native {
        installed: Vec<String>,
        t0: Vec<(String, String)>,
    },
    /// The unit's bytecode runs (the reason).
    Bytecode(String),
    /// The unit's source was compiled and runs (the reason).
    Source(String),
    /// Nothing runs (the reason).
    LoadError(String),
}

impl LoadKind {
    pub fn name(&self) -> &'static str {
        match self {
            LoadKind::Native { .. } => "LOAD",
            LoadKind::Bytecode(_) => "BYTECODE",
            LoadKind::Source(_) => "SOURCE",
            LoadKind::LoadError(_) => "LOAD ERROR",
        }
    }
}

/// A load and the run of the unit's script (absent after a LOAD ERROR).
#[derive(Clone, Debug)]
pub struct Loaded {
    pub kind: LoadKind,
    pub verdict: Verdict,
    pub run: Option<RunOutput>,
}

fn run_bytecode(s: &mut Session, bytes: &[u8], file: &str) -> RunOutput {
    take_output();
    let result = match CompiledUnit::from_bytes(bytes) {
        Ok(u) => s
            .iso
            .with_cx_checkpoint(|cx| cx.run_script(u, file).map(|v| cx.display(v)))
            .map_err(|e| exception_text(&e)),

        Err(e) => Err(format!("bytecode section: {e}")),
    };
    RunOutput {
        result,
        output: take_output(),
    }
}

fn run_source(s: &mut Session, bytes: &[u8], file: &str) -> RunOutput {
    let text = section(bytes, SectionKind::Source)
        .ok()
        .flatten()
        .and_then(|b| std::str::from_utf8(b).ok());
    match text {
        Some(t) => s.run(t, file),
        None => RunOutput {
            result: Err("no source text".into()),
            output: Vec::new(),
        },
    }
}

/// Loads the unit `bytes` into the session (H35 S4 against `host`) and runs its script.
pub fn load(us: &mut UnitSession, bytes: &[u8], host: &HostInfo, opts: &LoadOptions) -> Loaded {
    let verdict = ep9_unit::verdict(bytes, host);
    let file = "unit.js";
    match verdict.outcome() {
        Outcome::LoadError => Loaded {
            kind: LoadKind::LoadError(verdict.to_string()),
            verdict,
            run: None,
        },
        Outcome::Source => Loaded {
            kind: LoadKind::Source(verdict.to_string()),
            verdict,
            run: Some(run_source(&mut us.s, bytes, file)),
        },
        Outcome::Bytecode => {
            let run = section(bytes, SectionKind::Bytecode)
                .ok()
                .flatten()
                .map(|b| run_bytecode(&mut us.s, b, file));
            Loaded {
                kind: LoadKind::Bytecode(verdict.to_string()),
                verdict,
                run,
            }
        }
        Outcome::Load => match load_native(us, bytes, host, opts) {
            Ok((kind, run)) => Loaded {
                kind,
                verdict,
                run: Some(run),
            },
            Err(e) => Loaded {
                kind: LoadKind::LoadError(e),
                verdict,
                run: None,
            },
        },
    }
}

/// Steps 5-8 after a LOAD verdict. `Err` only when no section can be used.
fn load_native(
    us: &mut UnitSession,
    bytes: &[u8],
    host: &HostInfo,
    opts: &LoadOptions,
) -> Result<(LoadKind, RunOutput), String> {
    // Every step that can give a LOAD ERROR comes before the first run: the parse (it checks
    // the external-reference kinds and the intrinsic indices) and the link plan. After the
    // run, `link` cannot fail.
    let view = UnitView::parse(bytes)?;
    let plan = LinkPlan::new(&view, opts)?;
    let hash: [u8; 32] = view.header.content_hash;
    let fallback = |us: &mut UnitSession, why: String| {
        let run = run_bytecode(&mut us.s, view.bytecode, "unit.js");
        Ok((LoadKind::Bytecode(why), run))
    };
    if view.arch != host.target_arch {
        return fallback(us, "the function table is for another architecture".into());
    }
    if us.units.dir[view.slot as usize % SLOTS] != 0 {
        return fallback(
            us,
            format!("unit slot {} is already linked in this isolate", view.slot),
        );
    }
    let mapped = match map(&view, hash) {
        Ok(m) => m,
        Err(e) => return fallback(us, e),
    };
    context_sync();
    let run = run_bytecode(&mut us.s, view.bytecode, "unit.js");
    if run.result.is_err() {
        return Ok((
            LoadKind::Native {
                installed: Vec::new(),
                t0: vec![("<script>".into(), "the unit's script threw".into())],
            },
            run,
        ));
    }
    link(us, &view, &plan);
    let mut installed = Vec::new();
    let mut t0 = Vec::new();
    for f in mapped.functions.iter().filter(|f| f.kind == FnKind::Js) {
        match install(us, mapped, f) {
            Ok(()) => installed.push(f.name.clone()),
            Err(e) => t0.push((f.name.clone(), e)),
        }
    }
    Ok((LoadKind::Native { installed, t0 }, run))
}

/// What [`link`] writes, resolved before any section of the unit runs: the external
/// references and the intrinsic of each intrinsic link cell. A string link cell is made in
/// [`link`] (a GC point), but it cannot fail.
struct LinkPlan<'v> {
    ext: Vec<u64>,
    links: Vec<Planned<'v>>,
}

enum Planned<'v> {
    Intrinsic(IntrinsicId),
    Str(&'v str),
}

impl<'v> LinkPlan<'v> {
    fn new(view: &'v UnitView<'_>, opts: &LoadOptions) -> Result<LinkPlan<'v>, String> {
        let ext = view
            .rw
            .ext
            .iter()
            .map(|&kind| match kind {
                EXT_KIND_RUNTIME_TABLE => Ok(std::ptr::from_ref(
                    opts.runtime_table.unwrap_or_else(RuntimeTable::process),
                )
                .addr() as u64),
                k => Err(format!("unknown external reference kind {k}")),
            })
            .collect::<Result<_, _>>()?;
        let links = view
            .rw
            .links
            .iter()
            .map(|link| match link {
                Link::Intrinsic(k) => IntrinsicId::ALL
                    .get(*k as usize)
                    .map(|&id| Planned::Intrinsic(id))
                    .ok_or_else(|| format!("no intrinsic {k}")),
                Link::Str(s) => Ok(Planned::Str(s)),
            })
            .collect::<Result<_, _>>()?;
        Ok(LinkPlan { ext, links })
    }
}

/// Step 6: this isolate's RW copy of `view`'s unit, from `plan`. It cannot fail: the heap
/// holds the root ranges over the words from the first `add_root_range` on, and the words must
/// then live as long as the isolate (in `us.units`).
fn link(us: &mut UnitSession, view: &UnitView<'_>, plan: &LinkPlan<'_>) {
    let l = view.rw.layout;
    let words: Box<[AtomicU64]> = (0..l.words())
        .map(|i| {
            let value = (l.link_base()..l.counter_base()).contains(&i);
            AtomicU64::new(if value { VALUE_UNDEFINED } else { 0 })
        })
        .collect();
    for (i, &v) in plan.ext.iter().enumerate() {
        words[i].store(v, Relaxed);
    }
    let iso = &mut us.s.iso;
    let mut roots = Vec::new();
    for (range, strength) in [
        (l.link_base()..l.ic_base(), RootStrength::Strong),
        (l.ic_base()..l.counter_base(), RootStrength::Weak),
    ] {
        if range.is_empty() {
            continue;
        }
        let cells: &[AtomicU64] = &words[range];
        let slots = std::ptr::NonNull::slice_from_raw_parts(
            std::ptr::NonNull::from(cells).cast::<Slot>(),
            cells.len(),
        );
        // SAFETY: `Slot` is a `repr(transparent)` AtomicU64; the words hold Value bits
        // (undefined now, resolved Values below), never 0; the boxed words move into
        // `us.units` below with no return in between, and outlive the isolate (the
        // `UnitSession` field order), so they stay valid until the heap drops.
        roots.push(unsafe { iso.heap().add_root_range(slots, strength) });
    }
    for (i, link) in plan.links.iter().enumerate() {
        let v = match *link {
            Planned::Intrinsic(id) => NativeCx::intrinsic(iso, id),
            // A GC point; the cells resolved so far are in the strong range already.
            Planned::Str(s) => NativeCx::new_string(iso, s),
        };
        words[l.link_base() + i].store(v.to_bits(), Relaxed);
    }
    us.units.dir[l.slot as usize] = words.as_ptr().addr();
    NativeCx::set_unit_data(iso, us.units.dir.as_ptr().addr());
    us.units.rws.push(RwCopy {
        slot: l.slot,
        layout: l,
        words,
        roots,
    });
}

/// Step 8 for one JS function: the global function of that name must have exactly the
/// bytecode the code was made from.
fn install(us: &mut UnitSession, m: &Mapped, f: &FnEntry) -> Result<(), String> {
    let iso = &us.s.iso;
    let g = NativeCx::global_value(iso, &f.name).ok_or_else(|| format!("no global {}", f.name))?;
    let t = ep9_vm::native_target(iso, g).ok_or("not a function with bytecode")?;
    let d = super::format::function_digest(
        t.parameter_count,
        t.register_count,
        t.handler_count,
        t.bytecode,
    );
    if d != f.digest {
        return Err("its bytecode differs from the bytecode the code was made from".into());
    }
    // SAFETY: the code at `m.code(f)` was made by the unit emitter from exactly this bytecode
    // (the digest), so it has the T0 handler ABI and the (b) exit rules
    // (`install_native_b`); it reads `cx` at `layout` offsets and its RW copy through
    // `Cx::unit_data`, which `link` set for this isolate; the text never unmaps.
    unsafe { ep9_vm::install_native_b(iso, g, m.code(f)) }.map_err(|e| format!("{e:?}"))
}
