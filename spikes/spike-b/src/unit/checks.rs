//! The checks of `spike-b unit` (gate.sh unit, SPIKEB(unit)) and `tests/unit.rs`.
//!
//! | Check | What |
//! |---|---|
//! | (a) build | units A and B build for AArch64 and x86-64 from the CompiledUnit (no isolate); the same bytes twice; the text scan finds 0 relocations; the header gives LOAD (spike host), BYTECODE (no native form, the real host; another target) |
//! | (b) load | LOAD maps the text once per process, unchanged; the JS functions get Opt slots; output equals T0 (default, eager feedback, GC stress); counters and the IC cell count and record the calls |
//! | (c) cells | unit text reads its isolate's RW copy through x27 / r14; link cells (an intrinsic by stable index, a unit constant) survive GC in the strong range; an IC cell in the weak range is cleared when its callee dies, then filled again |
//! | (d) threads | 2 isolates on 2 threads run one mapped text at once, after a context-sync handshake; each reaches its own RW copy (isolate 2's runtime-table cell names a counting table) |
//! | (e) multi | units A and B (slots 0 and 1) in one isolate, loaded in both orders; each reaches its own RW copy |
//! | (f) unwind | the unwind self-test ran on the first unit's text; a native walk from a host function passes through a unit frame; an exception crosses a unit frame to a T0 catch; the JumpLoop poll stops a unit loop |
//! | (g) fallback | each mismatch ends in its S3 outcome (BYTECODE, SOURCE, LOAD ERROR), a slot conflict and a changed function fall back too, and the program output still equals T0; a bad link record (intrinsic index, external-reference kind) is a LOAD ERROR before anything runs, and a GC follows it |

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Barrier, OnceLock};
use std::time::{Duration, Instant};

use ep9_platform::unwind::native_return_addresses;
use ep9_unit::{Contract, HostInfo, Verdict, offsets};
use ep9_vm::{Args, Cx, DispatchKind, IntrinsicId, JsResult, Local, NativeCx, RuntimeTable, Value};

use super::build::{UnitSpec, build};
use super::emit::SLOTS;
use super::format::{FnKind, Link, UnitView, rehash, section, spike_host};
use super::load::{
    LoadKind, LoadOptions, Loaded, Mapped, UnitSession, load, mapped_units, self_test_result,
};
use super::scan::{self, Live};
use crate::emit::Arch;
use crate::harness::{Config, RunOutput, run_program};
use crate::structural::{LOOP_DEFS, MIXED_DEFS};

/// How big the runs are: the release gate binary, or the debug test build (debug threaded
/// T0 makes a real call per instruction).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Gate,
    Test,
}

impl Scale {
    fn n(self, stress: bool) -> u32 {
        match (self, stress) {
            (Scale::Gate, false) => 100_000,
            (Scale::Gate, true) => 400,
            (Scale::Test, false) => 300,
            (Scale::Test, true) => 40,
        }
    }
}

pub type CheckFn = fn(Scale) -> Result<Vec<String>, String>;

pub struct Check {
    pub name: &'static str,
    pub title: &'static str,
    pub run: CheckFn,
}

pub const CHECKS: [Check; 7] = [
    Check {
        name: "a",
        title: "units build for AArch64 and x86-64; the text scan finds 0 relocations; header outcomes",
        run: check_build,
    },
    Check {
        name: "b",
        title: "LOAD maps the text once, unchanged; unit code output equals T0 (default, eager, GC stress)",
        run: check_load,
    },
    Check {
        name: "c",
        title: "unit text reads its RW copy through the unit-data register; strong link cells, weak IC cells",
        run: check_cells,
    },
    Check {
        name: "d",
        title: "2 isolates on 2 threads run one text with their own RW copies (context-sync handshake)",
        run: check_threads,
    },
    Check {
        name: "e",
        title: "2 units in one isolate, loaded in both orders, reach their own RW copies",
        run: check_multi,
    },
    Check {
        name: "f",
        title: "unwind self-test on unit text; a native walk and an exception cross unit frames; poll",
        run: check_unwind,
    },
    Check {
        name: "g",
        title: "every mismatch falls back to its S3 outcome and the output still equals T0",
        run: check_fallback,
    },
];

/// Why this target has no unit code (no code generator), or `None`.
pub fn refusal() -> Option<String> {
    match Arch::HOST {
        Some(_) => None,
        None => Some(crate::emit::Refusal::Arch.to_string()),
    }
}

/// The host architecture (the checks run only where [`refusal`] is `None`).
fn host_arch() -> Arch {
    Arch::HOST.expect("refusal() checked it")
}

fn other_arch() -> Arch {
    match host_arch() {
        Arch::Aarch64 => Arch::X86_64,
        Arch::X86_64 => Arch::Aarch64,
    }
}

const STACK: usize = if cfg!(debug_assertions) {
    512 << 20
} else {
    64 << 20
};

/// Runs `check` on a thread with a big stack; a panic is a failure.
pub fn on_thread(check: CheckFn, scale: Scale) -> Result<Vec<String>, String> {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || check(scale))
        .map_err(|e| e.to_string())?
        .join()
        .unwrap_or_else(|p| {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            Err(format!("panicked: {msg}"))
        })
}

// ---- the units -------------------------------------------------------------------------------

/// Unit A (slot 0): the protocol loop kernel; `id` and `drive` get native code.
pub fn unit_a() -> UnitSpec {
    UnitSpec {
        name: "A",
        source: LOOP_DEFS.to_string(),
        slot: 0,
        native: vec!["id", "drive"],
        links: vec![
            Link::Intrinsic(IntrinsicId::ObjectPrototype as u16),
            Link::Str("sp-unit A".into()),
        ],
    }
}

/// Unit B (slot 1): the mixed recursion kernel; `jcall`, `twice`, `inc`, `lt` get native
/// code.
pub fn unit_b() -> UnitSpec {
    UnitSpec {
        name: "B",
        source: MIXED_DEFS.to_string(),
        slot: 1,
        native: vec!["jcall", "twice", "inc", "lt"],
        links: vec![
            Link::Intrinsic(IntrinsicId::FunctionPrototype as u16),
            Link::Str("sp-unit B".into()),
        ],
    }
}

