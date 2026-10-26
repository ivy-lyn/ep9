//! The spike's runner, on the pattern of `spike_b::harness` (spikes/spike-b/src/harness.rs:1-6):
//! compile JS, make an isolate with `print` and `gc`, run scripts and capture what `print`
//! writes, find functions, and install placement (b) code ([`Session::install_b`]).
//!
//! Two differences from Spike B's runner:
//! - The isolate has the builtins table (`ep9_builtins::TABLE`, as the `ep9` binary), because
//!   the protocol kernels are whole v8-v7 files (strings, `Math`, arrays).
//! - The session keeps each script's unit, so [`crate::facts`] can export the live facts of a
//!   unit and map a function name or source line to its stable function id.
//!
//! A [`Session`] owns its isolate and the JIT modules whose code the isolate runs, and drops
//! the isolate first (field order), so installed code never outlives its module.

use std::cell::RefCell;

use ep9_bytecode::{UnitString, VerifiedUnit};
use ep9_vm::{Args, Cx, Isolate, IsolateConfig, JsResult, Local, Value};
use spike_b::emit::{Finalize, FunctionCode, FunctionInput};
use spike_b::harness::{Config, RunOutput, compile, exception_text};
use spike_b::jit::{InMemory, JitModule, Placed};

thread_local! {
    static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
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

/// The isolate settings of a run: Spike B's [`Config`] plus the builtins table.
pub fn isolate_config(config: &Config) -> IsolateConfig {
    IsolateConfig {
        builtins: Some(&ep9_builtins::TABLE),
        ..config.isolate_config()
    }
}

/// One script that ran in a session: its unit and its source text.
pub struct Script {
    pub file: String,
    pub source: String,
    pub unit: VerifiedUnit,
}

impl Script {
    /// The stable ids (preorder, H2) of the functions named `name` in this script.
    pub fn functions_named(&self, name: &str) -> Vec<u32> {
        let want = UnitString::new(name);
        let unit = self.unit.unit();
        (0..unit.functions.len() as u32)
            .filter(|&f| {
                unit.functions[f as usize]
                    .name
                    .is_some_and(|n| *unit.string(n) == want)
            })
            .collect()
    }

    /// The stable ids of the functions whose source starts on line `line` (1-based) of the
    /// script text.
    pub fn functions_on_line(&self, line: u32) -> Vec<u32> {
        let Some(range) = line_range(&self.source, line) else {
            return Vec::new();
        };
        let unit = self.unit.unit();
        (0..unit.functions.len() as u32)
            .filter(|&f| {
                f != 0 && range.contains(&(unit.functions[f as usize].source_start as usize))
            })
            .collect()
    }
}

/// The byte range of line `line` (1-based) of `text`, without its line end.
fn line_range(text: &str, line: u32) -> Option<std::ops::Range<usize>> {
    let mut start = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i + 1 == line as usize {
            return Some(start..start + l.trim_end_matches(['\n', '\r']).len());
        }
        start += l.len();
    }
    None
}

/// An isolate with the builtins, `print` and `gc`, the scripts it ran, and the JIT modules
/// whose code it runs.
pub struct Session {
    // Field order is drop order: the isolate goes before the code it may run.
    pub iso: Isolate,
    modules: Vec<JitModule>,
    scripts: Vec<Script>,
    targets: u32,
}

impl Session {
    pub fn new(config: &Config) -> Result<Session, String> {
        let mut iso = Isolate::new(isolate_config(config)).map_err(|e| format!("{e:?}"))?;
        iso.register_host_function("print", 1, print);
        iso.register_host_function("gc", 0, gc);
        Ok(Session {
            iso,
            modules: Vec::new(),
            scripts: Vec::new(),
            targets: 0,
        })
    }

    /// Compiles and runs `source` as the script `file`. A script that compiles is kept
    /// ([`Session::scripts`]), even if it throws.
    pub fn run(&mut self, source: &str, file: &str) -> RunOutput {
        take_output();
        let result = match compile(source) {
            Ok(unit) => {
                self.scripts.push(Script {
                    file: file.to_string(),
                    source: source.to_string(),
                    unit: unit.clone(),
                });
                self.iso
                    .with_cx_checkpoint(|cx| cx.run_script(unit, file).map(|v| cx.display(v)))
                    .map_err(|e| exception_text(&e))
            }
            Err(e) => Err(e),
        };
        RunOutput {
            result,
            output: take_output(),
        }
    }

