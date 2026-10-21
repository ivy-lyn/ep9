//! The spike's runner: compile JS with ep9_frontend, make an isolate with `print` and `gc`,
//! JIT and install named functions ([`Session::jit`]), run scripts, and capture what `print`
//! writes.
//!
//! A [`Session`] owns its isolate and the JIT modules whose code the isolate runs, and drops
//! the isolate first (field order), so installed code never outlives its module.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::ops::Range;

use ep9_bytecode::VerifiedUnit;
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::{Args, Cx, Dispatch, Exception, Isolate, IsolateConfig, JsResult, Local, Value};

use crate::emit::{Emitter, Finalize, FunctionCode, FunctionInput, Refusal};
use crate::jit::{InMemory, JitModule, Placed, SpikeEmitter};

/// The T0 dispatch of the spike's runs.
///
/// x86-64 with `panic = "abort"` (the spike's release profile, Cargo.toml): rustc guards
/// every call through a `sysv64-unwind` handler pointer (`interp::Handler` on x86-64) with an
/// abort-on-unwind landing pad (`panic_cannot_unwind`), so no handler ends in a sibling call
/// (`cargo xtask disasm-check --binary` on the spike: 50 of 282 tail handlers end in a tail
/// jump), and a T0 loop of 10^5 iterations overflows an 8 MiB stack. The match loop does not
/// need sibling calls. Everywhere else: `Dispatch::DEFAULT`.
pub const DISPATCH: Dispatch = if cfg!(all(target_arch = "x86_64", panic = "abort")) {
    Dispatch::Match
} else {
    Dispatch::DEFAULT
};

/// Isolate settings of a run.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Collect before every n-th allocation (0 = off).
    pub gc_stress: u32,
    /// Run the heap verifier after every collection.
    pub verify: bool,
    /// Fill freed cells with the zap word.
    pub zap: bool,
    pub feedback_threshold: u32,
    pub dispatch: Dispatch,
    pub vm_stack_slots: usize,
    /// Keep the cells the last conservative scan found (`Heap::last_conservative_roots`).
    pub record_roots: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            gc_stress: 0,
            verify: false,
            zap: cfg!(debug_assertions),
            feedback_threshold: IsolateConfig::DEFAULT_FEEDBACK_THRESHOLD,
            dispatch: DISPATCH,
            vm_stack_slots: IsolateConfig::default().vm_stack_slots,
            record_roots: false,
        }
    }
}

impl Config {
    /// GC stress: a collection before every allocation, the verifier after each, zapping.
    pub fn stress() -> Config {
        Config {
            gc_stress: 1,
            verify: true,
            zap: true,
            ..Config::default()
        }
    }

    /// The feedback vector at the first call (`--feedback-threshold 0`).
    pub fn eager() -> Config {
        Config {
            feedback_threshold: 0,
            ..Config::default()
        }
    }

    /// The three configurations of check (a): default, `--feedback-threshold 0`, GC stress.
    pub fn check_a() -> [(&'static str, Config); 3] {
        [
            ("default", Config::default()),
            ("feedback-threshold 0", Config::eager()),
            ("gc-stress+verify+zap", Config::stress()),
        ]
    }

    pub fn isolate_config(&self) -> IsolateConfig {
        IsolateConfig {
            heap: HeapConfig {
                gc_stress: self.gc_stress,
                verify_after_gc: self.verify,
                zap: self.zap,
                record_conservative_roots: self.record_roots,
                ..HeapConfig::default()
            },
            dispatch: self.dispatch,
            feedback_threshold: self.feedback_threshold,
            vm_stack_slots: self.vm_stack_slots,
            ..IsolateConfig::default()
        }
    }
}

/// Compiles a sloppy script.
pub fn compile(source: &str) -> Result<VerifiedUnit, String> {
    ep9_frontend::compile(
        source,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .map_err(|e| format!("compile error: {e:?}"))
}

thread_local! {
    static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The code of the live JIT modules of this thread's sessions.
    static JIT_RANGES: RefCell<Vec<Range<usize>>> = const { RefCell::new(Vec::new()) };
}

/// True if `pc` lies in the code of a live JIT module of a session on this thread (host
/// functions use it to find JIT frames in a native stack walk).
pub fn is_jit_pc(pc: usize) -> bool {
    JIT_RANGES.with(|r| r.borrow().iter().any(|c| c.contains(&pc)))
}

/// The lines `print` wrote on this thread since the last call.
pub fn take_output() -> Vec<String> {
    OUTPUT.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

/// `print(...args)`: ToString of each argument, space separated, as one captured line.
fn print<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let mut line = String::new();
    for (i, v) in args.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        let s = cx.to_rust_string(v)?;
        line.push_str(&s);
    }
    OUTPUT.with(|o| o.borrow_mut().push(line));
    Ok(Local::undefined())
}

/// `gc()`: a full collection.
fn gc<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.collect_garbage();
    Ok(Local::undefined())
}