/// Unit A at another slot with another constant (other content, its own mapping).
fn unit_a_at(slot: u32, tag: &str) -> UnitSpec {
    UnitSpec {
        name: "A'",
        slot,
        links: vec![
            Link::Intrinsic(IntrinsicId::ObjectPrototype as u16),
            Link::Str(format!("sp-unit A {tag}")),
        ],
        ..unit_a()
    }
}

/// The host-architecture bytes of units A and B, built once per process.
fn host_units() -> &'static (Vec<u8>, Vec<u8>) {
    static U: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();
    U.get_or_init(|| {
        let a = build(&unit_a(), host_arch()).expect("unit A");
        let b = build(&unit_b(), host_arch()).expect("unit B");
        (a, b)
    })
}

/// The main program of unit A's checks.
fn main_a(n: u32) -> String {
    format!(
        "print(drive(id, {n}));\nprint(drive(id, 0));\nprint(drive(half, 10));\nprint(drive(str, 3));\n"
    )
}

/// The main program of unit B's checks.
fn main_b(depth: u32) -> String {
    format!(
        "print(tdown({depth}));\nprint(twice(inc, 40), twice(inc, 2147483646), twice(inc, '1'));\n\
         print(lt(1, 2), lt(2, 1), lt('a', 'b'));\nprint(ttwice(5));\n"
    )
}

/// The T0 output of `defs` then `main` (no native code).
fn t0(config: &Config, defs: &str, main: &str) -> Result<Vec<String>, String> {
    let r = run_program(config, defs, &[], main)?;
    expect_ok(&r, "T0 run")?;
    Ok(r.output)
}

fn expect_ok(r: &RunOutput, what: &str) -> Result<(), String> {
    match &r.result {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

/// A session of `config` with `units` loaded on the spike host (each must give LOAD).
fn session_with(config: &Config, units: &[&[u8]]) -> Result<UnitSession, String> {
    let mut us = UnitSession::new(config)?;
    for (i, u) in units.iter().enumerate() {
        let l = load(&mut us, u, &spike_host(), &LoadOptions::default());
        expect_native(&l, &format!("unit {i}"))?;
    }
    Ok(us)
}

fn expect_native(l: &Loaded, what: &str) -> Result<(), String> {
    match &l.kind {
        LoadKind::Native { t0, .. } if t0.is_empty() => {
            expect_ok(l.run.as_ref().ok_or("no run")?, what)
        }
        k => Err(format!("{what}: {} ({k:?}), want LOAD", k.name())),
    }
}

/// The mapping of the unit `bytes` in this process.
fn mapping_of(bytes: &[u8]) -> Result<&'static Mapped, String> {
    let view = UnitView::parse(bytes)?;
    mapped_units()
        .into_iter()
        .find(|m| m.hash == view.header.content_hash)
        .ok_or_else(|| "the unit is not mapped".into())
}

/// Calls the unit text's cell reader: word `word` of this isolate's RW copy of `m`'s unit,
/// read through the unit-data register.
fn read_cell(us: &mut UnitSession, m: &Mapped, word: usize) -> Result<u64, String> {
    let rw = us.units.rw(m.slot).ok_or("the unit is not linked here")?;
    if word >= rw.layout.words() {
        return Err(format!("word {word} >= {}", rw.layout.words()));
    }
    let f = m.function(FnKind::Cell, "").ok_or("no cell reader")?;
    #[cfg(target_arch = "x86_64")]
    type CellFn = unsafe extern "sysv64" fn(*mut NativeCx, u64) -> u64;
    #[cfg(not(target_arch = "x86_64"))]
    type CellFn = unsafe extern "C" fn(*mut NativeCx, u64) -> u64;
    let cx = NativeCx::of(&mut us.s.iso);
    // SAFETY: the cell reader is unit code with this ABI (`a64::cell_reader`,
    // `x64::cell_reader`), mapped for the process; it reads `cx.unit_data[slot][word]`, and
    // the unit is linked in this isolate (its directory word is set) and `word` is in range.
    unsafe {
        let f = std::mem::transmute::<*const (), CellFn>(std::ptr::with_exposed_provenance(
            m.code(f).start,
        ));
        Ok(f(cx, word as u64))
    }
}

/// The slot kind of the global function `name`.
fn kind(us: &UnitSession, name: &str) -> Option<DispatchKind> {
    let f = NativeCx::global_value(&us.s.iso, name)?;
    ep9_vm::native_target(&us.s.iso, f)?.kind
}

/// A Value read from a root cell.
fn val(bits: u64) -> Value {
    // SAFETY: the bits come from a link or IC cell, a root range that holds Value bits only
    // (never 0), read while the isolate is idle.
    unsafe { Value::from_bits_unchecked(bits) }
}

// ---- (a) build -------------------------------------------------------------------------------

/// The live addresses of a session with unit A loaded (for the window scan).
fn live_addresses(us: &mut UnitSession, m: &Mapped) -> Live {
    let mut l = Live::default();
    let cx = NativeCx::of(&mut us.s.iso);
    l.point("the Cx", cx.addr());
    let rt = RuntimeTable::process();
    l.point("the runtime table", std::ptr::from_ref(rt).addr());
    for (n, f) in [
        ("poll", rt.poll as usize),
        ("call_any", rt.call_any as usize),
        ("add", rt.add as usize),
        ("unwind", rt.unwind as usize),
    ] {
        l.point(n, f);
    }
    for name in ["id", "drive", "half", "str"] {
        if let Some(f) = NativeCx::global_value(&us.s.iso, name) {
            l.point("a function cell", f.to_bits() as usize);
            if let Some(t) = ep9_vm::native_target(&us.s.iso, f) {
                l.point("a FunctionInfo", t.info);
                l.point("bytecode", t.bytecode.as_ptr().addr());
            }
        }
    }
    if let Some(rw) = us.units.rw(m.slot) {
        l.point("the RW copy", rw.base());
        l.point("a link cell value", rw.link(0) as usize);
    }
    l.point("the unit directory", us.units.directory());
    l.range("the unit text", m.text.clone());
    if let Ok(r) = ep9_platform::CodeRange::process() {
        l.range("the code range", r.range());
    }
    l
}

fn check_build(_: Scale) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let (a_host, _) = host_units();
    // A live session with unit A, for the addresses the scan looks for.
    let mut us = session_with(&Config::default(), &[a_host])?;
    let m = mapping_of(a_host)?;
    let live = live_addresses(&mut us, m);
    for arch in [Arch::Aarch64, Arch::X86_64] {
        for spec in [unit_a(), unit_b()] {
            let bytes = build(&spec, arch)?;
            if build(&spec, arch)? != bytes {
                return Err(format!(
                    "unit {} ({}): two builds differ",
                    spec.name,
                    arch.name()
                ));
            }
            let v = UnitView::parse(&bytes)?;
            let mut findings = scan::windows(v.text, &live);
            if arch == Arch::Aarch64 {
                let fns: Vec<_> = v
                    .functions
                    .iter()
                    .map(|f| f.text.start as usize..f.text.end as usize)
                    .collect();
                findings.extend(scan::aarch64(v.text, &fns, &live));
            }
            if !findings.is_empty() {
                return Err(format!(
                    "unit {} ({}): {} relocation findings: {findings:?}",
                    spec.name,
                    arch.name(),
                    findings.len()
                ));
            }
            let spike = ep9_unit::verdict(&bytes, &spike_host());
            let real = ep9_unit::verdict(&bytes, &HostInfo::current());
            let want_spike = if arch == host_arch() {
                matches!(spike, Verdict::Load)
            } else {
                matches!(spike, Verdict::Target { .. })
            };
            // This engine: the spike's deopt-metadata version differs (BYTECODE); a header
            // with this engine's versions has no native form (BYTECODE).
            let own = with_u32(&bytes, Contract::DeoptMeta.offset(), |_| 0);
            let own = ep9_unit::verdict(&own, &HostInfo::current());
            let real_ok = matches!(
                real,
                Verdict::Version {
                    contract: Contract::DeoptMeta,
                    ..
                }
            ) && (arch != host_arch()
                || matches!(own, Verdict::NoNativeForm(Contract::DeoptMeta)));
            if !want_spike || !real_ok {
                return Err(format!(
                    "unit {} ({}): spike host {spike}, real host {real}",
                    spec.name,
                    arch.name()
                ));
            }
            let js = v.functions.iter().filter(|f| f.kind == FnKind::Js).count();
            out.push(format!(
                "unit {} {:7}: {} bytes, text {} bytes ({js} JS functions + probe + cell \
                 reader), 0 relocation findings ({} windows{}); spike host: {spike}; this \
                 engine: {real}",
                spec.name,
                arch.name(),
                bytes.len(),
                v.text.len(),
                v.text.len().saturating_sub(7),
                if arch == Arch::Aarch64 {
                    format!(", {} instructions decoded", v.text.len() / 4)
                } else {
                    String::new()
                },
            ));
        }
    }
    out.push(format!(
        "scan: {} live addresses (+/- {} KiB) and {} ranges of this process",
        live.points.len(),
        scan::MARGIN >> 10,
        live.ranges.len()
    ));
    Ok(out)
}

