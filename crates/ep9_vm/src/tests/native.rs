//! Tests of the native entry (feature `spike-native` only; owners P1-NATIVE-ENTRY, then
//! P2-SPIKE-B): calls into hand-written native stubs, GC stress across a native frame, and an
//! exception that crosses one. P2-SPIKE-B adds: a generator function gets no native code
//! (`InstallError::Generator`, M2-P1-R1), and `call_from_rust` enters native code from Rust and
//! restores the entry state on every exit (D20). The P3 integration fixer adds the tests of
//! the decode-free T0 entry (`slow::call` `native_entry`, M2-P3-D21-ENTRY): every unprefixed
//! call shape takes it, every other call falls through to the decode path, and exceptions and
//! GC stress behave as on that path. The P3 review fix adds a full VM stack at that entry: the
//! VM stack's RangeError.
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
        // Appended by P4-PLACEMENT-B (placement (b)): the table is append-only.
        layout::RT_UNWIND,
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
    // Placement (b) (P4-PLACEMENT-B): the dispatch table, the unwind result fields, the
    // bytecode start and the native-word encoding.
    let iso = Isolate::new(IsolateConfig::default()).unwrap();
    let cx = core::ptr::from_ref::<crate::interp::Cx>(&iso.cx);
    // SAFETY: each offset names a field of the live Cx with the type read here.
    unsafe {
        let base = cx.cast::<u8>();
        let word = |off: usize| base.add(off).cast::<usize>().read();
        assert_eq!(layout::CX_TABLE, 0);
        assert_eq!(word(layout::CX_TABLE), iso.cx.table[0] as usize);
        assert_eq!(word(layout::CX_OUT_FP), iso.cx.out_fp.addr());
        assert_eq!(word(layout::CX_OUT_PC), iso.cx.out_pc.addr());
    }
    assert_eq!(
        layout::FI_CODE,
        core::mem::offset_of!(crate::unit::FunctionInfo, code)
    );
    assert_eq!(
        crate::frame::native(0x1234) - 0x1234,
        layout::NATIVE_WORD_OFFSET
    );
    assert_eq!(layout::NATIVE_WORD_OFFSET.count_ones(), 1);
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
/// receiver, `argc` its argument count, `p0`-`p2` (3 parameters) their parameter a0-a2, and
/// `big` (1 parameter, [`BIG_REGISTERS`] more registers) its parameter a0.
const ENTRY: [&str; 7] = ["recv", "srecv", "argc", "p0", "p1", "p2", "big"];

/// The extra registers of `big`: its frame does not fit in the smallest VM stack
/// (`vm_stack_slots` 1024, `Isolate::new`).
const BIG_REGISTERS: u32 = 1100;

