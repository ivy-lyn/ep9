//! Tests of the native entry (feature `spike-native` only; owners P1-NATIVE-ENTRY, then
//! P2-SPIKE-B): calls into hand-written native stubs, GC stress across a native frame, and an
//! exception that crosses one. P2-SPIKE-B adds: a generator function gets no native code
//! (`InstallError::Generator`, M2-P1-R1), and `call_from_rust` enters native code from Rust and
//! restores the entry state on every exit (D20). The P3 integration fixer adds the tests of
//! the decode-free T0 entry (`slow::call` `native_entry`, M2-P3-D21-ENTRY): every unprefixed
//! call shape takes it, every other call falls through to the decode path, and exceptions and
//! GC stress behave as on that path.
//!
//! The stubs are a few hand-encoded instructions per target (aarch64: AAPCS64 `extern "C"`;
//! x86-64: `extern "sysv64"`), checked with `llvm-mc --disassemble`:
//!
//! ```text
//! stub_param(i):  return a<i>                     ldur x0, [x1, #fp_param(i)]   | mov rax, [rsi + d]
//! stub_call(e):   return table.e(cx, fp, off, ..) frame record, ldr table, ldr fn, blr | push rbp, call
//! ```
//!
//! Each stub runs one function's bytecode: `id(x)` is `Ldar a0; Return` and its stub returns
//! a0; `g(f, x)` is `CallUndefinedReceiver1 a0, a1; Return` and its stub returns
//! `call_any(offset 0)`, and so on. Code pages are chunks of `CodeRange::process()`, committed
//! with `CodeChunk::commit`.

use core::ops::Range;

use ep9_bytecode::{FunctionId, HandlerKind, Opcode, Register, RegisterList, VerifiedUnit};
use ep9_platform::jit::WxMode;
use ep9_platform::{CodeRange, page_size};

use super::{Asm, config, isolate, meta, on_big_stack, pos, take_output, unit};
use crate::dispatch::DispatchKind;
use crate::frame::UNDEFINED;
use crate::native::{InstallError, cmp, layout};
use crate::{
    Args, Cx, Dispatch, Exception, Isolate, IsolateConfig, JsResult, Local, RegistryError, Value,
};

#[test]
fn the_native_cx_fields_are_set_at_isolate_creation() {
    let iso = Isolate::new(IsolateConfig::default()).unwrap();
    assert_eq!(iso.cx.native_limit, iso.rt.native_limit);
    assert_ne!(iso.cx.native_limit, 0);
    assert!(core::ptr::eq(
        iso.cx.runtime_table,
        &crate::native::RUNTIME_TABLE
    ));
}

#[test]
fn layout_offsets_read_the_live_isolate() {
    let iso = Isolate::new(IsolateConfig::default()).unwrap();
    let base = core::ptr::from_ref::<crate::interp::Cx>(&iso.cx).cast::<u8>();
    // SAFETY: each offset names a field of the live Cx with the type read here.
    unsafe {
        let word = |off: usize| base.add(off).cast::<usize>().read();
        assert_eq!(word(layout::CX_NATIVE_LIMIT), iso.rt.native_limit);
        assert_eq!(
            word(layout::CX_RUNTIME_TABLE),
            core::ptr::from_ref(&crate::native::RUNTIME_TABLE).addr()
        );
        assert_eq!(word(layout::CX_STACK_LIMIT), iso.cx.stack_limit.addr());
        assert_eq!(
            base.add(layout::CX_BUDGET).cast::<i64>().read(),
            iso.cx.budget
        );
        assert_eq!(word(layout::CX_GLOBAL_THIS) as u64, iso.rt.global());
        assert_eq!(word(layout::CX_INTERRUPT), iso.cx.interrupt.addr());
    }
    // The table: one pointer per entry, in declaration order.
    let rt = [
        layout::RT_POLL,
        layout::RT_CALL_ANY,
        layout::RT_ADD,
        layout::RT_INC,
        layout::RT_COMPARE,
        layout::RT_STACK_OVERFLOW,
        layout::RT_THROW,
    ];
    for (i, off) in rt.into_iter().enumerate() {
        assert_eq!(off, i * 8);
    }
    assert_eq!(size_of::<crate::native::RuntimeTable>(), rt.len() * 8);
    // Frames (FRAME 2, H14).
    assert_eq!(layout::fp_param(0), -80);
    assert_eq!(layout::fp_param(1), -88);
    assert_eq!(layout::FP_RECEIVER, -72);
    assert_eq!(layout::FP_CALLER_FP, -64);
    assert_eq!(layout::FP_FUNCTION_INFO, -32);
    assert_eq!(layout::FP_BYTECODE_OFFSET, -8);
    assert_eq!(layout::fp_register(3), 24);
    assert_eq!(layout::FI_SLOT_KIND, 0);
    assert_eq!(layout::FI_SLOT_ENTRY, 8);
    assert_eq!(layout::FUNCTION_INFO, 24);
}

// ---- hand-written native code ---------------------------------------------------------------

/// An operand of a table call in a stub.
#[derive(Clone, Copy)]
enum Src {
    /// Parameter `a<i>` of the stub's frame.
    Param(u32),
    /// A constant.
    Imm(u64),
}

#[cfg(target_arch = "aarch64")]
fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// `ldur x<t>, [x1, #off]` (aarch64, -256 <= off < 256).
#[cfg(target_arch = "aarch64")]
fn a64_ldur_fp(t: u32, off: isize) -> u32 {
    assert!((-256..256).contains(&off));
    0xF840_0000 | ((off as u32 & 0x1FF) << 12) | (1 << 5) | t
}

/// `ldr x<t>, [x<n>, #off]` (aarch64, unsigned scaled offset).
#[cfg(target_arch = "aarch64")]
fn a64_ldr(t: u32, n: u32, off: usize) -> u32 {
    assert!(off.is_multiple_of(8) && off / 8 < 4096);
    0xF940_0000 | (((off / 8) as u32) << 10) | (n << 5) | t
}

/// `movz`/`movk` x<t> = v (aarch64).
#[cfg(target_arch = "aarch64")]
fn a64_mov_imm(t: u32, v: u64) -> Vec<u32> {
    let mut out = vec![0xD280_0000 | ((v & 0xFFFF) as u32) << 5 | t];
    for hw in 1..4u32 {
        let part = ((v >> (16 * hw)) & 0xFFFF) as u32;
        if part != 0 {
            out.push(0xF280_0000 | hw << 21 | part << 5 | t);
        }
    }
    out
}

/// Native code for `return a<i>`.
fn stub_param(i: u32) -> Vec<u8> {
    stub_load(layout::fp_param(i))
}

/// Native code for `return [fp + off]` (a frame word: a parameter, the receiver, argc).
fn stub_load(off: isize) -> Vec<u8> {
    #[cfg(target_arch = "aarch64")]
    {
        words(&[a64_ldur_fp(0, off), 0xD65F_03C0])
    }
    #[cfg(target_arch = "x86_64")]
    {
        // mov rax, [rsi + disp32]; ret
        let mut v = vec![0x48, 0x8B, 0x86];
        v.extend_from_slice(&(off as i32).to_le_bytes());
        v.push(0xC3);
        v
    }
}