// ---- (b) load --------------------------------------------------------------------------------

fn check_load(scale: Scale) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let (a, _) = host_units();
    let view = UnitView::parse(a)?;
    let mut text = None;
    for (cname, config) in Config::check_a() {
        let stress = config.gc_stress != 0;
        let n = scale.n(stress);
        let mut us = UnitSession::new(&config)?;
        let l = load(&mut us, a, &spike_host(), &LoadOptions::default());
        expect_native(&l, "unit A")?;
        let LoadKind::Native { installed, .. } = &l.kind else {
            unreachable!()
        };
        for name in ["id", "drive"] {
            if kind(&us, name) != Some(DispatchKind::Opt) {
                return Err(format!("{name}: slot {:?}, want Opt", kind(&us, name)));
            }
        }
        let m = mapping_of(a)?;
        // SAFETY: the mapping is executable and never unmapped; it holds the text.
        let mapped = unsafe { std::slice::from_raw_parts(m.text.start as *const u8, m.text.len()) };
        if mapped != view.text {
            return Err("the mapped text differs from the file".into());
        }
        if *text.get_or_insert(m.text.clone()) != m.text {
            return Err("unit A was mapped twice in one process".into());
        }
        let main = main_a(n);
        let r = us.s.run(&main, "main.js");
        expect_ok(&r, "unit run")?;
        let want = t0(&config, LOOP_DEFS, &main)?;
        if r.output != want {
            return Err(format!("{cname}: unit {:?} != T0 {want:?}", r.output));
        }
        us.s.iso.collect_garbage();
        us.s.iso
            .verify_heap()
            .map_err(|e| format!("heap verifier: {:?}", e.errors))?;
        let rw = us.units.rw(0).ok_or("no RW copy")?;
        let f = |name: &str| view.function(FnKind::Js, name).cloned().ok_or("no fn");
        let (fd, fi) = (f("drive")?, f("id")?);
        let (cd, ci) = (rw.counter(fd.counter), rw.counter(fi.counter));
        let id_bits = NativeCx::global_value(&us.s.iso, "id")
            .ok_or("no id")?
            .to_bits();
        if cd != 4 || ci != u64::from(n) || rw.ic(fd.ic_base) != id_bits {
            return Err(format!(
                "{cname}: counters drive {cd} (want 4), id {ci} (want {n}); IC cell {:#x}, \
                 want id {id_bits:#x}",
                rw.ic(fd.ic_base)
            ));
        }
        out.push(format!(
            "{cname:22} installed {installed:?} (Opt slots); output = T0: {}; counters: drive \
             {cd}, id {ci}; drive's IC cell = id",
            r.output.join(" | ")
        ));
    }
    let t = text.ok_or("no run")?;
    out.push(format!(
        "unit A text: {} bytes at one address for the 3 isolates; mapped bytes = file bytes",
        t.len()
    ));
    Ok(out)
}

