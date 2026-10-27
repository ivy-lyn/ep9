//! The in-memory finalizer: one chunk of the process code range per module, the unwind data
//! registered before any code can run, then a dispatch-slot install per function.
//!
//! ```text
//! chunk (whole pages of CodeRange::process())
//!   [f0 code][pad to 16][f1 code]...[pad to 4][f0 UNWIND_INFO][f1 UNWIND_INFO]   (Windows only)
//! unix:    one .eh_frame section (one CIE, one FDE per function) -> EhFrameRegistration
//! Windows: one RUNTIME_FUNCTION per function -> FunctionTableRegistration (one per chunk)
//! ```
//!
//! Drop order: the registration, then the code memory, then the chunk (the code must outlive
//! the unwind data that describes it, H28). Every function installed from the module must be
//! uninstalled, or its isolate dropped, before the module drops.

use std::io;
use std::ops::Range;

use ep9_platform::jit::{CodeMemory, WxMode};
use ep9_platform::{CodeChunk, CodeRange, page_size};
use ep9_vm::{DispatchKind, InstallError, Isolate, Value};

use crate::emit::{Arch, Finalize, FunctionCode};

/// A function placed in a [`JitModule`].
#[derive(Clone, Debug)]
pub struct Placed {
    pub name: String,
    /// `FunctionCode::key`.
    pub key: usize,
    /// Its machine code in the code range (executable addresses).
    pub code: Range<usize>,
    /// The bytecode instructions it compiles (`FunctionCode::ops`).
    pub ops: Vec<(u32, ep9_bytecode::Opcode)>,
    /// `FunctionCode::ops_line`.
    pub ops_line: String,
}

#[cfg(unix)]
type Registration = ep9_platform::unwind::EhFrameRegistration;
#[cfg(windows)]
type Registration = ep9_platform::unwind::FunctionTableRegistration;

/// Live JIT code: the chunk, its code memory and the unwind registration that covers it.
pub struct JitModule {
    // Field order is drop order: the registration goes first, then the code, then the chunk.
    _registration: Registration,
    mem: CodeMemory<'static>,
    _chunk: Box<CodeChunk<'static>>,
    functions: Vec<Placed>,
}

impl std::fmt::Debug for JitModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JitModule")
            .field("mem", &self.mem)
            .field("functions", &self.functions)
            .finish()
    }
}

/// The [`Finalize`] that puts code in the process code range.
#[derive(Clone, Copy, Debug, Default)]
pub struct InMemory;

impl Finalize for InMemory {
    type Output = JitModule;
    type Error = io::Error;

    fn finalize(self, functions: Vec<FunctionCode>) -> io::Result<JitModule> {
        JitModule::new(functions)
    }
}

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

impl JitModule {
    /// Places `functions` (host architecture) in one new chunk of the process code range and
    /// registers their unwind data.
    pub fn new(functions: Vec<FunctionCode>) -> io::Result<JitModule> {
        let bad = |m: String| io::Error::new(io::ErrorKind::InvalidInput, m);
        if functions.is_empty() {
            return Err(bad("no functions".into()));
        }
        if let Some(f) = functions.iter().find(|f| Some(f.arch) != Arch::HOST) {
            return Err(bad(format!(
                "{} is {} code, not host code",
                f.name,
                f.arch.name()
            )));
        }
        // Layout: code, 16-aligned; then (Windows) the UNWIND_INFO blobs, 4-aligned.
        let mut starts = Vec::with_capacity(functions.len());
        let mut end = 0usize;
        for f in &functions {
            let s = align(end, 16);
            starts.push(s);
            end = s + f.code.len();
        }
        let mut info_at = Vec::new();
        if cfg!(windows) {
            for f in &functions {
                let w = f
                    .unwind
                    .win64
                    .as_ref()
                    .ok_or_else(|| bad(format!("{} has no UNWIND_INFO", f.name)))?;
                let s = align(end, 4);
                info_at.push(s);
                end = s + w.len();
            }
        }
        let size = align(end.max(1), page_size());
        let range = CodeRange::process().map_err(io::Error::other)?;
        let mut chunk = Box::new(range.alloc(size)?);
        let mem = chunk.commit(WxMode::DEFAULT)?;
        // SAFETY: `mem` borrows the boxed chunk, which lives at a fixed heap address and is
        // dropped after `mem` (field order of `JitModule`); the chunk is never used again
        // except to drop it, so extending the borrow to 'static never lets two users reach
        // its pages. The range itself is `&'static` (`CodeRange::process`).
        let mut mem: CodeMemory<'static> = unsafe { std::mem::transmute(mem) };
        let base = mem.exec_base();
        {
            // Every byte of the window is written (a reused MapJit page holds old bytes).
            let mut w = mem.open_write(0, end)?;
            let bytes = w.bytes();
            bytes.fill(0);
            for (f, &s) in functions.iter().zip(&starts) {
                bytes[s..s + f.code.len()].copy_from_slice(&f.code);
            }
            for (f, &s) in functions.iter().zip(&info_at) {
                let info = f.unwind.win64.as_ref().expect("checked above");
                bytes[s..s + info.len()].copy_from_slice(info);
            }
            w.close()?;
        }
        let registration = register(base, size, &functions, &starts, &info_at)?;
        let placed = functions
            .iter()
            .zip(&starts)
            .map(|(f, &s)| Placed {
                name: f.name.clone(),
                key: f.key,
                code: base + s..base + s + f.code.len(),
                ops: f.ops.clone(),
                ops_line: f.ops_line(),
            })
            .collect();
        Ok(JitModule {
            _registration: registration,
            mem,
            _chunk: chunk,
            functions: placed,
        })
    }

