//! Tests of the production native entry (M7-P3 A2-ENTRY; doc 13 §3.4, C-10). They build and
//! run in every build: no `jit` and no `spike-native` (the spike-native tests are
//! `tests/native.rs`). Every test name starts with `native_entry_`.
//!
//! | Tests | What they show |
//! |---|---|
//! | `native_entry_runtime_table_*` | MC-w30-8: `Cx::runtime_table` points at the process table in every build, and the table has the layout of `tests/golden/runtime-table.txt` (the spike-native list) |
//! | `native_entry_registry_*` | the registry entries: `install_code` writes the slot and one `CodeEntry`, `uninstall_code` removes both, and every refusal changes nothing |
//! | `native_entry_runs_*` | hand-assembled stubs in this test binary's text run through the Rust entry `call_native`; a stub calls back through `call_any` (T0, host and native callees), an exception crosses its frame, GC stress scans it |
//! | `native_entry_backtrace_*` | the system unwinder walks from a callee through a stub frame into the Rust caller of the entry (`std::backtrace`; with `jit` also `ep9_platform::unwind`) |
//! | `native_entry_registered_*` (`jit` only) | the stub bytes copied into the process code range, with their unwind information registered at run time (DWARF CFI with `__register_frame`, or `RtlAddGrowableFunctionTable`), entered and walked |
//!
//! The stubs (the [`NativeEntry`](crate::layout::NativeEntry) ABI: `extern "C"` on aarch64,
//! `extern "sysv64"` on x86-64, every OS). They are naked functions, so their bytes are
//! exactly the instructions below; `native_entry_stub_bytes_*` checks them against the hand
//! encoding (checked with `llvm-mc --disassemble`):
//!
//! ```text
//! stub_a0(cx, fp)    = a0                          aarch64: ldur x0, [x1, #-80]; ret
//!                                                  x86-64:  mov rax, [rsi - 80]; ret
//! stub_call0(cx, fp) = call_any(cx, fp, 0, 0)      aarch64: stp x29, x30, [sp, #-16]!; mov x29, sp;
//!   (offset 0 is the                                        ldr x16, [x0, #CX_RUNTIME_TABLE];
//!    CallUndefinedReceiver0)                                ldr x16, [x16, #RT_CALL_ANY];
//!                                                           mov w2, #0; mov x3, xzr; blr x16;
//!                                                           ldp x29, x30, [sp], #16; ret
//!                                                  x86-64:  push rbp; mov rbp, rsp;
//!                                                           mov rax, [rdi + CX_RUNTIME_TABLE];
//!                                                           xor edx, edx; xor ecx, ecx;
//!                                                           call [rax + RT_CALL_ANY]; pop rbp; ret
//! ```
//!
//! The frame stub has unwind information: `.cfi_*` directives (unix) or `.seh_*` directives
//! (Windows) in its text, and the same rules as run-time records for the registered copy.

use std::cell::RefCell;

use ep9_bytecode::{FunctionFlags, FunctionId, FunctionKind, RegisterList, VerifiedUnit};

use super::{
    CodeEntry, DispatchKind, InstallError, RUNTIME_TABLE, RuntimeTable, call_native, install_code,
    uninstall_code,
};
use crate::dispatch::{CodeRegistry, RegistryError};
use crate::frame::UNDEFINED;
use crate::layout;
use crate::tests::{Asm, config, isolate, meta, on_big_stack, pos, unit};
use crate::unit::FunctionInfo;
use crate::{Args, Cx, Dispatch, Exception, Isolate, IsolateConfig, JsResult, Local, Value};

// ---- MC-w30-8: the runtime table in every build ----------------------------------------------

/// The table entries, in declaration order: name, offset of the field, `layout::RT_*`, and
/// the function in the process table.
fn table_entries(t: &RuntimeTable) -> [(&'static str, usize, usize, usize); 12] {
    use core::mem::offset_of;
    macro_rules! e {
        ($f:ident, $rt:ident) => {
            (
                stringify!($f),
                offset_of!(RuntimeTable, $f),
                layout::$rt,
                t.$f as usize,
            )
        };
    }
    [
        e!(poll, RT_POLL),
        e!(call_any, RT_CALL_ANY),
        e!(add, RT_ADD),
        e!(inc, RT_INC),
        e!(compare, RT_COMPARE),
        e!(stack_overflow, RT_STACK_OVERFLOW),
        e!(throw, RT_THROW),
        e!(unwind, RT_UNWIND),
        e!(get_keyed, RT_GET_KEYED),
        e!(set_keyed, RT_SET_KEYED),
        e!(deopt, RT_DEOPT),
        e!(load_global, RT_LOAD_GLOBAL),
    ]
}

/// The (index, offset, name) rows of `tests/golden/runtime-table.txt`: the list that the
/// spike-native test `the_runtime_table_matches_the_golden` writes from `RuntimeTable::ENTRIES`.
fn golden_rows() -> Vec<(usize, usize, String)> {
    let p =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/runtime-table.txt");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 4, "a golden row is: index offset name type: {l:?}");
            (
                f[0].parse().unwrap(),
                f[1].parse().unwrap(),
                f[2].to_string(),
            )
        })
        .collect()
}

/// MC-w30-8 (w30-verify; JIT reviewer F4): `Cx::runtime_table` is the process table in every
/// build, with or without `jit` (before M7-P3 it was null without `spike-native`), and every
/// entry is at the offset native code bakes in (`layout::RT_*`, VM_LAYOUT 1) and in the
/// golden list. On the old code the first assertion fails: the pointer is null.
#[test]
fn native_entry_runtime_table_is_the_process_table_in_every_build() {
    let iso = isolate(IsolateConfig::default());
    assert!(!iso.cx.runtime_table.is_null(), "Cx::runtime_table is null");
    assert!(core::ptr::eq(iso.cx.runtime_table, &RUNTIME_TABLE));
    let base = core::ptr::from_ref::<crate::interp::Cx>(&iso.cx).cast::<u8>();
    // SAFETY: CX_RUNTIME_TABLE names the pointer-sized field `runtime_table` of the live Cx.
    let word = unsafe { base.add(layout::CX_RUNTIME_TABLE).cast::<usize>().read() };
    assert_eq!(word, core::ptr::from_ref(&RUNTIME_TABLE).addr());
    let table = core::ptr::from_ref(&RUNTIME_TABLE).cast::<u8>();
    let entries = table_entries(&RUNTIME_TABLE);
    for (i, &(name, off, rt, f)) in entries.iter().enumerate() {
        assert_eq!((off, rt), (i * 8, i * 8), "{name}");
        // SAFETY: `off` is the offset of a fn pointer field of the live process table.
        let at = unsafe { table.add(off).cast::<usize>().read() };
        assert_eq!(at, f, "{name}: the table word is the table function");
        assert_ne!(f, 0, "{name}");
    }
    assert_eq!(size_of::<RuntimeTable>(), entries.len() * 8);
    // The spike-native layout: the golden rows, in order, with nothing more or less.
    let want: Vec<(usize, usize, String)> = entries
        .iter()
        .enumerate()
        .map(|(i, &(name, off, _, _))| (i, off, name.to_string()))
        .collect();
    assert_eq!(golden_rows(), want);
}