// ---- (c) cells -------------------------------------------------------------------------------

fn check_cells(_: Scale) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let (a, _) = host_units();
    let config = Config {
        zap: true,
        verify: true,
        ..Config::default()
    };
    let mut us = session_with(&config, &[a])?;
    let m = mapping_of(a)?;
    let view = UnitView::parse(a)?;
    let l = view.rw.layout;
    let rw = us.units.rw(0).ok_or("no RW copy")?;
    if rw.roots() != 2 {
        return Err(format!("{} root ranges, want 2", rw.roots()));
    }
    let proto = NativeCx::intrinsic(&us.s.iso, IntrinsicId::ObjectPrototype).to_bits();
    // Every word through the unit text equals the copy; link 0 is this realm's
    // %Object.prototype%, ext 0 the runtime table.
    for w in 0..l.words() {
        let via_text = read_cell(&mut us, m, w)?;
        let direct = us.units.rw(0).ok_or("no RW copy")?.word(w);
        if via_text != direct {
            return Err(format!(
                "word {w}: unit text read {via_text:#x}, the copy holds {direct:#x}"
            ));
        }
    }
    let link0 = read_cell(&mut us, m, l.link_base())?;
    let ext0 = read_cell(&mut us, m, 0)?;
    if link0 != proto || ext0 != std::ptr::from_ref(RuntimeTable::process()).addr() as u64 {
        return Err(format!(
            "link 0 {link0:#x} (want {proto:#x}), ext 0 {ext0:#x}"
        ));
    }
    // The strong range keeps the unit constant (no other reference) across collections.
    for _ in 0..3 {
        us.s.iso.collect_garbage();
    }
    us.s.iso
        .verify_heap()
        .map_err(|e| format!("heap verifier: {:?}", e.errors))?;
    let s_bits = read_cell(&mut us, m, l.link_base() + 1)?;
    let shown =
        us.s.iso
            .string_view(val(s_bits))
            .map_or_else(|| format!("{:?}", val(s_bits)), |s| s.to_string_lossy());

    if shown != "sp-unit A" {
        return Err(format!(
            "the unit constant reads {shown:?} after 3 collections"
        ));
    }
    out.push(format!(
        "{} RW words through the unit text = the copy; link 0 = %Object.prototype% \
         (intrinsic {}), ext 0 = the runtime table; 2 root ranges",
        l.words(),
        IntrinsicId::ObjectPrototype as u16
    ));
    out.push(format!(
        "link 1 (unit constant, strong) after 3 collections: {shown:?}"
    ));
    // The weak IC cell: a callee that dies is cleared; a live one stays.
    let site = view
        .function(FnKind::Js, "drive")
        .ok_or("no drive")?
        .ic_base;
    us.s.run(
        "function once() { return drive(function (x) { return x; }, 3); }",
        "once.js",
    )
    .result?;
    let mut cleared = None;
    for k in 0..8 {
        let (h, masked) = fill_ic(&mut us, site, k)?;
        scrub_stack();
        us.s.iso.collect_garbage();
        let alive = us.s.iso.heap().weak(h).is_some();
        us.s.iso.heap().drop_weak(h);
        let after = us.units.rw(0).ok_or("no RW copy")?.ic(site);
        let undefined = after == ep9_value::encoding::VALUE_UNDEFINED;
        if alive != (after == masked ^ MASK) || (!alive && !undefined) {
            return Err(format!(
                "try {k}: weak handle alive {alive}, IC cell {:#x} -> {after:#x}",
                masked ^ MASK
            ));
        }
        if !alive {
            cleared = Some(k);
            break;
        }
    }
    let k = cleared.ok_or("the dead closure stayed in the IC cell for 8 collections")?;
    let r = us.s.run("print(drive(id, 2));", "main.js");
    expect_ok(&r, "drive(id, 2)")?;
    let id = NativeCx::global_value(&us.s.iso, "id")
        .ok_or("no id")?
        .to_bits();
    if us.units.rw(0).ok_or("no RW copy")?.ic(site) != id {
        return Err("the cleared IC cell was not filled again".into());
    }
    out.push(format!(
        "IC cell (weak): the dead closure was cleared to undefined by collection {} (the \
         weak handle agrees), then drive(id, 2) filled it with id",
        k + 1
    ));
    Ok(out)
}

/// Hides cell bits from the conservative stack scan while a test holds them.
const MASK: u64 = 0x5A5A_0000_0000_0005;

/// Runs `once()` (drive with a new closure) and returns a weak handle to the closure that
/// drive's IC cell now holds, and the cell's bits XOR [`MASK`] (so no copy of the bits on
/// this thread's stack keeps the closure alive in the conservative scan).
#[inline(never)]
fn fill_ic(us: &mut UnitSession, site: u32, k: u32) -> Result<(ep9_heap::WeakHandle, u64), String> {
    let r = us.s.run("print(once());", "main.js");
    expect_ok(&r, "once()")?;
    let ic = us.units.rw(0).ok_or("no RW copy")?.ic(site);
    let id = NativeCx::global_value(&us.s.iso, "id")
        .ok_or("no id")?
        .to_bits();
    if ic == ep9_value::encoding::VALUE_UNDEFINED || ic == id {
        return Err(format!(
            "try {k}: IC cell {ic:#x} after once(), want the closure"
        ));
    }
    let h = us.s.iso.heap().new_weak(val(ic));
    Ok((h, std::hint::black_box(ic ^ MASK)))
}

/// Overwrites stack below the caller, so stale words of returned frames do not keep a cell
/// alive in the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    std::hint::black_box(&mut words);
}

// ---- (d) threads -----------------------------------------------------------------------------

