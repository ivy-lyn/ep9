//! SP-Facts (WI-SC8, V10-D35; protocol §6.3, condition C5c): config P, the same graph builder
//! fed by a PROFILED FactSnapshot file instead of the live feedback of the run.
//!
//! ```text
//!  training process   spike-c sp-facts write <kernel> <file>
//!    defs.js (stub harness + Math.random + the kernel file) ; warm.js = loop(1), in T0
//!    facts::live (the defs.js unit) ──to_profiled──> encode ──> <file>
//!                     (FACT_SCHEMA_VERSION 1, source PROFILED; the .ep9prof container is A3)
//!  config P           a new process: spike-c sp-facts check <kernel> <file>, spike-c once p
//!    defs.js ; read <file> (decode: another schema version is refused; the source must be
//!      PROFILED; validate vs the defs.js unit: a stale file is refused, a record that does
//!      not fit is dropped and listed)
//!    ; compile the hot set from the file: each hot function the compiler accepts (a refused
//!      one stays in T0 and is named, the config C rule of M5-P2-ORCH-1)
//!    ; install (placement (b), with the exit tables)  ── before the kernel's first run() ──
//!    ; warm.js = loop(1): the start-up. The exits taken here are counted: a shape guard
//!      reads the live IC entry (a CellRef, V10-D35), and T0 records nothing until the
//!      function's first exit, so the first calls exit into T0, which then records
//!    ; loop.js = loop(n) (the timed region in `once p`)
//!  pass               every printed line equal to T0 and to config C; heap verifier clean
//! ```
//!
//! A wrong profile costs exits, never output (R4, MC-w15-18): [`wrong_profiles`] runs config
//! P with files that do not describe the run (another kernel's file, another run of the same
//! unit, the records shuffled across slots of one kind, no records, another schema version, a
//! RUNTIME file) and requires T0's output from each.

use std::process::ExitCode;
use std::time::Instant;

use ep9_facts::{
    BinaryHint, CompareHint, FACT_SCHEMA_VERSION, FactHeader, FactSnapshot, FactSource, SlotFacts,
};
use spike_b::emit::FunctionCode;
use spike_b::harness::{Config, RunOutput};

use crate::harness::{Session, Target};
use crate::kernels::{self, Kernel};
use crate::{CompileReport, facts, tier};

/// The script that holds the kernel's functions: the unit a PROFILED file describes.
pub const DEFS: &str = "defs.js";

/// Where the facts of a compile come from.
#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    /// The live facts of what the session ran so far (config C).
    Live,
    /// A PROFILED snapshot, read and validated against the unit (config P).
    Profiled(&'a FactSnapshot),
}

/// One compiled hot function, not installed yet.
pub struct Compiled {
    pub name: &'static str,
    pub target: Target,
    pub code: FunctionCode,
    pub report: CompileReport,
}

/// The hot set of a kernel as the compiler took it: each hot function compiled, or refused
/// with the reason (it then stays in T0).
pub struct HotSet {
    pub compiled: Vec<Compiled>,
    pub refused: Vec<(&'static str, String)>,
}

impl HotSet {
    /// The names of the compiled functions, in hot order.
    pub fn names(&self) -> Vec<&'static str> {
        self.compiled.iter().map(|c| c.name).collect()
    }

    /// Code bytes, fact guards and unbacked guards of the whole set.
    pub fn code(&self) -> (usize, usize, usize) {
        self.compiled.iter().fold((0, 0, 0), |(b, f, u), c| {
            (
                b + c.report.emit.code_bytes,
                f + c.report.fact_guards,
                u + c.report.unbacked_guards,
            )
        })
    }

    /// One line: what compiled (bytes, guards) and what was refused.
    pub fn line(&self) -> String {
        let c: Vec<String> = self
            .compiled
            .iter()
            .map(|c| {
                format!(
                    "{} ({} bytes, guards {} fact + {} unbacked)",
                    c.name,
                    c.report.emit.code_bytes,
                    c.report.fact_guards,
                    c.report.unbacked_guards
                )
            })
            .collect();
        let r: Vec<String> = self
            .refused
            .iter()
            .map(|(n, e)| format!("{n} ({e})"))
            .collect();
        format!(
            "compiled [{}]; stays in T0 [{}]",
            c.join(", "),
            r.join("; ")
        )
    }