/// Native code for `return table[entry](cx, fp, off, srcs..)` (at most 3 sources).
fn stub_call(entry: usize, off: u32, srcs: &[Src]) -> Vec<u8> {
    assert!(srcs.len() <= 3);
    #[cfg(target_arch = "aarch64")]
    {
        let mut w = vec![
            0xA9BF_7BFD, // stp x29, x30, [sp, #-16]!
            0x9100_03FD, // mov x29, sp
            a64_ldr(16, 0, layout::CX_RUNTIME_TABLE),
            a64_ldr(16, 16, entry),
            0x5280_0000 | (off << 5) | 2, // movz w2, #off
        ];
        assert!(off < 1 << 16);
        for (k, s) in srcs.iter().enumerate() {
            let t = 3 + k as u32;
            match *s {
                Src::Param(i) => w.push(a64_ldur_fp(t, layout::fp_param(i))),
                Src::Imm(v) => w.extend(a64_mov_imm(t, v)),
            }
        }
        w.extend([
            0xD63F_0200, // blr x16
            0xA8C1_7BFD, // ldp x29, x30, [sp], #16
            0xD65F_03C0, // ret
        ]);
        words(&w)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let d32 = |v: &mut Vec<u8>, d: isize| v.extend_from_slice(&(d as i32).to_le_bytes());
        // push rbp; mov rbp, rsp (the stack is 16-byte aligned for the call)
        let mut v = vec![0x55, 0x48, 0x89, 0xE5];
        // mov rax, [rdi + CX_RUNTIME_TABLE]; mov rax, [rax + entry]
        v.extend([0x48, 0x8B, 0x87]);
        d32(&mut v, layout::CX_RUNTIME_TABLE as isize);
        v.extend([0x48, 0x8B, 0x80]);
        d32(&mut v, entry as isize);
        // mov edx, off
        v.push(0xBA);
        v.extend_from_slice(&off.to_le_bytes());
        // rcx, r8, r9: mov reg, [rsi + disp32] / mov reg, imm64
        let load = [[0x48, 0x8B, 0x8E], [0x4C, 0x8B, 0x86], [0x4C, 0x8B, 0x8E]];
        let imm = [[0x48, 0xB9], [0x49, 0xB8], [0x49, 0xB9]];
        for (k, s) in srcs.iter().enumerate() {
            match *s {
                Src::Param(i) => {
                    v.extend(load[k]);
                    d32(&mut v, layout::fp_param(i));
                }
                Src::Imm(x) => {
                    v.extend(imm[k]);
                    v.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        // call rax; pop rbp; ret
        v.extend([0xFF, 0xD0, 0x5D, 0xC3]);
        v
    }
}

/// Native code for `r = call_any(cx, fp, off, undefined); return r == 0 ? 0 : a<i>`: it
/// reads its own frame after a nested call, so the nested frames must lie above it.
fn stub_call_keep(off: u32, i: u32) -> Vec<u8> {
    #[cfg(target_arch = "aarch64")]
    {
        let mut w = vec![
            0xA9BE_7BFD, // stp x29, x30, [sp, #-32]!
            0x9100_03FD, // mov x29, sp
            0xF900_0BF3, // str x19, [sp, #16]
            0xAA01_03F3, // mov x19, x1
            a64_ldr(16, 0, layout::CX_RUNTIME_TABLE),
            a64_ldr(16, 16, layout::RT_CALL_ANY),
            0x5280_0000 | (off << 5) | 2, // movz w2, #off
        ];
        w.extend(a64_mov_imm(3, UNDEFINED));
        let p = layout::fp_param(i);
        assert!((-256..256).contains(&p));
        // ldur x0, [x19, #p]
        let reload = 0xF840_0000 | ((p as u32 & 0x1FF) << 12) | (19 << 5);
        w.extend([
            0xD63F_0200, // blr x16
            0xB400_0040, // cbz x0, +8
            reload,
            0xF940_0BF3, // ldr x19, [sp, #16]
            0xA8C2_7BFD, // ldp x29, x30, [sp], #32
            0xD65F_03C0, // ret
        ]);
        words(&w)
    }
    #[cfg(target_arch = "x86_64")]
    {
        // push rbp; mov rbp, rsp; push rbx; sub rsp, 8; mov rbx, rsi
        let mut v = vec![
            0x55, 0x48, 0x89, 0xE5, 0x53, 0x48, 0x83, 0xEC, 0x08, 0x48, 0x89, 0xF3,
        ];
        // mov rax, [rdi + CX_RUNTIME_TABLE]; mov rax, [rax + RT_CALL_ANY]
        v.extend([0x48, 0x8B, 0x87]);
        v.extend_from_slice(&(layout::CX_RUNTIME_TABLE as i32).to_le_bytes());
        v.extend([0x48, 0x8B, 0x80]);
        v.extend_from_slice(&(layout::RT_CALL_ANY as i32).to_le_bytes());
        // mov edx, off; movabs rcx, UNDEFINED
        v.push(0xBA);
        v.extend_from_slice(&off.to_le_bytes());
        v.extend([0x48, 0xB9]);
        v.extend_from_slice(&UNDEFINED.to_le_bytes());
        // call rax; test rax, rax; jz +7; mov rax, [rbx + disp32]
        v.extend([0xFF, 0xD0, 0x48, 0x85, 0xC0, 0x74, 0x07, 0x48, 0x8B, 0x83]);
        v.extend_from_slice(&(layout::fp_param(i) as i32).to_le_bytes());
        // add rsp, 8; pop rbx; pop rbp; ret
        v.extend([0x48, 0x83, 0xC4, 0x08, 0x5B, 0x5D, 0xC3]);
        v
    }
}

/// Bytes per stub in the code page.
const STUB_SIZE: usize = 128;

/// Writes `stubs` into one page of the process code range (a [`CodeRange::process`] chunk),
/// then runs `f` with each stub's code range. The page is released after `f`, so every
/// isolate that runs the stubs must drop inside `f`.
fn with_stubs<R>(stubs: &[Vec<u8>], f: impl FnOnce(&[Range<usize>]) -> R) -> R {
    let range = CodeRange::process().expect("reserve the process code range");
    let mut chunk = range.alloc(page_size()).expect("a chunk of the code range");
    let mut mem = chunk.commit(WxMode::DEFAULT).expect("commit one code page");
    assert!(stubs.len() * STUB_SIZE <= mem.len());
    let mut ranges = Vec::new();
    for (i, s) in stubs.iter().enumerate() {
        assert!(s.len() <= STUB_SIZE);
        let off = i * STUB_SIZE;
        let mut w = mem.open_write(off, s.len()).expect("open a write window");
        w.bytes().copy_from_slice(s);
        w.close().expect("close the write window");
        let start = mem.exec_addr(off);
        ranges.push(start..start + s.len());
    }
    f(&ranges)
}

// ---- the program -------------------------------------------------------------------------

/// Source of the functions unit (positions for stack traces).
const DEFS: &str = "function g(f, x) { return f(x); }\n\
function t(x) { throw new Error(x); }\n\
function r(x) { return g(r, x); }\n";

/// The functions unit: every function is a global. Natively run: id, g, add2, inc1, lt,
/// nthrow, poll, overflow, which, keep. T0 only: t, r, bump.
fn defs() -> VerifiedUnit {
    let p = |n: &str| pos(DEFS, n);
    // id(x): Ldar a0; Return
    let mut id = Asm::new(meta("id", 1, 0, 0, 0));
    let a0 = id.b.parameter(0);
    id.b.ldar(a0).return_();
    // g(f, x): return f(x)
    let mut g = Asm::new(meta("g", 2, 0, p("function g"), p("function t") - 1));
    let (a0, a1) = (g.b.parameter(0), g.b.parameter(1));
    g.b.set_expression_position(p("f(x)"));
    g.b.call_undefined_receiver1(a0, a1, g.call).return_();
    // add2(a, b): return a + b
    let mut add2 = Asm::new(meta("add2", 2, 0, 0, 0));
    let (a0, a1) = (add2.b.parameter(0), add2.b.parameter(1));
    add2.b.ldar(a1).add(a0, add2.bin).return_();
    // inc1(x): return ++x
    let mut inc1 = Asm::new(meta("inc1", 1, 0, 0, 0));
    let a0 = inc1.b.parameter(0);
    inc1.b.ldar(a0).inc(inc1.bin).return_();
    // lt(a, b): return a < b
    let mut lt = Asm::new(meta("lt", 2, 0, 0, 0));
    let (a0, a1) = (lt.b.parameter(0), lt.b.parameter(1));
    lt.b.ldar(a1).test_less_than(a0, lt.cmp).return_();
    // nthrow(x): throw x
    let mut nthrow = Asm::new(meta("nthrow", 1, 0, 0, 0));
    let a0 = nthrow.b.parameter(0);
    nthrow.b.ldar(a0).throw();
    // poll(): return true (the stub polls first)
    let mut poll = Asm::new(meta("poll", 0, 0, 0, 0));
    poll.b.lda_true().return_();
    // overflow(): its stub throws the native stack RangeError
    let mut overflow = Asm::new(meta("overflow", 0, 0, 0, 0));
    overflow.b.lda_undefined().return_();
    // t(x): throw new Error(x)
    let mut t = Asm::new(meta("t", 1, 0, p("function t"), p("function r") - 1));
    let args = RegisterList::new(t.a.get(0).local_index().unwrap(), 1);
    let a0 = t.b.parameter(0);
    t.b.ldar(a0).star(args.get(0));
    t.b.set_expression_position(p("new Error"));
    t.construct_global("Error", args);
    t.b.throw();
    // r(x): return g(r, x)
    let mut r = Asm::new(meta("r", 1, 0, p("function r"), DEFS.len() as u32));
    let a0 = r.b.parameter(0);
    let (gr, rr) = (r.f, r.a.get(0));
    r.lda_global("g");
    r.b.star(gr);
    r.lda_global("r");
    r.b.star(rr);
    r.b.set_expression_position(p("g(r, x)"));
    r.b.call_undefined_receiver2(gr, rr, a0, r.call).return_();
    // keep(f, x): f(x); return x
    let mut keep = Asm::new(meta("keep", 2, 0, 0, 0));
    let (a0, a1) = (keep.b.parameter(0), keep.b.parameter(1));
    keep.b
        .call_undefined_receiver1(a0, a1, keep.call)
        .ldar(a1)
        .return_();
    // which(a, b): Ldar a0; Return. Its stub returns a1, so a result shows which tier ran.
    let mut which = Asm::new(meta("which", 2, 0, 0, 0));
    let a0 = which.b.parameter(0);
    which.b.ldar(a0).return_();
    // bump(x): return x + "!" (allocates)
    let mut bump = Asm::new(meta("bump", 1, 0, 0, 0));
    let a0 = bump.b.parameter(0);
    let x = bump.b.new_register();
    bump.b.ldar(a0).star(x);
    bump.lda_str("!");
    bump.b.add(x, bump.bin).return_();

    let fns = vec![
        ("id", id),
        ("g", g),
        ("add2", add2),
        ("inc1", inc1),
        ("lt", lt),
        ("nthrow", nthrow),
        ("poll", poll),
        ("overflow", overflow),
        ("t", t),
        ("r", r),
        ("bump", bump),
        ("which", which),
        ("keep", keep),
    ];
    let mut s = Asm::script(DEFS.len());
    for (i, (name, _)) in fns.iter().enumerate() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns.into_iter().map(|(_, a)| a.finish()));
    unit(DEFS, all)
}

/// The stubs of the natively run functions, in `NATIVE` order. `offs` gives the bytecode
/// offset of the opcode a stub's table call stands for.
const NATIVE: [&str; 10] = [
    "id", "g", "add2", "inc1", "lt", "nthrow", "poll", "overflow", "which", "keep",
];

fn stubs(offs: impl Fn(&str, Opcode) -> u32) -> Vec<Vec<u8>> {
    use Src::{Imm, Param};
    vec![
        stub_param(0),
        stub_call(
            layout::RT_CALL_ANY,
            offs("g", Opcode::CallUndefinedReceiver1),
            &[Imm(UNDEFINED)],
        ),
        stub_call(
            layout::RT_ADD,
            offs("add2", Opcode::Add),
            &[Param(0), Param(1)],
        ),
        stub_call(layout::RT_INC, offs("inc1", Opcode::Inc), &[Param(0)]),
        stub_call(
            layout::RT_COMPARE,
            offs("lt", Opcode::TestLessThan),
            &[Imm(u64::from(cmp::LT)), Param(0), Param(1)],
        ),
        stub_call(layout::RT_THROW, offs("nthrow", Opcode::Throw), &[Param(0)]),
        stub_call(layout::RT_POLL, 0, &[]),
        stub_call(layout::RT_STACK_OVERFLOW, 0, &[]),
        // `which`: returns a1 where its bytecode returns a0.
        stub_param(1),
        stub_call_keep(offs("keep", Opcode::CallUndefinedReceiver1), 1),
    ]
}

/// The bytecode offset of the first `op` in the global function `name` of `iso`.
fn offset_in(iso: &Isolate, name: &str, op: Opcode) -> u32 {
    let f = iso.global(name).unwrap();
    let t = crate::native_target(iso, f).unwrap();
    ep9_bytecode::Instructions::new(t.bytecode)
        .map(|i| i.unwrap())
        .find(|i| i.opcode == op)
        .unwrap_or_else(|| panic!("no {op:?} in {name}"))
        .offset
}

/// An isolate (with `print`) that ran `defs()`, with the native stubs installed.
fn native_isolate(config: IsolateConfig, code: &[Range<usize>]) -> Isolate {
    let mut iso = isolate(config);
    iso.run_script(defs(), "defs.js").unwrap();
    for (name, range) in NATIVE.iter().zip(code) {
        let f = iso.global(name).unwrap();
        // SAFETY: `range` holds the stub for `name` (`stubs`), which runs its bytecode and
        // stays mapped until `with_stubs` returns, after the isolate drops.
        unsafe { crate::install_native(&iso, f, DispatchKind::Baseline, range.clone()) }.unwrap();
    }
    iso
}

/// Builds the stubs for the offsets of a fresh `defs()` isolate, installs them in a new
/// isolate per config, and runs `f` on it (on a big native stack).
fn run_native<R: Send + 'static>(
    config: IsolateConfig,
    f: impl FnOnce(&mut Isolate) -> R + Send + 'static,
) -> R {
    on_big_stack(move || {
        let probe = {
            let mut iso = isolate(IsolateConfig::default());
            iso.run_script(defs(), "defs.js").unwrap();
            stubs(|n, op| offset_in(&iso, n, op))
        };
        with_stubs(&probe, |code| {
            let mut iso = native_isolate(config, code);
            let r = f(&mut iso);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
            r
        })
    })
}

/// A script that calls `print(<name>(args..))` for each row (`args`: `Lit` values).
#[derive(Clone)]
enum Lit {
    Int(i32),
    Str(&'static str),
    Global(&'static str),
}

fn calls_unit(rows: &[(&str, Vec<Lit>)]) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let f = s.b.new_register();
    let args = s.b.new_register_list(2);
    for (name, lits) in rows {
        for (k, l) in lits.iter().enumerate() {
            match l {
                Lit::Int(i) => {
                    s.b.lda_smi(*i);
                }
                Lit::Str(t) => {
                    s.lda_str(t);
                }
                Lit::Global(g) => {
                    s.lda_global(g);
                }
            }
            s.b.star(args.get(k as u32));
        }
        s.lda_global(name);
        s.b.star(f);
        match lits.len() {
            0 => s.b.call_undefined_receiver0(f, s.call),
            1 => s.b.call_undefined_receiver1(f, args.get(0), s.call),
            _ => {
                s.b.call_undefined_receiver2(f, args.get(0), args.get(1), s.call)
            }
        };
        s.print_acc();
    }
    s.lda_str("done");
    s.b.return_();
    unit("", vec![s.finish()])
}

fn run_rows(
    iso: &mut Isolate,
    rows: &[(&str, Vec<Lit>)],
) -> (Result<String, Exception>, Vec<String>) {
    take_output();
    let r = iso
        .run_script(calls_unit(rows), "calls.js")
        .map(|v| iso.display(v));
    (r, take_output())
}

// ---- tests ---------------------------------------------------------------------------------

#[test]
fn t0_calls_native_code_and_native_code_calls_back() {
    use Lit::{Global, Int, Str};
    let rows = vec![
        ("id", vec![Int(41)]),
        // native g -> T0 bump (allocates) through call_any
        ("g", vec![Global("bump"), Str("a")]),
        // native g -> native inc1 through call_any (JIT -> JIT)
        ("g", vec![Global("inc1"), Int(41)]),
        // native g -> host print through call_any
        ("g", vec![Global("print"), Str("hi")]),
        ("add2", vec![Int(1), Int(2)]),
        ("add2", vec![Str("a"), Str("b")]),
        ("inc1", vec![Str("41")]),
        ("lt", vec![Int(1), Int(2)]),
        ("lt", vec![Str("b"), Str("a")]),
        ("poll", vec![]),
        // The stub ran, not the bytecode: a1, not a0.
        ("which", vec![Int(1), Int(2)]),
        // native keep -> T0 bump (allocates), then keep reads its own a1 again
        ("keep", vec![Global("bump"), Str("k")]),
        // native keep -> native g -> T0 bump: two native frames under the T0 one
        ("keep", vec![Global("g"), Global("bump")]),
    ];
    let want = [
        "41",
        "a!",
        "42",
        "hi",
        "undefined",
        "3",
        "ab",
        "42",
        "true",
        "false",
        "true",
        "2",
        "k",
        "[object Function]",
    ];
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let rows = rows.clone();
            let (r, out) = run_native(config(d, stress), move |iso| run_rows(iso, &rows));
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(r.as_deref(), Ok("done"), "{label}");
            assert_eq!(out, want, "{label}");
        }
    }
}