/// Calls of each runtime-table entry through the counting table.
static COUNTS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];

macro_rules! counting {
    ($($name:ident = $idx:literal, $field:ident, ($($a:ident: $t:ty),*);)*) => {$(
        #[cfg(target_arch = "x86_64")]
        unsafe extern "sysv64" fn $name($($a: $t),*) -> u64 {
            COUNTS[$idx].fetch_add(1, Relaxed);
            // SAFETY: the same arguments native code passed, to the real entry.
            unsafe { (RuntimeTable::process().$field)($($a),*) }
        }
        #[cfg(not(target_arch = "x86_64"))]
        unsafe extern "C" fn $name($($a: $t),*) -> u64 {
            COUNTS[$idx].fetch_add(1, Relaxed);
            // SAFETY: the same arguments native code passed, to the real entry.
            unsafe { (RuntimeTable::process().$field)($($a),*) }
        }
    )*};
}

counting! {
    c_poll = 0, poll, (cx: *mut NativeCx, fp: *mut u64, off: u32);
    c_call_any = 1, call_any, (cx: *mut NativeCx, fp: *mut u64, off: u32, v: u64);
    c_add = 2, add, (cx: *mut NativeCx, fp: *mut u64, off: u32, l: u64, r: u64);
    c_inc = 3, inc, (cx: *mut NativeCx, fp: *mut u64, off: u32, v: u64);
    c_compare = 4, compare, (cx: *mut NativeCx, fp: *mut u64, off: u32, op: u32, l: u64, r: u64);
    c_stack_overflow = 5, stack_overflow, (cx: *mut NativeCx, fp: *mut u64, off: u32);
    c_throw = 6, throw, (cx: *mut NativeCx, fp: *mut u64, off: u32, v: u64);
    c_unwind = 7, unwind, (cx: *mut NativeCx, fp: *mut u64, off: u32);
    c_get_keyed = 8, get_keyed, (cx: *mut NativeCx, fp: *mut u64, off: u32, o: u64, k: u64);
    c_set_keyed = 9, set_keyed, (cx: *mut NativeCx, fp: *mut u64, off: u32, o: u64, k: u64, v: u64);
    c_deopt = 10, deopt, (cx: *mut NativeCx, fp: *mut u64, off: u32, dump: u64, native_fp: u64);
}

/// A runtime table that counts each call, then runs the process table's entry.
static COUNTING: RuntimeTable = RuntimeTable {
    poll: c_poll,
    call_any: c_call_any,
    add: c_add,
    inc: c_inc,
    compare: c_compare,
    stack_overflow: c_stack_overflow,
    throw: c_throw,
    unwind: c_unwind,
    get_keyed: c_get_keyed,
    set_keyed: c_set_keyed,
    deopt: c_deopt,
};

/// What one thread of check (d) saw.
#[derive(Debug)]
struct ThreadRun {
    output: Vec<String>,
    text: std::ops::Range<usize>,
    link0: u64,
    proto: u64,
    rw_base: usize,
    drive_calls: u64,
}

fn thread_run(
    bytes: Arc<Vec<u8>>,
    opts: LoadOptions,
    mapped: Arc<Barrier>,
    first: bool,
    start: Arc<Barrier>,
    main: String,
) -> Result<ThreadRun, String> {
    let mut us = UnitSession::new(&Config::default())?;
    if !first {
        // The handshake: thread 1 has mapped and flushed the text (its window closed).
        mapped.wait();
    }
    let l = load(&mut us, &bytes, &spike_host(), &opts);
    if first {
        mapped.wait();
    }
    expect_native(&l, "unit A'")?;
    let m = mapping_of(&bytes)?;
    let view = UnitView::parse(&bytes)?;
    let link0 = read_cell(&mut us, m, view.rw.layout.link_base())?;
    let proto = NativeCx::intrinsic(&us.s.iso, IntrinsicId::ObjectPrototype).to_bits();
    start.wait();
    let r = us.s.run(&main, "main.js");
    expect_ok(&r, "unit run")?;
    let rw = us.units.rw(view.slot).ok_or("no RW copy")?;
    let fd = view.function(FnKind::Js, "drive").ok_or("no drive")?;
    Ok(ThreadRun {
        output: r.output,
        text: m.text.clone(),
        link0,
        proto,
        rw_base: rw.base(),
        drive_calls: rw.counter(fd.counter),
    })
}

fn check_threads(scale: Scale) -> Result<Vec<String>, String> {
    // Its own unit (slot 2): thread 1 maps it here, thread 2 runs what thread 1 wrote.
    let bytes = Arc::new(build(&unit_a_at(2, "threads"), host_arch())?);
    if mapping_of(&bytes).is_ok() {
        return Err("the unit of check (d) is mapped already".into());
    }
    for c in &COUNTS {
        c.store(0, Relaxed);
    }
    let (n1, n2) = (scale.n(false), scale.n(false) / 2 + 7);
    let mapped = Arc::new(Barrier::new(2));
    let start = Arc::new(Barrier::new(2));
    let spawn = |opts: LoadOptions, first: bool, n: u32| {
        let (b, m, s) = (bytes.clone(), mapped.clone(), start.clone());
        let main = format!("print(drive(id, {n}));\n");
        std::thread::Builder::new()
            .stack_size(STACK)
            .spawn(move || thread_run(b, opts, m, first, s, main))
            .map_err(|e| e.to_string())
    };
    let t1 = spawn(LoadOptions::default(), true, n1)?;
    let t2 = spawn(
        LoadOptions {
            runtime_table: Some(&COUNTING),
        },
        false,
        n2,
    )?;
    let r1 = t1.join().map_err(|_| "thread 1 panicked")??;
    let r2 = t2.join().map_err(|_| "thread 2 panicked")??;
    if r1.text != r2.text {
        return Err("the two isolates ran two copies of the text".into());
    }
    if r1.output != [n1.to_string()] || r2.output != [n2.to_string()] {
        return Err(format!("outputs {:?} {:?}", r1.output, r2.output));
    }
    if r1.link0 != r1.proto || r2.link0 != r2.proto || r1.link0 == r2.link0 {
        return Err(format!("link 0 per isolate: {r1:?} {r2:?}"));
    }
    let calls = COUNTS[1].load(Relaxed);
    if calls != u64::from(n2) || r1.drive_calls != 1 || r2.drive_calls != 1 {
        return Err(format!(
            "counting table: {calls} call_any calls, want {n2} (isolate 2 only); drive \
             counters {} {}",
            r1.drive_calls, r2.drive_calls
        ));
    }
    if r1.rw_base == r2.rw_base {
        return Err("one RW copy for two isolates".into());
    }
    Ok(vec![
        format!(
            "thread 1 mapped unit A' (slot 2), then thread 2 ran it after the handshake (isb / \
             cpuid); one text at {:#x}..{:#x}",
            r1.text.start, r1.text.end
        ),
        format!(
            "isolate 1: drive(id, {n1}) = {}; isolate 2: drive(id, {n2}) = {}; each link 0 = its \
             own %Object.prototype% ({:#x} != {:#x})",
            r1.output[0], r2.output[0], r1.link0, r2.link0
        ),
        format!(
            "isolate 2's runtime-table cell names the counting table: {calls} call_any calls \
             = its {n2} calls; isolate 1's calls did not go through it"
        ),
    ])
}