    /// Installs every compiled function (placement (b), with its exit table).
    ///
    /// # Safety
    /// `self` is what [`compile_hot_set`] returned for `s`, with no JS run in `s` since and
    /// no field changed (the code bytes, targets and reports are public, and
    /// [`Session::install_targets`] can check only the function key): each code is then
    /// the placement (b) code `emit_a64` made for its target and each meta its own exit
    /// table, the contract of [`Session::install_targets`].
    ///
    /// ```compile_fail,E0133
    /// fn f(set: spike_c::sp_facts::HotSet, s: &mut spike_c::harness::Session) {
    ///     let _ = set.install(s); // E0133: an unsafe fn
    /// }
    /// ```
    pub unsafe fn install(self, s: &mut Session) -> Result<Installed, String> {
        let mut targets = Vec::new();
        let mut codes = Vec::new();
        let mut metas = Vec::new();
        let mut set = Vec::new();
        for c in self.compiled {
            metas.push(Some(c.report.emit.meta.encode()));
            targets.push(c.target.clone());
            codes.push(c.code);
            set.push((c.name, c.target, c.report));
        }
        if !codes.is_empty() {
            // SAFETY: the caller's contract (# Safety): each code is the placement (b) code
            // `emit_a64` made for its target from `input_of_target` in `compile_hot_set`
            // (which runs no JS between the first input and the last compile, and the caller
            // runs none between the compile and this install), unchanged, and each meta is its
            // own exit table; emit_a64 keeps the (b) contract as for `tier::run_full` (the T0
            // handler ABI, `ep9_vm::layout` offsets proved by `vm_layout::check`, only
            // RuntimeTable calls, callee-saved registers kept, every exit the tail jump to
            // `cx.table[*pc]`).
            unsafe { s.install_targets(&targets, codes, metas) }?;
        }
        for (name, t, _) in &set {
            if s.slot_kind_of(t) != Some(ep9_vm::DispatchKind::Opt) {
                return Err(format!(
                    "{name}: slot kind {:?} after install",
                    s.slot_kind_of(t)
                ));
            }
        }
        Ok(Installed { set })
    }
}

/// The installed hot set: name, target and compile report of each function.
pub struct Installed {
    pub set: Vec<(&'static str, Target, CompileReport)>,
}

impl Installed {
    /// The exit counters of each installed function (record order).
    pub fn counts(&self, s: &Session) -> Result<Vec<Vec<u64>>, String> {
        self.set
            .iter()
            .map(|(n, t, _)| {
                s.exit_counts_of(t)
                    .ok_or_else(|| format!("{n}: no exit counters after the install"))
            })
            .collect()
    }

    /// The sum of every exit counter.
    pub fn total(&self, s: &Session) -> Result<u64, String> {
        Ok(self.counts(s)?.iter().flatten().sum())
    }