#[test]
fn native_slots_and_the_code_registry() {
    run_native(IsolateConfig::default(), |iso| {
        let id = iso.global("id").unwrap();
        let t = crate::native_target(iso, id).unwrap();
        assert_eq!(t.kind, Some(DispatchKind::Baseline));
        assert_eq!((t.name, t.parameter_count), ("id", 1));
        // SAFETY: a FUNCTION value's info word (native_target checked the kind).
        let fi = crate::object::function_info(id.to_bits());
        let entry = fi.slot.entry();
        let e = crate::lookup_code(iso, entry).unwrap();
        assert_eq!(e.kind, DispatchKind::Baseline);
        assert_eq!(e.function, t.info);
        // The bytecode range is still registered (T0 frames, H14).
        assert_eq!(
            crate::lookup_code(iso, fi.code.addr()).map(|e| e.kind),
            Some(DispatchKind::Interp)
        );
        // Install errors.
        // SAFETY: every call below is refused before the slot changes.
        unsafe {
            let r = entry..entry + 4;
            assert_eq!(
                crate::install_native(iso, id, DispatchKind::Baseline, r.clone()),
                Err(InstallError::AlreadyNative)
            );
            assert_eq!(
                crate::install_native(iso, id, DispatchKind::Opt, r.clone()),
                Err(InstallError::Kind(DispatchKind::Opt))
            );
            let print = iso.global("print").unwrap();
            assert_eq!(
                crate::install_native(iso, print, DispatchKind::Baseline, r.clone()),
                Err(InstallError::NotBytecode)
            );
            assert_eq!(
                crate::install_native(iso, Value::int32(1), DispatchKind::Baseline, r),
                Err(InstallError::NotBytecode)
            );
        }
        // Back to T0: the same call now runs the bytecode (`which` gives a0 again).
        let which = iso.global("which").unwrap();
        let (r, out) = run_rows(iso, &[("which", vec![Lit::Int(1), Lit::Int(2)])]);
        assert_eq!(
            (r.as_deref(), out.as_slice()),
            (Ok("done"), &["2".to_string()][..])
        );
        crate::uninstall_native(iso, which).unwrap();
        let (r, out) = run_rows(iso, &[("which", vec![Lit::Int(1), Lit::Int(2)])]);
        assert_eq!(
            (r.as_deref(), out.as_slice()),
            (Ok("done"), &["1".to_string()][..])
        );
        let removed = crate::uninstall_native(iso, id).unwrap();
        assert_eq!(removed.start, entry);
        assert_eq!(
            crate::uninstall_native(iso, id),
            Err(InstallError::NotNative)
        );
        assert!(crate::lookup_code(iso, entry).is_none());
        assert_eq!(fi.slot.kind(), Some(DispatchKind::Interp));
        let (r, out) = run_rows(iso, &[("id", vec![Lit::Int(7)])]);
        assert_eq!(
            (r.as_deref(), out.as_slice()),
            (Ok("done"), &["7".to_string()][..])
        );
        // An overlapping range is refused by the registry.
        let g = iso.global("g").unwrap();
        let g_entry = crate::object::function_info(g.to_bits()).slot.entry();
        // SAFETY: refused before the slot changes.
        let r = unsafe {
            crate::install_native(iso, id, DispatchKind::AotBaseline, g_entry..g_entry + 8)
        };
        assert_eq!(
            r,
            Err(InstallError::Registry(RegistryError::Overlap(g_entry)))
        );
    });
}