// ---- (e) multi -------------------------------------------------------------------------------

fn check_multi(scale: Scale) -> Result<Vec<String>, String> {
    let (a, b) = host_units();
    let n = scale.n(false) / 10;
    let main = format!("{}{}", main_a(n), main_b(30));
    let defs = format!("{LOOP_DEFS}{MIXED_DEFS}");
    let want = t0(&Config::default(), &defs, &main)?;
    let mut out = Vec::new();
    for (order, units) in [("A, B", [&a[..], &b[..]]), ("B, A", [&b[..], &a[..]])] {
        let mut us = session_with(&Config::default(), &units)?;
        let r = us.s.run(&main, "main.js");
        expect_ok(&r, "unit run")?;
        if r.output != want {
            return Err(format!("{order}: {:?} != T0 {want:?}", r.output));
        }
        let (ma, mb) = (mapping_of(a)?, mapping_of(b)?);
        let (va, vb) = (UnitView::parse(a)?, UnitView::parse(b)?);
        let fp = NativeCx::intrinsic(&us.s.iso, IntrinsicId::FunctionPrototype).to_bits();
        let op = NativeCx::intrinsic(&us.s.iso, IntrinsicId::ObjectPrototype).to_bits();
        let la = read_cell(&mut us, ma, va.rw.layout.link_base())?;
        let lb = read_cell(&mut us, mb, vb.rw.layout.link_base())?;
        if la != op || lb != fp {
            return Err(format!("{order}: link 0 of A {la:#x}, of B {lb:#x}"));
        }
        let (d0, d1) = (us.units.directory_word(0), us.units.directory_word(1));
        if d0 == 0 || d1 == 0 || d0 == d1 {
            return Err(format!("{order}: directory words {d0:#x} {d1:#x}"));
        }
        let rwa = us.units.rw(0).ok_or("no A")?;
        let rwb = us.units.rw(1).ok_or("no B")?;
        let c = |v: &UnitView<'_>, rw: &super::load::RwCopy, n: &str| {
            v.function(FnKind::Js, n).map(|f| rw.counter(f.counter))
        };
        let (drive, twice) = (c(&va, rwa, "drive"), c(&vb, rwb, "twice"));
        if drive != Some(4) || twice != Some(8) {
            return Err(format!(
                "{order}: counters drive {drive:?}, twice {twice:?}"
            ));
        }
        out.push(format!(
            "load order {order}: output = T0 ({} lines); slot 0 -> A's copy (link 0 = \
             %Object.prototype%), slot 1 -> B's copy (link 0 = %Function.prototype%); \
             counters drive 4, twice 8",
            r.output.len()
        ));
    }
    Ok(out)
}

// ---- (f) unwind ------------------------------------------------------------------------------