/// The result of one script run and the lines it printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutput {
    /// `Ok(display of the completion value)`, or `Err(message + stack)`.
    pub result: Result<String, String>,
    pub output: Vec<String>,
}

impl RunOutput {
    /// All of it as text: the printed lines, then the result line.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for l in &self.output {
            let _ = writeln!(s, "{l}");
        }
        match &self.result {
            Ok(v) => {
                let _ = writeln!(s, "=> {v}");
            }
            Err(e) => {
                let _ = writeln!(s, "=> uncaught {e}");
            }
        }
        s
    }
}

/// The text of an uncaught exception: message, then its stack (if any).
pub fn exception_text(e: &Exception) -> String {
    match &e.stack {
        Some(st) if !e.terminated => st.clone(),
        _ => e.message.clone(),
    }
}

/// An isolate with `print` and `gc`, and the JIT modules whose code it runs.
pub struct Session {
    // Field order is drop order: the isolate goes before the code it may run.
    pub iso: Isolate,
    modules: Vec<JitModule>,
}

impl Session {
    pub fn new(config: &Config) -> Result<Session, String> {
        let mut iso = Isolate::new(config.isolate_config()).map_err(|e| format!("{e:?}"))?;
        iso.register_host_function("print", 1, print);
        iso.register_host_function("gc", 0, gc);
        Ok(Session {
            iso,
            modules: Vec::new(),
        })
    }

    /// Compiles and runs `source` as the script `file`.
    pub fn run(&mut self, source: &str, file: &str) -> RunOutput {
        take_output();
        let result = match compile(source) {
            Ok(unit) => self
                .iso
                .run_script(unit, file)
                .map(|v| self.iso.display(v))
                .map_err(|e| exception_text(&e)),
            Err(e) => Err(e),
        };
        RunOutput {
            result,
            output: take_output(),
        }
    }

    /// The emitter input for the global function `name`.
    fn input(&self, name: &str) -> Result<(Value, FunctionInput<'_>), String> {
        let f = self
            .iso
            .global(name)
            .ok_or_else(|| format!("no global {name}"))?;
        let t = ep9_vm::native_target(&self.iso, f)
            .ok_or_else(|| format!("{name} is not a function with bytecode"))?;
        Ok((
            f,
            FunctionInput {
                name: t.name,
                bytecode: t.bytecode,
                parameter_count: t.parameter_count,
                register_count: t.register_count,
                handler_count: t.handler_count,
                key: t.info,
            },
        ))
    }

    /// Compiles the global function `name` without installing it.
    pub fn compile_function(&self, name: &str) -> Result<FunctionCode, String> {
        let (_, input) = self.input(name)?;
        let mut e = SpikeEmitter::host().ok_or_else(|| Refusal::Arch.to_string())?;
        e.emit(&input).map_err(|r| format!("{name}: {r}"))
    }

    /// JIT-compiles the global functions `names` into one module and installs each (a
    /// Baseline slot): every call from T0 or JIT code then runs the native code. Fails if
    /// any of them is refused (it would stay in T0).
    pub fn jit(&mut self, names: &[&str]) -> Result<&[Placed], String> {
        let mut codes = Vec::with_capacity(names.len());
        for name in names {
            codes.push(self.compile_function(name)?);
        }
        self.install(names, codes)
    }

