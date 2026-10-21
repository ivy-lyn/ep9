//! The unix registration mode, chosen at run time by a self-test (H28).
//!
//! libgcc and LLVM (or Apple) libunwind both export `__register_frame`, but they read its
//! argument differently: libgcc takes a whole `.eh_frame` section, libunwind takes one FDE.
//! Which one a binary uses is a link-time fact. A Rust build for linux-gnu links libgcc_s;
//! zig cc links LLVM libunwind statically, with hidden symbols. So the mode is not chosen by
//! target or by `dlsym`: a self-test registers a probe, calls it, and walks through it.
//!
//! ```text
//!   probe section: CIE | FDE (other code) | FDE (probe)       the probe FDE is not the first
//!   1. whole-section: __register_frame(section)               libgcc reads every record
//!        run: call the probe; its callback walks the stack
//!        the walk passes the probe frame and goes on into the caller's frames?
//!          yes -> WholeSection
//!          no  -> __deregister_frame(section)
//!   2. per-FDE: __register_frame(fde) for each FDE            libunwind takes one FDE
//!        same run and check -> PerFde
//!   3. both miss -> an error; every later registration fails, so native code does not start
//! ```
//!
//! - Whole-section goes first. libgcc reads from any FDE pointer to the terminator, so a
//!   per-FDE registration would pass under libgcc too, with overlapping objects. libunwind
//!   ignores a section pointer: its FDE decoder rejects the CIE at the start ("FDE is really
//!   a CIE"), and `__deregister_frame` of it removes nothing.
//! - The mode is chosen once per process. In a build with `jit` (or a unit test), the first
//!   [`super::EhFrameRegistration::register`] runs the self-test on a built-in probe in a page
//!   of its own. A jitless (`unwind` only) build has no code of its own to run: its loader
//!   calls [`self_test`] on a probe in the first loaded unit's text first; until then every
//!   registration fails.

use std::cell::RefCell;
use std::fmt;
use std::io;
use std::ops::Range;
use std::sync::OnceLock;

use super::cfi::EhFrameSection;
use super::unix::EhFrameRegistration;

/// How `__register_frame` takes an `.eh_frame` section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegistrationMode {
    /// One call with the section start: libgcc reads every record up to the terminator.
    WholeSection,
    /// One call per FDE: LLVM and Apple libunwind decode exactly one FDE per call.
    PerFde,
}

impl RegistrationMode {
    /// The order in which the self-test tries the modes (see the module doc).
    pub const ORDER: [RegistrationMode; 2] =
        [RegistrationMode::WholeSection, RegistrationMode::PerFde];

    pub const fn name(self) -> &'static str {
        match self {
            RegistrationMode::WholeSection => "whole-section",
            RegistrationMode::PerFde => "per-fde",
        }
    }
}

impl fmt::Display for RegistrationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The callback a probe calls. It walks the stack and returns its argument.
pub type ProbeCallback = extern "C-unwind" fn(u64) -> u64;

/// A probe: `probe(x, cb)` calls `cb` once, from a frame that its FDE describes, and returns.
pub type ProbeFn = unsafe extern "C-unwind" fn(u64, ProbeCallback) -> u64;

/// Generated code for the self-test, with its unwind info.
#[derive(Debug)]
pub struct SelfTestProbe {
    pub entry: ProbeFn,
    /// The probe's code. The return address of its call to the callback lies in
    /// `(code.start, code.end]`.
    pub code: Range<usize>,
    /// An `.eh_frame` section with an FDE that covers `code`. Put another FDE before it, so a
    /// mode that registers only the first FDE of a section fails the test.
    pub section: EhFrameSection,
}

/// The mode, or why no mode works. Set once per process.
static MODE: OnceLock<Result<RegistrationMode, String>> = OnceLock::new();

fn unsupported(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, msg)
}

/// The registration mode of this process. With `jit` (or in a unit test) the first call runs
/// the self-test on the built-in probe. Without `jit` it fails until [`self_test`] has run.
/// Fails for good if the self-test found no working mode: native code must not start then.
pub fn registration_mode() -> io::Result<RegistrationMode> {
    #[cfg(any(feature = "jit", test))]
    let r = MODE.get_or_init(builtin::select);
    #[cfg(not(any(feature = "jit", test)))]
    let Some(r) = MODE.get() else {
        return Err(unsupported(
            "unwind registration mode not chosen yet: call unwind::self_test on a probe in \
             the loaded code first (H28)",
        ));
    };
    r.as_ref().copied().map_err(|m| unsupported(m))
}