#[test]
fn exceptions_cross_native_frames() {
    fn make() -> VerifiedUnit {
        // try { g(t, "boom") } catch (e) { print(e.message) }   T0 throw under native g
        // try { nthrow("x") } catch (e) { print(e) }             native throw
        // try { overflow() } catch (e) { print(e.message) }      native stack RangeError
        // try { new id(1) } catch (e) { print(e.message) }       construct stays in T0
        let mut s = Asm::script(0);
        let ctx = s.b.new_register();
        let e = s.b.new_register();
        let args = RegisterList::new(s.a.get(0).local_index().unwrap(), 2);
        let guarded = |s: &mut Asm, body: &dyn Fn(&mut Asm), message: bool| {
            let (start, end, handler, done) = (
                s.b.new_label(),
                s.b.new_label(),
                s.b.new_label(),
                s.b.new_label(),
            );
            s.b.lda_current_context().star(ctx);
            s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
            s.b.bind(start);
            body(s);
            s.b.bind(end);
            s.b.jump(done);
            s.b.bind(handler);
            if message {
                s.b.star(e);
                s.get_named(e, "message");
            }
            s.print_acc();
            s.b.bind(done);
        };
        let call2 = |s: &mut Asm, f: &str, a: &str, b: &str| {
            s.lda_global(a);
            s.b.star(args.get(0));
            s.lda_str(b);
            s.b.star(args.get(1));
            s.lda_global(f);
            s.b.star(s.f);
            s.b.call_undefined_receiver2(s.f, args.get(0), args.get(1), s.call);
        };
        guarded(&mut s, &|s| call2(s, "g", "t", "boom"), true);
        guarded(
            &mut s,
            &|s| {
                s.lda_str("x");
                s.b.star(args.get(0));
                s.lda_global("nthrow");
                s.b.star(s.f);
                s.b.call_undefined_receiver1(s.f, args.get(0), s.call);
            },
            false,
        );
        guarded(
            &mut s,
            &|s| {
                s.lda_global("overflow");
                s.b.star(s.f);
                s.b.call_undefined_receiver0(s.f, s.call);
            },
            true,
        );
        guarded(
            &mut s,
            &|s| {
                s.b.lda_smi(1).star(args.get(0));
                let one = RegisterList::new(args.get(0).local_index().unwrap(), 1);
                s.construct_global("id", one);
            },
            true,
        );
        // Uncaught: g(t, "late") leaves the script.
        call2(&mut s, "g", "t", "late");
        s.lda_str("not reached");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, out) = run_native(config(d, stress), |iso| {
                take_output();
                let r = iso.run_script(make(), "exc.js").map(|v| iso.display(v));
                (r, take_output())
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(
                out,
                [
                    "boom",
                    "x",
                    "Maximum call stack size exceeded",
                    "not implemented in ep9 v0: calling code of this dispatch kind",
                ],
                "{label}"
            );
            let e = r.unwrap_err();
            assert_eq!(e.message, "Error: late", "{label}");
            // The native frame of g is in the stack at its call of f, between t and the
            // script (DEFS line 1 `return f(x)`: column 27).
            let col = |needle: &str| {
                let p = pos(DEFS, needle) as usize;
                let line_start = DEFS[..p].rfind('\n').map_or(0, |i| i + 1);
                let line = DEFS[..p].matches('\n').count() + 1;
                format!("defs.js:{line}:{}", p - line_start + 1)
            };
            let stack = e.stack.unwrap();
            assert_eq!(
                stack,
                format!(
                    "Error: late\n    at t ({})\n    at g ({})\n    at exc.js:1:1",
                    col("new Error"),
                    col("f(x)")
                ),
                "{label}"
            );
        }
    }
}