/// Two isolates share the one process table (native code and unit text are not per isolate).
#[test]
fn native_entry_runtime_table_is_one_per_process() {
    let a = isolate(IsolateConfig::default());
    let b = Isolate::new(IsolateConfig::default()).unwrap();
    assert!(core::ptr::eq(a.cx.runtime_table, b.cx.runtime_table));
    let other = std::thread::spawn(|| {
        let c = Isolate::new(IsolateConfig::default()).unwrap();
        c.cx.runtime_table.addr()
    })
    .join()
    .unwrap();
    assert_eq!(other, a.cx.runtime_table.addr());
}

// ---- the program -----------------------------------------------------------------------------

/// The source of the functions unit (positions for stack traces).
const SRC: &str = "function g(f) { return f(); }\n\
function t() { throw new Error(\"boom\"); }\n\
function bump() { return \"a\" + \"!\"; }\n";

/// The functions unit; every function is a global. Run natively (by the tests that install
/// them): id (stub_a0), g (stub_call0). T0 only: t, bump. Refused by `install_code`: kc (a
/// class constructor), gen (a generator function).
fn defs() -> VerifiedUnit {
    let p = |n: &str| pos(SRC, n);
    // id(x): Ldar a0; Return
    let mut id = Asm::new(meta("id", 1, 0, 0, 0));
    let a0 = id.b.parameter(0);
    id.b.ldar(a0).return_();
    // g(f): return f() -- the call is at offset 0, the offset stub_call0 passes.
    let mut g = Asm::new(meta("g", 1, 0, p("function g"), p("function t") - 1));
    let a0 = g.b.parameter(0);
    g.b.set_expression_position(p("f()"));
    g.b.call_undefined_receiver0(a0, g.call).return_();
    // t(): throw new Error("boom")
    let mut t = Asm::new(meta("t", 0, 0, p("function t"), p("function bump") - 1));
    let args = RegisterList::new(t.a.get(0).local_index().unwrap(), 1);
    t.lda_str("boom");
    t.b.star(args.get(0));
    t.b.set_expression_position(p("new Error"));
    t.construct_global("Error", args);
    t.b.throw();
    // bump(): return "a" + "!" (allocates)
    let mut bump = Asm::new(meta("bump", 0, 0, p("function bump"), SRC.len() as u32));
    let x = bump.b.new_register();
    bump.lda_str("a");
    bump.b.star(x);
    bump.lda_str("!");
    bump.b.add(x, bump.bin).return_();
    // kc: a class constructor (never entered here).
    let mut m = meta("kc", 0, 0, 0, 0);
    m.kind = FunctionKind::ClassConstructor;
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::STRICT;
    let mut kc = Asm::new(m);
    kc.b.lda_undefined().return_();
    let fns = vec![("id", id), ("g", g), ("t", t), ("bump", bump), ("kc", kc)];
    let mut s = Asm::script(SRC.len());
    for (i, (name, _)) in fns.iter().enumerate() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns.into_iter().map(|(_, a)| a.finish()));
    unit(SRC, all)
}

/// An isolate that ran `defs()`, with the host functions `walk` and `fail`.
fn defs_isolate(config: IsolateConfig) -> Isolate {
    let mut iso = isolate(config);
    iso.register_host_function("walk", 0, walk);
    iso.register_host_function("fail", 0, fail);
    iso.with_cx_checkpoint(|cx| cx.run_script(defs(), "defs.js").map(|_| ()))
        .unwrap();
    iso
}

/// The FunctionInfo of the global function `name`.
fn info<'i>(iso: &'i Isolate, name: &str) -> &'i FunctionInfo {
    let f = iso.global_raw(name).unwrap().to_bits();
    assert!(crate::object::is_function(f), "{name}");
    crate::object::function_info(f)
}

/// The Rust caller of the entry that the backtrace tests look for: Call(`f`, undefined,
/// `args`) through [`call_native`]. `None` if `f` has no native code.
#[inline(never)]
fn call_through_entry(
    iso: &mut Isolate,
    f: Value,
    args: &[Value],
) -> Option<Result<Value, Exception>> {
    let f = f.to_bits();
    let mut entered = false;
    let r = iso.with_cx(|acx| {
        let cx = acx.inner();
        let arg = |i: u32| args[i as usize].to_bits();
        let Some(r) = call_native(cx, f, UNDEFINED, args.len() as u32, arg) else {
            return Ok(UNDEFINED);
        };
        entered = true;
        r
    });
    entered.then(|| r.map(crate::runtime::val))
}

/// The VM state that the entry must put back on every exit (D20): the published top frame,
/// the scratch top and `Cx::out_fp`/`out_pc`.
fn vm_state(iso: &Isolate) -> (usize, usize, usize, usize) {
    let st = iso.rt.stack();
    (
        st.fp().addr(),
        st.scratch_top().addr(),
        iso.cx.out_fp.addr(),
        iso.cx.out_pc.addr(),
    )
}

// ---- host functions ----------------------------------------------------------------------------

thread_local! {
    /// What `walk` saw: the std backtrace text and, with `jit`, the system unwinder's return
    /// addresses.
    static WALKED: RefCell<Option<(String, Vec<usize>)>> = const { RefCell::new(None) };
}

/// `walk()`: records the native stack (it runs under a stub frame when a stub calls it) and
/// returns 42.
fn walk<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let text = std::backtrace::Backtrace::force_capture().to_string();
    #[cfg(feature = "jit")]
    let ips = ep9_platform::unwind::native_return_addresses(512);
    #[cfg(not(feature = "jit"))]
    let ips = Vec::new();
    WALKED.with(|w| *w.borrow_mut() = Some((text, ips)));
    Ok(Local::from_bits(Value::int32(42).to_bits()))
}

/// `fail()`: throws a TypeError from Rust.
fn fail<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Err(cx.throw_type_error("native entry probe"))
}

// ---- the stubs (naked functions in this binary's text) ----------------------------------------

/// The offset of the `a0` slot from fp, negated (the x86-64 form `[rsi - P0]`).
const P0: isize = -layout::fp_param(0);