/// What a `dlsym` of `__unw_add_dynamic_fde` suggests: [`RegistrationMode::PerFde`] if the
/// symbol is found. Only a diagnostic: it misses a static libunwind with hidden symbols (zig
/// cc), so the self-test decides.
pub fn dlsym_guess() -> RegistrationMode {
    // SAFETY: dlsym only looks up a NUL-terminated name; RTLD_DEFAULT searches the global
    // scope.
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"__unw_add_dynamic_fde".as_ptr()) };
    if p.is_null() {
        RegistrationMode::WholeSection
    } else {
        RegistrationMode::PerFde
    }
}

/// Runs the self-test on `probe` and returns the mode. If no mode was chosen yet, this chooses
/// it for the process (whole-section, then per-FDE). If one was, this checks `probe` with it.
/// The probe's registration is removed before the call returns.
///
/// # Safety
/// - `probe.entry` points to code with the [`ProbeFn`] contract inside `probe.code`, and that
///   code stays mapped and executable until this returns.
/// - Each FDE of `probe.section` describes its code correctly (see
///   [`EhFrameRegistration::register`]); no live registration overlaps its span.
pub unsafe fn self_test(probe: SelfTestProbe) -> io::Result<RegistrationMode> {
    let mut chose = false;
    let r = MODE.get_or_init(|| {
        chose = true;
        // SAFETY: the caller's contract.
        unsafe { select(&probe) }
    });
    match r {
        Ok(mode) if !chose => {
            // SAFETY: the caller's contract.
            unsafe { try_mode(&probe, *mode) }
                .map(|()| *mode)
                .map_err(|e| unsupported(&format!("unwind self-test ({mode}): {e}")))
        }
        Ok(mode) => Ok(*mode),
        Err(m) => Err(unsupported(m)),
    }
}

/// Tries the modes in [`RegistrationMode::ORDER`]; the first whose walk passes wins.
///
/// # Safety
/// As [`self_test`].
pub(super) unsafe fn select(probe: &SelfTestProbe) -> Result<RegistrationMode, String> {
    let mut misses = Vec::new();
    for mode in RegistrationMode::ORDER {
        // SAFETY: the caller's contract.
        match unsafe { try_mode(probe, mode) } {
            Ok(()) => return Ok(mode),
            Err(e) => misses.push(format!("{mode}: {e}")),
        }
    }
    Err(format!(
        "unwind self-test failed: the system unwinder does not walk through generated code \
         in any registration mode ({}; dlsym guess: {}). Native code cannot run (H28)",
        misses.join("; "),
        dlsym_guess(),
    ))
}

/// Registers the probe in `mode`, runs it, checks the walk, and deregisters.
///
/// # Safety
/// As [`self_test`].
pub(super) unsafe fn try_mode(probe: &SelfTestProbe, mode: RegistrationMode) -> Result<(), String> {
    // SAFETY: the caller's contract; `reg` drops before this returns, and only this thread
    // runs the probe.
    let reg = unsafe { EhFrameRegistration::register_mode(probe.section.clone(), mode) }
        .map_err(|e| format!("register: {e}"))?;
    // SAFETY: the caller's contract; the probe's unwind info is registered.
    let (base, walk) = unsafe { run(probe.entry) };
    drop(reg);
    check(&probe.code, &base, walk.as_deref())
}

thread_local! {
    /// The walk that the probe callback took on this thread.
    static WALK: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
}

/// Guards against a walk that loops through a bad frame. A real stack is far shallower.
pub(super) const MAX_FRAMES: usize = 1 << 16;

extern "C-unwind" fn probe_callback(x: u64) -> u64 {
    let ips = super::native_return_addresses(MAX_FRAMES);
    WALK.with(|w| *w.borrow_mut() = Some(ips));
    x
}

#[inline(never)]
fn baseline_walk() -> Vec<usize> {
    std::hint::black_box(super::native_return_addresses(MAX_FRAMES))
}

/// Takes a walk from a plain Rust callee of this frame, then one from the probe's callback.
/// Both walks pass this frame, so the frames after it are the same in both.
///
/// # Safety
/// `entry` has the [`ProbeFn`] contract and its unwind info is registered.
#[inline(never)]
unsafe fn run(entry: ProbeFn) -> (Vec<usize>, Option<Vec<usize>>) {
    let baseline = (std::hint::black_box(baseline_walk as fn() -> Vec<usize>))();
    WALK.with(|w| w.borrow_mut().take());
    // SAFETY: the caller's contract.
    let r = unsafe { std::hint::black_box(entry)(1, probe_callback) };
    std::hint::black_box(r);
    let walk = WALK.with(|w| w.borrow_mut().take());
    (baseline, walk)
}