#[test]
fn deep_t0_native_recursion_is_a_catchable_range_error() {
    // r(x) = g(r, x): T0 r -> native g -> call_any -> T0 r -> ... until a native or VM stack
    // limit throws; the script catches it.
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let ctx = s.b.new_register();
        let e = s.b.new_register();
        let (start, end, handler, done) = (
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
            s.b.new_label(),
        );
        s.b.lda_current_context().star(ctx);
        s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
        s.b.bind(start);
        s.b.lda_smi(1).star(s.a.get(0));
        s.lda_global("r");
        s.b.star(s.f);
        s.b.call_undefined_receiver1(s.f, s.a.get(0), s.call);
        s.b.bind(end);
        s.b.jump(done);
        s.b.bind(handler);
        s.b.star(e);
        s.get_named(e, "message");
        s.print_acc();
        s.b.bind(done);
        s.lda_str("after");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        let (r, out) = std::thread::Builder::new()
            .stack_size(if cfg!(debug_assertions) {
                64 << 20
            } else {
                4 << 20
            })
            .spawn(move || {
                let probe = {
                    let mut iso = isolate(IsolateConfig::default());
                    iso.run_script(defs(), "defs.js").unwrap();
                    stubs(|n, op| offset_in(&iso, n, op))
                };
                with_stubs(&probe, |code| {
                    let mut iso = native_isolate(config(d, false), code);
                    take_output();
                    let r = iso.run_script(make(), "rec.js").map(|v| iso.display(v));
                    (r, take_output())
                })
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(r.as_deref(), Ok("after"), "{}", d.name());
        assert_eq!(out, ["Maximum call stack size exceeded"], "{}", d.name());
    }
}

#[test]
fn termination_stops_at_a_native_poll() {
    let e = run_native(IsolateConfig::default(), |iso| {
        iso.terminate_handle().terminate();
        let (r, _) = run_rows(iso, &[("poll", vec![])]);
        r.unwrap_err()
    });
    assert!(e.terminated, "{e:?}");
}

fn boom<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    panic!("boom from a host function under native code");
}

/// The environment variable that makes the panic test run its child part.
const PANIC_CHILD: &str = "EP9_NATIVE_PANIC_CHILD";

#[test]
fn a_panic_under_native_code_aborts() {
    if std::env::var_os(PANIC_CHILD).is_some() {
        // Child: native g calls the host function `boom` through call_any; its panic reaches
        // the extern "C" / "sysv64" boundary of call_any and aborts there.
        run_native(IsolateConfig::default(), |iso| {
            iso.register_host_function("boom", 1, boom);
            let _ = run_rows(iso, &[("g", vec![Lit::Global("boom"), Lit::Int(1)])]);
        });
        unreachable!("the panic did not abort");
    }
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "tests::native::a_panic_under_native_code_aborts",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PANIC_CHILD, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("boom from a host function under native code"),
        "{stderr}"
    );
    assert!(
        stderr.contains("panic in a function that cannot unwind"),
        "{stderr}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            out.status.signal(),
            Some(6),
            "SIGABRT expected: {:?}",
            out.status
        );
    }
    #[cfg(windows)]
    assert_eq!(
        out.status.code(),
        Some(0xC000_0409_u32 as i32),
        "{:?}",
        out.status
    );
}

#[test]
fn stub_bytes_are_the_documented_instructions() {
    // The encodings above, as `llvm-mc --disassemble` prints them (aarch64 / x86-64).
    let call = stub_call(0x40, 5, &[Src::Param(1), Src::Imm(0x1_0002)]);
    #[cfg(target_arch = "aarch64")]
    assert_eq!(
        call.chunks(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect::<Vec<_>>()[3..],
        [
            0xF940_2210, // ldr x16, [x16, #0x40]
            0x5280_00A2, // mov w2, #5
            0xF85A_8023, // ldur x3, [x1, #-88]
            0xD280_0044, // mov x4, #2
            0xF2A0_0024, // movk x4, #1, lsl #16
            0xD63F_0200, // blr x16
            0xA8C1_7BFD, // ldp x29, x30, [sp], #16
            0xD65F_03C0, // ret
        ]
    );
    #[cfg(target_arch = "x86_64")]
    assert_eq!(
        call[11..],
        [
            0x48, 0x8B, 0x80, 0x40, 0, 0, 0, // mov rax, [rax + 0x40]
            0xBA, 5, 0, 0, 0, // mov edx, 5
            0x48, 0x8B, 0x8E, 0xA8, 0xFF, 0xFF, 0xFF, // mov rcx, [rsi - 88]
            0x49, 0xB8, 2, 0, 1, 0, 0, 0, 0, 0, // movabs r8, 0x10002
            0xFF, 0xD0, 0x5D, 0xC3, // call rax; pop rbp; ret
        ]
    );
}

// ---- generator functions get no native code (M2-P1-R1) --------------------------------------

/// `function* gen() { yield 1; yield 2; }` in the V8 shape of P1-GEN-CG (the prologue, the
/// initial suspend, one resume point per `yield`; tests/generator.rs `Gen`), and a script that
/// defines it as a global and makes three generators `ga`, `gb`, `gc`.
fn generator_unit() -> VerifiedUnit {
    use ep9_bytecode::{FunctionFlags, RuntimeId};
    let mut m = meta("gen", 0, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::GENERATOR;
    let mut s = Asm::new(m);
    let g = s.b.new_register();
    let v = s.b.new_register();
    let t = s.b.new_register_list(2);
    let save = RegisterList::new(0, s.b.register_count());
    let (table, resumes) = s.b.jump_table(3);
    let this = s.b.receiver();
    s.b.lda_new_target().star(g);
    s.b.switch_on_generator_state(g, table);
    s.b.lda_current_function().star(t.get(0));
    s.b.ldar(this).star(t.get(1));
    s.b.call_runtime(RuntimeId::CreateGeneratorObject, t, 2);
    s.b.star(g);
    for k in 0..3u32 {
        if k > 0 {
            // yield k: {value: k, done: false}
            s.b.lda_smi(k as i32)
                .star(t.get(0))
                .lda_false()
                .star(t.get(1));
            s.b.call_runtime(RuntimeId::CreateIterResultObject, t, 2);
        }
        s.b.suspend_generator(g, save, save.len(), k);
        s.b.bind(resumes[k as usize]);
        s.b.resume_generator(g, save, save.len());
        s.b.star(v);
        let gl = RegisterList::new(g.local_index().unwrap(), 1);
        s.b.call_runtime(RuntimeId::GeneratorGetResumeMode, gl, 1);
        let (modes, l) = s.b.jump_table(2);
        let cont = s.b.new_label();
        s.b.switch_on_smi(modes, 1);
        s.b.jump(cont);
        s.b.bind(l[0]);
        s.b.ldar(v).return_();
        s.b.bind(l[1]);
        s.b.ldar(v).throw();
        s.b.bind(cont);
    }
    s.b.lda_undefined().return_();
    let mut sc = Asm::script(0);
    let k = sc.b.function_constant(FunctionId(1));
    sc.b.create_closure(k, sc.closure, 0);
    sc.sta_global("gen");
    for name in ["ga", "gb", "gc"] {
        sc.lda_global("gen");
        sc.b.star(sc.f);
        sc.b.call_undefined_receiver0(sc.f, sc.call);
        sc.sta_global(name);
    }
    sc.b.lda_undefined().return_();
    unit("", vec![sc.finish(), s.finish()])
}

/// `it[method](arg)` from Rust, as `"value/done"`, or the thrown value as `"throw value"`.
fn resume(iso: &mut Isolate, it: &str, method: &str, arg: Value) -> String {
    let it = iso.global(it).unwrap();
    let m = {
        let cx = &mut *iso.cx;
        let k = crate::atoms::intern_str(cx.rt(), method);
        crate::runtime::val(crate::property::get(cx, it.to_bits(), k).unwrap())
    };
    match iso.call(m, it, &[arg]) {
        Ok(r) => {
            let cx = &mut *iso.cx;
            let mut field = |name: &str| {
                let k = crate::atoms::intern_str(cx.rt(), name);
                crate::conv::describe(crate::property::get(cx, r.to_bits(), k).unwrap())
            };
            format!("{}/{}", field("value"), field("done"))
        }
        Err(e) => format!("throw {}", e.message),
    }
}

#[test]
fn a_generator_function_gets_no_native_code_and_its_generators_still_run() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let mut iso = isolate(config(d, stress));
            iso.run_script(generator_unit(), "gen.js").unwrap();
            let genf = iso.global("gen").unwrap();
            let fi = crate::object::function_info(genf.to_bits());
            assert!(fi.is_generator(), "{label}");
            // `ga` runs its first step before the install attempt.
            assert_eq!(resume(&mut iso, "ga", "next", Value::UNDEFINED), "1/false");
            // A code address that holds no code: the install is refused before the slot or the
            // registry changes, so nothing ever runs it.
            let code = 0x1000..0x1010;
            for kind in [DispatchKind::Baseline, DispatchKind::AotBaseline] {
                // SAFETY: refused before the slot changes (the range is never entered).
                let r = unsafe { crate::install_native(&iso, genf, kind, code.clone()) };
                assert_eq!(r, Err(InstallError::Generator), "{label}");
            }
            assert_eq!(fi.slot.kind(), Some(DispatchKind::Interp), "{label}");
            assert!(crate::lookup_code(&iso, code.start).is_none(), "{label}");
            assert!(crate::native::native_callee(genf.to_bits()).is_none());
            assert_eq!(
                crate::uninstall_native(&iso, genf),
                Err(InstallError::NotNative)
            );
            // Generators made before the attempt resume in T0: next, return, throw.
            let rows = [
                ("ga", "next", Value::UNDEFINED, "2/false"),
                ("ga", "next", Value::UNDEFINED, "undefined/true"),
                ("gb", "next", Value::UNDEFINED, "1/false"),
                ("gb", "return", Value::int32(5), "5/true"),
                ("gb", "next", Value::UNDEFINED, "undefined/true"),
                ("gc", "next", Value::UNDEFINED, "1/false"),
                ("gc", "throw", Value::int32(6), "throw 6"),
                ("gc", "next", Value::UNDEFINED, "undefined/true"),
            ];
            for (it, method, arg, want) in rows {
                assert_eq!(
                    resume(&mut iso, it, method, arg),
                    want,
                    "{label} {it}.{method}"
                );
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        }
    }
}