/// `return a0`: a leaf with no frame and no stack change (so no unwind information).
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
unsafe extern "C" fn stub_a0(_cx: *mut layout::NativeCx, _fp: *mut u64) -> u64 {
    core::arch::naked_asm!("ldur x0, [x1, #-{p0}]", "ret", p0 = const P0);
}

/// `return a0`: a leaf with no frame and no stack change (so no unwind information).
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
unsafe extern "sysv64" fn stub_a0(_cx: *mut layout::NativeCx, _fp: *mut u64) -> u64 {
    core::arch::naked_asm!("mov rax, qword ptr [rsi - {p0}]", "ret", p0 = const P0);
}

/// `return call_any(cx, fp, 0, 0)`, with a frame record and its CFI.
#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
unsafe extern "C" fn ep9_a2entry_stub_call0(_cx: *mut layout::NativeCx, _fp: *mut u64) -> u64 {
    core::arch::naked_asm!(
        ".cfi_startproc",
        "stp x29, x30, [sp, #-16]!",
        ".cfi_def_cfa_offset 16",
        ".cfi_offset x30, -8",
        ".cfi_offset x29, -16",
        "mov x29, sp",
        ".cfi_def_cfa x29, 16",
        "ldr x16, [x0, #{table}]",
        "ldr x16, [x16, #{call_any}]",
        "mov w2, #0",
        "mov x3, xzr",
        "blr x16",
        "ldp x29, x30, [sp], #16",
        ".cfi_def_cfa sp, 0",
        ".cfi_restore x30",
        ".cfi_restore x29",
        "ret",
        ".cfi_endproc",
        table = const layout::CX_RUNTIME_TABLE,
        call_any = const layout::RT_CALL_ANY,
    );
}

/// `return call_any(cx, fp, 0, 0)`, with a frame and its CFI.
#[cfg(all(target_arch = "x86_64", not(windows)))]
#[unsafe(no_mangle)]
#[unsafe(naked)]
unsafe extern "sysv64" fn ep9_a2entry_stub_call0(_cx: *mut layout::NativeCx, _fp: *mut u64) -> u64 {
    core::arch::naked_asm!(
        ".cfi_startproc",
        "push rbp",
        ".cfi_def_cfa_offset 16",
        ".cfi_offset rbp, -16",
        "mov rbp, rsp",
        ".cfi_def_cfa_register rbp",
        "mov rax, qword ptr [rdi + {table}]",
        "xor edx, edx",
        "xor ecx, ecx",
        "call qword ptr [rax + {call_any}]",
        "pop rbp",
        ".cfi_def_cfa rsp, 8",
        "ret",
        ".cfi_endproc",
        table = const layout::CX_RUNTIME_TABLE,
        call_any = const layout::RT_CALL_ANY,
    );
}

/// `return call_any(cx, fp, 0, 0)`, with a frame and its Win64 unwind information (the
/// `sysv64` ABI on Windows too: the NativeEntry type).
#[cfg(all(target_arch = "x86_64", windows))]
#[unsafe(no_mangle)]
#[unsafe(naked)]
unsafe extern "sysv64" fn ep9_a2entry_stub_call0(_cx: *mut layout::NativeCx, _fp: *mut u64) -> u64 {
    core::arch::naked_asm!(
        ".seh_proc {me}",
        "push rbp",
        ".seh_pushreg rbp",
        ".seh_endprologue",
        "mov rbp, rsp",
        "mov rax, qword ptr [rdi + {table}]",
        "xor edx, edx",
        "xor ecx, ecx",
        "call qword ptr [rax + {call_any}]",
        "pop rbp",
        "ret",
        ".seh_endproc",
        me = sym ep9_a2entry_stub_call0,
        table = const layout::CX_RUNTIME_TABLE,
        call_any = const layout::RT_CALL_ANY,
    );
}

/// The symbol name of the frame stub (for messages).
const CALL0_NAME: &str = "ep9_a2entry_stub_call0";

/// The hand encoding of `stub_a0`.
fn a0_bytes() -> Vec<u8> {
    #[cfg(target_arch = "aarch64")]
    {
        // ldur x0, [x1, #-80]; ret
        let ldur = 0xF840_0000u32 | ((layout::fp_param(0) as u32 & 0x1FF) << 12) | (1 << 5);
        words(&[ldur, 0xD65F_03C0])
    }
    #[cfg(target_arch = "x86_64")]
    {
        // mov rax, [rsi + disp8]; ret
        let d = i8::try_from(layout::fp_param(0)).unwrap();
        vec![0x48, 0x8B, 0x46, d as u8, 0xC3]
    }
}

/// The hand encoding of `ep9_a2entry_stub_call0` and the offset of the return address of
/// its call of `call_any`.
fn call0_bytes() -> (Vec<u8>, usize) {
    #[cfg(target_arch = "aarch64")]
    {
        let ldr = |t: u32, n: u32, off: usize| {
            assert!(off.is_multiple_of(8) && off / 8 < 4096);
            0xF940_0000 | (((off / 8) as u32) << 10) | (n << 5) | t
        };
        let w = [
            0xA9BF_7BFD, // stp x29, x30, [sp, #-16]!
            0x9100_03FD, // mov x29, sp
            ldr(16, 0, layout::CX_RUNTIME_TABLE),
            ldr(16, 16, layout::RT_CALL_ANY),
            0x5280_0002, // mov w2, #0
            0xAA1F_03E3, // mov x3, xzr
            0xD63F_0200, // blr x16
            0xA8C1_7BFD, // ldp x29, x30, [sp], #16
            0xD65F_03C0, // ret
        ];
        (words(&w), 28)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut v = vec![0x55, 0x48, 0x89, 0xE5]; // push rbp; mov rbp, rsp
        // mov rax, [rdi + CX_RUNTIME_TABLE] (disp32: the table pointer is past the dispatch
        // table)
        let table = i32::try_from(layout::CX_RUNTIME_TABLE).unwrap();
        assert!(table > 127);
        v.extend([0x48, 0x8B, 0x87]);
        v.extend(table.to_le_bytes());
        v.extend([0x31, 0xD2, 0x31, 0xC9]); // xor edx, edx; xor ecx, ecx
        let e = u8::try_from(layout::RT_CALL_ANY).unwrap();
        assert!(e < 128);
        v.extend([0xFF, 0x50, e]); // call [rax + disp8]
        let ret = v.len();
        v.extend([0x5D, 0xC3]); // pop rbp; ret
        (v, ret)
    }
}