/// The walk passes the probe if it holds a return address into the probe and, after it and
/// the return address into [`run`], exactly the frames that the baseline walk ends with. A
/// missing FDE stops the walk at the probe frame; a wrong one leaves the real frames.
pub(super) fn check(
    code: &Range<usize>,
    baseline: &[usize],
    walk: Option<&[usize]>,
) -> Result<(), String> {
    let walk = walk.ok_or("the probe did not call its callback")?;
    if walk.len() >= MAX_FRAMES || baseline.len() >= MAX_FRAMES {
        return Err(format!("the walk did not end within {MAX_FRAMES} frames"));
    }
    let at = walk
        .iter()
        .position(|&ip| code.start < ip && ip <= code.end)
        .ok_or_else(|| {
            format!(
                "no frame returns into the probe {:#x}..{:#x} (the walk ends after {} frames; \
                 no FDE found for the probe?)",
                code.start,
                code.end,
                walk.len()
            )
        })?;
    let tail = walk.get(at + 2..).unwrap_or_default();
    if tail.is_empty() {
        return Err(format!(
            "the walk stops at the probe frame {:#x} (the unwinder found no FDE for it)",
            walk[at]
        ));
    }
    if !baseline.ends_with(tail) || baseline.len() == tail.len() {
        return Err(format!(
            "the walk after the probe frame {:#x} does not reach the caller's frames \
             ({} frames after it, {} in the baseline)",
            walk[at],
            tail.len(),
            baseline.len()
        ));
    }
    Ok(())
}

/// The built-in probe: a page of its own from the process code range, freed after the test.
#[cfg(any(feature = "jit", test))]
mod builtin {
    use super::{ProbeFn, RegistrationMode, SelfTestProbe};
    use crate::jit::WxMode;
    use crate::unwind::cfi::{CfiProgram, Cie, EhFrameBuilder, reg};
    use crate::va::{CodeRange, page_size};

    /// Offset of a second function (a bare return) with an FDE of its own, placed before the
    /// probe's FDE in the section.
    pub(in crate::unwind) const OTHER_OFF: usize = 64;

    /// `u64 probe(u64 x, u64 (*cb)(u64)) { return cb(x + 1) + x; }` with a frame record and
    /// a callee-saved register live across the call. Same bytes as the
    /// `unwind_through_generated_frame` test in `jit::tests` (checked with `clang -c` and
    /// `objdump -d`).
    pub(in crate::unwind) fn probe_code() -> Vec<u8> {
        #[cfg(target_arch = "aarch64")]
        {
            [
                0xa9be_7bfdu32, //  0 stp x29, x30, [sp, #-32]!
                0x9100_03fd,    //  4 mov x29, sp
                0xa901_53f3,    //  8 stp x19, x20, [sp, #16]
                0xaa00_03f3,    // 12 mov x19, x0
                0x9100_0400,    // 16 add x0, x0, #1
                0xd63f_0020,    // 20 blr x1
                0x8b13_0000,    // 24 add x0, x0, x19
                0xa941_53f3,    // 28 ldp x19, x20, [sp, #16]
                0xa8c2_7bfd,    // 32 ldp x29, x30, [sp], #32
                0xd65f_03c0,    // 36 ret
            ]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
        }
        #[cfg(target_arch = "x86_64")]
        {
            #[rustfmt::skip]
            let v = vec![
                0x55,                   //  0 push rbp
                0x48, 0x89, 0xe5,       //  1 mov rbp, rsp
                0x53,                   //  4 push rbx
                0x48, 0x83, 0xec, 0x08, //  5 sub rsp, 8
                0x48, 0x89, 0xfb,       //  9 mov rbx, rdi
                0x48, 0x8d, 0x7f, 0x01, // 12 lea rdi, [rdi+1]
                0xff, 0xd6,             // 16 call rsi
                0x48, 0x01, 0xd8,       // 18 add rax, rbx
                0x48, 0x83, 0xc4, 0x08, // 21 add rsp, 8
                0x5b,                   // 25 pop rbx
                0x5d,                   // 26 pop rbp
                0xc3,                   // 27 ret
            ];
            v
        }
    }