// ---- Rust -> native (`call_from_rust`, M2-P2-SPIKEB-RUSTENTRY) ------------------------------

/// What `call_from_rust` must put back (D20): the published top frame, the scratch top and
/// `out_fp`/`out_pc`.
fn entry_state(iso: &Isolate) -> [usize; 4] {
    let st = iso.rt.stack();
    [
        st.fp().addr(),
        st.scratch_top().addr(),
        iso.cx.out_fp.addr(),
        iso.cx.out_pc.addr(),
    ]
}

/// A new string (a Rust local: the conservative scan keeps it).
fn string(iso: &mut Isolate, s: &str) -> Value {
    crate::runtime::val(crate::string::new_string(&iso.rt, s))
}

#[test]
fn call_from_rust_enters_native_code_and_restores_the_entry_state() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let out = run_native(config(d, stress), move |iso| {
                let g = |iso: &Isolate, n: &str| iso.global(n).unwrap();
                // `call_from_rust` puts back all four words; `Isolate::call` (T0) keeps the
                // stack depth (fp, scratch top) but leaves `out_fp`/`out_pc` as they are.
                let depth = entry_state(iso)[..2].to_vec();
                let mut rows = Vec::new();
                let mut show =
                    |iso: &mut Isolate, r: Result<Value, Exception>, before: [usize; 4]| {
                        let s = match r {
                            Ok(v) => iso.display(v),
                            Err(e) => format!("Err({}, terminated={})", e.message, e.terminated),
                        };
                        assert_eq!(entry_state(iso)[..2], depth[..], "stack depth after {s}");
                        if before[0] != usize::MAX {
                            assert_eq!(entry_state(iso), before, "entry state after {s}");
                        }
                        rows.push(s);
                    };
                let plain = [usize::MAX; 4];
                let u = Value::UNDEFINED;
                // native id; native g -> native inc1 (JIT -> JIT); native g -> T0 bump (allocates).
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, g(iso, "id"), u, &[Value::int32(41)]);
                show(iso, r, st);
                let (gf, inc1) = (g(iso, "g"), g(iso, "inc1"));
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, gf, u, &[inc1, Value::int32(41)]);
                show(iso, r, st);
                let a = string(iso, "a");
                let bump = g(iso, "bump");
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, gf, u, &[bump, a]);
                show(iso, r, st);
                // An exception from native code comes back as Err; the next calls work.
                let x = string(iso, "x");
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, g(iso, "nthrow"), u, &[x]);
                show(iso, r, st);
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, g(iso, "id"), u, &[Value::int32(7)]);
                show(iso, r, st);
                let b = string(iso, "b");
                let r = iso.call(bump, u, &[b]);
                show(iso, r, plain);
                // A terminated call (the native poll sees the request), then both entries work.
                iso.terminate_handle().terminate();
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, g(iso, "poll"), u, &[]);
                show(iso, r, st);
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, gf, u, &[inc1, Value::int32(1)]);
                show(iso, r, st);
                let c = string(iso, "c");
                let r = iso.call(bump, u, &[c]);
                show(iso, r, plain);
                // The native stack RangeError of a native prologue (`overflow`).
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, g(iso, "overflow"), u, &[]);
                show(iso, r, st);
                // A function without native code takes the T0 path.
                let z = string(iso, "z");
                let st = entry_state(iso);
                let r = crate::call_from_rust(iso, bump, u, &[z]);
                show(iso, r, st);
                // A Rust caller's scratch Values stay below the native frame and the scratch
                // top comes back.
                let base = iso.rt.stack().base();
                let keep = string(iso, "kept");
                // SAFETY: the stack is empty (no frame is live), so `base` is free; it gets a
                // valid Value before the scratch top moves past it.
                unsafe { *base = keep.to_bits() };
                iso.rt.stack().set_scratch_top(base.wrapping_add(1));
                let before_scratch = entry_state(iso);
                let r = crate::call_from_rust(iso, gf, u, &[bump, a]);
                assert_eq!(entry_state(iso), before_scratch);
                // SAFETY: the scratch slot written above.
                let kept = unsafe { *base };
                assert_eq!(iso.display(crate::runtime::val(kept)), "kept");
                iso.rt.stack().clear_scratch();
                let s = iso.display(r.unwrap());
                rows.push(s);
                rows
            });
            assert_eq!(
                out,
                [
                    "41",
                    "42",
                    "a!",
                    "Err(x, terminated=false)",
                    "7",
                    "b!",
                    "Err(execution terminated, terminated=true)",
                    "2",
                    "c!",
                    "Err(RangeError: Maximum call stack size exceeded, terminated=false)",
                    "z!",
                    "a!",
                ],
                "{label}"
            );
        }
    }
}