#[cfg(target_arch = "aarch64")]
fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The first byte of the code of the function at `addr`: the address itself, or on Windows
/// the target of an incremental-link jump thunk (`jmp rel32`) if the linker made one.
fn code_start(addr: usize) -> usize {
    #[cfg(all(target_arch = "x86_64", windows))]
    {
        // SAFETY: `addr` is the address of a function of this binary: at least 5 bytes of text.
        let b = unsafe { core::slice::from_raw_parts(addr as *const u8, 5) };
        if b[0] == 0xE9 {
            let rel = i32::from_le_bytes([b[1], b[2], b[3], b[4]]);
            return addr.wrapping_add(5).wrapping_add_signed(rel as isize);
        }
    }
    addr
}

/// The code range of a stub of this binary whose hand encoding is `bytes`.
fn stub_range(f: usize, bytes: &[u8]) -> core::ops::Range<usize> {
    let start = code_start(f);
    start..start + bytes.len()
}

/// The text of each stub is its hand encoding.
#[test]
fn native_entry_stub_bytes_are_the_hand_encoding() {
    let a0 = a0_bytes();
    let (call0, _) = call0_bytes();
    for (name, f, want) in [
        ("stub_a0", stub_a0 as *const () as usize, a0),
        (
            CALL0_NAME,
            ep9_a2entry_stub_call0 as *const () as usize,
            call0,
        ),
    ] {
        let r = stub_range(f, &want);
        // SAFETY: the stub's text is at least as long as its hand encoding (the asm above).
        let got = unsafe { core::slice::from_raw_parts(r.start as *const u8, want.len()) };
        assert_eq!(got, &want[..], "{name}");
    }
}

/// Installs the stubs for `id` (stub_a0) and `g` (stub_call0) as AotBaseline code (the A2
/// unit kind).
fn install_stubs(iso: &Isolate) -> (CodeEntry, CodeEntry) {
    let (call0, _) = call0_bytes();
    let a = stub_range(stub_a0 as *const () as usize, &a0_bytes());
    let c = stub_range(ep9_a2entry_stub_call0 as *const () as usize, &call0);
    // SAFETY: each range is the text of a stub with the NativeEntry ABI that runs the bytecode
    // of its function (`id`: Ldar a0; Return; `g`: the call at offset 0, then Return), and the
    // text lives as long as the process.
    unsafe {
        let e1 = install_code(&iso.rt, info(iso, "id"), DispatchKind::AotBaseline, a).unwrap();
        let e2 = install_code(&iso.rt, info(iso, "g"), DispatchKind::AotBaseline, c).unwrap();
        (e1, e2)
    }
}

// ---- the registry entries --------------------------------------------------------------------

/// `install_code` writes the slot and one registry entry; `uninstall_code` removes both and
/// puts the function back on T0. Every refusal leaves the slot and the registry as they were.
#[test]
fn native_entry_registry_entries_and_refusals() {
    let iso = defs_isolate(IsolateConfig::default());
    let id = info(&iso, "id");
    let n0 = iso.rt.registry.borrow().len();
    let bytecode = id.code.addr();
    assert_eq!(
        super::native_callee(iso.global_raw("id").unwrap().to_bits()).map(|_| ()),
        None
    );
    let (e, _) = install_stubs(&iso);
    assert_eq!(iso.rt.registry.borrow().len(), n0 + 2);
    assert_eq!(e.kind, DispatchKind::AotBaseline);
    assert_eq!(e.function, core::ptr::from_ref(id).addr());
    assert_eq!(id.slot.kind(), Some(DispatchKind::AotBaseline));
    assert_eq!(id.slot.entry(), e.start);
    assert!(super::native_callee(iso.global_raw("id").unwrap().to_bits()).is_some());
    // The pc lookup names the code and still names the bytecode (T0 frames, H14).
    let reg = |pc: usize| iso.rt.registry.borrow().lookup(pc);
    assert_eq!(reg(e.start), Some(e));
    assert_eq!(reg(e.start + e.len - 1), Some(e));
    assert_eq!(reg(bytecode).map(|e| e.kind), Some(DispatchKind::Interp));

    // Refusals: nothing changes.
    let other = e.start..e.start + e.len;
    let t = info(&iso, "t");
    let kc = info(&iso, "kc");
    let print = {
        let f = iso.global_raw("print").unwrap().to_bits();
        crate::object::function_info(f)
    };
    let free = {
        // A range no entry holds: the text of `install_stubs` (never run as native code).
        let f = code_start(install_stubs as *const () as usize);
        f..f + 4
    };
    // SAFETY: every call below is refused before the slot or the registry changes.
    unsafe {
        for k in [
            DispatchKind::Interp,
            DispatchKind::Opt,
            DispatchKind::AotOpt,
        ] {
            assert_eq!(
                install_code(&iso.rt, t, k, free.clone()),
                Err(InstallError::Kind(k))
            );
        }
        let base = DispatchKind::Baseline;
        assert_eq!(
            install_code(&iso.rt, id, base, free.clone()),
            Err(InstallError::AlreadyNative)
        );
        assert_eq!(
            install_code(&iso.rt, print, base, free.clone()),
            Err(InstallError::NotBytecode)
        );
        assert_eq!(
            install_code(&iso.rt, kc, base, free.clone()),
            Err(InstallError::ClassConstructor)
        );
        assert_eq!(
            install_code(&iso.rt, t, base, other.clone()),
            Err(InstallError::Registry(RegistryError::Overlap(e.start)))
        );
        assert_eq!(
            install_code(&iso.rt, t, base, free.start..free.start),
            Err(InstallError::Registry(RegistryError::Empty))
        );
    }
    assert_eq!(t.slot.kind(), Some(DispatchKind::Interp));
    assert_eq!(iso.rt.registry.borrow().len(), n0 + 2);
    assert_eq!(uninstall_code(&iso.rt, t), Err(InstallError::NotNative));

    // Uninstall: back on T0, the entry is gone, the bytecode entry stays.
    assert_eq!(uninstall_code(&iso.rt, id), Ok(e));
    assert_eq!(id.slot.kind(), Some(DispatchKind::Interp));
    assert_eq!(id.slot.entry(), bytecode);
    assert_eq!(reg(e.start), None);
    assert_eq!(reg(bytecode).map(|e| e.kind), Some(DispatchKind::Interp));
    assert_eq!(uninstall_code(&iso.rt, id), Err(InstallError::NotNative));
    assert_eq!(iso.rt.registry.borrow().len(), n0 + 1);
    // A second install of the same code works again.
    // SAFETY: as in `install_stubs`.
    let e2 = unsafe { install_code(&iso.rt, id, DispatchKind::Baseline, e.range()) }.unwrap();
    assert_eq!(e2.range(), e.range());
}