    /// `ret`.
    pub(in crate::unwind) fn other_code() -> Vec<u8> {
        #[cfg(target_arch = "aarch64")]
        {
            0xd65f_03c0u32.to_le_bytes().to_vec()
        }
        #[cfg(target_arch = "x86_64")]
        {
            vec![0xc3]
        }
    }

    /// The probe's CFI after the CIE rules, at every instruction boundary of prolog and
    /// epilogue.
    pub(in crate::unwind) fn probe_cfi() -> CfiProgram {
        let mut p = CfiProgram::new();
        #[cfg(target_arch = "aarch64")]
        {
            use reg::aarch64::*;
            p.advance_loc(1)
                .def_cfa_offset(32)
                .offset(FP, 4)
                .offset(LR, 3)
                .advance_loc(1)
                .def_cfa_register(FP)
                .advance_loc(1)
                .offset(X19, 2)
                .offset(X20, 1)
                .advance_loc(5)
                .restore(X19)
                .restore(X20)
                .advance_loc(1)
                .def_cfa(SP, 0)
                .restore(FP)
                .restore(LR);
        }
        #[cfg(target_arch = "x86_64")]
        {
            use reg::x86_64::*;
            p.advance_loc(1)
                .def_cfa_offset(16)
                .offset(RBP, 2)
                .advance_loc(3)
                .def_cfa_register(RBP)
                .advance_loc(1)
                .offset(RBX, 3)
                .advance_loc(21)
                .restore(RBX)
                .advance_loc(1)
                .def_cfa(RSP, 8)
                .restore(RBP);
        }
        p
    }

    /// Runs `f` with the probe and the `other` function mapped executable in one page (a chunk
    /// of [`CodeRange::process`]); `section` builds the probe's `.eh_frame` from (page base, probe length).
    pub(in crate::unwind) fn with_probe<T>(
        section: impl FnOnce(usize, usize) -> crate::unwind::cfi::EhFrameSection,
        f: impl FnOnce(&SelfTestProbe) -> T,
    ) -> Result<T, String> {
        // One page of the process code range (H07-Q6), where JIT code and units live.
        let range =
            CodeRange::process().map_err(|e| format!("reserve the process code range: {e}"))?;
        let mut chunk = range
            .alloc(page_size())
            .map_err(|e| format!("take a code page: {e}"))?;
        let (code, other) = (probe_code(), other_code());
        let mut mem = chunk
            .commit(WxMode::DEFAULT)
            .map_err(|e| format!("commit a code page: {e}"))?;
        {
            let mut w = mem
                .open_write(0, OTHER_OFF + other.len())
                .map_err(|e| format!("open a write window: {e}"))?;
            let b = w.bytes();
            b[..code.len()].copy_from_slice(&code);
            b[OTHER_OFF..].copy_from_slice(&other);
            w.close()
                .map_err(|e| format!("close the write window: {e}"))?;
        }
        let base = mem.exec_base();
        // SAFETY: offset 0 holds the probe, written above; it has the ProbeFn signature.
        let entry: ProbeFn = unsafe { std::mem::transmute::<usize, ProbeFn>(base) };
        let probe = SelfTestProbe {
            entry,
            code: base..base + code.len(),
            section: section(base, code.len()),
        };
        // `mem`, then `chunk`, drop after `f` and its registrations.
        Ok(f(&probe))
    }

    /// The probe's real section: CIE, the FDE of `other`, the FDE of the probe.
    pub(in crate::unwind) fn probe_section(
        base: usize,
        len: usize,
    ) -> crate::unwind::cfi::EhFrameSection {
        let mut b = EhFrameBuilder::new(&Cie::host());
        b.add_fde(base + OTHER_OFF, other_code().len(), &CfiProgram::new());
        b.add_fde(base, len, &probe_cfi());
        b.finish()
    }

    /// The process-wide choice with the built-in probe.
    pub(super) fn select() -> Result<RegistrationMode, String> {
        // SAFETY: `with_probe` maps the probe for the duration of the closure; the section
        // describes it exactly; the page is new, so no registration overlaps it.
        with_probe(probe_section, |p| unsafe { super::select(p) }).map_err(|e| {
            format!("unwind self-test could not run: {e}. Native code cannot run (H28)")
        })?
    }
}

#[cfg(test)]
pub(super) use builtin::{OTHER_OFF, probe_section, with_probe};