    /// The emitter input of the global function `name` (for code made outside the emitter,
    /// such as the probe of check (g)).
    pub fn function_input(&self, name: &str) -> Result<FunctionInput<'_>, String> {
        self.input(name).map(|(_, i)| i)
    }

    /// Places `codes` (made from the global functions `names`, same order) in one module and
    /// installs each. All or nothing: if an install is refused, the functions this call
    /// installed before it go back to T0 before the module drops, so no dispatch slot or
    /// registry entry points at freed code.
    pub fn install(
        &mut self,
        names: &[&str],
        codes: Vec<FunctionCode>,
    ) -> Result<&[Placed], String> {
        let mut fns = Vec::with_capacity(names.len());
        for (name, code) in names.iter().zip(&codes) {
            let (f, input) = self.input(name)?;
            if input.key != code.key {
                return Err(format!("{name}: the code was made for another function"));
            }
            fns.push(f);
        }
        let module = InMemory.finalize(codes).map_err(|e| e.to_string())?;
        for (i, &f) in fns.iter().enumerate() {
            // SAFETY: function `i` of the module was compiled from `f` (same global, the
            // isolate did not run in between). The module outlives the installation: on
            // success it goes to `self.modules`, which drops after `self.iso`; on a refusal
            // the loop below uninstalls functions 0..i before the module drops.
            if let Err(e) = unsafe { module.install(&self.iso, f, i) } {
                // No JS ran since the installs, so each uninstall finds its own entry. If one
                // fails anyway, keep the module: its code must stay mapped.
                let stuck = fns[..i]
                    .iter()
                    .filter(|&&g| ep9_vm::uninstall_native(&self.iso, g).is_err())
                    .count();
                if stuck > 0 {
                    self.keep(module);
                }
                return Err(format!("{}: install refused: {e:?}", names[i]));
            }
        }
        self.keep(module);
        Ok(self.modules.last().expect("pushed").functions())
    }

    /// Records `module` as live: its code ranges for [`is_jit_pc`], the module in
    /// `self.modules`.
    fn keep(&mut self, module: JitModule) {
        JIT_RANGES.with(|r| {
            r.borrow_mut()
                .extend(module.functions().iter().map(|p| p.code.clone()))
        });
        self.modules.push(module);
    }

    /// The JIT modules of this session.
    pub fn modules(&self) -> &[JitModule] {
        &self.modules
    }

    /// True if `pc` lies in JIT code of this session.
    pub fn is_jit_pc(&self, pc: usize) -> bool {
        self.modules.iter().any(|m| m.contains(pc))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        JIT_RANGES.with(|r| {
            r.borrow_mut()
                .retain(|c| !self.modules.iter().any(|m| m.contains(c.start)));
        });
    }
}