/// A generator function gets no native code (`call::run_resumed` resumes in T0 only).
#[test]
fn native_entry_registry_refuses_a_generator() {
    let mut iso = isolate(IsolateConfig::default());
    iso.with_cx_checkpoint(|cx| cx.run_script(generator_unit(), "gen.js").map(|_| ()))
        .unwrap();
    let gen_fi = info(&iso, "gen");
    assert!(gen_fi.is_generator());
    let code = stub_range(stub_a0 as *const () as usize, &a0_bytes());
    // SAFETY: refused before the slot or the registry changes.
    let r = unsafe { install_code(&iso.rt, gen_fi, DispatchKind::AotBaseline, code.clone()) };
    assert_eq!(r, Err(InstallError::Generator));
    assert_eq!(gen_fi.slot.kind(), Some(DispatchKind::Interp));
    // An async function and an async generator resume through `call::run_resumed` too (T0
    // only), so `install_code` refuses them in the same way (Codex M7-P3-CODEX-R1 A2-ENTRY 1).
    for (name, flags) in [
        (
            "af",
            FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::ASYNC,
        ),
        (
            "ag",
            FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::ASYNC | FunctionFlags::GENERATOR,
        ),
    ] {
        let mut iso = isolate(IsolateConfig::default());
        iso.with_cx_checkpoint(|cx| {
            cx.run_script(flagged_unit(name, flags), "async.js")
                .map(|_| ())
        })
        .unwrap();
        let fi = info(&iso, name);
        assert!(!fi.is_generator(), "{name}");
        assert!(fi.is_async_function() || fi.is_async_generator(), "{name}");
        let n0 = iso.rt.registry.borrow().len();
        // SAFETY: refused before the slot or the registry changes.
        let r = unsafe { install_code(&iso.rt, fi, DispatchKind::AotBaseline, code.clone()) };
        assert_eq!(r, Err(InstallError::Generator), "{name}");
        assert_eq!(fi.slot.kind(), Some(DispatchKind::Interp), "{name}");
        assert_eq!(iso.rt.registry.borrow().len(), n0, "{name}");
    }
}

/// `function name() { return undefined; }` with the function flags `flags`, as a global.
fn flagged_unit(name: &str, flags: FunctionFlags) -> VerifiedUnit {
    let mut m = meta(name, 0, 0, 0, 0);
    m.flags = flags;
    let mut s = Asm::new(m);
    s.b.lda_undefined().return_();
    let mut sc = Asm::script(0);
    let k = sc.b.function_constant(FunctionId(1));
    sc.b.create_closure(k, sc.closure, 0);
    sc.sta_global(name);
    sc.b.lda_undefined().return_();
    unit("", vec![sc.finish(), s.finish()])
}

/// `function* gen() {}` in the generator shape (the prologue and the initial suspend), as a
/// global.
fn generator_unit() -> VerifiedUnit {
    use ep9_bytecode::RuntimeId;
    let mut m = meta("gen", 0, 0, 0, 0);
    m.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::GENERATOR;
    let mut s = Asm::new(m);
    let g = s.b.new_register();
    let t = s.b.new_register_list(2);
    let save = RegisterList::new(0, s.b.register_count());
    let (table, resumes) = s.b.jump_table(1);
    let this = s.b.receiver();
    s.b.lda_new_target().star(g);
    s.b.switch_on_generator_state(g, table);
    s.b.lda_current_function().star(t.get(0));
    s.b.ldar(this).star(t.get(1));
    s.b.call_runtime(RuntimeId::CreateGeneratorObject, t, 2);
    s.b.star(g);
    s.b.suspend_generator(g, save, save.len(), 0);
    s.b.bind(resumes[0]);
    s.b.resume_generator(g, save, save.len());
    s.b.lda_undefined().return_();
    let mut sc = Asm::script(0);
    let k = sc.b.function_constant(FunctionId(1));
    sc.b.create_closure(k, sc.closure, 0);
    sc.sta_global("gen");
    sc.b.lda_undefined().return_();
    unit("", vec![sc.finish(), s.finish()])
}

/// The spike installer (`install_native`) and the production one refuse the same cases with
/// the same errors, so the two bodies cannot drift apart (the spike body stays as it was).
#[cfg(feature = "spike-native")]
#[test]
fn native_entry_registry_refusals_equal_the_spike_installer() {
    let iso = defs_isolate(IsolateConfig::default());
    let free = {
        let f = code_start(install_stubs as *const () as usize);
        f..f + 4
    };
    for name in ["t", "kc", "print"] {
        let f = iso.global_raw(name).unwrap();
        let fi = crate::object::function_info(f.to_bits());
        for k in [
            DispatchKind::Interp,
            DispatchKind::Opt,
            DispatchKind::Baseline,
        ] {
            if k == DispatchKind::Baseline && name == "t" {
                continue; // accepted by both
            }
            // SAFETY: every call is refused before the slot or the registry changes.
            let (a, b) = unsafe {
                (
                    crate::install_native(&iso, f, k, free.clone()),
                    install_code(&iso.rt, fi, k, free.clone()).map(|_| ()),
                )
            };
            assert_eq!(a, b, "{name} {k:?}");
            assert!(a.is_err(), "{name} {k:?}");
        }
    }
}

// ---- the entry runs hand-assembled code --------------------------------------------------------

/// The script `print(g(<callee>))` through T0 (T0 -> native `g` -> callee), for the T0 path
/// of `call_any` under a T0 caller.
fn calls_unit(callee: &str) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let f = s.b.new_register();
    let a = s.b.new_register();
    s.lda_global(callee);
    s.b.star(a);
    s.lda_global("g");
    s.b.star(f);
    s.b.call_undefined_receiver1(f, a, s.call);
    s.print_acc();
    s.lda_str("done");
    s.b.return_();
    unit("", vec![s.finish()])
}