    /// The exits between two [`Installed::counts`] readings, as `name: n (guards gA gB)`.
    pub fn exit_line(&self, before: &[Vec<u64>], after: &[Vec<u64>]) -> String {
        let parts: Vec<String> = self
            .set
            .iter()
            .zip(before.iter().zip(after))
            .map(|((name, _, report), (b, a))| {
                let d: Vec<u64> = a.iter().zip(b).map(|(a, b)| a - b).collect();
                let n: u64 = d.iter().sum();
                let g: Vec<String> = tier::fired(report, &d)
                    .iter()
                    .map(|g| format!("g{g}"))
                    .collect();
                if n == 0 {
                    format!("{name}: 0")
                } else {
                    format!("{name}: {n} (guards {})", g.join(" "))
                }
            })
            .collect();
        if parts.is_empty() {
            "nothing installed".into()
        } else {
            parts.join("; ")
        }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.set.iter().map(|(n, _, _)| *n).collect()
    }
}

/// Compiles each hot function of `k` in the script `file` of `s` (the kernel file's line L is
/// line L + `offset` there) from `source`. A function the compiler refuses is listed in
/// [`HotSet::refused`]; any other failure (no such function, no facts) is an error. The
/// function values are taken first (`Session::function_value` runs a one-line script that
/// calls nothing), so no JS runs between the first emitter input and the last compile.
pub fn compile_hot_set(
    s: &mut Session,
    k: &Kernel,
    file: &str,
    offset: u32,
    source: Source<'_>,
) -> Result<HotSet, String> {
    let mut values = Vec::new();
    for hot in k.hot {
        values.push(match hot.reach {
            Some(reach) => Some(s.function_value(reach)?),
            None => None,
        });
    }
    let script = s.script(file).ok_or_else(|| format!("no script {file}"))?;
    let live;
    let snapshot = match source {
        Source::Live => {
            live = facts::live(s, script)?;
            &live
        }
        Source::Profiled(p) => p,
    };
    let mut set = HotSet {
        compiled: Vec::new(),
        refused: Vec::new(),
    };
    for (hot, value) in k.hot.iter().zip(values) {
        let ids = script.functions_on_line(hot.lines.0 + offset);
        let [id] = ids[..] else {
            return Err(format!(
                "{}: {} functions on line {}",
                hot.name,
                ids.len(),
                hot.lines.0
            ));
        };
        let target = match value {
            Some(f) => Target::Value(f),
            None => Target::Id {
                file: file.to_string(),
                id,
            },
        };
        let input = s.input_of_target(&target)?;
        match crate::compile_input(&input, &facts::for_function(snapshot, id)) {
            Ok((code, report)) => set.compiled.push(Compiled {
                name: hot.name,
                target,
                code,
                report,
            }),
            Err(r) => set.refused.push((hot.name, r.to_string())),
        }
    }
    Ok(set)
}

/// The driver texts of `k`: defs (the same text for every n), loop(1) and loop(n).
struct Texts {
    defs: String,
    loop1: String,
    loop_n: String,
    offset: u32,
    d1: kernels::Driver,
    dn: kernels::Driver,
}

fn texts(k: &Kernel, n: u32) -> Result<Texts, String> {
    let d1 = kernels::driver(k, 1)?;
    let dn = kernels::driver(k, n)?;
    let (defs, loop1) = tier::split(&d1, k.file)?;
    let (defs_n, loop_n) = tier::split(&dn, k.file)?;
    // Only the first line (a comment that names n) may differ; the unit is d1's defs.
    let body = |t: &str| t.split_once('\n').map(|x| x.1.to_string());
    if body(&defs) != body(&defs_n) || d1.offset != dn.offset {
        return Err("the drivers differ before the loop".into());
    }
    Ok(Texts {
        defs,
        loop1,
        loop_n,
        offset: d1.offset,
        d1,
        dn,
    })
}

/// A session with the kernel's defs.js run.
fn defs_session(t: &Texts, config: &Config) -> Result<Session, String> {
    let mut s = Session::new(&config.threaded())?;
    s.run(&t.defs, DEFS)
        .result
        .map_err(|e| format!("defs: {e}"))?;
    Ok(s)
}

/// The training run (the file writer's side): defs.js and loop(1) in T0, then the live facts
/// of the defs.js unit as a PROFILED file (encoded).
pub fn train(k: &Kernel) -> Result<Vec<u8>, String> {
    train_with(k, None)
}

/// [`train`], with `script` run instead of loop(1) if given (another run of the same unit:
/// [`wrong_profiles`]).
pub fn train_with(k: &Kernel, script: Option<&str>) -> Result<Vec<u8>, String> {
    let t = texts(k, 1)?;
    let mut s = defs_session(&t, &Config::default())?;
    s.run(script.unwrap_or(&t.loop1), "warm.js")
        .result
        .map_err(|e| format!("training run: {e}"))?;
    let unit = s.script(DEFS).ok_or("no defs script")?;
    Ok(facts::to_profiled(&facts::live(&s, unit)?)?.encode())
}

/// What one T0 run printed: loop(1), then loop(n).
pub struct T0Run {
    pub warm: RunOutput,
    pub timed: RunOutput,
    pub secs: f64,
}

/// T0: defs.js, loop(1), loop(n).
pub fn run_t0(k: &Kernel, n: u32, config: &Config) -> Result<T0Run, String> {
    let t = texts(k, n)?;
    let mut s = defs_session(&t, config)?;
    let warm = s.run(&t.loop1, "warm.js");
    let at = Instant::now();
    let timed = s.run(&t.loop_n, "loop.js");
    let secs = at.elapsed().as_secs_f64();
    Ok(T0Run { warm, timed, secs })
}

/// One config C or config P run.
pub struct Run {
    /// The file's state (P only): `None` = accepted, else why it was refused (the hot set then
    /// stays in T0).
    pub refused_file: Option<String>,
    /// Records the validation dropped (P only).
    pub dropped: usize,
    /// The hot set line ([`HotSet::line`]).
    pub set_line: String,
    /// The names of the compiled and installed functions.
    pub installed: Vec<&'static str>,
    /// Code bytes, fact guards and unbacked guards of the set.
    pub code: (usize, usize, usize),
    /// The code and exit table digests of each compiled function (FNV-1a 64; no address).
    pub digests: Vec<(u64, u64)>,
    pub warm: RunOutput,
    pub timed: RunOutput,
    pub secs: f64,
    /// Exits in loop(1) (for P: the start-up) and in loop(n), each as a total and a line.
    pub warm_exits: (u64, String),
    pub timed_exits: (u64, String),
}

/// Config C: defs.js, loop(1) in T0, the hot set compiled from the live facts and
/// installed, loop(n).
pub fn run_c(k: &Kernel, n: u32, config: &Config) -> Result<Run, String> {
    let t = texts(k, n)?;
    let mut s = defs_session(&t, config)?;
    let warm = s.run(&t.loop1, "warm.js");
    warm.result.clone().map_err(|e| format!("C warm-up: {e}"))?;
    let set = compile_hot_set(&mut s, k, DEFS, t.offset, Source::Live)?;
    finish(s, &t, set, None, 0, Some(warm))
}

/// Config P: defs.js; the PROFILED file `bytes` read and validated against the defs.js unit;
/// the hot set compiled from it and installed; then loop(1) (the start-up) and loop(n). A
/// refused file is not an error here: the run goes on with the hot set in T0
/// ([`Run::refused_file`]).
pub fn run_p(k: &Kernel, n: u32, config: &Config, bytes: &[u8]) -> Result<Run, String> {
    let t = texts(k, n)?;
    let mut s = defs_session(&t, config)?;
    let unit = s.script(DEFS).ok_or("no defs script")?.unit.unit();
    let (set, refused_file, dropped) = match facts::read_profiled(bytes, unit) {
        Ok((snap, dropped)) => (
            compile_hot_set(&mut s, k, DEFS, t.offset, Source::Profiled(&snap))?,
            None,
            dropped.len(),
        ),
        Err(e) => (
            HotSet {
                compiled: Vec::new(),
                refused: Vec::new(),
            },
            Some(e),
            0,
        ),
    };
    finish(s, &t, set, refused_file, dropped, None)
}

/// Installs `set`, runs loop(1) unless `warm` holds its output already, then loop(n); a
/// full collection and the heap verifier at the end.
fn finish(
    mut s: Session,
    t: &Texts,
    set: HotSet,
    refused_file: Option<String>,
    dropped: usize,
    warm: Option<RunOutput>,
) -> Result<Run, String> {
    let set_line = set.line();
    let code = set.code();
    let digests = set
        .compiled
        .iter()
        .map(|c| (fnv1a64(&c.code.code), fnv1a64(&c.report.emit.meta.encode())))
        .collect();
    // SAFETY: `set` is `compile_hot_set`'s result for `s` (run_c, run_p), unchanged, and no
    // JS ran in `s` since (`line`, `code` and the digests only read it).
    let inst = unsafe { set.install(&mut s) }?;
    let c0 = inst.counts(&s)?;
    let (warm, c1) = match warm {
        Some(w) => (w, c0.clone()),
        None => {
            let w = s.run(&t.loop1, "warm.js");
            (w, inst.counts(&s)?)
        }
    };
    let at = Instant::now();
    let timed = s.run(&t.loop_n, "loop.js");
    let secs = at.elapsed().as_secs_f64();
    let c2 = inst.counts(&s)?;
    s.verify()?;
    let sum = |a: &[Vec<u64>], b: &[Vec<u64>]| -> u64 {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .map(|(a, b)| b - a)
            .sum()
    };
    Ok(Run {
        refused_file,
        dropped,
        set_line,
        installed: inst.names(),
        code,
        digests,
        warm,
        timed,
        secs,
        warm_exits: (sum(&c0, &c1), inst.exit_line(&c0, &c1)),
        timed_exits: (sum(&c1, &c2), inst.exit_line(&c1, &c2)),
    })
}

/// FNV-1a, 64 bits (code and exit table digests in the reports).
pub fn fnv1a64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| {
        (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// True if both runs printed the same lines and ended the same way (loop(1) and loop(n)).
fn same(t0: &T0Run, r: &Run) -> bool {
    r.warm == t0.warm && r.timed == t0.timed
}

/// The check of a right file (`sp-facts check`, `gate.sh spfacts`): T0, config C and config P
/// of `k` in this process (P reads `bytes`, which another process wrote). Returns the report
/// lines and the verdict: P's file accepted with no record dropped, P compiled the same set
/// as C (not empty), every line of P equal to T0 and C (loop(1) and loop(n)), loop(n) a right
/// driver run, both heap verifiers clean.
pub fn check(k: &Kernel, n: u32, bytes: &[u8]) -> Result<(Vec<String>, bool), String> {
    let config = Config::default();
    let t = texts(k, n)?;
    crate::vm_layout::check(&mut Session::new(&config.threaded())?)
        .map_err(|e| format!("vm layout: {e}"))?;
    let t0 = run_t0(k, n, &config)?;
    let c = run_c(k, n, &config)?;
    let p = run_p(k, n, &config, bytes)?;
    let mut lines = Vec::new();
    let mut ok = true;
    let mut fail = |lines: &mut Vec<String>, why: String| {
        lines.push(format!("{}: FAIL: {why}", k.name));
        ok = false;
    };
    lines.push(format!("{}: config C: {}", k.name, c.set_line));
    match &p.refused_file {
        Some(e) => fail(&mut lines, format!("the file was refused: {e}")),
        None => lines.push(format!(
            "{}: config P: file accepted, {} records dropped; {}",
            k.name, p.dropped, p.set_line
        )),
    }
    if p.dropped != 0 {
        fail(&mut lines, format!("{} records dropped", p.dropped));
    }
    if p.installed.is_empty() || p.installed != c.installed {
        fail(
            &mut lines,
            format!("P compiled {:?}, C {:?}", p.installed, c.installed),
        );
    }
    lines.push(format!(
        "{}: P code and exit tables {} config C's ({} functions)",
        k.name,
        if p.digests == c.digests {
            "byte-equal to"
        } else {
            "differ from"
        },
        p.digests.len()
    ));
    if t0.warm.result.is_err() || t0.timed.result.is_err() || !t.dn.matches(&t0.timed.output) {
        fail(&mut lines, format!("T0 is not a right run: {:?}", t0.timed));
    }
    if !t.d1.matches(&t0.warm.output) {
        fail(&mut lines, "T0 loop(1) is not a right run".into());
    }
    if !same(&t0, &c) {
        fail(&mut lines, format!("C differs from T0: {:?}", c.timed));
    }
    if !same(&t0, &p) {
        fail(
            &mut lines,
            format!("P differs from T0: {:?} {:?}", p.warm, p.timed),
        );
    }
    lines.push(format!(
        "{}: n = {n}: P output {} T0 and C (loop(1) and loop(n)); start-up exits (loop(1)) {}; \
         loop(n) exits {}; C loop(n) exits {}; heap verifier clean",
        k.name,
        if same(&t0, &p) && same(&t0, &c) {
            "equal to"
        } else {
            "NOT equal to"
        },
        p.warm_exits.1,
        p.timed_exits.1,
        c.timed_exits.1
    ));
    lines.push(format!(
        "{}: one run each, a report: T0 {:.3} s, C {:.3} s, P {:.3} s (loop(n))",
        k.name, t0.secs, c.secs, p.secs
    ));
    Ok((lines, ok))
}

/// The records of `s` permuted across the slots of one kind (all functions of the unit), by
/// a seeded Fisher-Yates (splitmix64): every record keeps a slot of its own kind, so the
/// file still validates, but no record need describe its slot.
pub fn shuffled(s: &FactSnapshot, seed: u64) -> Result<FactSnapshot, String> {
    use std::collections::BTreeMap;
    let mut by_kind: BTreeMap<u8, Vec<(ep9_facts::SlotKey, SlotFacts)>> = BTreeMap::new();
    for (k, f) in s.records() {
        by_kind
            .entry(f.kind() as u8)
            .or_default()
            .push((k, f.clone()));
    }
    let mut out = FactSnapshot::new(*s.header());
    let mut state = seed;
    for (_, mut v) in by_kind {
        let keys: Vec<_> = v.iter().map(|(k, _)| *k).collect();
        for i in (1..v.len()).rev() {
            let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
        for (key, (_, f)) in keys.into_iter().zip(v) {
            out.insert(key.function, key.slot, f)
                .map_err(|e| format!("record {}/{}: {e:?}", key.function, key.slot))?;
        }
    }
    Ok(out)
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `s` with every BinaryOp record set to `bin` and every CompareOp record set to `cmp`; all
/// other records (the named loads among them) are kept, so the code compiled from it passes
/// the same shape guards as from `s` and reaches the arithmetic the changed facts lower.
pub fn numeric_as(
    s: &FactSnapshot,
    bin: BinaryHint,
    cmp: CompareHint,
) -> Result<FactSnapshot, String> {
    let mut out = FactSnapshot::new(*s.header());
    for (k, f) in s.records() {
        let f = match f {
            SlotFacts::BinaryOp(_) => SlotFacts::BinaryOp(bin),
            SlotFacts::CompareOp(_) => SlotFacts::CompareOp(cmp),
            f => f.clone(),
        };
        out.insert(k.function, k.slot, f)
            .map_err(|e| format!("record {}/{}: {e:?}", k.function, k.slot))?;
    }
    Ok(out)
}

/// `bytes` (an encoded snapshot) with its schema version field set to `version`.
pub fn with_schema_version(bytes: &[u8], version: u32) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let at = ep9_facts::offsets::SCHEMA_VERSION;
    b[at..at + 4].copy_from_slice(&version.to_le_bytes());
    b
}

/// The "other run" of a kernel for [`wrong_profiles`]: a script that calls the first hot
/// function on inputs the kernel never makes, so its facts describe another run of the same
/// unit (doubles where the kernel has int32 digits, literal objects where it has its own
/// shapes). `None` for a kernel with no such script.
fn other_run(k: &Kernel) -> Option<&'static str> {
    match k.name {
        "crypto" => Some(
            "var __spA = { other: 1, array: [0.5, 1.25, 2.75, 3.5, 4.25] };\n\
             var __spW = { array: [0.25, 0.75, 1.5, 2.5, 3.25, 4.75, 5.5] };\n\
             for (var __i = 0; __i < 40; __i++) am3.call(__spA, 0, 1.5, __spW, 1, 0.5, 4);\n",
        ),
        _ => None,
    }
}

/// What a wrong-profile case must show besides T0's output.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    /// The file is refused (another kernel, another schema, a RUNTIME file).
    Refuse,
    /// The file is accepted; what compiles from it may exit anywhere or stay in T0.
    Accept,
    /// The file is accepted, the right file's hot set compiles from it to other code (the
    /// changed facts reached the lowering), and loop(n) runs that code with no exit (so the
    /// arithmetic lowered from the wrong facts ran).
    Reach,
}

/// One wrong-profile case and what config P did with it.
pub struct WrongCase {
    pub name: String,
    /// Expected: the file is refused (another kernel, another schema, a RUNTIME file).
    pub refuse: bool,
    pub run: Run,
}

/// The wrong-profile test (R4, MC-w15-18; `sp-facts wrong`, `gate.sh spfacts`): config P of
/// `k` with files that do not describe the run. Pass: each case prints T0's lines (loop(1)
/// and loop(n)) with the heap verifier clean; the files of another kernel, another schema
/// version and a RUNTIME source are refused; code compiled from a wrong file ran and took an
/// exit in at least one case; and the wrong numeric facts (named-load facts kept) compile to
/// other code that loop(n) runs with no exit ([`Need::Reach`]), since the other cases exit at
/// the first named load (g0) before any arithmetic.
pub fn wrong_profiles(k: &'static Kernel, n: u32) -> Result<(Vec<String>, bool), String> {
    let config = Config::default();
    let t0 = run_t0(k, n, &config)?;
    let right = train(k)?;
    let snap = FactSnapshot::decode(&right).map_err(|e| format!("decode: {e:?}"))?;
    let other_kernel = kernels::KERNELS
        .iter()
        .find(|o| o.name != k.name)
        .ok_or("no other kernel")?;
    let base = run_p(k, n, &config, &right)?;
    let mut cases: Vec<(String, Need, Vec<u8>)> = vec![(
        format!("another kernel's file ({})", other_kernel.name),
        Need::Refuse,
        train(other_kernel)?,
    )];
    if let Some(script) = other_run(k) {
        cases.push((
            "another run of the same unit".into(),
            Need::Accept,
            train_with(k, Some(script))?,
        ));
    }
    for seed in 1..=4u64 {
        cases.push((
            format!("records shuffled across slots of one kind (seed {seed})"),
            Need::Accept,
            shuffled(&snap, seed)?.encode(),
        ));
    }
    cases.push((
        "numeric facts NUMBER, named-load facts kept".into(),
        Need::Reach,
        numeric_as(&snap, BinaryHint::NUMBER, CompareHint::NUMBER)?.encode(),
    ));
    cases.push((
        "no records".into(),
        Need::Accept,
        FactSnapshot::new(*snap.header()).encode(),
    ));
    cases.push((
        format!("FACT_SCHEMA_VERSION {}", FACT_SCHEMA_VERSION + 1),
        Need::Refuse,
        with_schema_version(&right, FACT_SCHEMA_VERSION + 1),
    ));
    cases.push((
        "source RUNTIME".into(),
        Need::Refuse,
        FactSnapshot::new(FactHeader {
            source: FactSource::Runtime,
            ..*snap.header()
        })
        .encode(),
    ));
    let mut lines = Vec::new();
    let mut ok = true;
    let mut exited = false;
    for (name, need, bytes) in cases {
        let run = run_p(k, n, &config, &bytes)?;
        let equal = same(&t0, &run);
        let refused = run.refused_file.is_some();
        exited |= !run.installed.is_empty() && run.warm_exits.0 + run.timed_exits.0 > 0;
        let reached = !run.installed.is_empty()
            && run.installed == base.installed
            && run.digests != base.digests
            && run.timed_exits.0 == 0;
        let right = equal && refused == (need == Need::Refuse) && (need != Need::Reach || reached);
        ok &= right;
        lines.push(format!(
            "{}: {name}: {}; output {}; exits loop(1) {}, loop(n) {}{}; loop(n) {:.3} s (T0 {:.3} \
             s, one run, a report){}",
            k.name,
            match &run.refused_file {
                Some(e) => format!("file refused ({e}), the hot set stays in T0"),
                None => format!("file accepted, {} dropped; {}", run.dropped, run.set_line),
            },
            if equal {
                "equal to T0"
            } else {
                "DIFFERS from T0"
            },
            run.warm_exits.1,
            run.timed_exits.1,
            match (need, reached) {
                (Need::Reach, true) =>
                    "; other code than the right file's, reached compiled arithmetic",
                (Need::Reach, false) => "; did NOT reach compiled arithmetic",
                _ => "",
            },
            run.secs,
            t0.secs,
            if right { "" } else { ": FAIL" }
        ));
    }
    if !exited {
        ok = false;
        lines.push(format!(
            "{}: FAIL: no wrong file gave installed code that took an exit",
            k.name
        ));
    }
    Ok((lines, ok))
}

/// `spike-c sp-facts ...` (main.rs).
///
/// ```text
/// spike-c sp-facts write <kernel> <file>            the training run; writes the PROFILED file
/// spike-c sp-facts check <kernel> <file> [--n N]    config P from <file> vs T0 and config C
/// spike-c sp-facts wrong <kernel> [--n N]           the wrong-profile cases (default n = 1)
/// spike-c sp-facts time <kernel> [--rounds R] [--n N]
///                                                   class S report of t(C) / t(P) (Mac report)
/// ```
pub fn main(args: &[String]) -> ExitCode {
    match run_main(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("spike-c sp-facts: {e}");
            ExitCode::from(2)
        }
    }
}

fn kernel_arg(name: Option<&String>) -> Result<&'static Kernel, String> {
    let name = name.ok_or("a kernel name is needed")?;
    kernels::kernel(name).ok_or_else(|| format!("unknown kernel {name}"))
}