    /// The scripts that compiled in this session, in run order.
    pub fn scripts(&self) -> &[Script] {
        &self.scripts
    }

    /// The script named `file` (the last one, if it ran more than once).
    pub fn script(&self, file: &str) -> Option<&Script> {
        self.scripts.iter().rev().find(|s| s.file == file)
    }

    /// The function value of the JS expression `expr` (for example
    /// `Scheduler.prototype.schedule`). The value stays reachable from a global
    /// (`__spikeCTarget<n>`), so it lives as long as the session.
    pub fn function_value(&mut self, expr: &str) -> Result<Value, String> {
        let name = format!("__spikeCTarget{}", self.targets);
        self.targets += 1;
        let r = self.run(&format!("var {name} = ({expr});"), "target.js");
        r.result.map_err(|e| format!("{expr}: {e}"))?;
        ep9_vm::NativeCx::global_value(&self.iso, &name).ok_or_else(|| format!("no global {name}"))
    }

    /// The emitter input of the function value `f`.
    pub fn input_of(&self, f: Value) -> Result<FunctionInput<'_>, String> {
        let t = ep9_vm::native_target(&self.iso, f)
            .ok_or_else(|| "not a function with bytecode".to_string())?;
        Ok(FunctionInput {
            name: t.name,
            bytecode: t.bytecode,
            parameter_count: t.parameter_count,
            register_count: t.register_count,
            handler_count: t.handler_count,
            key: t.info,
        })
    }

    /// The emitter input of the global function `name`.
    pub fn function_input(&self, name: &str) -> Result<FunctionInput<'_>, String> {
        let f = ep9_vm::NativeCx::global_value(&self.iso, name)
            .ok_or_else(|| format!("no global {name}"))?;
        self.input_of(f).map_err(|e| format!("{name}: {e}"))
    }

    /// The dispatch-slot kind of the function value `f` (`Interp` = T0, `Opt` = placement (b)
    /// code installed).
    pub fn slot_kind(&self, f: Value) -> Option<ep9_vm::DispatchKind> {
        ep9_vm::native_target(&self.iso, f).and_then(|t| t.kind)
    }

    /// Places `codes` (made from the function values `fns`, same order) in one module and
    /// installs each in an Opt slot (placement (b), `ep9_vm::install_native_b`). All or
    /// nothing, as `spike_b::harness::Session::install_as`: if an install is refused, the
    /// functions this call installed before it go back to T0 before the module drops.
    pub fn install_b(
        &mut self,
        fns: &[Value],
        codes: Vec<FunctionCode>,
    ) -> Result<&[Placed], String> {
        if fns.len() != codes.len() {
            return Err(format!("{} functions, {} codes", fns.len(), codes.len()));
        }
        for (i, (&f, code)) in fns.iter().zip(&codes).enumerate() {
            let input = self.input_of(f).map_err(|e| format!("function {i}: {e}"))?;
            if input.key != code.key {
                return Err(format!(
                    "{}: the code was made for another function",
                    code.name
                ));
            }
        }
        let module = InMemory.finalize(codes).map_err(|e| e.to_string())?;
        for (i, &f) in fns.iter().enumerate() {
            let range = module.functions()[i].code.clone();
            // SAFETY: function `i` of the module was made for `f` (the key check above: the
            // same FunctionInfo, and no JS ran since) by an emitter of placement (b), so it
            // has the T0 handler ABI and the (b) exit rules that `install_native_b` requires.
            // The module outlives the installation: on success it goes to `self.modules`,
            // which drops after `self.iso`; on a refusal the loop below uninstalls functions
            // 0..i before the module drops (or keeps the module if one cannot be uninstalled).
            let r = unsafe { ep9_vm::install_native_b(&self.iso, f, range) };
            if let Err(e) = r {
                let stuck = fns[..i]
                    .iter()
                    .filter(|&&g| ep9_vm::uninstall_native(&self.iso, g).is_err())
                    .count();
                let name = module.functions()[i].name.clone();
                if stuck > 0 {
                    self.modules.push(module);
                }
                return Err(format!("{name}: install refused: {e:?}"));
            }
        }
        self.modules.push(module);
        Ok(self.modules.last().expect("pushed").functions())
    }

    /// The JIT modules of this session.
    pub fn modules(&self) -> &[JitModule] {
        &self.modules
    }
}

