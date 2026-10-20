//! BytecodeGen at the depth cap: every nesting form at `AST_CAP` compiles on the real worker
//! stack (no abort, no RangeError from the codegen guard), and a probe that measures the
//! codegen stack bytes per AST level (run on demand, numbers for `stack.rs`).
//!
//! ```text
//! cargo test -p ep9_frontend [--release] --lib bytecodegen::stack_tests -- --include-ignored --nocapture
//! ```

use std::fmt::Write as _;
use std::hint::black_box;

use oxc_allocator::Allocator;

use super::{CodegenError, CompileOptions, compile};
use crate::forms::{FORMS, Form};
use crate::{AST_CAP, CompileError, SourceOpts, Strictness, depth};

fn opts(form: &Form) -> SourceOpts {
    if form.module {
        SourceOpts::module()
    } else {
        SourceOpts::script(Strictness::Sloppy)
    }
}

/// AST depth of a small instance (on the test thread; small n only).
fn ast_depth(form: &Form, n: usize) -> u32 {
    let src = (form.build)(n);
    let allocator = Allocator::default();
    let program = crate::parse(&allocator, &src, opts(form)).expect("small instances parse");
    depth::check_ast_depth(&program, u32::MAX).expect("no cap")
}

/// `(a, b)` with `depth(n) = a * n + b`.
fn linear(form: &Form) -> (u32, u32) {
    let (d10, d11, d17) = (
        ast_depth(form, 10),
        ast_depth(form, 11),
        ast_depth(form, 17),
    );
    let a = d11 - d10;
    let b = d10 - 10 * a;
    assert_eq!(d17, 17 * a + b, "{}: not linear", form.name);
    (a, b)
}

/// The largest n with AST depth <= AST_CAP, or `None` for forms without AST depth (regex).
fn n_cap(form: &Form) -> Option<usize> {
    let (a, b) = linear(form);
    (a > 0).then(|| usize::try_from((AST_CAP - b) / a).unwrap())
}

/// Every form at the cap: the unit compiles, or the form is a v0-unsupported construct.
#[test]
fn every_form_compiles_at_the_cap() {
    let mut failures = Vec::new();
    for form in FORMS {
        let Some(n) = n_cap(form) else {
            continue;
        };
        let src = (form.build)(n);
        match compile(&src, opts(form), CompileOptions::default()) {
            Ok(_) | Err(CodegenError::Unsupported(_)) => {}
            Err(CodegenError::Compile(CompileError::Syntax(e))) if form.module => {
                // A module-only form that is an early error is not a codegen question.
                failures.push(format!("{} n={n}: early error {e:?}", form.name));
            }
            Err(e) => failures.push(format!("{} n={n}: {e}", form.name)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const PATTERN: u64 = 0x5A5A_C3C3_A5A5_3C3C;
/// Bytes between the measuring frame and the painted region.
const GAP: usize = 16 << 10;

/// Paint `bytes` of stack below the caller, run `f`, return its deepest stack use in bytes.
#[inline(never)]
fn measure<R>(bytes: usize, f: impl FnOnce() -> R) -> (usize, R) {
    let anchor = 0u8;
    let sp = black_box(&raw const anchor) as usize;
    let low = (sp - bytes) & !7;
    let high = (sp - GAP) & !7;
    let mut addr = low;
    while addr < high {
        // SAFETY: `[low, high)` lies inside this thread's stack: the probe thread has a 2 GiB
        // stack, `bytes` is at most 1 GiB, and the region ends 16 KiB below the current stack
        // pointer, so no live frame and no red zone uses it. Volatile writes keep the compiler
        // from removing them.
        unsafe { std::ptr::write_volatile(addr as *mut u64, PATTERN) };
        addr += 8;
    }
    let result = black_box(f());
    let mut addr = low;
    // SAFETY: as above: reads of this thread's own stack below every live frame.
    while addr < high && unsafe { std::ptr::read_volatile(addr as *const u64) } == PATTERN {
        addr += 8;
    }
    assert!(addr > low, "the painted region ({bytes} B) is too small");
    (sp.saturating_sub(addr), result)
}

/// Codegen stack bytes for one source (scope pass excluded: it runs before the measurement).
fn codegen_bytes(form: &Form, n: usize) -> (u32, usize) {
    let src = (form.build)(n);
    let opts = opts(form);
    let allocator = Allocator::default();
    let program = crate::parse(&allocator, &src, opts).expect("parses");
    let ast = depth::check_ast_depth(&program, u32::MAX).expect("no cap");
    let semantic = oxc_semantic::SemanticBuilder::new_compiler()
        .build(&program)
        .semantic;
    let fe = crate::FrontEnd {
        source: &src,
        opts,
        program: &program,
        semantic: &semantic,
    };
    let sa = crate::scope::analyze(&fe).expect("scope pass");
    let (bytes, _unit) = measure(1 << 30, || {
        super::generate(&fe, &sa, CompileOptions::default())
    });
    (ast, bytes)
}

/// Measures codegen stack bytes per AST level for every form (n = 500 -> 1500).
#[test]
#[ignore = "measurement; run on demand with --ignored --nocapture"]
fn measure_codegen_stack() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let out = std::thread::Builder::new()
        .stack_size(2 << 30)
        .spawn(move || {
            let mut out = String::new();
            writeln!(out, "profile {profile}, arch {}", std::env::consts::ARCH).unwrap();
            writeln!(out, "| form | codegen B/AST level | AST@1500 |").unwrap();
            writeln!(out, "|---|---|---|").unwrap();
            let mut worst = (0f64, "");
            for form in FORMS {
                if n_cap(form).is_none() || form.module && ast_depth(form, 10) == 0 {
                    continue;
                }
                let (a1, b1) = codegen_bytes(form, 500);
                let (a2, b2) = codegen_bytes(form, 1500);
                let per = (b2.saturating_sub(b1)) as f64 / f64::from(a2 - a1);
                if per > worst.0 {
                    worst = (per, form.name);
                }
                writeln!(out, "| {} | {per:.0} | {a2} |", form.name).unwrap();
            }
            // Budget: the worker stack (stack.rs) for AST_CAP levels, with the 1.5x margin.
            let budget = crate::stack::WORKER_STACK_BYTES as f64 / f64::from(AST_CAP) / 1.5;
            writeln!(
                out,
                "worst: {} at {:.0} B/level; budget {budget:.0} B/level",
                worst.1, worst.0
            )
            .unwrap();
            (out, worst.0 < budget)
        })
        .unwrap()
        .join()
        .unwrap();
    println!("{}", out.0);
    assert!(out.1, "codegen needs more stack per level than the budget");
}