/// Runs `defs` (function declarations), JIT-compiles `jit` (may be empty: all T0), then runs
/// `main`. Returns the main run, or the setup error.
pub fn run_program(
    config: &Config,
    defs: &str,
    jit: &[&str],
    main: &str,
) -> Result<RunOutput, String> {
    let mut s = Session::new(config)?;
    let d = s.run(defs, "defs.js");
    if let Err(e) = d.result {
        return Err(format!("defs: {e}"));
    }
    if !jit.is_empty() {
        s.jit(jit)?;
    }
    let r = s.run(main, "main.js");
    s.iso.collect_garbage();
    s.iso
        .verify_heap()
        .map_err(|e| format!("heap verifier after the run: {:?}", e.errors))?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structural::LOOP_DEFS;
    use ep9_vm::DispatchKind;

    /// Configs J and JC of the protocol: Rust calls the JIT-installed `drive(f, n)` through
    /// `call_from_rust` and gets n, also under GC stress. A throw and a termination come back
    /// as `Err`, and the next `call_from_rust` and `Isolate::call` still work.
    #[test]
    fn call_from_rust_runs_jit_drive() {
        for config in [Config::default(), Config::stress()] {
            let mut s = Session::new(&config).expect("session");
            s.run(LOOP_DEFS, "defs.js").result.expect("defs");
            let boom = "function boom(x) { if (x === 3) throw new Error('b' + x); return x; }";
            s.run(boom, "boom.js").result.expect("boom");
            s.jit(&["id", "drive"]).expect("jit");
            let g = |s: &Session, n: &str| s.iso.global(n).expect("global");
            let (drive, id, boom) = (g(&s, "drive"), g(&s, "id"), g(&s, "boom"));
            let u = Value::UNDEFINED;
            let n = if config.gc_stress != 0 { 200 } else { 100_000 };
            let call = |s: &mut Session, f: Value, n: i32| {
                ep9_vm::call_from_rust(&mut s.iso, drive, u, &[f, Value::int32(n)])
                    .map(|v| s.iso.display(v))
            };
            assert_eq!(call(&mut s, id, n).expect("drive(id, n)"), n.to_string());
            let e = call(&mut s, boom, 10).expect_err("drive(boom, 10) throws");
            assert!(e.message.contains("b3") && !e.terminated, "{e:?}");
            assert_eq!(call(&mut s, id, 5).expect("after the throw"), "5");
            // `boom` stays in T0; `Isolate::call` on a native slot throws not_implemented
            // (call.rs), so the JIT entry from Rust is `call_from_rust`.
            let r = s
                .iso
                .call(boom, u, &[Value::int32(9)])
                .expect("Isolate::call");
            assert_eq!(s.iso.display(r), "9");
            s.iso.terminate_handle().terminate();
            let e = call(&mut s, id, 1 << 30).expect_err("terminated");
            assert!(e.terminated, "{e:?}");
            assert_eq!(call(&mut s, id, 7).expect("after the termination"), "7");
            let e = s
                .iso
                .call(drive, u, &[id, Value::int32(3)])
                .expect_err("native slot");
            assert!(e.message.contains("not implemented"), "{e:?}");
            assert_eq!(call(&mut s, id, 11).expect("last"), "11");
        }
    }

    /// The JIT ranges this thread has recorded.
    fn jit_ranges() -> Vec<Range<usize>> {
        JIT_RANGES.with(|r| r.borrow().clone())
    }

    /// A failed install in a batch (`id` is already native, so install 1 of 2 is refused)
    /// leaves no function of that batch installed, keeps no module and no JIT range, and the
    /// session goes on: the next module (which may reuse the freed chunk) installs, and
    /// `drive` still runs in T0 with the right result.
    #[test]
    fn a_refused_install_uninstalls_the_batch() {
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(LOOP_DEFS, "defs.js").result.expect("defs");
        s.run("function other(x) { return 0; }", "other.js")
            .result
            .expect("other");
        let before = jit_ranges();
        s.jit(&["id"]).expect("jit id");
        let kept = jit_ranges();
        assert_eq!(kept.len(), before.len() + 1, "{kept:?}");
        let e = s.jit(&["drive", "id"]).expect_err("id is already native");
        assert!(e.contains("id: install refused: AlreadyNative"), "{e}");
        let kind = |s: &Session, n: &str| {
            let f = s.iso.global(n).expect("global");
            ep9_vm::native_target(&s.iso, f).expect("bytecode").kind
        };
        assert_eq!(
            kind(&s, "drive"),
            Some(DispatchKind::Interp),
            "drive rolled back"
        );
        assert_eq!(
            kind(&s, "id"),
            Some(DispatchKind::Baseline),
            "id keeps its code"
        );
        assert_eq!(s.modules().len(), 1);
        assert_eq!(jit_ranges(), kept, "the dropped module left a JIT range");
        s.jit(&["other"]).expect("jit other after the refusal");
        let r = s.run("print(drive(id, 10)); print(other(1));", "main.js");
        assert_eq!(r.result, Ok("undefined".to_string()));
        assert_eq!(r.output, ["10", "0"]);
        drop(s);
        assert_eq!(jit_ranges(), before);
    }
}