thread_local! {
    static WALK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// `bt(x)`: records a native stack walk from here.
fn bt<'s>(_cx: &mut Cx<'s>, _t: Local<'s>, _a: Args<'s>, _n: Local<'s>) -> JsResult<Local<'s>> {
    let ips = native_return_addresses(4096);
    WALK.with(|w| *w.borrow_mut() = ips);
    Ok(Local::undefined())
}

#[inline(never)]
fn outer_walk() -> Vec<usize> {
    native_return_addresses(4096)
}

fn in_unit_text(pc: usize) -> bool {
    mapped_units().iter().any(|m| m.text.contains(&pc))
}

#[inline(never)]
fn check_unwind(scale: Scale) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let (a, _) = host_units();
    let mut us = session_with(&Config::default(), &[a])?;
    match self_test_result() {
        Some(Ok(mode)) => out.push(format!(
            "unwind self-test on the first mapped unit's probe: {mode}"
        )),
        Some(Err(e)) => return Err(format!("unwind self-test: {e}")),
        None => return Err("no unit mapped, no self-test".into()),
    }
    // A walk from a host function that unit code (drive) called.
    us.s.iso.register_host_function("bt", 1, bt);
    let outer = outer_walk();
    let r = us.s.run("drive(bt, 1);\n", "main.js");
    expect_ok(&r, "drive(bt, 1)")?;
    let inner = WALK.with(|w| std::mem::take(&mut *w.borrow_mut()));
    let unit: Vec<usize> = (0..inner.len())
        .filter(|&i| in_unit_text(inner[i]))
        .collect();
    let common = inner
        .iter()
        .rev()
        .zip(outer.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let last = *unit
        .last()
        .ok_or_else(|| format!("no unit frame in the walk ({} frames)", inner.len()))?;
    if common < 2 || inner.len() - common <= last {
        return Err(format!(
            "the walk stops at the unit frame: {} frames, unit at {unit:?}, {common} shared",
            inner.len()
        ));
    }
    out.push(format!(
        "walk from bt(): {} frames, unit frames at {unit:?}, then {} more; the last {common} \
         equal the frames that called the run",
        inner.len(),
        inner.len() - last - 1
    ));
    // An exception thrown under a unit frame reaches a T0 catch.
    let defs = "function boom(x) { throw new Error('b' + x); }\n";
    let main = "try { drive(boom, 3); } catch (e) { print('caught ' + e.message); }\n\
                print(drive(id, 5));\n";
    us.s.run(defs, "boom.js").result?;
    let r = us.s.run(main, "main.js");
    expect_ok(&r, "throw")?;
    let want = t0(&Config::default(), &format!("{LOOP_DEFS}{defs}"), main)?;
    if r.output != want {
        return Err(format!("exception: {:?} != T0 {want:?}", r.output));
    }
    out.push(format!(
        "exception across drive (unit): {}",
        r.output.join(" | ")
    ));
    // The JumpLoop poll stops a unit loop.
    let h = us.s.iso.terminate_handle();
    let delay = Duration::from_millis(if scale == Scale::Gate { 200 } else { 100 });
    let t = std::thread::spawn(move || {
        std::thread::sleep(delay);
        h.terminate();
    });
    let start = Instant::now();
    let r = us.s.run("drive(id, 2000000000);", "spin.js");
    let took = start.elapsed();
    t.join().map_err(|_| "terminator thread panicked")?;
    match &r.result {
        Err(e) if e.contains("terminated") => {}
        other => return Err(format!("drive(id, 2e9) was not terminated: {other:?}")),
    }
    let again = us.s.run("print(drive(id, 5));", "again.js");
    expect_ok(&again, "after the termination")?;
    if again.output != ["5"] {
        return Err(format!("after the termination: {:?}", again.output));
    }
    out.push(format!(
        "drive(id, 2e9) in unit code terminated {took:?} after the start (request at \
         {delay:?}); drive(id, 5) = 5 after it"
    ));
    Ok(out)
}

// ---- (g) fallback ----------------------------------------------------------------------------

/// `bytes` with `f` applied to its header field at `off` (little-endian u32), rehashed.
fn with_u32(bytes: &[u8], off: usize, f: impl Fn(u32) -> u32) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let v = u32::from_le_bytes(b[off..off + 4].try_into().unwrap());
    b[off..off + 4].copy_from_slice(&f(v).to_le_bytes());
    rehash(&mut b);
    b
}

/// The offset in `bytes` of byte `at` of its RW template section.
fn rw_offset(bytes: &[u8], at: usize) -> Result<usize, String> {
    let t = section(bytes, ep9_unit::SectionKind::RwTemplate)?.ok_or("no RW template")?;
    Ok(t.as_ptr().addr() - bytes.as_ptr().addr() + at)
}

/// RW template bytes: "SPUW", version, slot, ext, links, ics, counters (7 words), then the
/// external-reference kinds (unit A: 1), then link 0 (kind, intrinsic index).
const RW_EXT0: usize = 28;
const RW_LINK0_INTRINSIC: usize = 36;

/// One fallback case: its bytes, the outcome name it must give, and a word its reason holds.
struct Case {
    name: &'static str,
    bytes: Vec<u8>,
    want: &'static str,
    reason: &'static str,
    host: HostInfo,
}