/// Runs `source` on a new session, then a full collection and the heap verifier.
pub fn run_checked(config: &Config, source: &str, file: &str) -> Result<RunOutput, String> {
    let mut s = Session::new(config)?;
    let r = s.run(source, file);
    s.iso.collect_garbage();
    s.iso
        .verify_heap()
        .map_err(|e| format!("heap verifier after the run: {:?}", e.errors))?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spike_b::emit::Emitter;

    /// The builtins are there (`Math`, string methods), and `print` captures.
    #[test]
    fn builtins_and_print() {
        let r = run_checked(
            &Config::default(),
            "print(Math.floor(7 / 2), 'ab'.charCodeAt(1), [1, 2, 3].length);",
            "b.js",
        )
        .expect("run");
        assert_eq!(r.result, Ok("undefined".to_string()));
        assert_eq!(r.output, ["3 98 3"]);
    }

    /// Function lookup by name, by source line and by expression.
    #[test]
    fn function_lookup() {
        let src = "function f(a) { return a & 3; }\nvar o = { m: function () { return 1; } };\n";
        let mut s = Session::new(&Config::default()).expect("session");
        s.run(src, "l.js").result.expect("run");
        let script = s.script("l.js").expect("script");
        let f = script.functions_named("f");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(script.functions_on_line(1), f);
        assert_eq!(script.functions_on_line(2).len(), 1);
        assert!(script.functions_on_line(3).is_empty());
        let m = s.function_value("o.m").expect("o.m");
        let input = s.input_of(m).expect("input");
        assert!(!input.bytecode.is_empty());
        let none = s.function_value("o.nope").expect("an undefined value");
        assert!(s.input_of(none).is_err());
        assert!(s.function_value("o.m(").is_err(), "a syntax error");
        assert!(s.function_input("f").is_ok());
        assert!(s.function_input("nope").is_err());
    }

    /// The install path SPIKEC-1 uses: Spike B's placement (b) emitter makes code for a loop
    /// function, [`Session::install_b`] installs it in an Opt slot of an isolate with the
    /// builtins, and the output equals T0's. The T0 dispatch is the threaded one (placement
    /// (b) code runs only under it; the debug match loop runs the bytecode instead).
    #[test]
    fn install_b_matches_t0() {
        let Some(mut emitter) = spike_b::jit_b::BEmitter::host() else {
            eprintln!("skip: no placement (b) emitter for this architecture");
            return;
        };
        let defs =
            "function sum(n) { let a = 0; for (let i = 0; i < n; i++) { a = a + i; } return a; }";
        let main = "print(sum(1000), sum(0), sum(70000));";
        let config = Config::default().threaded();
        let t0 = {
            let mut s = Session::new(&config).expect("session");
            s.run(defs, "defs.js").result.expect("defs");
            s.run(main, "main.js")
        };
        let mut s = Session::new(&config).expect("session");
        s.run(defs, "defs.js").result.expect("defs");
        let f = s.function_value("sum").expect("sum");
        let code = emitter
            .emit(&s.input_of(f).expect("input"))
            .expect("(b) code");
        s.install_b(&[f], vec![code]).expect("install");
        assert_eq!(s.slot_kind(f), Some(ep9_vm::DispatchKind::Opt));
        let jit = s.run(main, "main.js");
        assert_eq!(jit, t0);
        assert_eq!(jit.output, ["499500 0 2449965000"]);
        // A second install of the same function is refused and leaves no module behind.
        let code = emitter
            .emit(&s.input_of(f).expect("input"))
            .expect("(b) code");
        let e = s.install_b(&[f], vec![code]).expect_err("already native");
        assert!(e.contains("AlreadyNative"), "{e}");
        assert_eq!(s.modules().len(), 1);
    }
}