fn entry_defs() -> VerifiedUnit {
    let mut fns = Vec::new();
    for name in ENTRY {
        let params = match name {
            "big" => 1,
            _ if name.starts_with('p') => 3,
            _ => 0,
        };
        let mut m = meta(name, params, 0, 0, 0);
        if name == "srecv" {
            m.flags = ep9_bytecode::FunctionFlags::SIMPLE_PARAMETERS
                | ep9_bytecode::FunctionFlags::STRICT;
        }
        let mut f = Asm::new(m);
        if name == "big" {
            f.b.new_register_list(BIG_REGISTERS);
        }
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
        stub_param(0),
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
    // (shape, callee, values (receiver first for a property shape), printed result). The
    // sloppy `recv` gets the string receiver "R" wrapped (a String object, H14-Q2):
    // `enter_native` binds `this` as every other entry does since M3-P0 P0-D10.
    let rows: Vec<(Shape, &str, Vec<Lit>, &str)> = vec![
        (CallProperty0, "recv", vec![Str("R")], "[object String]"),
        (CallProperty0, "argc", vec![Str("R")], "0"),
        (CallProperty1, "p0", vec![Str("R"), Str("x")], "x"),
        (CallProperty1, "argc", vec![Str("R"), Int(7)], "1"),
        (CallProperty1, "p1", vec![Str("R"), Str("x")], "undefined"),
        (
            CallProperty1,
            "recv",
            vec![Str("R"), Str("x")],
            "[object String]",
        ),
        (CallProperty2, "p1", vec![Str("R"), Str("x"), Str("y")], "y"),
        (
            CallProperty2,
            "recv",
            vec![Str("R"), Int(1), Int(2)],
            "[object String]",
        ),
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
        (CallProperty, "recv", vec![Str("R")], "[object String]"),
        (
            CallAnyReceiver,
            "recv",
            vec![Str("R"), Int(1)],
            "[object String]",
        ),
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

#[test]
fn a_full_vm_stack_at_the_entry_is_the_vm_stack_range_error() {
    // The entry takes `big(x)` and `p0(y)`; `big`'s frame does not fit in a 1024-slot VM
    // stack, so its push in `enter_native` fails (`push_frame` -> None -> `stack_overflow`).
    // `big`'s stub calls nothing and the entry has no native stack check, so with the default
    // VM stack the same call returns x: only the VM stack size makes the RangeError.
    fn make() -> VerifiedUnit {
        let mut s = Asm::script(0);
        let f = s.b.new_register();
        let l = s.b.new_register_list(1);
        for (callee, arg) in [("big", "x"), ("p0", "y")] {
            guarded(&mut s, true, |s| {
                emit_call(
                    s,
                    Shape::CallUndefinedReceiver1,
                    callee,
                    &[Lit::Str(arg)],
                    f,
                    l,
                );
            });
        }
        s.lda_str("done");
        s.b.return_();
        unit("", vec![s.finish()])
    }
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            for slots in [1024, IsolateConfig::default().vm_stack_slots] {
                let config = IsolateConfig {
                    vm_stack_slots: slots,
                    ..config(d, stress)
                };
                let (r, out, hits) = run_entry(config, |iso| {
                    take_output();
                    let before = entry_hits();
                    let r = iso.run_script(make(), "full.js").map(|v| iso.display(v));
                    (r, take_output(), entry_hits() - before)
                });
                let label = format!("{} stress={stress} vm_stack_slots={slots}", d.name());
                assert_eq!(r.as_deref(), Ok("done"), "{label}");
                let big = if slots == 1024 {
                    "Maximum call stack size exceeded"
                } else {
                    "x"
                };
                assert_eq!(out, [big, "y"], "{label}");
                // Both calls took the entry (the count is taken before the frame push).
                assert_eq!(hits, 2, "{label}");
            }
        }
    }
}

// ---- placement (b) (P4-PLACEMENT-B) -------------------------------------------------------------

/// Placement (b) of D21 (P4-PLACEMENT-B): JIT frames on the VM stack, entered with the 4
/// handler arguments (`native::install_native_b`). The stubs exist for both handler ABIs: the
/// AArch64 one (the Rust ABI there: x0-x3 in, x0 out; pinned by
/// `the_handler_abi_passes_cx_pc_fp_acc_in_x0_to_x3`) and, since M3-P0 P0-B-X64, the x86-64
/// one (`extern "sysv64"` on every OS: rdi, rsi, rdx, rcx in, rax out; pinned by
/// `the_handler_abi_passes_cx_pc_fp_acc_in_rdi_rsi_rdx_rcx`). The tests after the encoders run
/// on both; on x86-64 they run in CI (linux-x64, windows-x64) and in the x86 sandbox.
///
/// A (b) stub runs one function's bytecode against its frame and leaves with a tail jump to
/// `cx.table[*pc]`, as the jit_b code does (x86-64: the same steps with the jit_b x86-64 frame,
/// r15 = cx, r13 = fp, and the exit `rex.w jmp qword ptr [rdi + rax*8]`):
///
/// ```text
/// leaf:   ldur x3, [x2, #word]                          acc = a frame word
///         ldur x1, [x2, #ret] ; ldur x2, [x2, #caller]   the frame's return pc and caller fp
///         eor x1/x2, #2^49                               decode the native words
///         ldrb w9, [x1] ; ldr x16, [x0, x9, lsl #3] ; br x16   x0 = cx, x3 = acc
/// framed: stp x29, x30, [sp, #-32]! ; mov x29, sp ; stp x26, x28, [sp, #16]
///         mov x28, x0 ; mov x26, x2 ; <table call (cx, fp, off, ..)> ; cbz x0, unwind
///         [reload a frame word] ; x3 = x0 ; x1, x2 from the frame
///   leave: mov x0, x28 ; ldrb/ldr ; ldp x26, x28 ; ldp x29, x30 ; br x16
///   unwind: RuntimeTable::unwind(cx, fp, off) ; x3 = x0 ; x1 = cx.out_pc ; x2 = cx.out_fp ; b leave
/// ```
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
mod placement_b {
    use super::*;

    /// AArch64 encodings of the (b) stubs (checked against `clang -c` in
    /// `b_stub_bytes_are_the_documented_instructions`).
    #[cfg(target_arch = "aarch64")]
    #[derive(Default)]
    pub(super) struct A64 {
        w: Vec<u32>,
    }

    #[cfg(target_arch = "aarch64")]
    impl A64 {
        fn op(&mut self, w: u32) -> &mut Self {
            self.w.push(w);
            self
        }
        /// `ldur x<t>, [x<n>, #off]`.
        fn ldur(&mut self, t: u32, n: u32, off: isize) -> &mut Self {
            assert!((-256..256).contains(&off));
            self.op(0xF840_0000 | ((off as u32 & 0x1FF) << 12) | (n << 5) | t)
        }
        fn ldr(&mut self, t: u32, n: u32, off: usize) -> &mut Self {
            self.op(a64_ldr(t, n, off))
        }
        /// `mov x<d>, x<m>`.
        fn mov(&mut self, d: u32, m: u32) -> &mut Self {
            self.op(0xAA00_03E0 | (m << 16) | d)
        }
        fn imm(&mut self, d: u32, v: u64) -> &mut Self {
            self.w.extend(a64_mov_imm(d, v));
            self
        }
        /// `eor x<d>, x<d>, #2^49` (decodes a native word).
        fn decode(&mut self, d: u32) -> &mut Self {
            assert_eq!(layout::NATIVE_WORD_OFFSET, 1 << 49);
            self.op(0xD24F_0000 | (d << 5) | d)
        }
        /// `cmp x<n>, x<m>`.
        fn cmp(&mut self, n: u32, m: u32) -> &mut Self {
            self.op(0xEB00_001F | (m << 16) | (n << 5))
        }
        fn here(&self) -> usize {
            self.w.len()
        }
        /// Points the branch at `at` (`b`, `b.cond` or `cbz`) to `to`.
        fn patch(&mut self, at: usize, to: usize) {
            let d = to as i64 - at as i64;
            let w = self.w[at];
            self.w[at] = if w >> 26 == 0b000101 {
                w | (d as u32 & 0x03FF_FFFF)
            } else {
                w | ((d as u32 & 0x7FFFF) << 5)
            };
        }
        /// x1 = return pc, x2 = caller fp of the frame in x<f> (decoded).
        fn ret_state(&mut self, f: u32) -> &mut Self {
            self.ldur(1, f, layout::FP_RETURN_PC)
                .ldur(2, f, layout::FP_CALLER_FP)
                .decode(1)
                .decode(2)
        }
        /// The tail jump: `ldrb w9, [x1] ; ldr x16, [x0, x9, lsl #3] ; br x16`.
        fn jump(&mut self) -> &mut Self {
            self.op(0x3940_0029).op(0xF869_7810).op(0xD61F_0200)
        }
        fn bytes(&self) -> Vec<u8> {
            words(&self.w)
        }
    }

    /// A leaf (b) stub: `return [fp + off]`.
    #[cfg(target_arch = "aarch64")]
    pub(super) fn b_load(off: isize) -> Vec<u8> {
        let mut a = A64::default();
        a.ldur(3, 2, off).ret_state(2).jump();
        a.bytes()
    }

    /// A framed (b) stub: `r = table[entry](cx, fp, off, srcs..)`; 0 unwinds; else `return r`
    /// (or the frame word `reload`, read after the call).
    #[cfg(target_arch = "aarch64")]
    pub(super) fn b_call(entry: usize, off: u32, srcs: &[Src], reload: Option<isize>) -> Vec<u8> {
        assert!(srcs.len() <= 3 && off < 1 << 16);
        let mut a = A64::default();
        a.op(0xA9BE_7BFD) // stp x29, x30, [sp, #-32]!
            .op(0x9100_03FD) // mov x29, sp
            .op(0xA901_73FA) // stp x26, x28, [sp, #16]
            .mov(28, 0)
            .mov(26, 2)
            .mov(1, 26)
            .op(0x5280_0000 | (off << 5) | 2); // movz w2, #off
        for (k, s) in srcs.iter().enumerate() {
            let t = 3 + k as u32;
            match *s {
                Src::Param(i) => a.ldur(t, 26, layout::fp_param(i)),
                Src::Imm(v) => a.imm(t, v),
            };
        }
        a.ldr(16, 28, layout::CX_RUNTIME_TABLE)
            .ldr(16, 16, entry)
            .op(0xD63F_0200); // blr x16
        let cbz = a.here();
        a.op(0xB400_0000); // cbz x0, unwind
        if let Some(r) = reload {
            a.ldur(0, 26, r);
        }
        a.mov(3, 0).ret_state(26);
        let leave = a.here();
        a.mov(0, 28)
            .op(0x3940_0029) // ldrb w9, [x1]
            .op(0xF869_7810) // ldr x16, [x0, x9, lsl #3]
            .op(0xA941_73FA) // ldp x26, x28, [sp, #16]
            .op(0xA8C2_7BFD) // ldp x29, x30, [sp], #32
            .op(0xD61F_0200); // br x16
        let unwind = a.here();
        a.patch(cbz, unwind);
        a.mov(0, 28)
            .mov(1, 26)
            .op(0x5280_0000 | (off << 5) | 2)
            .ldr(16, 28, layout::CX_RUNTIME_TABLE)
            .ldr(16, 16, layout::RT_UNWIND)
            .op(0xD63F_0200)
            .mov(3, 0)
            .ldr(1, 28, layout::CX_OUT_PC)
            .ldr(2, 28, layout::CX_OUT_FP);
        let b = a.here();
        a.op(0x1400_0000);
        a.patch(b, leave);
        a.bytes()
    }

    /// The (b) stub of `pin(x)`: checks the 4 handler arguments, then returns acc (x3). x0 must
    /// be the `Cx` (its runtime table pointer), x1 the callee's bytecode start (from x2's
    /// FunctionInfo word), x2 its frame. A failed check returns the int32 -1.
    #[cfg(target_arch = "aarch64")]
    pub(super) fn b_pin() -> Vec<u8> {
        let mut a = A64::default();
        let mut fails = Vec::new();
        a.ldur(9, 2, layout::FP_FUNCTION_INFO)
            .decode(9)
            .ldr(9, 9, layout::FI_CODE)
            .cmp(9, 1);
        fails.push(a.here());
        a.op(0x5400_0001); // b.ne fail
        a.ldr(9, 0, layout::CX_RUNTIME_TABLE).imm(
            10,
            core::ptr::from_ref(&crate::native::RUNTIME_TABLE).addr() as u64,
        );
        a.cmp(9, 10);
        fails.push(a.here());
        a.op(0x5400_0001); // b.ne fail
        let tail = a.here();
        a.ret_state(2).jump();
        let fail = a.here();
        for f in fails {
            a.patch(f, fail);
        }
        a.imm(3, crate::frame::int32_bits(-1));
        let b = a.here();
        a.op(0x1400_0000);
        a.patch(b, tail);
        a.bytes()
    }

    /// `x0 + 2 x1 + 4 x2 + 8 x3` then `ret`: called directly as a `Handler`.
    #[cfg(target_arch = "aarch64")]
    fn weigh() -> Vec<u8> {
        words(&[0x8B01_0400, 0x8B02_0800, 0x8B03_0C00, 0xD65F_03C0])
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn b_stub_bytes_are_the_documented_instructions() {
        // As `clang -c -target arm64-apple-macos` + `llvm-objdump -d` print them.
        let mut a = A64::default();
        a.mov(28, 0)
            .mov(26, 2)
            .mov(0, 28)
            .mov(1, 26)
            .mov(3, 0)
            .decode(1)
            .decode(2)
            .ldur(1, 2, -56)
            .ldur(2, 2, -64)
            .ldur(1, 26, -56)
            .ldur(9, 2, -32)
            .ldur(3, 2, -80)
            .ldr(9, 9, 24)
            .cmp(9, 1)
            .cmp(9, 10)
            .jump();
        assert_eq!(
            a.w,
            [
                0xAA00_03FC, // mov x28, x0
                0xAA02_03FA, // mov x26, x2
                0xAA1C_03E0, // mov x0, x28
                0xAA1A_03E1, // mov x1, x26
                0xAA00_03E3, // mov x3, x0
                0xD24F_0021, // eor x1, x1, #0x2000000000000
                0xD24F_0042, // eor x2, x2, #0x2000000000000
                0xF85C_8041, // ldur x1, [x2, #-0x38]
                0xF85C_0042, // ldur x2, [x2, #-0x40]
                0xF85C_8341, // ldur x1, [x26, #-0x38]
                0xF85E_0049, // ldur x9, [x2, #-0x20]
                0xF85B_0043, // ldur x3, [x2, #-0x50]
                0xF940_0D29, // ldr x9, [x9, #0x18]
                0xEB01_013F, // cmp x9, x1
                0xEB0A_013F, // cmp x9, x10
                0x3940_0029, // ldrb w9, [x1]
                0xF869_7810, // ldr x16, [x0, x9, lsl #3]
                0xD61F_0200, // br x16
            ]
        );
        // The frame offsets these stubs use (H14, FRAME 2).
        assert_eq!(
            (
                layout::FP_RETURN_PC,
                layout::FP_CALLER_FP,
                layout::FP_FUNCTION_INFO
            ),
            (-56, -64, -32)
        );
        let w = |v: Vec<u8>| -> Vec<u32> {
            v.chunks(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect()
        };
        assert_eq!(
            w(weigh()),
            [0x8B01_0400, 0x8B02_0800, 0x8B03_0C00, 0xD65F_03C0] // add x0, x0, x1, lsl #1 ..
        );
        // Branch patching: a forward cbz and a backward b.
        let call = w(b_call(layout::RT_POLL, 0, &[], None));
        let cbz = call.iter().position(|&x| x >> 24 == 0xB4).unwrap();
        let b = call.len() - 1;
        assert_eq!(call[b] >> 26, 0b000101);
        let back = (call[b] & 0x03FF_FFFF) as i32 - (1 << 26);
        let leave = (b as i32 + back) as usize;
        assert_eq!(
            call[leave], 0xAA1C_03E0,
            "b goes back to `leave: mov x0, x28`"
        );
        let fwd = ((call[cbz] >> 5) & 0x7FFFF) as usize;
        assert_eq!(
            call[cbz + fwd],
            0xAA1C_03E0,
            "cbz goes to `unwind: mov x0, x28`"
        );
        assert_eq!(call[cbz + fwd + 1], 0xAA1A_03E1);
    }

    /// The aarch64 handler ABI that (b) code relies on (plan-v6 task 6): `Handler` is the Rust
    /// ABI there (an unstable ABI; rustc 1.99.1 puts `(cx, pc, fp, acc)` in x0-x3 and the result
    /// in x0). A production (b) needs an `extern "C"` handler ABI on aarch64 (an H15 rewrite).
    #[cfg(target_arch = "aarch64")]
    #[test]
    fn the_handler_abi_passes_cx_pc_fp_acc_in_x0_to_x3() {
        with_b_stubs(&[weigh()], |code| {
            let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
            // SAFETY: the stub is a function with 4 register arguments and a register result
            // that reads no memory.
            let h: crate::interp::Handler = unsafe {
                core::mem::transmute::<*const (), crate::interp::Handler>(
                    core::ptr::with_exposed_provenance(code[0].start),
                )
            };
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let cx_addr = core::ptr::from_mut(cx).addr() as u64;
            let (pc, fp, acc) = (0x1000_usize, 0x2_0000_usize, 0x30_0000_u64);
            // SAFETY: as above; the pointers are never read.
            let r = unsafe {
                h(
                    cx,
                    core::ptr::with_exposed_provenance(pc),
                    core::ptr::with_exposed_provenance_mut(fp),
                    acc,
                )
            };
            let want = cx_addr
                .wrapping_add(2 * pc as u64)
                .wrapping_add(4 * fp as u64)
                .wrapping_add(8 * acc);
            assert_eq!(r, want, "x0 + 2 x1 + 4 x2 + 8 x3");
        });
    }

    /// x86-64 encodings of the (b) stubs (checked against `llvm-mc -triple=x86_64` in
    /// `b_stub_bytes_are_the_documented_x86_64_instructions`). Register numbers are the x86
    /// ones: rax 0, rcx 1, rdx 2, rbx 3, rsp 4, rbp 5, rsi 6, rdi 7, r8-r15 8-15. The
    /// handler ABI is `extern "sysv64"` on every OS: cx in rdi, pc in rsi, fp in rdx, acc in
    /// rcx (pinned by `the_handler_abi_passes_cx_pc_fp_acc_in_rdi_rsi_rdx_rcx`). The encoder
    /// and its byte test build on every host (no x86-64 code runs there); the stubs run on
    /// x86-64 only.
    #[derive(Default)]
    pub(super) struct X64 {
        b: Vec<u8>,
    }

    impl X64 {
        fn op(&mut self, b: &[u8]) -> &mut Self {
            self.b.extend_from_slice(b);
            self
        }
        /// A ModRM memory operand `[r<n> + off]` with register field `r` (disp8 when it fits,
        /// else disp32; `n` is never rsp or r12, which need a SIB byte).
        fn mem(&mut self, r: u8, n: u8, off: isize) -> &mut Self {
            assert_ne!(n & 7, 4, "no SIB base");
            if let Ok(d) = i8::try_from(off) {
                self.op(&[0x40 | ((r & 7) << 3) | (n & 7), d as u8])
            } else {
                let d = i32::try_from(off).expect("disp32");
                self.op(&[0x80 | ((r & 7) << 3) | (n & 7)]);
                self.op(&d.to_le_bytes())
            }
        }
        /// `mov r<d>, qword ptr [r<n> + off]`.
        fn load(&mut self, d: u8, n: u8, off: isize) -> &mut Self {
            self.op(&[0x48 | ((d >> 3) << 2) | (n >> 3), 0x8B])
                .mem(d, n, off)
        }
        /// `mov r<d>, r<s>`.
        fn mov(&mut self, d: u8, s: u8) -> &mut Self {
            self.op(&[
                0x48 | ((s >> 3) << 2) | (d >> 3),
                0x89,
                0xC0 | ((s & 7) << 3) | (d & 7),
            ])
        }
        /// `movabs r<d>, v`.
        fn imm(&mut self, d: u8, v: u64) -> &mut Self {
            self.op(&[0x48 | (d >> 3), 0xB8 | (d & 7)])
                .op(&v.to_le_bytes())
        }
        /// `mov edx, v` (the bytecode offset operand of a table call).
        fn edx(&mut self, v: u32) -> &mut Self {
            self.op(&[0xBA]).op(&v.to_le_bytes())
        }
        /// `btc r<d>, 49` (decodes a native word: toggles 2^49).
        fn decode(&mut self, d: u8) -> &mut Self {
            assert_eq!(layout::NATIVE_WORD_OFFSET, 1 << 49);
            self.op(&[0x48 | (d >> 3), 0x0F, 0xBA, 0xF8 | (d & 7), 49])
        }
        /// `cmp r<n>, r<m>`.
        fn cmp(&mut self, n: u8, m: u8) -> &mut Self {
            self.op(&[
                0x48 | ((m >> 3) << 2) | (n >> 3),
                0x39,
                0xC0 | ((m & 7) << 3) | (n & 7),
            ])
        }
        /// `call qword ptr [rax + entry]`.
        fn call(&mut self, entry: usize) -> &mut Self {
            self.op(&[0xFF]).mem(2, 0, entry as isize)
        }
        /// `test rax, rax`.
        fn test_rax(&mut self) -> &mut Self {
            self.op(&[0x48, 0x85, 0xC0])
        }
        /// rsi = return pc, rdx = caller fp of the frame in r<f> (decoded).
        fn ret_state(&mut self, f: u8) -> &mut Self {
            self.load(6, f, layout::FP_RETURN_PC)
                .load(2, f, layout::FP_CALLER_FP)
                .decode(6)
                .decode(2)
        }
        /// `push rbp ; push r15 ; push r13 ; lea rbp, [rsp + 16]` (the jit_b frame).
        fn frame(&mut self) -> &mut Self {
            self.op(&[0x55, 0x41, 0x57, 0x41, 0x55, 0x48, 0x8D, 0x6C, 0x24, 0x10])
        }
        /// `movzx eax, byte ptr [rsi]`, then (`framed`) `lea rsp, [rbp - 16] ; pop r13 ; pop
        /// r15 ; pop rbp`, then the exit jump `rex.w jmp qword ptr [rdi + rax*8]`.
        fn jump(&mut self, framed: bool) -> &mut Self {
            self.op(&[0x0F, 0xB6, 0x06]);
            if framed {
                self.op(&[0x48, 0x8D, 0x65, 0xF0, 0x41, 0x5D, 0x41, 0x5F, 0x5D]);
            }
            self.op(&[0x48, 0xFF, 0x24, 0xC7])
        }
        fn here(&self) -> usize {
            self.b.len()
        }
        /// A `jcc rel32` (`cc`: the second opcode byte, 0x84 jz, 0x85 jnz) or, with `cc` 0,
        /// a `jmp rel32`, to be patched; returns its offset.
        fn branch(&mut self, cc: u8) -> usize {
            let at = self.here();
            if cc == 0 {
                self.op(&[0xE9, 0, 0, 0, 0]);
            } else {
                self.op(&[0x0F, cc, 0, 0, 0, 0]);
            }
            at
        }
        /// Points the branch at `at` (from [`X64::branch`]) to `to`.
        fn patch(&mut self, at: usize, to: usize) {
            let len = if self.b[at] == 0xE9 { 5 } else { 6 };
            let rel = i32::try_from(to as isize - (at + len) as isize).expect("rel32");
            self.b[at + len - 4..at + len].copy_from_slice(&rel.to_le_bytes());
        }
        fn bytes(&self) -> Vec<u8> {
            self.b.clone()
        }
    }

    /// A leaf (b) stub: `return [fp + off]` (x86-64).
    fn x64_b_load(off: isize) -> Vec<u8> {
        let mut a = X64::default();
        a.load(1, 2, off).ret_state(2).jump(false);
        a.bytes()
    }

    /// A framed (b) stub (x86-64): `r = table[entry](cx, fp, off, srcs..)`; 0 unwinds; else
    /// `return r` (or the frame word `reload`, read after the call). The frame is jit_b's:
    /// r15 = cx, r13 = fp, rsp 16-byte aligned at the call.
    fn x64_b_call(entry: usize, off: u32, srcs: &[Src], reload: Option<isize>) -> Vec<u8> {
        assert!(srcs.len() <= 3);
        let mut a = X64::default();
        a.frame().mov(15, 7).mov(13, 2).mov(6, 13).edx(off);
        for (k, s) in srcs.iter().enumerate() {
            let t = [1, 8, 9][k];
            match *s {
                Src::Param(i) => a.load(t, 13, layout::fp_param(i)),
                Src::Imm(v) => a.imm(t, v),
            };
        }
        a.load(0, 15, layout::CX_RUNTIME_TABLE as isize)
            .call(entry)
            .test_rax();
        let jz = a.branch(0x84);
        if let Some(r) = reload {
            a.load(0, 13, r);
        }
        a.mov(1, 0).ret_state(13);
        let leave = a.here();
        a.mov(7, 15).jump(true);
        let unwind = a.here();
        a.patch(jz, unwind);
        a.mov(7, 15)
            .mov(6, 13)
            .edx(off)
            .load(0, 15, layout::CX_RUNTIME_TABLE as isize)
            .call(layout::RT_UNWIND)
            .mov(1, 0)
            .load(6, 15, layout::CX_OUT_PC as isize)
            .load(2, 15, layout::CX_OUT_FP as isize);
        let back = a.branch(0);
        a.patch(back, leave);
        a.bytes()
    }

    /// The (b) stub of `pin(x)` (x86-64): checks the 4 handler arguments, then returns acc
    /// (rcx). rdi must be the `Cx` (its runtime table pointer), rsi the callee's bytecode start
    /// (from rdx's FunctionInfo word), rdx its frame. A failed check returns the int32 -1.
    fn x64_b_pin() -> Vec<u8> {
        let mut a = X64::default();
        a.load(9, 2, layout::FP_FUNCTION_INFO)
            .decode(9)
            .load(9, 9, layout::FI_CODE as isize)
            .cmp(9, 6);
        let f1 = a.branch(0x85);
        a.load(9, 7, layout::CX_RUNTIME_TABLE as isize).imm(
            10,
            core::ptr::from_ref(&crate::native::RUNTIME_TABLE).addr() as u64,
        );
        a.cmp(9, 10);
        let f2 = a.branch(0x85);
        let tail = a.here();
        a.ret_state(2).jump(false);
        let fail = a.here();
        a.patch(f1, fail);
        a.patch(f2, fail);
        a.imm(1, crate::frame::int32_bits(-1));
        let b = a.branch(0);
        a.patch(b, tail);
        a.bytes()
    }

    /// `rdi + 2 rsi + 4 rdx + 8 rcx` then `ret`: called directly as a `Handler`.
    fn x64_weigh() -> Vec<u8> {
        vec![
            0x48, 0x8D, 0x04, 0x77, // lea rax, [rdi + 2*rsi]
            0x48, 0x8D, 0x04, 0x90, // lea rax, [rax + 4*rdx]
            0x48, 0x8D, 0x04, 0xC8, // lea rax, [rax + 8*rcx]
            0xC3, // ret
        ]
    }

    /// The x86-64 (b) stubs on this target.
    #[cfg(target_arch = "x86_64")]
    pub(super) fn b_load(off: isize) -> Vec<u8> {
        x64_b_load(off)
    }
    #[cfg(target_arch = "x86_64")]
    pub(super) fn b_call(entry: usize, off: u32, srcs: &[Src], reload: Option<isize>) -> Vec<u8> {
        x64_b_call(entry, off, srcs, reload)
    }
    #[cfg(target_arch = "x86_64")]
    pub(super) fn b_pin() -> Vec<u8> {
        x64_b_pin()
    }
    #[cfg(target_arch = "x86_64")]
    fn weigh() -> Vec<u8> {
        x64_weigh()
    }

    /// The x86-64 encodings (any host: it only compares bytes).
    #[test]
    fn b_stub_bytes_are_the_documented_x86_64_instructions() {
        // As `llvm-mc -triple=x86_64 -show-encoding` prints them (intel syntax input).
        let mut a = X64::default();
        a.mov(15, 7) // mov r15, rdi
            .mov(13, 2) // mov r13, rdx
            .mov(7, 15) // mov rdi, r15
            .mov(6, 13) // mov rsi, r13
            .mov(1, 0) // mov rcx, rax
            .decode(6) // btc rsi, 49
            .decode(2) // btc rdx, 49
            .decode(9) // btc r9, 49
            .load(6, 2, -56) // mov rsi, qword ptr [rdx - 56]
            .load(2, 2, -64) // mov rdx, qword ptr [rdx - 64]
            .load(6, 13, -56) // mov rsi, qword ptr [r13 - 56]
            .load(9, 2, -32) // mov r9, qword ptr [rdx - 32]
            .load(1, 2, -80) // mov rcx, qword ptr [rdx - 80]
            .load(9, 9, 24) // mov r9, qword ptr [r9 + 24]
            .load(0, 15, 2128) // mov rax, qword ptr [r15 + 2128]
            .load(8, 13, -88) // mov r8, qword ptr [r13 - 88]
            .cmp(9, 6) // cmp r9, rsi
            .cmp(9, 10) // cmp r9, r10
            .imm(10, 0x1_2345_6789) // movabs r10, 0x123456789
            .edx(5) // mov edx, 5
            .call(8) // call qword ptr [rax + 8]
            .test_rax() // test rax, rax
            .jump(false); // movzx eax, byte ptr [rsi] ; rex64 jmp qword ptr [rdi + 8*rax]
        #[rustfmt::skip]
        let want: [&[u8]; 23] = [
            &[0x49, 0x89, 0xFF],
            &[0x49, 0x89, 0xD5],
            &[0x4C, 0x89, 0xFF],
            &[0x4C, 0x89, 0xEE],
            &[0x48, 0x89, 0xC1],
            &[0x48, 0x0F, 0xBA, 0xFE, 0x31],
            &[0x48, 0x0F, 0xBA, 0xFA, 0x31],
            &[0x49, 0x0F, 0xBA, 0xF9, 0x31],
            &[0x48, 0x8B, 0x72, 0xC8],
            &[0x48, 0x8B, 0x52, 0xC0],
            &[0x49, 0x8B, 0x75, 0xC8],
            &[0x4C, 0x8B, 0x4A, 0xE0],
            &[0x48, 0x8B, 0x4A, 0xB0],
            &[0x4D, 0x8B, 0x49, 0x18],
            &[0x49, 0x8B, 0x87, 0x50, 0x08, 0x00, 0x00],
            &[0x4D, 0x8B, 0x45, 0xA8],
            &[0x49, 0x39, 0xF1],
            &[0x4D, 0x39, 0xD1],
            &[0x49, 0xBA, 0x89, 0x67, 0x45, 0x23, 0x01, 0x00, 0x00, 0x00],
            &[0xBA, 0x05, 0x00, 0x00, 0x00],
            &[0xFF, 0x50, 0x08],
            &[0x48, 0x85, 0xC0],
            &[0x0F, 0xB6, 0x06, 0x48, 0xFF, 0x24, 0xC7],
        ];
        assert_eq!(a.b, want.concat());
        // The leaf stub `return this` (llvm-mc --disassemble): mov rcx, [rdx - 72] ; mov rsi,
        // [rdx - 56] ; mov rdx, [rdx - 64] ; btc rsi, 49 ; btc rdx, 49 ; movzx eax, byte ptr
        // [rsi] ; rex64 jmp qword ptr [rdi + 8*rax].
        assert_eq!(layout::FP_RECEIVER, -72);
        assert_eq!(
            x64_b_load(layout::FP_RECEIVER),
            [
                0x48, 0x8B, 0x4A, 0xB8, 0x48, 0x8B, 0x72, 0xC8, 0x48, 0x8B, 0x52, 0xC0, 0x48, 0x0F,
                0xBA, 0xFE, 0x31, 0x48, 0x0F, 0xBA, 0xFA, 0x31, 0x0F, 0xB6, 0x06, 0x48, 0xFF, 0x24,
                0xC7,
            ]
        );
        // `pin`: two checks that branch forward to `fail` (jnz rel32), the leaf exit, then
        // `fail: movabs rcx, -1 (int32) ; jmp` back to the exit.
        let pin = x64_b_pin();
        assert_eq!(pin[..4], [0x4C, 0x8B, 0x4A, 0xE0], "mov r9, [rdx - 32]");
        assert_eq!(
            pin[pin.len() - 15..pin.len() - 13],
            [0x48, 0xB9],
            "movabs rcx"
        );
        assert_eq!(pin[pin.len() - 5], 0xE9, "jmp back");
        // lea rax, [rdi + 2*rsi] ; lea rax, [rax + 4*rdx] ; lea rax, [rax + 8*rcx] ; ret
        assert_eq!(
            x64_weigh(),
            [
                0x48, 0x8D, 0x04, 0x77, 0x48, 0x8D, 0x04, 0x90, 0x48, 0x8D, 0x04, 0xC8, 0xC3
            ]
        );
        // The frame offsets these stubs use (H14, FRAME 2).
        assert_eq!(
            (
                layout::FP_RETURN_PC,
                layout::FP_CALLER_FP,
                layout::FP_FUNCTION_INFO
            ),
            (-56, -64, -32)
        );
        // The frame and the epilogue are jit_b's (spikes/spike-b/src/jit_b/x64.rs): push rbp ;
        // push r15 ; push r13 ; lea rbp, [rsp + 16] / lea rsp, [rbp - 16] ; pop r13 ; pop r15 ;
        // pop rbp ; rex64 jmp qword ptr [rdi + 8*rax].
        let call = x64_b_call(layout::RT_POLL, 0, &[], None);
        assert_eq!(
            call[..10],
            [0x55, 0x41, 0x57, 0x41, 0x55, 0x48, 0x8D, 0x6C, 0x24, 0x10]
        );
        let epi = [
            0x48, 0x8D, 0x65, 0xF0, 0x41, 0x5D, 0x41, 0x5F, 0x5D, 0x48, 0xFF, 0x24, 0xC7,
        ];
        let at = call
            .windows(epi.len())
            .position(|w| w == epi)
            .expect("the epilogue");
        // Branch patching: the forward jz goes to `unwind: mov rdi, r15` (right after the
        // epilogue), the backward jmp at the end goes to `leave: mov rdi, r15`.
        let unwind = at + epi.len();
        assert_eq!(call[unwind..unwind + 3], [0x4C, 0x89, 0xFF]);
        let jz = call.windows(2).position(|w| w == [0x0F, 0x84]).expect("jz");
        let rel = i32::from_le_bytes(call[jz + 2..jz + 6].try_into().unwrap());
        assert_eq!(
            jz as isize + 6 + rel as isize,
            unwind as isize,
            "jz -> unwind"
        );
        let jmp = call.len() - 5;
        assert_eq!(call[jmp], 0xE9);
        let rel = i32::from_le_bytes(call[jmp + 1..].try_into().unwrap());
        let leave = (jmp as isize + 5 + rel as isize) as usize;
        assert_eq!(call[leave..leave + 6], [0x4C, 0x89, 0xFF, 0x0F, 0xB6, 0x06]);
        assert_eq!(
            leave + 6,
            at,
            "leave: mov rdi, r15 ; movzx eax, [rsi] ; epilogue"
        );
    }

    /// The x86-64 handler ABI that (b) code relies on (P0-D8): `Handler` is `extern "sysv64"`
    /// (`-unwind` under `panic = "unwind"`) on every OS, Windows too: `(cx, pc, fp, acc)` in
    /// rdi, rsi, rdx, rcx and the result in rax. A stable ABI (unlike the aarch64 Rust ABI).
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_handler_abi_passes_cx_pc_fp_acc_in_rdi_rsi_rdx_rcx() {
        with_b_stubs(&[weigh()], |code| {
            let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
            // SAFETY: the stub is a function with 4 register arguments and a register result
            // that reads no memory.
            let h: crate::interp::Handler = unsafe {
                core::mem::transmute::<*const (), crate::interp::Handler>(
                    core::ptr::with_exposed_provenance(code[0].start),
                )
            };
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let cx_addr = core::ptr::from_mut(cx).addr() as u64;
            let (pc, fp, acc) = (0x1000_usize, 0x2_0000_usize, 0x30_0000_u64);
            // SAFETY: as above; the pointers are never read.
            let r = unsafe {
                h(
                    cx,
                    core::ptr::with_exposed_provenance(pc),
                    core::ptr::with_exposed_provenance_mut(fp),
                    acc,
                )
            };
            let want = cx_addr
                .wrapping_add(2 * pc as u64)
                .wrapping_add(4 * fp as u64)
                .wrapping_add(8 * acc);
            assert_eq!(r, want, "rdi + 2 rsi + 4 rdx + 8 rcx");
        });
    }

    /// Bytes per (b) stub in the code pages.
    const B_STUB_SIZE: usize = 256;

    /// [`with_stubs`] with room for the framed (b) stubs.
    pub(super) fn with_b_stubs<R>(stubs: &[Vec<u8>], f: impl FnOnce(&[Range<usize>]) -> R) -> R {
        let range = CodeRange::process().expect("reserve the process code range");
        let len = (stubs.len() * B_STUB_SIZE).div_ceil(page_size()) * page_size();
        let mut chunk = range.alloc(len).expect("a chunk of the code range");
        let mut mem = chunk
            .commit(WxMode::DEFAULT)
            .expect("commit the code pages");
        let mut ranges = Vec::new();
        for (i, s) in stubs.iter().enumerate() {
            assert!(s.len() <= B_STUB_SIZE, "{} bytes", s.len());
            let off = i * B_STUB_SIZE;
            let mut w = mem.open_write(off, s.len()).expect("open a write window");
            w.bytes().copy_from_slice(s);
            w.close().expect("close the write window");
            let start = mem.exec_addr(off);
            ranges.push(start..start + s.len());
        }
        f(&ranges)
    }

    /// The (b) stubs of `defs()` (same order as [`NATIVE`]): each runs its function's bytecode,
    /// except `which` (returns a1, its bytecode a0) and `overflow` (throws the native stack
    /// RangeError, its bytecode returns undefined).
    fn b_stubs(offs: impl Fn(&str, Opcode) -> u32) -> Vec<Vec<u8>> {
        use Src::{Imm, Param};
        let call1 = Opcode::CallUndefinedReceiver1;
        vec![
            b_load(layout::fp_param(0)),
            b_call(
                layout::RT_CALL_ANY,
                offs("g", call1),
                &[Imm(UNDEFINED)],
                None,
            ),
            b_call(
                layout::RT_ADD,
                offs("add2", Opcode::Add),
                &[Param(0), Param(1)],
                None,
            ),
            b_call(layout::RT_INC, offs("inc1", Opcode::Inc), &[Param(0)], None),
            b_call(
                layout::RT_COMPARE,
                offs("lt", Opcode::TestLessThan),
                &[Imm(u64::from(cmp::LT)), Param(0), Param(1)],
                None,
            ),
            b_call(
                layout::RT_THROW,
                offs("nthrow", Opcode::Throw),
                &[Param(0)],
                None,
            ),
            b_call(layout::RT_POLL, 0, &[], None),
            b_call(layout::RT_STACK_OVERFLOW, 0, &[], None),
            b_load(layout::fp_param(1)),
            b_call(
                layout::RT_CALL_ANY,
                offs("keep", call1),
                &[Imm(UNDEFINED)],
                Some(layout::fp_param(1)),
            ),
        ]
    }

    /// The (b) stubs of [`ENTRY`] (`entry_defs()`; their bytecode returns undefined).
    fn b_entry_stubs() -> Vec<Vec<u8>> {
        vec![
            b_load(layout::FP_RECEIVER),
            b_load(layout::FP_RECEIVER),
            b_load(layout::FP_ARGC),
            b_load(layout::fp_param(0)),
            b_load(layout::fp_param(1)),
            b_load(layout::fp_param(2)),
            b_load(layout::fp_param(0)),
        ]
    }

    /// `pin(x)`: `LdaUndefined; Return`; its (b) stub is [`b_pin`].
    fn pin_defs() -> VerifiedUnit {
        let mut f = Asm::new(meta("pin", 1, 0, 0, 0));
        f.b.lda_undefined().return_();
        let mut s = Asm::script(0);
        let k = s.b.function_constant(FunctionId(1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global("pin");
        s.b.lda_undefined().return_();
        unit("", vec![s.finish(), f.finish()])
    }

    /// How a test installs the native stubs of `defs()`.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Install {
        /// Placement (b): `install_native_b` (the Opt slot).
        B,
        /// M2-P4-B-AB: the placement (a) stubs in AotBaseline slots (T0's hot entry).
        AHot,
    }

    /// An isolate with `defs()`, `entry_defs()` and `pin_defs()`, every native function
    /// installed as `how` says, run on a big native stack.
    fn run_b<R: Send + 'static>(
        how: Install,
        config: IsolateConfig,
        f: impl FnOnce(&mut Isolate) -> R + Send + 'static,
    ) -> R {
        on_big_stack(move || {
            let offs = {
                let mut iso = isolate(IsolateConfig::default());
                iso.run_script(defs(), "defs.js").unwrap();
                NATIVE
                    .iter()
                    .map(|n| {
                        let op = match *n {
                            "g" | "keep" => Some(Opcode::CallUndefinedReceiver1),
                            "add2" => Some(Opcode::Add),
                            "inc1" => Some(Opcode::Inc),
                            "lt" => Some(Opcode::TestLessThan),
                            "nthrow" => Some(Opcode::Throw),
                            _ => None,
                        };
                        (*n, op.map_or(0, |op| offset_in(&iso, n, op)))
                    })
                    .collect::<Vec<_>>()
            };
            let off = move |n: &str, _: Opcode| offs.iter().find(|(m, _)| *m == n).unwrap().1;
            let mut all = match how {
                Install::B => b_stubs(&off),
                Install::AHot => stubs(&off),
            };
            match how {
                Install::B => all.extend(b_entry_stubs()),
                Install::AHot => all.extend(entry_stubs()),
            }
            all.push(b_pin());
            with_b_stubs(&all, |code| {
                let mut iso = isolate(config);
                iso.run_script(defs(), "defs.js").unwrap();
                iso.run_script(entry_defs(), "entry-defs.js").unwrap();
                iso.run_script(pin_defs(), "pin.js").unwrap();
                let mut names: Vec<&str> = NATIVE.to_vec();
                names.extend(ENTRY);
                if how == Install::B {
                    names.push("pin");
                }
                for (name, range) in names.iter().zip(code) {
                    let f = iso.global(name).unwrap();
                    // SAFETY: `range` holds the stub for `name`, which runs its bytecode with
                    // the ABI of the install (b: the handler ABI; a: `NativeEntry`) and stays
                    // mapped until `with_b_stubs` returns, after the isolate drops.
                    let r = unsafe {
                        match how {
                            Install::B => crate::install_native_b(&iso, f, range.clone()),
                            Install::AHot => crate::install_native(
                                &iso,
                                f,
                                DispatchKind::AotBaseline,
                                range.clone(),
                            ),
                        }
                    };
                    r.unwrap();
                }
                let r = f(&mut iso);
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                r
            })
        })
    }

    /// (b) stub entries, (b) cold-arm entries, decode-free (a) entries and A-hot entries of
    /// this thread.
    fn counts() -> [u64; 4] {
        [
            crate::native::B_STUB_ENTRIES.with(core::cell::Cell::get),
            crate::native::B_COLD_ENTRIES.with(core::cell::Cell::get),
            entry_hits(),
            crate::native::A_HOT_ENTRIES.with(core::cell::Cell::get),
        ]
    }

    fn delta(before: [u64; 4]) -> [u64; 4] {
        let now = counts();
        core::array::from_fn(|i| now[i] - before[i])
    }

    /// The rows of [`t0_calls_native_code_and_native_code_calls_back`], with the outputs of
    /// placement (b) code: `which` gives a1 where its bytecode gives a0.
    fn calls_rows() -> (Vec<(&'static str, Vec<Lit>)>, Vec<&'static str>) {
        use Lit::{Global, Int, Str};
        let rows = vec![
            ("id", vec![Int(41)]),
            ("g", vec![Global("bump"), Str("a")]),
            ("g", vec![Global("inc1"), Int(41)]),
            ("g", vec![Global("print"), Str("hi")]),
            ("add2", vec![Int(1), Int(2)]),
            ("add2", vec![Str("a"), Str("b")]),
            ("inc1", vec![Str("41")]),
            ("lt", vec![Int(1), Int(2)]),
            ("lt", vec![Str("b"), Str("a")]),
            ("poll", vec![]),
            ("which", vec![Int(1), Int(2)]),
            ("keep", vec![Global("bump"), Str("k")]),
            ("keep", vec![Global("g"), Global("bump")]),
            // (b) keep -> call_any -> (b) g -> call_any -> (b) inc1: two nested runs.
            ("keep", vec![Global("g"), Global("inc1")]),
        ];
        let want = vec![
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
            "[object Function]",
        ];
        (rows, want)
    }

    #[test]
    fn t0_calls_b_code_and_b_code_calls_back() {
        let (rows, want) = calls_rows();
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let rows2 = rows.clone();
                let (r, out, n) = run_b(Install::B, config(d, stress), move |iso| {
                    let before = counts();
                    let (r, out) = run_rows(iso, &rows2);
                    (r, out, delta(before))
                });
                let label = format!("{} stress={stress}", d.name());
                assert_eq!(r.as_deref(), Ok("done"), "{label}");
                let mut want = want.clone();
                if d == Dispatch::Match {
                    // D4: the match loop runs the bytecode of a (b) function (which: a0).
                    want[11] = "1";
                }
                assert_eq!(out, want, "{label}");
                // Under a threaded expansion every (b) call enters the stub: 14 rows from T0,
                // and 4 nested calls through call_any (row 3: inc1; row 13: g; row 14: g and
                // inc1). The match loop never enters (b) code (D4).
                let stub = if d == Dispatch::Match { 0 } else { 18 };
                assert_eq!(n, [stub, 0, 0, 0], "{label}: [stub, cold arm, (a), A-hot]");
            }
        }
    }

    /// The 4 handler arguments reach (b) code in x0-x3, and its tail jump back gives T0 the
    /// accumulator in x3: `pin` checks x0 (the Cx), x1 (its bytecode start) and x2 (its frame)
    /// and returns x3. On the hot path x3 is the caller's accumulator at the call (`pin`
    /// itself: the script loads the callee last); on the cold arm (the budget is out),
    /// `slow::call`'s result (undefined). A failed check returns -1; a wrong register for any
    /// argument gives a wrong result or a crash.
    #[test]
    fn b_code_gets_the_four_handler_arguments_and_tail_jumps_back() {
        fn make() -> VerifiedUnit {
            let mut s = Asm::script(0);
            let f = s.b.new_register();
            let l = s.b.new_register_list(2);
            emit_call(
                &mut s,
                Shape::CallUndefinedReceiver1,
                "pin",
                &[Lit::Int(7)],
                f,
                l,
            );
            s.print_acc();
            s.lda_str("done");
            s.b.return_();
            unit("", vec![s.finish()])
        }
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let runs = run_b(Install::B, config(d, stress), |iso| {
                    let mut runs = Vec::new();
                    for budget in [None, Some(1)] {
                        if let Some(b) = budget {
                            iso.cx.budget = b;
                        }
                        take_output();
                        let before = counts();
                        let r = iso.run_script(make(), "pin.js").map(|v| iso.display(v));
                        runs.push((r, take_output(), delta(before)));
                    }
                    runs
                });
                let label = format!("{} stress={stress}", d.name());
                let threaded = d != Dispatch::Match;
                for (i, (r, out, n)) in runs.into_iter().enumerate() {
                    let label = format!("{label} {}", ["hot", "cold"][i]);
                    assert_eq!(r.as_deref(), Ok("done"), "{label}");
                    let want = match (threaded, i) {
                        (true, 0) => "[object Function]",
                        _ => "undefined",
                    };
                    assert_eq!(out, [want], "{label}");
                    assert_eq!(n, [u64::from(threaded), i as u64, 0, 0], "{label}");
                }
            }
        }
    }

    /// Every call shape of T0 enters (b) code on the hot path (one stub entry each), with the
    /// receiver and arguments of the (a) entry test, except a `CallAnyReceiver`/`CallProperty`
    /// with count 0 (no receiver: `call_list_with_receiver` goes cold) and the 8 calls of a
    /// sloppy callee with the string receiver "R" (H14-Q2: `call_fast` goes cold and
    /// `enter_b_cold` wraps it, so `recv` returns a String object), which take the cold arm.
    /// A prefixed (`Wide`) call takes the hot path too (`scaled::step` runs the same fast
    /// body). The cold arm also takes the spread call, the bound callee and the call that finds
    /// the budget out. Under the match loop the bytecode runs (undefined).
    #[test]
    fn every_call_shape_enters_b_code_hot_and_the_rest_cold() {
        use Lit::{Global, Int, Str};
        use Shape::*;
        let rows: Vec<(Shape, &str, Vec<Lit>, &str)> = vec![
            (CallProperty0, "recv", vec![Str("R")], "[object String]"),
            (CallProperty0, "argc", vec![Str("R")], "0"),
            (CallProperty1, "p0", vec![Str("R"), Str("x")], "x"),
            (CallProperty1, "p1", vec![Str("R"), Str("x")], "undefined"),
            (CallProperty2, "p1", vec![Str("R"), Str("x"), Str("y")], "y"),
            (
                CallProperty,
                "p2",
                vec![Str("R"), Str("x"), Str("y"), Str("z")],
                "z",
            ),
            (CallProperty, "recv", vec![Str("R")], "[object String]"),
            (
                CallAnyReceiver,
                "recv",
                vec![Str("R"), Int(1)],
                "[object String]",
            ),
            (CallAnyReceiver, "srecv", vec![], "undefined"),
            (CallUndefinedReceiver0, "argc", vec![], "0"),
            (CallUndefinedReceiver0, "recv", vec![], "[object Object]"),
            (CallUndefinedReceiver1, "p0", vec![Str("x")], "x"),
            (CallUndefinedReceiver2, "p1", vec![Str("x"), Str("y")], "y"),
            (
                CallUndefinedReceiver,
                "argc",
                vec![Int(1), Int(2), Int(3), Int(4)],
                "4",
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
        let cold = || {
            let mut s = Asm::script(0);
            let f = s.b.new_register();
            let l = s.b.new_register_list(3);
            let big = s.b.new_register_list(140);
            let (wf, wa) = (big.get(139), big.get(138));
            // 1. Wide CallUndefinedReceiver1 p0("w") (the hot path through `scaled::step`).
            s.lda_str("w");
            s.b.star(wa);
            s.lda_global("p0");
            s.b.star(wf);
            s.b.call_undefined_receiver1(wf, wa, s.call);
            s.print_acc();
            // 2. CallWithSpread p0(...[]) with receiver undefined: argc 0, a0 undefined.
            s.b.lda_undefined().star(l.get(0));
            s.b.create_empty_array_literal(s.lit).star(l.get(1));
            s.lda_global("argc");
            s.b.star(f);
            let two = RegisterList::new(l.get(0).local_index().unwrap(), 2);
            s.b.call_with_spread(f, two, 2, s.call);
            s.print_acc();
            // 3. A bound (b) function: bind(p0, undefined)("x").
            s.lda_global("p0");
            s.b.star(l.get(0));
            s.b.lda_undefined().star(l.get(1));
            s.lda_global("bind");
            s.b.star(f);
            s.b.call_property1(f, l.get(0), l.get(1), s.call);
            s.b.star(f);
            s.lda_str("bx");
            s.b.star(l.get(2));
            s.b.call_undefined_receiver1(f, l.get(2), s.call);
            s.print_acc();
            s.lda_str("done");
            s.b.return_();
            unit("", vec![s.finish()])
        };
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
        let want: Vec<String> = rows.iter().map(|r| r.3.to_string()).collect();
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let make = make.clone();
                let (hot, cold_run, budget) = run_b(Install::B, config(d, stress), move |iso| {
                    iso.register_host_function("bind", 1, bind);
                    take_output();
                    let before = counts();
                    let r = iso.run_script(make(), "shapes.js").map(|v| iso.display(v));
                    let hot = (r, take_output(), delta(before));
                    let before = counts();
                    let r = iso.run_script(cold(), "cold.js").map(|v| iso.display(v));
                    let cold_run = (r, take_output(), delta(before));
                    // The budget poll: with 1 unit left, `call_fast` goes cold at once.
                    iso.cx.budget = 1;
                    let before = counts();
                    let (r, out) = run_rows(iso, &[("p0", vec![Lit::Str("b")])]);
                    (hot, cold_run, (r, out, delta(before)))
                });
                let label = format!("{} stress={stress}", d.name());
                let threaded = d != Dispatch::Match;
                assert_eq!(hot.0.as_deref(), Ok("done"), "{label}");
                if threaded {
                    assert_eq!(hot.1, want, "{label}");
                    // One stub entry per row; `srecv` (count 0) through the cold arm; the last
                    // row's g calls T0 bump.
                    assert_eq!(hot.2, [rows.len() as u64, 9, 0, 0], "{label}");
                } else {
                    let mut w: Vec<String> = vec!["undefined".into(); rows.len()];
                    // g's bytecode calls bump: the same result as its (b) stub.
                    *w.last_mut().unwrap() = "c!".into();
                    assert_eq!(hot.1, w, "{label}");
                    assert_eq!(hot.2, [0, 9, 0, 0], "{label}");
                }
                assert_eq!(cold_run.0.as_deref(), Ok("done"), "{label}");
                if threaded {
                    assert_eq!(cold_run.1, ["w", "0", "bx"], "{label}");
                    // Wide: hot; spread and bound: the cold arm.
                    assert_eq!(cold_run.2, [3, 2, 0, 0], "{label}");
                } else {
                    assert_eq!(cold_run.1, ["undefined"; 3], "{label}");
                    assert_eq!(cold_run.2, [0, 2, 0, 0], "{label}");
                }
                assert_eq!(budget.0.as_deref(), Ok("done"), "{label}");
                let b = if threaded { "b" } else { "undefined" };
                assert_eq!(budget.1, [b], "{label}");
                assert_eq!(budget.2, [u64::from(threaded), 1, 0, 0], "{label}");
            }
        }
    }

    /// The paths that do not enter (b) code: `[[Construct]]` and a call from Rust through
    /// `call::call_with` (`Isolate::call`) throw "not implemented", as for placement (a);
    /// `call_from_rust` enters it (a nested dispatch) and puts the entry state back.
    #[test]
    fn construct_and_isolate_call_refuse_b_code_and_call_from_rust_runs_it() {
        for &d in Dispatch::ALL {
            let label = d.name();
            run_b(Install::B, config(d, false), move |iso| {
                let ni = "not implemented in ep9 v0: calling code of this dispatch kind";
                let p0 = iso.global("p0").unwrap();
                let e = iso
                    .call(p0, Value::UNDEFINED, &[Value::int32(1)])
                    .unwrap_err();
                assert!(e.message.contains(ni), "{label}: {e:?}");
                let rows = [("id", vec![Lit::Int(3)])];
                let (r, _) = run_rows(iso, &rows);
                assert_eq!(r.as_deref(), Ok("done"), "{label}");
                // new id(1)
                let mut s = Asm::script(0);
                let l = s.b.new_register_list(1);
                s.b.lda_smi(1).star(l.get(0));
                let one = RegisterList::new(l.get(0).local_index().unwrap(), 1);
                s.construct_global("id", one);
                s.b.return_();
                let e = iso
                    .run_script(unit("", vec![s.finish()]), "new.js")
                    .unwrap_err();
                assert!(e.message.contains(ni), "{label}: {e:?}");
                let before = entry_state(iso);
                let stub0 = counts();
                let r = crate::call_from_rust(iso, p0, Value::UNDEFINED, &[Value::int32(5)]);
                // The match loop runs p0's bytecode (undefined), D4.
                let want = if d == Dispatch::Match {
                    "undefined"
                } else {
                    "5"
                };
                assert_eq!(r.map(|v| iso.display(v)).as_deref(), Ok(want), "{label}");
                let n = delta(stub0);
                assert_eq!(n[0], u64::from(d != Dispatch::Match), "{label}");
                assert_eq!(entry_state(iso), before, "{label}");
                let nthrow = iso.global("nthrow").unwrap();
                let s = string(iso, "rt");
                let e = crate::call_from_rust(iso, nthrow, Value::UNDEFINED, &[s]).unwrap_err();
                assert_eq!(e.message, "rt", "{label}");
                assert_eq!(entry_state(iso), before, "{label}");
            });
        }
    }

    /// Exceptions: a throw in a (b) table call (`nthrow`: `RuntimeTable::throw`), a T0 throw
    /// under (b) `g` (call_any returns 0), the native stack RangeError of `overflow` — each
    /// unwinds through `RuntimeTable::unwind` to the T0 handler, and the frame goes on. An
    /// uncaught one names the (b) frame of `g` in Error.stack at its call of `f`.
    #[test]
    fn exceptions_in_b_code_unwind_to_t0_handlers() {
        use Lit::{Global, Str};
        fn make() -> VerifiedUnit {
            let mut s = Asm::script(0);
            let f = s.b.new_register();
            let l = s.b.new_register_list(4);
            guarded(&mut s, false, |s| {
                let v = [Str("R"), Str("e1")];
                emit_call(s, Shape::CallProperty1, "nthrow", &v, f, l);
            });
            guarded(&mut s, true, |s| {
                let v = [Str("R"), Global("t"), Str("e2")];
                emit_call(s, Shape::CallAnyReceiver, "g", &v, f, l);
            });
            // (b) keep -> (b) g -> T0 t: two (b) frames, the throw crosses both.
            guarded(&mut s, true, |s| {
                let v = [Global("g"), Global("t")];
                emit_call(s, Shape::CallUndefinedReceiver2, "keep", &v, f, l);
            });
            guarded(&mut s, true, |s| {
                emit_call(s, Shape::CallProperty0, "overflow", &[Str("R")], f, l);
            });
            s.b.ldar(l.get(0));
            s.print_acc();
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
        let col = |needle: &str| {
            let p = pos(DEFS, needle) as usize;
            let line_start = DEFS[..p].rfind('\n').map_or(0, |i| i + 1);
            let line = DEFS[..p].matches('\n').count() + 1;
            format!("defs.js:{line}:{}", p - line_start + 1)
        };
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let (r, out) = run_b(Install::B, config(d, stress), |iso| {
                    take_output();
                    let r = iso.run_script(make(), "exc.js").map(|v| iso.display(v));
                    (r, take_output())
                });
                let label = format!("{} stress={stress}", d.name());
                // keep(g, t): g(t) -> t(undefined) throws `new Error(undefined)`: message "".
                let overflow = if d == Dispatch::Match {
                    "undefined"
                } else {
                    "Maximum call stack size exceeded"
                };
                assert_eq!(out, ["e1", "e2", "", overflow, "R"], "{label}");
                let e = r.unwrap_err();
                assert_eq!(e.message, "Error: late", "{label}");
                assert_eq!(
                    e.stack.unwrap(),
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

    /// A full VM stack at a (b) call is the VM stack's RangeError, on the hot path and on the
    /// cold arm (`big`'s frame does not fit in 1024 slots).
    #[test]
    fn a_full_vm_stack_at_a_b_call_is_the_vm_stack_range_error() {
        fn make() -> VerifiedUnit {
            let mut s = Asm::script(0);
            let f = s.b.new_register();
            let l = s.b.new_register_list(1);
            for (callee, arg) in [("big", "x"), ("p0", "y")] {
                guarded(&mut s, true, |s| {
                    let v = [Lit::Str(arg)];
                    emit_call(s, Shape::CallUndefinedReceiver1, callee, &v, f, l);
                });
            }
            s.lda_str("done");
            s.b.return_();
            unit("", vec![s.finish()])
        }
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                for slots in [1024, IsolateConfig::default().vm_stack_slots] {
                    let config = IsolateConfig {
                        vm_stack_slots: slots,
                        ..config(d, stress)
                    };
                    let (r, out, n) = run_b(Install::B, config, |iso| {
                        take_output();
                        let before = counts();
                        let r = iso.run_script(make(), "full.js").map(|v| iso.display(v));
                        (r, take_output(), delta(before))
                    });
                    let label = format!("{} stress={stress} slots={slots}", d.name());
                    assert_eq!(r.as_deref(), Ok("done"), "{label}");
                    let threaded = d != Dispatch::Match;
                    let (big, y) = match (slots == 1024, threaded) {
                        (true, _) => ("Maximum call stack size exceeded", "y"),
                        (false, true) => ("x", "y"),
                        (false, false) => ("undefined", "undefined"),
                    };
                    let y = if threaded { y } else { "undefined" };
                    assert_eq!(out, [big, y], "{label}");
                    // The failed push on the hot path goes cold, and the cold arm's push
                    // fails too (no stub entry for big at 1024 slots).
                    let big_cold = u64::from(slots == 1024);
                    let stubs = if threaded { 2 - big_cold } else { 0 };
                    assert_eq!(n, [stubs, big_cold, 0, 0], "{label}");
                }
            }
        }
    }

    /// The interrupt poll in (b) code (`RuntimeTable::poll`): a termination stops the script
    /// at the poll; the next run works. Deep T0 <-> (b) recursion (`r` -> (b) `g` -> call_any
    /// -> `r` ...) ends in a catchable RangeError (the native stack check of the nested runs).
    #[test]
    fn termination_and_deep_recursion_under_b_code() {
        // The match loop runs `poll`'s bytecode, which has no poll (D4): threaded only.
        for &d in Dispatch::ALL.iter().filter(|&&d| d != Dispatch::Match) {
            let label = d.name();
            let e = run_b(Install::B, config(d, false), |iso| {
                iso.terminate_handle().terminate();
                let (r, _) = run_rows(iso, &[("poll", vec![])]);
                let e = r.unwrap_err();
                let (again, out) = run_rows(iso, &[("poll", vec![])]);
                let want = (Ok("done"), &["true".to_string()][..]);
                assert_eq!((again.as_deref(), out.as_slice()), want);
                e
            });
            assert!(e.terminated, "{label}: {e:?}");
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
                        b_stubs(|n, op| offset_in(&iso, n, op))
                    };
                    with_b_stubs(&probe, |code| {
                        let mut iso = isolate(config(d, false));
                        iso.run_script(defs(), "defs.js").unwrap();
                        for (name, range) in NATIVE.iter().zip(code) {
                            let f = iso.global(name).unwrap();
                            // SAFETY: as in `run_b`.
                            unsafe { crate::install_native_b(&iso, f, range.clone()) }.unwrap();
                        }
                        take_output();
                        let r = iso.run_script(rec_unit(), "rec.js").map(|v| iso.display(v));
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

    /// `try { r(1) } catch (e) { print(e.message) } return "after"`.
    fn rec_unit() -> VerifiedUnit {
        let mut s = Asm::script(0);
        guarded(&mut s, true, |s| {
            s.b.lda_smi(1).star(s.a.get(0));
            s.lda_global("r");
            s.b.star(s.f);
            s.b.call_undefined_receiver1(s.f, s.a.get(0), s.call);
            s.lda_str("no throw");
        });
        s.lda_str("after");
        s.b.return_();
        unit("", vec![s.finish()])
    }

    /// M2-P4-B-AB (config A-hot): placement (a) code in an AotBaseline slot is entered from
    /// T0's `call_fast` (`OP_ENTER_A_HOT`, `native::a_hot_call`), not from `slow::call`: the
    /// results equal the (a) results, and the decode-free cold entry counts nothing. The
    /// exceptions of the (a) code unwind from its frame to the T0 handlers.
    #[test]
    fn a_hot_enters_placement_a_code_from_call_fast() {
        let (rows, mut want) = calls_rows();
        // The (a) stubs: `which` gives a1 too, like the (b) ones.
        want[11] = "2";
        for &d in Dispatch::ALL {
            for stress in [false, true] {
                let rows2 = rows.clone();
                let (r, out, n, exc) = run_b(Install::AHot, config(d, stress), move |iso| {
                    let before = counts();
                    let (r, out) = run_rows(iso, &rows2);
                    let n = delta(before);
                    let (exc, _) = run_rows(iso, &[("nthrow", vec![Lit::Str("boom")])]);
                    (r, out, n, exc.map_err(|e| e.message))
                });
                let label = format!("{} stress={stress}", d.name());
                assert_eq!(r.as_deref(), Ok("done"), "{label}");
                let mut want = want.clone();
                if d == Dispatch::Match {
                    want[11] = "1";
                }
                assert_eq!(out, want, "{label}");
                assert_eq!(exc, Err("boom".to_string()), "{label}");
                // Threaded: each of the 14 rows enters its (a) code from `call_fast`; the
                // nested calls go through call_any (no count) and nothing takes the cold
                // decode-free entry. The match loop runs the bytecode at the stub (D4).
                let hot = if d == Dispatch::Match {
                    0
                } else {
                    rows.len() as u64
                };
                assert_eq!(n, [0, 0, 0, hot], "{label}");
            }
        }
    }

    // ---- the D14 gap (M3-P0 P0-D10): the receiver of a native callee --------------------------

    /// The callees of [`a_primitive_receiver_of_a_sloppy_native_callee_is_wrapped`]: `callr(f,
    /// r, x)` is `return f.call(r, x)` (`CallProperty1 a0, a1, a2`); `tw` (sloppy) and `stw`
    /// (strict) return `this`; `ta` (sloppy) returns a0. Their stubs do what their bytecode
    /// does, so every dispatch gives the same output (the match loop runs the bytecode of a
    /// (b) function, D4).
    const THIS_FNS: [&str; 4] = ["callr", "tw", "stw", "ta"];

    fn this_defs() -> VerifiedUnit {
        let mut callr = Asm::new(meta("callr", 3, 0, 0, 0));
        let (a0, a1, a2) = (
            callr.b.parameter(0),
            callr.b.parameter(1),
            callr.b.parameter(2),
        );
        let slot = callr.call;
        callr.b.call_property1(a0, a1, a2, slot).return_();
        let mut fns = vec![callr];
        for name in ["tw", "stw"] {
            let mut m = meta(name, 1, 0, 0, 0);
            if name == "stw" {
                m.flags = ep9_bytecode::FunctionFlags::SIMPLE_PARAMETERS
                    | ep9_bytecode::FunctionFlags::STRICT;
            }
            let mut f = Asm::new(m);
            let this = f.b.receiver();
            f.b.ldar(this).return_();
            fns.push(f);
        }
        let mut ta = Asm::new(meta("ta", 1, 0, 0, 0));
        let a0 = ta.b.parameter(0);
        ta.b.ldar(a0).return_();
        fns.push(ta);
        let mut s = Asm::script(0);
        for (i, name) in THIS_FNS.iter().enumerate() {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        s.b.lda_undefined().return_();
        let mut all = vec![s.finish()];
        all.extend(fns.into_iter().map(Asm::finish));
        unit("", all)
    }

    /// The D14 gap is closed (M3-P0 P0-D10): `enter_native` (placement (a)) and `run_b`
    /// (placement (b)) bind `this` with `try_bind_this` and wrap a primitive receiver of a
    /// sloppy callee (ToObject, `wrap_receiver`) after the frame push, as
    /// `call_function_object` does; a strict callee gets the receiver as it is. Each callee is
    /// entered through `call_any` (the native `callr` runs its `CallProperty1`) and through
    /// `call_from_rust`, with a number and a string receiver, on every dispatch, with and
    /// without GC stress (the wrap allocates: a GC there must find the copied arguments).
    /// Under a threaded expansion every (b) call enters (b) code: `callr` from T0 (hot), its
    /// callee through `call_any` -> `run_b`, and each `call_from_rust` -> `run_b`.
    #[test]
    fn a_primitive_receiver_of_a_sloppy_native_callee_is_wrapped() {
        use Lit::{Global, Int, Str};
        // (callee, receiver, want): `typeof this === "object"` shows as the wrapper's display.
        let rows: [(&str, Lit, &str); 4] = [
            ("tw", Int(5), "[object Number]"),
            ("tw", Str("R"), "[object String]"),
            ("stw", Int(5), "5"),
            ("ta", Int(5), "x"),
        ];
        let make = {
            let rows = rows.clone();
            move || {
                let mut s = Asm::script(0);
                let f = s.b.new_register();
                let l = s.b.new_register_list(3);
                for (callee, recv, _) in &rows {
                    let vals = [Global(callee), recv.clone(), Str("x")];
                    emit_call(&mut s, Shape::CallUndefinedReceiver, "callr", &vals, f, l);
                    s.print_acc();
                }
                s.lda_str("done");
                s.b.return_();
                unit("", vec![s.finish()])
            }
        };
        let want: Vec<&str> = rows.iter().map(|r| r.2).collect();
        for b in [false, true] {
            for &d in Dispatch::ALL {
                for stress in [false, true] {
                    let make = make.clone();
                    let rows = rows.clone();
                    let (script, rust, n) = on_big_stack(move || {
                        let offs = {
                            let mut iso = isolate(IsolateConfig::default());
                            iso.run_script(this_defs(), "this.js").unwrap();
                            offset_in(&iso, "callr", Opcode::CallProperty1)
                        };
                        let stubs = if b {
                            vec![
                                b_call(layout::RT_CALL_ANY, offs, &[Src::Imm(UNDEFINED)], None),
                                b_load(layout::FP_RECEIVER),
                                b_load(layout::FP_RECEIVER),
                                b_load(layout::fp_param(0)),
                            ]
                        } else {
                            vec![
                                stub_call(layout::RT_CALL_ANY, offs, &[Src::Imm(UNDEFINED)]),
                                stub_load(layout::FP_RECEIVER),
                                stub_load(layout::FP_RECEIVER),
                                stub_param(0),
                            ]
                        };
                        with_b_stubs(&stubs, |code| {
                            let mut iso = isolate(config(d, stress));
                            iso.run_script(this_defs(), "this.js").unwrap();
                            for (name, range) in THIS_FNS.iter().zip(code) {
                                let f = iso.global(name).unwrap();
                                // SAFETY: `range` holds the stub for `name`, made for the
                                // install's ABI (b: the handler ABI; a: `NativeEntry`); it stays
                                // mapped until `with_b_stubs` returns, after the isolate drops.
                                let r = unsafe {
                                    if b {
                                        crate::install_native_b(&iso, f, range.clone())
                                    } else {
                                        crate::install_native(
                                            &iso,
                                            f,
                                            DispatchKind::Baseline,
                                            range.clone(),
                                        )
                                    }
                                };
                                r.unwrap();
                            }
                            take_output();
                            let before = counts();
                            let r = iso.run_script(make(), "recv.js").map(|v| iso.display(v));
                            let script = (r, take_output());
                            let mut rust = Vec::new();
                            for (callee, recv, _) in &rows {
                                let f = iso.global(callee).unwrap();
                                let this = match recv {
                                    Int(i) => Value::int32(*i),
                                    Str(t) => string(&mut iso, t),
                                    Global(_) => unreachable!(),
                                };
                                let x = string(&mut iso, "x");
                                let st = entry_state(&iso);
                                let r = crate::call_from_rust(&mut iso, f, this, &[x]);
                                assert_eq!(entry_state(&iso), st);
                                // `print` shows the value as the script rows do.
                                let print = iso.global("print").unwrap();
                                let r = r.and_then(|v| iso.call(print, Value::UNDEFINED, &[v]));
                                rust.push(r.map(|_| take_output().concat()).map_err(|e| e.message));
                            }
                            let n = delta(before);
                            iso.collect_garbage();
                            iso.verify_heap().unwrap();
                            (script, rust, n)
                        })
                    });
                    let label = format!(
                        "{} {} stress={stress}",
                        if b { "(b)" } else { "(a)" },
                        d.name()
                    );
                    assert_eq!(script.0.as_deref(), Ok("done"), "{label}");
                    assert_eq!(script.1, want, "{label}: through call_any");
                    let rust_want: Vec<Result<String, String>> =
                        want.iter().map(|w| Ok(w.to_string())).collect();
                    assert_eq!(rust, rust_want, "{label}: through call_from_rust");
                    if b {
                        // Threaded: 4 rows x (callr + its callee) + 4 calls from Rust, all in
                        // (b) code. The match loop runs `callr`'s bytecode, whose call of a
                        // sloppy (b) callee with a primitive receiver takes the cold arm
                        // (`enter_b_cold` wraps it; rows 1, 2 and 4).
                        let want_n = if d == Dispatch::Match {
                            [0, 3, 0, 0]
                        } else {
                            [12, 0, 0, 0]
                        };
                        assert_eq!(n, want_n, "{label}: [stub, cold arm, (a), A-hot]");
                    }
                }
            }
        }
    }
}