/// The value after `flag` in `args`, parsed.
fn flag<T: std::str::FromStr>(args: &[String], flag: &str) -> Result<Option<T>, String> {
    match args.iter().position(|a| a == flag) {
        None => Ok(None),
        Some(i) => {
            let v = args
                .get(i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            v.parse()
                .map(Some)
                .map_err(|_| format!("bad {flag} value {v}"))
        }
    }
}

fn run_main(args: &[String]) -> Result<bool, String> {
    let usage = "usage: spike-c sp-facts [write <kernel> <file> | check <kernel> <file> [--n N] \
                 | wrong <kernel> [--n N] | time <kernel> [--rounds R] [--n N]]";
    let cmd = args.first().ok_or(usage)?;
    let rest = &args[1..];
    match cmd.as_str() {
        "write" => {
            let k = kernel_arg(rest.first())?;
            let path = rest.get(1).ok_or("write needs <file>")?;
            let bytes = train(k)?;
            std::fs::write(path, &bytes).map_err(|e| format!("{path}: {e}"))?;
            let s = FactSnapshot::decode(&bytes).map_err(|e| format!("decode: {e:?}"))?;
            println!(
                "{}: wrote {path}: {} bytes, {} records, source {:?}, FACT_SCHEMA_VERSION {}",
                k.name,
                bytes.len(),
                s.len(),
                s.header().source,
                FACT_SCHEMA_VERSION
            );
            let mut kinds = std::collections::BTreeMap::new();
            for (_, f) in s.records() {
                *kinds.entry(facts::kind_name(f)).or_insert(0usize) += 1;
            }
            let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
            println!("{}: records by kind: {}", k.name, kinds.join(", "));
            Ok(true)
        }
        "check" => {
            let k = kernel_arg(rest.first())?;
            let path = rest.get(1).ok_or("check needs <file>")?;
            let n = flag(rest, "--n")?.unwrap_or(k.n);
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            let (lines, ok) = check(k, n, &bytes)?;
            for l in lines {
                println!("{l}");
            }
            Ok(ok)
        }
        "wrong" => {
            let k = kernel_arg(rest.first())?;
            let n = flag(rest, "--n")?.unwrap_or(1);
            let (lines, ok) = wrong_profiles(k, n)?;
            for l in lines {
                println!("{l}");
            }
            Ok(ok)
        }
        "time" => {
            let k = kernel_arg(rest.first())?;
            let rounds = flag(rest, "--rounds")?.unwrap_or(15usize);
            let n = flag(rest, "--n")?.unwrap_or(k.n);
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            // The file is written by a process of its own, as in the protocol (§6.3).
            let path = std::env::temp_dir().join(format!(
                "spike-c-sp-facts-{}-{}.facts",
                k.name,
                std::process::id()
            ));
            let w = std::process::Command::new(&exe)
                .args(["sp-facts", "write", k.name])
                .arg(&path)
                .output()
                .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
            if !w.status.success() {
                return Err(format!(
                    "the training process failed: {}",
                    String::from_utf8_lossy(&w.stderr).trim()
                ));
            }
            let r = crate::timing::time_p(&exe, k, rounds, n, &path);
            let _ = std::fs::remove_file(&path);
            let (report, ok) = r?;
            print!("{report}");
            Ok(ok)
        }
        _ => Err(usage.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "\
function P(x) { this.x = x; this.y = x & 7; }\n\
function sum(ps) { var s = 0; for (var i = 0; i < ps.length; i++) { s = (s + ps[i].x + ps[i].y) | 0; } return s; }\n\
var ps = []; for (var i = 0; i < 50; i++) ps.push(new P(i));\n\
var r = 0; for (var k = 0; k < 40; k++) r = sum(ps);\n\
print(r);\n";

    /// A PROFILED file of `SRC` and its unit's session.
    fn profiled() -> (Session, Vec<u8>) {
        let mut s = Session::new(&Config::default()).expect("session");
        assert!(s.run(SRC, "f.js").result.is_ok());
        let script = s.script("f.js").expect("script");
        let bytes = facts::to_profiled(&facts::live(&s, script).expect("live"))
            .expect("profiled")
            .encode();
        (s, bytes)
    }

    /// A file of another FACT_SCHEMA_VERSION is refused whole (decode reads the version
    /// first), and so is one with a version field of 0; the right one reads back.
    #[test]
    fn another_schema_version_is_refused() {
        let (s, bytes) = profiled();
        let unit = s.script("f.js").expect("script").unit.unit();
        assert!(facts::read_profiled(&bytes, unit).is_ok());
        for v in [0, FACT_SCHEMA_VERSION + 1, u32::MAX] {
            let e = facts::read_profiled(&with_schema_version(&bytes, v), unit)
                .expect_err("another schema");
            assert!(e.contains(&format!("schema version {v}")), "{e}");
        }
    }

    /// `numeric_as` changes only the BinaryOp and CompareOp records.
    #[test]
    fn numeric_as_keeps_the_other_records() {
        let (_, bytes) = profiled();
        let snap = FactSnapshot::decode(&bytes).expect("decode");
        let m = numeric_as(&snap, BinaryHint::NUMBER, CompareHint::NUMBER).expect("numeric");
        assert_eq!(m.len(), snap.len());
        let mut numeric = 0;
        for (k, f) in m.records() {
            match f {
                SlotFacts::BinaryOp(h) => {
                    numeric += 1;
                    assert_eq!(*h, BinaryHint::NUMBER);
                }
                SlotFacts::CompareOp(h) => {
                    numeric += 1;
                    assert_eq!(*h, CompareHint::NUMBER);
                }
                f => assert_eq!(Some(f), snap.get(k.function, k.slot)),
            }
        }
        assert!(numeric > 0);
    }

    /// The shuffled file keeps the record count and every record on a slot of its own kind,
    /// so it validates against the unit with nothing dropped; another seed gives another
    /// file; seed order is deterministic.
    #[test]
    fn a_shuffled_file_still_validates() {
        let (s, bytes) = profiled();
        let unit = s.script("f.js").expect("script").unit.unit();
        let snap = FactSnapshot::decode(&bytes).expect("decode");
        let a = shuffled(&snap, 1).expect("shuffle");
        assert_eq!(a.len(), snap.len());
        assert_eq!(a, shuffled(&snap, 1).expect("again"));
        for (k, f) in a.records() {
            assert_eq!(
                f.kind(),
                snap.get(k.function, k.slot).expect("same keys").kind()
            );
        }
        let (_, dropped) = facts::read_profiled(&a.encode(), unit).expect("validates");
        assert!(dropped.is_empty(), "{dropped:?}");
        assert!((1..=8).any(|seed| shuffled(&snap, seed).expect("shuffle") != snap));
    }

    fn crypto_ready() -> Option<&'static Kernel> {
        if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64)
            || !kernels::bench_present()
        {
            eprintln!("skip: aarch64 and the v8-v7 checkout only");
            return None;
        }
        kernels::kernel("crypto")
    }

    /// Config P on crypto, n = 1: the file written by the training run is accepted with no
    /// record dropped, am3 (the hot set the compiler takes today) is compiled from it before
    /// the first run() and installed, every line equals T0 and config C, and the code equals
    /// config C's byte for byte (the same facts).
    #[test]
    fn config_p_equals_t0_and_c_on_crypto() {
        let Some(k) = crypto_ready() else { return };
        let bytes = train(k).expect("train");
        let (lines, ok) = check(k, 1, &bytes).expect("check");
        assert!(ok, "{lines:#?}");
        let p = run_p(k, 1, &Config::default(), &bytes).expect("p");
        assert_eq!(p.installed, ["am3"], "{}", p.set_line);
        let c = run_c(k, 1, &Config::default()).expect("c");
        assert_eq!(p.digests, c.digests);
    }

    /// R4 (MC-w15-18): wrong files cost exits, never output.
    #[test]
    fn a_wrong_profile_keeps_the_output() {
        let Some(k) = crypto_ready() else { return };
        let (lines, ok) = wrong_profiles(k, 1).expect("wrong");
        assert!(ok, "{lines:#?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("numeric facts NUMBER")
                    && l.contains("reached compiled arithmetic")),
            "{lines:#?}"
        );
    }
}
