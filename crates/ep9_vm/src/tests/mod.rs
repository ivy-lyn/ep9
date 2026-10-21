//! Interpreter tests. Programs are hand-assembled with the `ep9_bytecode` builder (no
//! BytecodeGen here). `run_all` runs a program under every dispatch expansion, with and
//! without GC stress (a collection before every allocation, then the heap verifier), and
//! checks that all runs agree.
//!
//! This file is frozen after the week-4 scaffold and again after M2-P0, which declared the
//! month-2 modules (`abi_index`, `class_fields`, `eval`, `generator`, `isolate_api`, `jobs`,
//! `native` (feature `spike-native`), `slots`, `with_stmt`): each area adds its tests to its own module below, and host
//! functions for tests live in `support.rs`.

#![allow(clippy::unwrap_used)]

mod abi;
mod abi_index;
mod accessors;
mod basic;
mod builtins;
mod calls;
mod class_fields;
mod eval;
mod exceptions;
mod feedback;
mod gc;
mod generator;
mod ic;
mod interp;
mod isolate_api;
mod iter;
mod jobs;
#[cfg(feature = "spike-native")]
mod native;
mod objects;
mod shapes;
mod slots;
mod strings;
mod support;
mod symbols;
mod with_stmt;
mod wrappers;

#[path = "../../../ep9_bytecode/tests/common/mod.rs"]
pub(crate) mod common;

use ep9_bytecode::{
    CodeKind, FeedbackSlot, FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionFlags,
    FunctionId, FunctionKind, FunctionMeta, Register, RegisterList, ScopeFlags, ScopeId, ScopeKind,
    UnitBuilder, UnitOptions, UnitString, VariableMode, VerifiedUnit,
};
use ep9_heap::HeapConfig;

use crate::{Dispatch, Exception, Isolate, IsolateConfig};

pub(crate) use support::{print, take_output};

pub(crate) fn opts() -> UnitOptions {
    UnitOptions {
        code_kind: CodeKind::Script,
        strict: false,
        typescript: None,
        engine_build_id: [0; 16],
        embed_source: true,
    }
}

/// A unit from `src` and its functions in id order (function 0 is the script).
pub(crate) fn unit(src: &str, fns: Vec<FunctionCode>) -> VerifiedUnit {
    unit_with_scopes(src, vec![], fns)
}