#[test]
fn call_from_rust_deep_recursion_is_a_range_error_and_the_next_call_works() {
    // native g -> call_any -> T0 r -> native g ... from Rust, until a stack limit throws.
    for &d in Dispatch::ALL {
        let out = std::thread::Builder::new()
            .stack_size(if cfg!(debug_assertions) {
                64 << 20
            } else {
                4 << 20
            })
            .spawn(move || {
                let probe = {
                    let mut iso = isolate(IsolateConfig::default());
                    iso.run_script(defs(), "defs.js").unwrap();
                    stubs(|n, op| offset_in(&iso, n, op))
                };
                with_stubs(&probe, |code| {
                    let mut iso = native_isolate(config(d, false), code);
                    let before = entry_state(&iso);
                    let (g, r) = (iso.global("g").unwrap(), iso.global("r").unwrap());
                    let e =
                        crate::call_from_rust(&mut iso, g, Value::UNDEFINED, &[r, Value::int32(1)])
                            .unwrap_err();
                    assert_eq!(entry_state(&iso), before);
                    let id = iso.global("id").unwrap();
                    let again =
                        crate::call_from_rust(&mut iso, id, Value::UNDEFINED, &[Value::int32(3)])
                            .unwrap();
                    (e.message, iso.display(again))
                })
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(
            out,
            (
                "RangeError: Maximum call stack size exceeded".to_string(),
                "3".to_string()
            ),
            "{}",
            d.name()
        );
    }
}

// ---- the decode-free T0 entry (M2-P3-D21-ENTRY) -----------------------------------------------

/// The extra natively run functions of the entry tests, in stub order. Their bytecode is
/// `LdaUndefined; Return`; only the stubs run. `recv` and `srecv` (strict) return their
/// receiver, `argc` its argument count, `p0`-`p2` (3 parameters) their parameter a0-a2.
const ENTRY: [&str; 6] = ["recv", "srecv", "argc", "p0", "p1", "p2"];

fn entry_defs() -> VerifiedUnit {
    let mut fns = Vec::new();
    for name in ENTRY {
        let params = if name.starts_with('p') { 3 } else { 0 };
        let mut m = meta(name, params, 0, 0, 0);
        if name == "srecv" {
            m.flags = ep9_bytecode::FunctionFlags::SIMPLE_PARAMETERS
                | ep9_bytecode::FunctionFlags::STRICT;
        }
        let mut f = Asm::new(m);
        f.b.lda_undefined().return_();
        fns.push(f);
    }
    let mut s = Asm::script(0);
    for (i, name) in ENTRY.iter().enumerate() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns.into_iter().map(Asm::finish));
    unit("", all)
}

fn entry_stubs() -> Vec<Vec<u8>> {
    vec![
        stub_load(layout::FP_RECEIVER),
        stub_load(layout::FP_RECEIVER),
        stub_load(layout::FP_ARGC),
        stub_param(0),
        stub_param(1),
        stub_param(2),
    ]
}

/// The decode-free entry count of this thread (`native::T0_ENTRY_HITS`).
fn entry_hits() -> u64 {
    crate::native::T0_ENTRY_HITS.with(core::cell::Cell::get)
}

/// As [`run_native`], with the `ENTRY` functions defined and natively run too.
fn run_entry<R: Send + 'static>(
    config: IsolateConfig,
    f: impl FnOnce(&mut Isolate) -> R + Send + 'static,
) -> R {
    on_big_stack(move || {
        let mut all = {
            let mut iso = isolate(IsolateConfig::default());
            iso.run_script(defs(), "defs.js").unwrap();
            stubs(|n, op| offset_in(&iso, n, op))
        };
        all.extend(entry_stubs());
        with_stubs(&all, |code| {
            let mut iso = native_isolate(config, code);
            iso.run_script(entry_defs(), "entry-defs.js").unwrap();
            for (name, range) in ENTRY.iter().zip(&code[NATIVE.len()..]) {
                let f = iso.global(name).unwrap();
                // SAFETY: `range` holds the stub for `name` (`entry_stubs`), which stays
                // mapped until `with_stubs` returns, after the isolate drops.
                unsafe { crate::install_native(&iso, f, DispatchKind::Baseline, range.clone()) }
                    .unwrap();
            }
            let r = f(&mut iso);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
            r
        })
    })
}

/// A call instruction of the entry tests. Property shapes take the receiver as their first
/// value.
#[derive(Clone, Copy, Debug)]
enum Shape {
    CallProperty0,
    CallProperty1,
    CallProperty2,
    CallProperty,
    CallAnyReceiver,
    CallUndefinedReceiver0,
    CallUndefinedReceiver1,
    CallUndefinedReceiver2,
    CallUndefinedReceiver,
}

/// Emits `acc = callee(..)` in the shape `shape`: the values go to the list `l` (first the
/// receiver for a property shape), the callee to `f`.
fn emit_call(s: &mut Asm, shape: Shape, callee: &str, vals: &[Lit], f: Register, l: RegisterList) {
    for (k, v) in vals.iter().enumerate() {
        match v {
            Lit::Int(i) => {
                s.b.lda_smi(*i);
            }
            Lit::Str(t) => {
                s.lda_str(t);
            }
            Lit::Global(g) => {
                s.lda_global(g);
            }
        }
        s.b.star(l.get(k as u32));
    }
    s.lda_global(callee);
    s.b.star(f);
    let n = vals.len() as u32;
    let list = RegisterList::new(l.get(0).local_index().unwrap(), n);
    let slot = s.call;
    match shape {
        Shape::CallProperty0 => s.b.call_property0(f, l.get(0), slot),
        Shape::CallProperty1 => s.b.call_property1(f, l.get(0), l.get(1), slot),
        Shape::CallProperty2 => s.b.call_property2(f, l.get(0), l.get(1), l.get(2), slot),
        Shape::CallProperty => s.b.call_property(f, list, n, slot),
        Shape::CallAnyReceiver => s.b.call_any_receiver(f, list, n, slot),
        Shape::CallUndefinedReceiver0 => s.b.call_undefined_receiver0(f, slot),
        Shape::CallUndefinedReceiver1 => s.b.call_undefined_receiver1(f, l.get(0), slot),
        Shape::CallUndefinedReceiver2 => s.b.call_undefined_receiver2(f, l.get(0), l.get(1), slot),
        Shape::CallUndefinedReceiver => s.b.call_undefined_receiver(f, list, n, slot),
    };
}

/// `try { <body> } catch (e) { print(e.message if `message`, else e) }`; the body's
/// accumulator is printed when it does not throw.
fn guarded(s: &mut Asm, message: bool, body: impl FnOnce(&mut Asm)) {
    let ctx = s.b.new_register();
    let e = s.b.new_register();
    let (start, end, handler, done) = (
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
        s.b.new_label(),
    );
    s.b.lda_current_context().star(ctx);
    s.b.add_handler(start, end, handler, ctx, HandlerKind::Caught);
    s.b.bind(start);
    body(s);
    s.b.bind(end);
    s.print_acc();
    s.b.jump(done);
    s.b.bind(handler);
    if message {
        s.b.star(e);
        s.get_named(e, "message");
    }
    s.print_acc();
    s.b.bind(done);
}

#[test]
fn the_decode_free_entry_takes_every_unprefixed_call_shape() {
    use Lit::{Global, Int, Str};
    use Shape::*;
    // (shape, callee, values (receiver first for a property shape), printed result)
    let rows: Vec<(Shape, &str, Vec<Lit>, &str)> = vec![
        (CallProperty0, "recv", vec![Str("R")], "R"),
        (CallProperty0, "argc", vec![Str("R")], "0"),
        (CallProperty1, "p0", vec![Str("R"), Str("x")], "x"),
        (CallProperty1, "argc", vec![Str("R"), Int(7)], "1"),
        (CallProperty1, "p1", vec![Str("R"), Str("x")], "undefined"),
        (CallProperty1, "recv", vec![Str("R"), Str("x")], "R"),
        (CallProperty2, "p1", vec![Str("R"), Str("x"), Str("y")], "y"),
        (CallProperty2, "recv", vec![Str("R"), Int(1), Int(2)], "R"),
        (
            CallProperty,
            "p2",
            vec![Str("R"), Str("x"), Str("y"), Str("z")],
            "z",
        ),
        (
            CallProperty,
            "argc",
            vec![Str("R"), Int(1), Int(2), Int(3)],
            "3",
        ),
        (CallProperty, "recv", vec![Str("R")], "R"),
        (CallAnyReceiver, "recv", vec![Str("R"), Int(1)], "R"),
        (CallAnyReceiver, "p0", vec![Str("R"), Str("a")], "a"),
        // Count 0: no receiver, so undefined; the strict callee keeps it.
        (CallAnyReceiver, "srecv", vec![], "undefined"),
        (CallUndefinedReceiver0, "argc", vec![], "0"),
        (CallUndefinedReceiver0, "srecv", vec![], "undefined"),
        // A sloppy callee gets the global object for an undefined receiver.
        (CallUndefinedReceiver0, "recv", vec![], "[object Object]"),
        (CallUndefinedReceiver1, "p0", vec![Str("x")], "x"),
        (CallUndefinedReceiver1, "p2", vec![Str("x")], "undefined"),
        (CallUndefinedReceiver2, "p1", vec![Str("x"), Str("y")], "y"),
        (CallUndefinedReceiver2, "argc", vec![Int(1), Int(2)], "2"),
        (
            CallUndefinedReceiver,
            "p2",
            vec![Str("x"), Str("y"), Str("z")],
            "z",
        ),
        (
            CallUndefinedReceiver,
            "argc",
            vec![Int(1), Int(2), Int(3), Int(4)],
            "4",
        ),
        (CallUndefinedReceiver, "argc", vec![], "0"),
        // Native code that calls back: g -> call_any -> T0 bump (allocates).
        (
            CallProperty2,
            "g",
            vec![Str("R"), Global("bump"), Str("a")],
            "a!",
        ),
        (
            CallAnyReceiver,
            "g",
            vec![Str("R"), Global("bump"), Str("b")],
            "b!",
        ),
        (
            CallUndefinedReceiver,
            "g",
            vec![Global("bump"), Str("c")],
            "c!",
        ),
    ];
    let make = {
        let rows = rows.clone();
        move || {
            let mut s = Asm::script(0);
            let f = s.b.new_register();
            let l = s.b.new_register_list(4);
            for (shape, callee, vals, _) in &rows {
                emit_call(&mut s, *shape, callee, vals, f, l);
                s.print_acc();
            }
            s.lda_str("done");
            s.b.return_();
            unit("", vec![s.finish()])
        }
    };
    let want: Vec<String> = rows.iter().map(|r| r.3.to_string()).collect();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let make = make.clone();
            let (r, out, hits) = run_entry(config(d, stress), move |iso| {
                take_output();
                let before = entry_hits();
                let r = iso.run_script(make(), "entry.js").map(|v| iso.display(v));
                (r, take_output(), entry_hits() - before)
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(r.as_deref(), Ok("done"), "{label}");
            assert_eq!(out, want, "{label}");
            // One entry per row; `print` (host) and `bump` (T0, called from native code
            // through call_any) do not take it.
            assert_eq!(hits, rows.len() as u64, "{label}");
        }
    }
}