/// The entry runs stub code from Rust under every dispatch expansion, with and without GC
/// stress: `id` returns its a0; `g` calls back through `call_any` into T0 (`bump`, which
/// allocates), into a host function (`walk`) and into native code (`id`, so the native frame
/// of `g` is under a nested entry). The VM state is put back after each call. A T0 call of
/// a native-slotted function still goes cold and is refused without `spike-native` (doc 13
/// §3.4: T0 paths are unchanged). Without `jit`, so the default `cargo test -p ep9_vm` runs it
/// (C-10); on the old code the module was behind `spike-native` and nothing here built.
#[test]
fn native_entry_runs_stub_code_from_rust_and_from_t0() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let r = on_big_stack(move || {
                let mut iso = defs_isolate(config(d, stress));
                install_stubs(&iso);
                let before = vm_state(&iso);
                let mut out = Vec::new();
                let id = iso.global_raw("id").unwrap();
                let g = iso.global_raw("g").unwrap();
                let r = call_through_entry(&mut iso, id, &[Value::int32(41)]).unwrap();
                out.push(format!("{:?}", r.map(|v| v.to_bits())));
                for callee in ["bump", "walk", "id"] {
                    let c = iso.global_raw(callee).unwrap();
                    let r = call_through_entry(&mut iso, g, &[c]).unwrap().unwrap();
                    let s = iso.with_cx(|cx| {
                        Ok(cx.display(Local::<crate::JsValue>::from_bits(r.to_bits())))
                    });
                    out.push(s.unwrap());
                    assert_eq!(vm_state(&iso), before, "{callee}");
                }
                // A function with no native code is not entered (the caller takes T0).
                let t = iso.global_raw("t").unwrap();
                assert!(call_through_entry(&mut iso, t, &[]).is_none());
                // A T0 call of the native g: entered (then g calls back into T0 and into the
                // native id) by `slow::call`, in every build (M7-P5 A2-RW, MC-w30-2).
                crate::tests::take_output();
                for callee in ["bump", "id"] {
                    let r = iso.with_cx_checkpoint(|cx| {
                        cx.run_script(calls_unit(callee), "calls.js")
                            .map(|v| cx.display(v))
                    });
                    let printed = crate::tests::take_output();
                    out.push(match r {
                        Ok(v) => format!("Ok({v:?}) {printed:?}"),
                        Err(e) => e.message,
                    });
                }
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                out
            });
            // T0's cold call path (`slow::call`) enters native code in every build: with
            // `spike-native` at its early entry, and in every build late in `do_call`
            // (`call_native_cold`; M7-P5 A2-RW, MC-w30-2). Before M7-P5 the default build
            // refused the call with "not implemented". Both paths give the same result.
            let t0 = |out: &str| format!("Ok(\"done\") [\"{out}\"]");
            assert_eq!(
                r,
                [
                    format!("Ok({})", Value::int32(41).to_bits()),
                    "a!".to_string(),
                    "42".to_string(),
                    "undefined".to_string(),
                    t0("a!"),
                    t0("undefined"),
                ],
                "{label}"
            );
        }
    }
}

/// An exception crosses the native frame: the callee throws, `call_any` returns 0, the stub
/// returns 0, and the entry gives `Err` with the VM state put back. Error.stack names the
/// native frame of `g` at its call (H14: native frames are canonical VM frames).
#[test]
fn native_entry_runs_an_exception_through_a_native_frame() {
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let label = format!("{} stress={stress}", d.name());
            let (js, host, stack) = on_big_stack(move || {
                let mut iso = defs_isolate(config(d, stress));
                install_stubs(&iso);
                let before = vm_state(&iso);
                let g = iso.global_raw("g").unwrap();
                let t = iso.global_raw("t").unwrap();
                let fail = iso.global_raw("fail").unwrap();
                let e1 = call_through_entry(&mut iso, g, &[t]).unwrap().unwrap_err();
                assert_eq!(vm_state(&iso), before);
                let e2 = call_through_entry(&mut iso, g, &[fail])
                    .unwrap()
                    .unwrap_err();
                assert_eq!(vm_state(&iso), before);
                assert_eq!(iso.rt.pending(), 0);
                // The isolate still runs after the throws.
                let id = iso.global_raw("id").unwrap();
                let ok = call_through_entry(&mut iso, id, &[Value::int32(7)]).unwrap();
                assert_eq!(ok.map(|v| v.to_bits()), Ok(Value::int32(7).to_bits()));
                iso.collect_garbage();
                iso.verify_heap().unwrap();
                (e1.message, e2.message, e1.stack.unwrap_or_default())
            });
            assert_eq!(js, "Error: boom", "{label}");
            assert_eq!(host, "TypeError: native entry probe", "{label}");
            let col = |needle: &str| {
                let p = pos(SRC, needle) as usize;
                let line_start = SRC[..p].rfind('\n').map_or(0, |i| i + 1);
                let line = SRC[..p].matches('\n').count() + 1;
                format!("defs.js:{line}:{}", p - line_start + 1)
            };
            assert_eq!(
                stack,
                format!(
                    "Error: boom\n    at t ({})\n    at g ({})",
                    col("new Error"),
                    col("f()")
                ),
                "{label}"
            );
        }
    }
}

// ---- backtraces through a native frame ---------------------------------------------------------

/// The frames of a `std::backtrace` text: (frame number, symbol name) per symbol line. Each
/// symbol line has its own number, also the line of an inlined function (the `Display` of
/// `Backtrace` gives each symbol a new number).
fn frames(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .filter_map(|l| {
            let (n, name) = l.trim_start().split_once(": ")?;
            Some((n.parse().ok()?, name))
        })
        .collect()
}

/// The bare function name of a symbol name: no generic part, no module path. The debug
/// information sets the form. Full debug information (the dev profile) gives
/// `ep9_vm::native::enter_native::<..>`. The release profile (`debug = "line-tables-only"`)
/// gives the DWARF name only: `enter_native<..>`, `rt_call_any`. Both give `enter_native`.
fn fn_name(sym: &str) -> &str {
    let s = sym.split('<').next().unwrap_or(sym).trim_end_matches(':');
    s.rsplit("::").next().unwrap_or(s)
}

/// The walk of a `std::backtrace` text through the stub frame, or why it is not there: the
/// table function `rt_call_any`, then exactly one frame (the stub; its symbol name is the
/// linker's choice), then the entry (`enter_native`, or `enter_from_rust` where it is
/// inlined), and later `call_through_entry`. The names are compared exactly.
fn stub_walk(text: &str) -> Result<(), String> {
    let fr = frames(text);
    let find = |from: usize, names: &[&str]| {
        fr.iter()
            .skip(from)
            .position(|&(_, n)| names.contains(&fn_name(n)))
            .map(|i| i + from)
    };
    let table =
        find(0, &["rt_call_any"]).ok_or_else(|| format!("no rt_call_any frame:\n{text}"))?;
    let entry = find(table + 1, &["enter_native", "enter_from_rust"])
        .ok_or_else(|| format!("the walk stops in the stub frame:\n{text}"))?;
    if fr[entry].0 != fr[table].0 + 2 {
        return Err(format!(
            "one native frame between the table function and the entry:\n{text}"
        ));
    }
    find(entry, &["call_through_entry"])
        .ok_or_else(|| format!("the walk misses the Rust caller of the entry:\n{text}"))?;
    Ok(())
}