/// A scope for `unit_with_scopes`: kind, outer scope and slot names with modes.
pub(crate) type ScopeDef = (ScopeKind, Option<u32>, Vec<(&'static str, VariableMode)>);

/// A unit with scope descriptors (scope ids in list order; every scope needs a context).
pub(crate) fn unit_with_scopes(
    src: &str,
    scopes: Vec<ScopeDef>,
    fns: Vec<FunctionCode>,
) -> VerifiedUnit {
    let mut u = UnitBuilder::new(src, opts());
    for (kind, outer, slots) in scopes {
        let flags = if kind == ScopeKind::Script {
            ScopeFlags::default()
        } else {
            ScopeFlags::NEEDS_CONTEXT
        };
        let slots = slots
            .into_iter()
            .map(|(n, m)| (UnitString::new(n), m))
            .collect();
        u.add_scope(kind, flags, outer.map(ScopeId), slots);
    }
    for (i, f) in fns.into_iter().enumerate() {
        u.set_function(FunctionId(i as u32), f);
    }
    u.finish().unwrap()
}

/// Byte offset of `needle` in `src` (positions for location tests).
pub(crate) fn pos(src: &str, needle: &str) -> u32 {
    src.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in source")) as u32
}

/// Metadata of a normal (sloppy) function `name` with `params` parameters, child of `parent`,
/// over source bytes `start..end`.
pub(crate) fn meta(name: &str, params: u32, parent: u32, start: u32, end: u32) -> FunctionMeta {
    FunctionMeta {
        kind: FunctionKind::Normal,
        flags: FunctionFlags::SIMPLE_PARAMETERS,
        name: Some(UnitString::new(name)),
        parent: Some(FunctionId(parent)),
        outer_scope: None,
        source_start: start,
        source_end: end,
        parameter_count: params,
        length: params,
    }
}

/// A function builder with a few registers and feedback slots that tests use everywhere.
pub(crate) struct Asm {
    pub b: FunctionBuilder,
    /// Scratch registers for `print_acc` and calls.
    pub f: Register,
    pub a: RegisterList,
    pub call: FeedbackSlot,
    pub bin: FeedbackSlot,
    pub cmp: FeedbackSlot,
    pub load: FeedbackSlot,
    pub store: FeedbackSlot,
    pub named: FeedbackSlot,
    pub set_named: FeedbackSlot,
    pub keyed: FeedbackSlot,
    pub set_keyed: FeedbackSlot,
    pub closure: FeedbackSlot,
    pub lit: FeedbackSlot,
    pub loop_: FeedbackSlot,
}

impl Asm {
    pub fn new(m: FunctionMeta) -> Asm {
        let mut b = FunctionBuilder::new(m);
        let f = b.new_register();
        let a = b.new_register_list(3);
        Asm {
            call: b.feedback_slot(FeedbackSlotKind::Call),
            bin: b.feedback_slot(FeedbackSlotKind::BinaryOp),
            cmp: b.feedback_slot(FeedbackSlotKind::CompareOp),
            load: b.feedback_slot(FeedbackSlotKind::LoadGlobal),
            store: b.feedback_slot(FeedbackSlotKind::StoreGlobal),
            named: b.feedback_slot(FeedbackSlotKind::LoadNamed),
            set_named: b.feedback_slot(FeedbackSlotKind::StoreNamed),
            keyed: b.feedback_slot(FeedbackSlotKind::LoadKeyed),
            set_keyed: b.feedback_slot(FeedbackSlotKind::StoreKeyed),
            closure: b.feedback_slot(FeedbackSlotKind::CreateClosure),
            lit: b.feedback_slot(FeedbackSlotKind::Literal),
            loop_: b.feedback_slot(FeedbackSlotKind::JumpLoop),
            b,
            f,
            a,
        }
    }

    /// The script entry function (sloppy).
    pub fn script(src_len: usize) -> Asm {
        Asm::new(FunctionMeta::entry(CodeKind::Script, false, src_len as u32))
    }

    /// `acc = globalThis[name]` (ReferenceError if missing).
    pub fn lda_global(&mut self, name: &str) -> &mut Self {
        let k = self.b.string_constant(name);
        self.b.lda_global(k, self.load);
        self
    }

    /// `globalThis[name] = acc`.
    pub fn sta_global(&mut self, name: &str) -> &mut Self {
        let k = self.b.string_constant(name);
        self.b.sta_global(k, self.store);
        self
    }

    pub fn lda_str(&mut self, s: &str) -> &mut Self {
        let k = self.b.string_constant(s);
        self.b.lda_constant(k);
        self
    }

    pub fn lda_num(&mut self, d: f64) -> &mut Self {
        let k = self.b.number_constant(d);
        self.b.lda_constant(k);
        self
    }

    /// `print(acc)`. Clobbers `f`, `a[0]` and the accumulator.
    pub fn print_acc(&mut self) -> &mut Self {
        let (f, a0) = (self.f, self.a.get(0));
        self.b.star(a0);
        self.lda_global("print");
        self.b.star(f);
        self.b.call_undefined_receiver1(f, a0, self.call);
        self
    }

    /// `acc = obj.name`.
    pub fn get_named(&mut self, obj: Register, name: &str) -> &mut Self {
        let k = self.b.string_constant(name);
        self.b.get_named_property(obj, k, self.named);
        self
    }

    /// `obj.name = acc`.
    pub fn set_named(&mut self, obj: Register, name: &str) -> &mut Self {
        let k = self.b.string_constant(name);
        self.b.set_named_property(obj, k, self.set_named);
        self
    }

    /// `acc = new Ctor(args)` for the global constructor `ctor` with the arguments in
    /// `args` (clobbers `f`).
    pub fn construct_global(&mut self, ctor: &str, args: RegisterList) -> &mut Self {
        let f = self.f;
        self.lda_global(ctor);
        self.b.star(f);
        self.b.construct(f, args, args.len(), self.call);
        self
    }

    pub fn finish(self) -> FunctionCode {
        self.b.finish().unwrap()
    }
}

/// What one run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    pub result: Result<String, Exception>,
    pub output: Vec<String>,
}

pub(crate) fn config(dispatch: Dispatch, stress: bool) -> IsolateConfig {
    IsolateConfig {
        heap: HeapConfig {
            gc_stress: u32::from(stress),
            verify_after_gc: stress,
            ..HeapConfig::default()
        },
        dispatch,
        ..IsolateConfig::default()
    }
}

/// A new isolate with `print`.
pub(crate) fn isolate(config: IsolateConfig) -> Isolate {
    let mut iso = Isolate::new(config).unwrap();
    iso.register_host_function("print", 1, print);
    iso
}

/// Runs `unit` in a new isolate; checks the heap after the run.
pub(crate) fn run_in(config: IsolateConfig, unit: VerifiedUnit) -> Run {
    let mut iso = isolate(config);
    take_output();
    let r = iso.run_script(unit, "test.js").map(|v| iso.display(v));
    let output = take_output();
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    Run { result: r, output }
}

/// Runs `f` on a thread with a big native stack. Debug builds have no tail calls, so the
/// sibling expansion grows the native stack by one frame per instruction there.
pub(crate) fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Runs the unit from `make` under every dispatch expansion, with and without GC stress, and
/// checks that all runs agree. Returns the run.
pub(crate) fn run_all(make: fn() -> VerifiedUnit) -> Run {
    let mut first: Option<(String, Run)> = None;
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let r = on_big_stack(move || run_in(config(d, stress), make()));
            let label = format!("{} stress={stress}", d.name());
            match &first {
                None => first = Some((label, r)),
                Some((l0, r0)) => assert_eq!(&r, r0, "{label} differs from {l0}"),
            }
        }
    }
    first.unwrap().1
}

/// The result text of a run that must succeed.
pub(crate) fn ok(r: &Run) -> &str {
    match &r.result {
        Ok(s) => s,
        Err(e) => panic!("uncaught exception: {e}\n{:?}", e.stack),
    }
}