fn check_fallback(scale: Scale) -> Result<Vec<String>, String> {
    let (a, _) = host_units();
    let spike = spike_host();
    let real = HostInfo::current();
    let n = scale.n(true);
    let main = main_a(n);
    let want_out = t0(&Config::default(), LOOP_DEFS, &main)?;
    let mut cases = vec![
        Case {
            name: "this engine (DEOPT_META 0)",
            bytes: a.clone(),
            want: "BYTECODE",
            reason: "deopt_meta",
            host: real,
        },
        Case {
            name: "this engine's header (DEOPT_META 0)",
            bytes: with_u32(a, Contract::DeoptMeta.offset(), |_| 0),
            want: "BYTECODE",
            reason: "no native form",
            host: real,
        },
        Case {
            name: "another target (cross-built text)",
            bytes: build(&unit_a(), other_arch())?,
            want: "BYTECODE",
            reason: "target",
            host: spike,
        },
        Case {
            name: "unknown CPU feature bit",
            bytes: {
                let mut b = a.clone();
                let o = offsets::CPU_REQUIRED;
                b[o..o + 8].copy_from_slice(&(1u64 << 63).to_le_bytes());
                rehash(&mut b);
                b
            },
            want: "BYTECODE",
            reason: "CPU",
            host: spike,
        },
        Case {
            name: "heap page size",
            bytes: {
                let mut b = a.clone();
                b[offsets::HEAP_PAGE_LOG2] += 1;
                rehash(&mut b);
                b
            },
            want: "BYTECODE",
            reason: "heap page",
            host: spike,
        },
        Case {
            name: "engine build id",
            bytes: {
                let mut b = a.clone();
                b[offsets::ENGINE_BUILD_ID] ^= 0x5A;
                rehash(&mut b);
                b
            },
            want: "SOURCE",
            reason: "engine build",
            host: spike,
        },
        Case {
            name: "bad magic",
            bytes: {
                let mut b = a.clone();
                b[1] ^= 0x20;
                b
            },
            want: "LOAD ERROR",
            reason: "magic",
            host: spike,
        },
        Case {
            name: "a text byte changed (hash)",
            bytes: {
                let mut b = a.clone();
                let t = section(&b, ep9_unit::SectionKind::Text)?.ok_or("no text")?;
                let at = t.as_ptr().addr() - b.as_ptr().addr() + 8;
                b[at] ^= 1;
                b
            },
            want: "LOAD ERROR",
            reason: "hash",
            host: spike,
        },
        Case {
            name: "truncated",
            bytes: a[..a.len() - 9].to_vec(),
            want: "LOAD ERROR",
            reason: "size",
            host: spike,
        },
        // The link records: the verdict is LOAD, the unit's link fails. fallback_run shows
        // that nothing ran and that a GC after the error finds no root range of a freed
        // RW copy (SPUNIT-1).
        Case {
            name: "link: intrinsic 9999",
            bytes: with_u32(a, rw_offset(a, RW_LINK0_INTRINSIC)?, |_| 9999),
            want: "LOAD ERROR",
            reason: "no intrinsic 9999",
            host: spike,
        },
        Case {
            name: "link: external reference kind 7",
            bytes: with_u32(a, rw_offset(a, RW_EXT0)?, |_| 7),
            want: "LOAD ERROR",
            reason: "external reference kind 7",
            host: spike,
        },
    ];
    for (c, reason) in [
        (Contract::BytecodeAbi, "source"),
        (Contract::ValueEncoding, "bytecode"),
        (Contract::ObjectLayout, "bytecode"),
        (Contract::Barrier, "bytecode"),
        (Contract::Frame, "bytecode"),
        (Contract::Ic, "bytecode"),
        (Contract::FactSchema, "bytecode"),
        (Contract::BuiltinAbi, "bytecode"),
    ] {
        cases.push(Case {
            name: c.name(),
            bytes: with_u32(a, c.offset(), |v| v + 1),
            want: if c == Contract::BytecodeAbi {
                "SOURCE"
            } else {
                "BYTECODE"
            },
            reason,
            host: spike,
        });
    }
    let mut out = Vec::new();
    for c in cases {
        let mut us = UnitSession::new(&Config::default())?;
        let l = load(&mut us, &c.bytes, &c.host, &LoadOptions::default());
        let reason = match &l.kind {
            LoadKind::Bytecode(r) | LoadKind::Source(r) | LoadKind::LoadError(r) => r.clone(),
            LoadKind::Native { .. } => String::new(),
        };
        if l.kind.name() != c.want || !reason.to_lowercase().contains(&c.reason.to_lowercase()) {
            return Err(format!(
                "{}: {} ({reason}), want {} ({})",
                c.name,
                l.kind.name(),
                c.want,
                c.reason
            ));
        }
        out.push(fallback_run(&mut us, &l, c.name, &main, &want_out)?);
        if c.name == "fact_schema" && !l.verdict.drops_hints() {
            return Err("fact_schema: the hints are not dropped".into());
        }
    }
    // A slot conflict: two units with one slot in one process.
    let x1 = build(&unit_a_at(63, "x1"), host_arch())?;
    let x2 = build(&unit_a_at(63, "x2"), host_arch())?;
    let mut us = UnitSession::new(&Config::default())?;
    expect_native(
        &load(&mut us, &x1, &spike, &LoadOptions::default()),
        "unit at slot 63",
    )?;
    let mut us = UnitSession::new(&Config::default())?;
    let l = load(&mut us, &x2, &spike, &LoadOptions::default());
    match &l.kind {
        LoadKind::Bytecode(r) if r.contains("slot 63") => {}
        k => return Err(format!("slot conflict: {k:?}")),
    }
    out.push(fallback_run(
        &mut us,
        &l,
        "slot conflict",
        &main,
        &want_out,
    )?);
    // A changed function: its table digest no longer matches its bytecode.
    let mut x3 = build(&unit_a_at(62, "x3"), host_arch())?;
    let v = UnitView::parse(&x3)?;
    let d = v.function(FnKind::Js, "drive").ok_or("no drive")?.digest;
    let at = x3
        .windows(32)
        .position(|w| w == d)
        .ok_or("no digest bytes")?;
    x3[at] ^= 1;
    rehash(&mut x3);
    let mut us = UnitSession::new(&Config::default())?;
    let l = load(&mut us, &x3, &spike, &LoadOptions::default());
    match &l.kind {
        LoadKind::Native { installed, t0 }
            if installed == &["id"] && t0.len() == 1 && t0[0].0 == "drive" => {}
        k => return Err(format!("changed function: {k:?}")),
    }
    out.push(fallback_run(
        &mut us,
        &l,
        "changed function",
        &main,
        &want_out,
    )?);
    if SLOTS != 64 {
        return Err(format!("{SLOTS} slots"));
    }
    Ok(out)
}

/// Runs the main program after a fallback load and compares it with T0. After a LOAD ERROR
/// nothing of the unit ran, so `drive` is not defined, the unit has no RW copy, and a GC
/// runs.
fn fallback_run(
    us: &mut UnitSession,
    l: &Loaded,
    name: &str,
    main: &str,
    want: &[String],
) -> Result<String, String> {
    let k = l.kind.name();
    match &l.kind {
        LoadKind::LoadError(r) => {
            if l.run.is_some() || NativeCx::global_value(&us.s.iso, "drive").is_some() {
                return Err(format!("{name}: a section ran after a load error"));
            }
            if us.units.rw(0).is_some() || us.units.directory_word(0) != 0 {
                return Err(format!("{name}: an RW copy stays after a load error"));
            }
            // A GC after the error: no root range may name words the loader freed.
            expect_ok(&us.s.run("gc(); gc();", "gc.js"), name)?;
            Ok(format!("{name:36} -> {k}: {r}; then gc()"))
        }
        _ => {
            expect_ok(l.run.as_ref().ok_or("no run")?, name)?;
            let r = us.s.run(main, "main.js");
            expect_ok(&r, name)?;
            if r.output != want {
                return Err(format!("{name}: {:?} != T0 {want:?}", r.output));
            }
            let natives: Vec<&str> = ["id", "drive"]
                .into_iter()
                .filter(|n| kind(us, n) == Some(DispatchKind::Opt))
                .collect();
            let why = match &l.kind {
                LoadKind::Bytecode(r) | LoadKind::Source(r) => r.clone(),
                LoadKind::Native { t0, .. } => format!("T0: {t0:?}"),
                LoadKind::LoadError(_) => unreachable!(),
            };
            Ok(format!(
                "{name:36} -> {k}: {why}; output = T0; native: {natives:?}"
            ))
        }
    }
}