/// The system unwinder walks from `walk` (called by the stub through `call_any`) through the
/// stub frame into the Rust caller of the entry. `std::backtrace` (every build): the frame
/// right after the table function `rt_call_any` is the stub (one frame; its symbol name is
/// the linker's choice), and the next one is the entry (`enter_native`, or `enter_from_rust`
/// where it is inlined), then `call_through_entry`. With `jit`, `ep9_platform::unwind` sees
/// the stub's exact return address and goes on past it. The stub's unwind information is in
/// this binary's text (the `.cfi_*` or `.seh_*` directives).
#[test]
fn native_entry_backtrace_passes_through_a_native_frame() {
    let mut iso = defs_isolate(IsolateConfig::default());
    let (_, g_entry) = install_stubs(&iso);
    let g = iso.global_raw("g").unwrap();
    let walk = iso.global_raw("walk").unwrap();
    WALKED.with(|w| w.borrow_mut().take());
    let r = call_through_entry(&mut iso, g, &[walk]).unwrap().unwrap();
    assert_eq!(r.to_bits(), Value::int32(42).to_bits());
    let (text, ips) = WALKED.with(|w| w.borrow_mut().take()).expect("walk ran");
    if let Err(e) = stub_walk(&text) {
        panic!("{e}");
    }
    let ret = g_entry.start + call0_bytes().1;
    if cfg!(feature = "jit") {
        let at = ips.iter().position(|&ip| ip == ret).unwrap_or_else(|| {
            panic!("the walk misses the stub frame (return address {ret:#x}): {ips:x?}")
        });
        assert!(
            ips.len() - at >= 4,
            "the walk stops at the stub frame: {ips:x?}"
        );
    }
    // The registry entry holds the return address: a stack walker finds the function.
    let e = iso.rt.registry.borrow().lookup(ret).unwrap();
    assert_eq!(
        (e, e.function),
        (g_entry, core::ptr::from_ref(info(&iso, "g")).addr())
    );
}