    /// The placed functions, in the order given to [`JitModule::new`].
    pub fn functions(&self) -> &[Placed] {
        &self.functions
    }

    /// The executable range of the whole chunk.
    pub fn range(&self) -> Range<usize> {
        self.mem.exec_base()..self.mem.exec_base() + self.mem.len()
    }

    /// True if `pc` lies in this module's code.
    pub fn contains(&self, pc: usize) -> bool {
        self.functions.iter().any(|f| f.code.contains(&pc))
    }

    /// Installs function `index` of this module as the native code of `f` (a Baseline slot
    /// write and a code-registry entry, `ep9_vm::install_native`).
    ///
    /// # Safety
    /// The function at `index` was compiled from `f`'s bytecode (its `key` is `f`'s
    /// FunctionInfo address), and this module outlives the installation: uninstall `f`, or
    /// drop its isolate, before the module drops.
    pub unsafe fn install(
        &self,
        iso: &Isolate,
        f: Value,
        index: usize,
    ) -> Result<(), InstallError> {
        let p = &self.functions[index];
        // SAFETY: the code at `p.code` runs `f`'s bytecode with the NativeEntry ABI and the
        // RuntimeTable rules (the emitter's contract), and it stays mapped while the module
        // lives (caller contract).
        unsafe { ep9_vm::install_native(iso, f, DispatchKind::Baseline, p.code.clone()) }
    }
}

/// Registers the unwind data of `functions` (placed at `base + starts[i]`).
#[cfg(unix)]
fn register(
    base: usize,
    _size: usize,
    functions: &[FunctionCode],
    starts: &[usize],
    _info_at: &[usize],
) -> io::Result<Registration> {
    use ep9_platform::unwind::cfi::EhFrameBuilder;
    let arch = functions[0].arch;
    let mut b = EhFrameBuilder::new(&arch.cie());
    for (f, &s) in functions.iter().zip(starts) {
        b.add_fde(base + s, f.code.len(), &f.unwind.cfi);
    }
    let section = b.finish();
    // SAFETY: each FDE describes the code the emitter made (its prologue and epilogue rows);
    // the code stays mapped and unchanged until the registration drops (field order of
    // `JitModule`); the span of this section lies inside this module's own chunk, so it does
    // not overlap the span of any other live registration (chunks are disjoint).
    unsafe { ep9_platform::unwind::EhFrameRegistration::register(section) }
}

/// Registers the unwind data of `functions` (placed at `base + starts[i]`, UNWIND_INFO at
/// `base + info_at[i]`).
#[cfg(windows)]
fn register(
    base: usize,
    size: usize,
    functions: &[FunctionCode],
    starts: &[usize],
    info_at: &[usize],
) -> io::Result<Registration> {
    use ep9_platform::unwind::win64::RuntimeFunction;
    let entries: Box<[RuntimeFunction]> = functions
        .iter()
        .zip(starts)
        .zip(info_at)
        .map(|((f, &s), &i)| RuntimeFunction {
            begin: s as u32,
            end: (s + f.code.len()) as u32,
            unwind_info: i as u32,
        })
        .collect();
    // SAFETY: each UNWIND_INFO describes the prologue the emitter made; code and UNWIND_INFO
    // stay mapped and unchanged until the registration drops (field order of `JitModule`); no
    // other table covers this chunk (chunks are disjoint).
    unsafe { ep9_platform::unwind::FunctionTableRegistration::register(base..base + size, entries) }
}