#[test]
fn every_other_call_falls_through_to_the_decode_path() {
    /// `this` is the target, a0 the bound `this`, the rest the bound arguments.
    fn bind<'s>(
        cx: &mut Cx<'s>,
        this: Local<'s>,
        args: Args<'s>,
        _nt: Local<'s>,
    ) -> JsResult<Local<'s>> {
        let target = this.as_function().unwrap();
        cx.new_bound_function(target, args.get(0), args.rest(1))
            .map(|f| f.value())
    }
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let f = s.b.new_register();
        let l = s.b.new_register_list(3);
        // Locals past r127 need a Wide prefix.
        let big = s.b.new_register_list(140);
        let (wf, wa) = (big.get(139), big.get(138));
        // 1. A Wide CallUndefinedReceiver1 of native p0: the decode path's call_native.
        guarded(&mut s, true, |s| {
            s.lda_str("w");
            s.b.star(wa);
            s.lda_global("p0");
            s.b.star(wf);
            s.b.call_undefined_receiver1(wf, wa, s.call);
        });
        // 2. CallWithSpread of native p0 (receiver undefined, spread []): no native entry
        //    with a spread.
        guarded(&mut s, true, |s| {
            s.b.lda_undefined().star(l.get(0));
            s.b.create_empty_array_literal(s.lit).star(l.get(1));
            s.lda_global("p0");
            s.b.star(f);
            let two = RegisterList::new(l.get(0).local_index().unwrap(), 2);
            s.b.call_with_spread(f, two, 2, s.call);
        });
        // 3. Construct of native id: construct stays in T0.
        guarded(&mut s, true, |s| {
            s.b.lda_smi(1).star(l.get(0));
            let one = RegisterList::new(l.get(0).local_index().unwrap(), 1);
            s.construct_global("id", one);
        });
        // 4. A bound native function: the callee is BOUND, not a FUNCTION.
        guarded(&mut s, true, |s| {
            s.lda_global("p0");
            s.b.star(l.get(0));
            s.b.lda_undefined().star(l.get(1));
            s.lda_global("bind");
            s.b.star(f);
            s.b.call_property1(f, l.get(0), l.get(1), s.call);
            s.b.star(f);
            s.lda_str("x");
            s.b.star(l.get(2));
            s.b.call_undefined_receiver1(f, l.get(2), s.call);
        });
        // 5. A T0 callee (bump), a host callee (print, in every print), a value that is not
        //    callable.
        guarded(&mut s, true, |s| {
            emit_call(
                s,
                Shape::CallUndefinedReceiver1,
                "bump",
                &[Lit::Str("t0")],
                f,
                l,
            );
        });
        guarded(&mut s, true, |s| {
            s.b.lda_smi(1).star(f);
            s.b.call_undefined_receiver0(f, s.call);
        });
        s.lda_str("done");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    // The Wide call is really prefixed.
    let u = make();
    let wide = ep9_bytecode::Instructions::new(&u.function(FunctionId(0)).bytecode)
        .map(|i| i.unwrap())
        .filter(|i| i.opcode == Opcode::CallUndefinedReceiver1)
        .map(|i| i.scale)
        .collect::<Vec<_>>();
    assert!(wide.contains(&ep9_bytecode::OperandScale::Wide), "{wide:?}");
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, out, hits) = run_entry(config(d, stress), |iso| {
                iso.register_host_function("bind", 1, bind);
                take_output();
                let before = entry_hits();
                let r = iso.run_script(make(), "falls.js").map(|v| iso.display(v));
                (r, take_output(), entry_hits() - before)
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(r.as_deref(), Ok("done"), "{label}");
            let ni = "not implemented in ep9 v0: calling code of this dispatch kind";
            assert_eq!(
                out,
                ["w", ni, ni, ni, "t0!", "1 is not a function"],
                "{label}"
            );
            assert_eq!(hits, 0, "{label}");
        }
    }
}

#[test]
fn exceptions_from_the_entry_unwind_from_the_call() {
    use Lit::{Global, Str};
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let f = s.b.new_register();
        let l = s.b.new_register_list(4);
        // Native nthrow (throw a0) and native g -> T0 t (throw new Error(a0)), caught, in
        // three shapes; then the frame goes on with its registers intact.
        guarded(&mut s, false, |s| {
            emit_call(
                s,
                Shape::CallProperty1,
                "nthrow",
                &[Str("R"), Str("e1")],
                f,
                l,
            );
        });
        guarded(&mut s, false, |s| {
            emit_call(
                s,
                Shape::CallUndefinedReceiver,
                "nthrow",
                &[Str("e2")],
                f,
                l,
            );
        });
        guarded(&mut s, true, |s| {
            let v = [Str("R"), Global("t"), Str("e3")];
            emit_call(s, Shape::CallAnyReceiver, "g", &v, f, l);
        });
        guarded(&mut s, true, |s| {
            emit_call(s, Shape::CallProperty0, "overflow", &[Str("R")], f, l);
        });
        s.b.ldar(l.get(0));
        s.print_acc();
        // Uncaught, from a property call.
        emit_call(
            &mut s,
            Shape::CallProperty2,
            "g",
            &[Str("R"), Global("t"), Str("late")],
            f,
            l,
        );
        s.lda_str("not reached");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, out, hits) = run_entry(config(d, stress), |iso| {
                take_output();
                let before = entry_hits();
                let r = iso.run_script(make(), "exc.js").map(|v| iso.display(v));
                (r, take_output(), entry_hits() - before)
            });
            let label = format!("{} stress={stress}", d.name());
            assert_eq!(
                out,
                ["e1", "e2", "e3", "Maximum call stack size exceeded", "R"],
                "{label}"
            );
            let e = r.unwrap_err();
            assert_eq!(e.message, "Error: late", "{label}");
            // t's frame, then g's native frame at its call of f, then the script.
            let stack = e.stack.unwrap();
            assert!(
                stack.starts_with("Error: late\n    at t (defs.js:2:")
                    && stack.contains("\n    at g (defs.js:1:"),
                "{label}: {stack}"
            );
            assert_eq!(hits, 5, "{label}");
        }
    }
}