/// A part of the walk of a release build (macOS arm64, `debug = "line-tables-only"`).
const WALK_RELEASE: &str = "   4: walk
             at ./src/tests/native_entry.rs:271:16
   8: call_at
             at ./src/native.rs:647:5
   9: rt_call_any
             at ./src/native.rs:366:13
  10: _ep9_a2entry_stub_call0
  11: enter_native<ep9_vm::native::entry_tests::call_through_entry::{closure#0}::{closure_env#0}>
             at ./src/native.rs:261:22
  12: enter_from_rust<ep9_vm::native::entry_tests::call_through_entry::{closure#0}::{closure_env#0}>
             at ./src/native.rs:891:22
  14: {closure#0}
             at ./src/tests/native_entry.rs:234:23
  15: with_cx<u64, ep9_vm::native::entry_tests::call_through_entry::{closure_env#0}>
             at ./src/isolate.rs:403:13
  16: call_through_entry
             at ./src/tests/native_entry.rs:231:17
";

/// A part of the walk of a debug build (macOS arm64, full debug information).
const WALK_DEBUG: &str = "   3: ep9_vm::native::entry_tests::walk
             at ./src/tests/native_entry.rs:271:16
   7: ep9_vm::native::call_at
             at ./src/native.rs:647:5
   8: ep9_vm::native::rt_call_any
             at ./src/native.rs:366:13
   9: .Lfunc_end___RNvNtNtCsKkFYnt3Gb3_6ep9_vm6native11entry_tests7stub_a0
  10: ep9_vm::native::enter_native::<ep9_vm::native::entry_tests::call_through_entry::{closure#0}::{closure#0}>
             at ./src/native.rs:261:22
  11: ep9_vm::native::enter_from_rust::<ep9_vm::native::entry_tests::call_through_entry::{closure#0}::{closure#0}>
             at ./src/native.rs:891:22
  13: ep9_vm::native::entry_tests::call_through_entry::{closure#0}
             at ./src/tests/native_entry.rs:234:23
  14: <ep9_vm::isolate::Isolate>::with_cx::<u64, ep9_vm::native::entry_tests::call_through_entry::{closure#0}>
             at ./src/isolate.rs:403:13
  15: ep9_vm::native::entry_tests::call_through_entry
             at ./src/tests/native_entry.rs:231:17
";

/// `text` without the line of the stub frame (the line after `rt_call_any`), and each later
/// frame one number lower: the walk of an unwinder that does not see the stub frame.
fn without_stub(text: &str) -> String {
    let mut out = String::new();
    let mut state = 0; // 0: before rt_call_any, 1: the stub line is next, 2: after the stub line
    for l in text.lines() {
        let frame = l
            .trim_start()
            .split_once(": ")
            .and_then(|(n, name)| Some((n.parse::<usize>().ok()?, name)));
        match (state, frame) {
            (0, Some((_, name))) if fn_name(name) == "rt_call_any" => state = 1,
            (1, Some(_)) => {
                state = 2;
                continue;
            }
            (2, Some((n, name))) => {
                out += &format!("{:>4}: {name}\n", n - 1);
                continue;
            }
            _ => {}
        }
        out += l;
        out.push('\n');
    }
    assert_eq!(state, 2, "the sample has a stub line");
    out
}

/// The probe of `native_entry_backtrace_passes_through_a_native_frame` in both name forms:
/// it finds the walk through the stub frame, and it fails when the stub frame is not there.
#[test]
fn native_entry_backtrace_probe_needs_the_stub_frame() {
    for (sym, name) in [
        ("rt_call_any", "rt_call_any"),
        ("ep9_vm::native::rt_call_any", "rt_call_any"),
        ("enter_native<ep9_vm::X::{closure_env#0}>", "enter_native"),
        (
            "ep9_vm::native::enter_native::<ep9_vm::X::{closure#0}>",
            "enter_native",
        ),
        (
            "ep9_vm::native::entry_tests::call_through_entry",
            "call_through_entry",
        ),
        (
            "ep9_vm::native::entry_tests::call_through_entry::{closure#0}",
            "{closure#0}",
        ),
        ("<ep9_vm::isolate::Isolate>::with_cx::<u64>", ""),
        ("_ep9_a2entry_stub_call0", "_ep9_a2entry_stub_call0"),
    ] {
        assert_eq!(fn_name(sym), name, "{sym}");
    }
    for text in [WALK_RELEASE, WALK_DEBUG] {
        assert_eq!(stub_walk(text), Ok(()));
        let no_stub = without_stub(text);
        let e = stub_walk(&no_stub).unwrap_err();
        assert!(
            e.starts_with("one native frame between the table function and the entry"),
            "{e}"
        );
        // The names are compared exactly: a name that only contains `rt_call_any` is another
        // function.
        let other = text.replace("rt_call_any", "rt_call_any_slow");
        assert!(
            stub_walk(&other)
                .unwrap_err()
                .starts_with("no rt_call_any frame")
        );
    }
}

// ---- jit: the stub copied into the code range, its unwind information registered -------------

/// The run-time unwind records of the frame stub at `code` (the same rules as its
/// directives): an `.eh_frame` FDE (unix) or a RUNTIME_FUNCTION with its UNWIND_INFO (Windows),
/// registered with the system unwinder. Drop it before the code goes.
#[cfg(feature = "jit")]
mod registered {
    use super::*;
    use ep9_platform::jit::{CodeMemory, WxMode};
    use ep9_platform::{CodeRange, page_size, unwind};

    /// The unwind information at this offset of the page (Windows only).
    #[cfg(windows)]
    const UNWIND_INFO_OFF: usize = 256;

    /// The CFI of the frame stub after the CIE rules (each prologue and epilogue step).
    #[cfg(unix)]
    fn call0_cfi(len: usize, ret: usize) -> unwind::cfi::CfiProgram {
        use unwind::cfi::reg;
        let mut p = unwind::cfi::CfiProgram::new();
        #[cfg(target_arch = "aarch64")]
        {
            use reg::aarch64::*;
            // stp (1 insn): CFA = sp + 16, x29 at CFA-16, x30 at CFA-8; mov x29, sp: CFA = x29
            // + 16; after the ldp (insn 8): CFA = sp, both restored.
            let _ = (len, ret);
            p.advance_loc(1)
                .def_cfa_offset(16)
                .offset(FP, 2)
                .offset(LR, 1)
                .advance_loc(1)
                .def_cfa_register(FP)
                .advance_loc(6)
                .def_cfa(SP, 0)
                .restore(FP)
                .restore(LR);
        }
        #[cfg(target_arch = "x86_64")]
        {
            use reg::x86_64::*;
            // push rbp (1 byte): CFA = rsp + 16, rbp at CFA-16; mov rbp, rsp (3 bytes): CFA =
            // rbp + 16; after pop rbp (at `ret`): CFA = rsp + 8.
            let pop = ret;
            p.advance_loc(1)
                .def_cfa_offset(16)
                .offset(RBP, 2)
                .advance_loc(3)
                .def_cfa_register(RBP)
                .advance_loc((pop + 1 - 4) as u32)
                .def_cfa(RSP, 8)
                .restore(RBP);
            assert_eq!(pop + 2, len);
        }
        p
    }

    /// Copies `bytes` to offset `off` of `mem` through a write window.
    fn put(mem: &mut CodeMemory<'_>, off: usize, bytes: &[u8]) {
        let mut w = mem
            .open_write(off, bytes.len())
            .expect("open a write window");
        w.bytes().copy_from_slice(bytes);
        w.close().expect("close the write window");
    }

    /// Writes the frame stub at offset 0 of one page of the process code range, registers its
    /// unwind information, and runs `f` with the code range. The page and the registration go
    /// after `f`, so the isolate that runs the code must drop inside `f`.
    fn with_registered<R>(f: impl FnOnce(core::ops::Range<usize>) -> R) -> R {
        let (code, ret) = call0_bytes();
        let range = CodeRange::process().expect("reserve the process code range");
        let mut chunk = range.alloc(page_size()).expect("a chunk of the code range");
        let mut mem = chunk.commit(WxMode::DEFAULT).expect("commit one code page");
        put(&mut mem, 0, &code);
        let start = mem.exec_addr(0);
        let text = start..start + code.len();
        #[cfg(unix)]
        let reg = {
            let mut b = unwind::cfi::EhFrameBuilder::new(&unwind::cfi::Cie::host());
            b.add_fde(start, code.len(), &call0_cfi(code.len(), ret));
            // SAFETY: the FDE describes the code at `start` exactly; the registration drops
            // before the page.
            unsafe { unwind::EhFrameRegistration::register(b.finish()) }.expect("register the FDE")
        };
        #[cfg(windows)]
        let reg = {
            use unwind::win64::{RuntimeFunction, UnwindInfoBuilder, reg};
            let _ = ret;
            let info = UnwindInfoBuilder::new().push_nonvol(1, reg::RBP).encode();
            put(&mut mem, UNWIND_INFO_OFF, &info);
            let entry = RuntimeFunction {
                begin: 0,
                end: code.len() as u32,
                unwind_info: UNWIND_INFO_OFF as u32,
            };
            let all = mem.exec_base()..mem.exec_base() + mem.len();
            // SAFETY: the UNWIND_INFO describes the prologue of the code at offset 0 exactly;
            // the registration drops before the page.
            unsafe { unwind::FunctionTableRegistration::register(all, Box::new([entry])) }
                .expect("register the function table")
        };
        let r = f(text);
        drop(reg);
        drop(mem);
        r
    }

    /// The A2 shape (`jit` builds only, where this binary can map code): the stub's bytes
    /// in the code range, its unwind information registered at run time, installed as
    /// AotBaseline code, entered from Rust, and walked from the callee through its frame
    /// into the Rust caller (`ep9_platform::unwind`, the same unwinder as SP-JIT-mem).
    #[test]
    fn native_entry_registered_code_runs_and_unwinds() {
        with_registered(|text| {
            let mut iso = defs_isolate(IsolateConfig::default());
            let kind = DispatchKind::AotBaseline;
            // SAFETY: `text` holds the frame stub (it runs the bytecode of `g`) until
            // `with_registered` returns, after the isolate drops.
            let e = unsafe { install_code(&iso.rt, info(&iso, "g"), kind, text.clone()) }.unwrap();
            let g = iso.global_raw("g").unwrap();
            let walk = iso.global_raw("walk").unwrap();
            WALKED.with(|w| w.borrow_mut().take());
            let r = call_through_entry(&mut iso, g, &[walk]).unwrap().unwrap();
            assert_eq!(r.to_bits(), Value::int32(42).to_bits());
            let (_, ips) = WALKED.with(|w| w.borrow_mut().take()).expect("walk ran");
            let ret = text.start + call0_bytes().1;
            let at = ips.iter().position(|&ip| ip == ret).unwrap_or_else(|| {
                panic!("the walk misses the registered frame (return address {ret:#x}): {ips:x?}")
            });
            assert!(at >= 1, "the callee frames come first: {ips:x?}");
            assert!(
                ips.len() - at >= 4,
                "the walk stops at the registered frame: {ips:x?}"
            );
            // Past the native frame the walk is in Rust text again, not in the code range.
            let next = ips[at + 1];
            assert!(
                !CodeRange::process().unwrap().contains(next),
                "{next:#x}: {ips:x?}"
            );
            assert_eq!(iso.rt.registry.borrow().lookup(ret), Some(e));
            // A throw through the registered frame.
            let t = iso.global_raw("t").unwrap();
            let err = call_through_entry(&mut iso, g, &[t]).unwrap().unwrap_err();
            assert_eq!(err.message, "Error: boom");
            assert_eq!(uninstall_code(&iso.rt, info(&iso, "g")), Ok(e));
        });
    }
}
