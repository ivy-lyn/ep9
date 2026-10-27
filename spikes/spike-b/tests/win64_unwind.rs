//! Win64 unwind data of the placement (a) x86-64 JIT (P4-WIN-STATIC-1; H28). The input is
//! what `jit::x64` emits for the protocol kernels `drive` and `id`, plus the hand-written
//! probe of gate check (g) (`jit::hold_probe`, a second prologue shape).
//!
//! | Build | What runs | Who runs it |
//! |---|---|---|
//! | Windows (`cfg(windows)`) | [`live`]: the code placed with `JitModule::new` (`RtlAddGrowableFunctionTable`); per function, `RtlLookupFunctionEntry` finds its RUNTIME_FUNCTION and `RtlVirtualUnwind` from every prologue, body and epilogue state returns to its caller with the saved registers; a walk from a host function under running JIT code steps through each JIT frame to its caller | `cargo test` in `gate.sh structural` and the named CI step on windows-x64 |
//! | `--cfg win64_static` (any host) | [`static_dump`]: writes the x86-64 code and its UNWIND_INFO into a COFF object for `llvm-readobj --unwind` | `spikes/spike-b/win-static.sh` (`cargo rustc -- --cfg win64_static`) |
//! | any other build | nothing: the file compiles to an empty test crate | `gate.sh structural` on unix |
//!
//! The expected unwind results come from the machine code (the prologue pushes and the
//! `lea rbp` decoded from the bytes), not from the UNWIND_INFO, so a wrong UNWIND_INFO fails.
//! Two hand-made breaks of the UNWIND_INFO (pushes swapped, wrong frame offset) must fail.

// `win64_static` is set only by win-static.sh; the root Cargo.toml tables are frozen, so the
// cfg name is not declared there.
#![allow(unexpected_cfgs)]
#![cfg(any(windows, win64_static))]

use spike_b::emit::{Arch, Emitter, FunctionCode};
use spike_b::harness::{Config, Session};
use spike_b::jit::{SpikeEmitter, hold_probe};
use spike_b::structural::LOOP_DEFS;

/// The function of gate check (g) (structural.rs `HOLD_DEFS`).
const HOLD_DEFS: &str = "function hold(mk, scrub, gc) { let o = mk(); scrub(); gc(); return o; }\n";

/// The x86-64 code of `drive` and `id` (the emitter) and `hold` (the check-(g) probe), made on
/// this host: both generators build on every host (jit/mod.rs `SpikeEmitter::new`).
fn x64_functions() -> Vec<FunctionCode> {
    let mut s = Session::new(&Config::default()).expect("session");
    s.run(LOOP_DEFS, "defs.js").result.expect("defs");
    s.run(HOLD_DEFS, "hold.js").result.expect("hold defs");
    let mut e = SpikeEmitter::new(Arch::X86_64);
    let mut out = Vec::new();
    for name in ["drive", "id"] {
        let input = s.function_input(name).expect(name);
        out.push(e.emit(&input).unwrap_or_else(|r| panic!("{name}: {r}")));
    }
    let input = s.function_input("hold").expect("hold");
    out.push(hold_probe(Arch::X86_64, &input, 0).expect("hold probe"));
    for f in &out {
        assert!(f.unwind.win64.is_some(), "{}: no UNWIND_INFO", f.name);
    }
    out
}

/// The frame a function's machine code builds, decoded from its bytes.
///
/// ```text
/// prologue: push r_0 ; ... ; push r_{n-1} ; lea rbp, [rsp + disp]
/// epilogue: lea rsp, [rbp - disp] ; pop r_{n-1} ; ... ; pop r_0 ; ret
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
struct Frame {
    /// The pushed registers (Win64 numbers), in push order, with the offset after each push.
    pushes: Vec<(u32, u8)>,
    /// The `lea rbp, [rsp + disp]` displacement, and the offset after it (= the prologue size).
    fp_disp: u8,
    prolog_end: u32,
    /// The offset of the epilogue's `lea rsp`, then the offset of each pop and of `ret`.
    epilogue: Vec<u32>,
}

const RBP: u8 = 5;

/// Decodes `push r64` at `at`: `50+r` or `41 50+r`.
fn push_at(code: &[u8], at: usize) -> Option<(usize, u8)> {
    match code.get(at..)? {
        [0x41, b @ 0x50..=0x57, ..] => Some((2, b - 0x50 + 8)),
        [b @ 0x50..=0x57, ..] => Some((1, b - 0x50)),
        _ => None,
    }
}

/// Decodes `pop r64` at `at`: `58+r` or `41 58+r`.
fn pop_at(code: &[u8], at: usize) -> Option<(usize, u8)> {
    match code.get(at..)? {
        [0x41, b @ 0x58..=0x5f, ..] => Some((2, b - 0x58 + 8)),
        [b @ 0x58..=0x5f, ..] => Some((1, b - 0x58)),
        _ => None,
    }
}

impl Frame {
    /// Decodes the frame of `code`. Panics if the code does not have the shape above (an
    /// emitter change then needs this test changed with it).
    fn decode(name: &str, code: &[u8]) -> Frame {
        let mut at = 0usize;
        let mut pushes = Vec::new();
        while let Some((len, r)) = push_at(code, at) {
            at += len;
            pushes.push((at as u32, r));
        }
        // lea rbp, [rsp + disp8]: REX.W 8D, ModRM 01 101 100, SIB 00 100 100.
        let [0x48, 0x8d, 0x6c, 0x24, disp] = code[at..at + 5] else {
            panic!(
                "{name}: no `lea rbp, [rsp + disp8]` after the pushes: {:02x?}",
                &code[..at + 5]
            );
        };
        let prolog_end = (at + 5) as u32;
        assert!(
            !pushes.is_empty() && pushes[0].1 == RBP,
            "{name}: {pushes:?}"
        );
        // lea rsp, [rbp + disp8]: REX.W 8D, ModRM 01 100 101; then the pops in reverse order.
        let lea = [0x48, 0x8d, 0x65, disp.wrapping_neg()];
        let mut found = Vec::new();
        for start in (0..code.len().saturating_sub(4)).filter(|&i| code[i..i + 4] == lea) {
            let mut offs = vec![start as u32];
            let mut p = start + 4;
            let mut ok = true;
            for &(_, r) in pushes.iter().rev() {
                offs.push(p as u32);
                match pop_at(code, p) {
                    Some((len, got)) if got == r => p += len,
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && code.get(p) == Some(&0xc3) {
                offs.push(p as u32);
                found.push(offs);
            }
        }
        assert_eq!(found.len(), 1, "{name}: epilogues found: {found:?}");
        Frame {
            pushes,
            fp_disp: disp,
            prolog_end,
            epilogue: found.pop().expect("one"),
        }
    }
}

/// Writes the code and the UNWIND_INFO of `functions` into a COFF x86-64 object: `.text`
/// (each function 16-aligned, as jit/mem.rs places them), `.xdata` (each UNWIND_INFO
/// 4-aligned) and `.pdata` (one RUNTIME_FUNCTION each, with ADDR32NB relocations against a
/// symbol per function and against `.xdata`).
#[cfg(win64_static)]
fn coff_object(functions: &[FunctionCode]) -> Vec<u8> {
    const IMAGE_REL_AMD64_ADDR32NB: u16 = 3;
    fn align(v: usize, a: usize) -> usize {
        v.div_ceil(a) * a
    }
    let mut text = Vec::new();
    let mut xdata = Vec::new();
    // (code offset in .text, UNWIND_INFO offset in .xdata) of each function.
    let mut at = Vec::new();
    for f in functions {
        text.resize(align(text.len(), 16), 0xcc);
        let info = f.unwind.win64.as_ref().expect("UNWIND_INFO");
        xdata.resize(align(xdata.len(), 4), 0);
        at.push((text.len() as u32, xdata.len() as u32));
        text.extend_from_slice(&f.code);
        xdata.extend_from_slice(info);
    }
    // Symbols: 0 .text, 1 .xdata, 2 .pdata, then one per function.
    let mut pdata = Vec::new();
    let mut relocs = Vec::new();
    for (i, (f, &(_, info))) in functions.iter().zip(&at).enumerate() {
        let sym = 3 + i as u32;
        let base = pdata.len() as u32;
        // In-place addends: begin = sym + 0, end = sym + len, unwind = .xdata + offset.
        pdata.extend_from_slice(&0u32.to_le_bytes());
        pdata.extend_from_slice(&(f.code.len() as u32).to_le_bytes());
        pdata.extend_from_slice(&info.to_le_bytes());
        for (off, s) in [(0, sym), (4, sym), (8, 1)] {
            relocs.extend_from_slice(&(base + off).to_le_bytes());
            relocs.extend_from_slice(&s.to_le_bytes());
            relocs.extend_from_slice(&IMAGE_REL_AMD64_ADDR32NB.to_le_bytes());
        }
    }
    let nreloc = (relocs.len() / 10) as u16;
    let header = 20 + 3 * 40;
    let text_at = header;
    let xdata_at = text_at + text.len();
    let pdata_at = xdata_at + xdata.len();
    let reloc_at = pdata_at + pdata.len();
    let sym_at = reloc_at + relocs.len();
    let nsyms = 3 + functions.len();

    let mut o = Vec::new();
    o.extend_from_slice(&0x8664u16.to_le_bytes()); // IMAGE_FILE_MACHINE_AMD64
    o.extend_from_slice(&3u16.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(sym_at as u32).to_le_bytes());
    o.extend_from_slice(&(nsyms as u32).to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    let section =
        |o: &mut Vec<u8>, name: &[u8], size: usize, at: usize, rel: (usize, u16), ch: u32| {
            let mut n = [0u8; 8];
            n[..name.len()].copy_from_slice(name);
            o.extend_from_slice(&n);
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&(size as u32).to_le_bytes());
            o.extend_from_slice(&(at as u32).to_le_bytes());
            o.extend_from_slice(&(rel.0 as u32).to_le_bytes());
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&rel.1.to_le_bytes());
            o.extend_from_slice(&0u16.to_le_bytes());
            o.extend_from_slice(&ch.to_le_bytes());
        };
    // CNT_CODE | ALIGN_16BYTES | MEM_EXECUTE | MEM_READ; INITIALIZED_DATA | ALIGN_4BYTES | MEM_READ.
    section(&mut o, b".text", text.len(), text_at, (0, 0), 0x6050_0020);
    section(
        &mut o,
        b".xdata",
        xdata.len(),
        xdata_at,
        (0, 0),
        0x4030_0040,
    );
    section(
        &mut o,
        b".pdata",
        pdata.len(),
        pdata_at,
        (reloc_at, nreloc),
        0x4030_0040,
    );
    o.extend_from_slice(&text);
    o.extend_from_slice(&xdata);
    o.extend_from_slice(&pdata);
    o.extend_from_slice(&relocs);
    let symbol = |o: &mut Vec<u8>, name: &str, value: u32, section: i16, ty: u16, class: u8| {
        assert!(name.len() <= 8, "short names only: {name}");
        let mut n = [0u8; 8];
        n[..name.len()].copy_from_slice(name.as_bytes());
        o.extend_from_slice(&n);
        o.extend_from_slice(&value.to_le_bytes());
        o.extend_from_slice(&section.to_le_bytes());
        o.extend_from_slice(&ty.to_le_bytes());
        o.push(class);
        o.push(0);
    };
    const STATIC: u8 = 3;
    const EXTERNAL: u8 = 2;
    const FUNCTION: u16 = 0x20;
    symbol(&mut o, ".text", 0, 1, 0, STATIC);
    symbol(&mut o, ".xdata", 0, 2, 0, STATIC);
    symbol(&mut o, ".pdata", 0, 3, 0, STATIC);
    for (f, &(start, _)) in functions.iter().zip(&at) {
        symbol(&mut o, &f.name, start, 1, FUNCTION, EXTERNAL);
    }
    o.extend_from_slice(&4u32.to_le_bytes()); // empty string table
    o
}

/// `win-static.sh` step 2: writes `win64-jit.obj` and `functions.txt` into
/// `$EP9_WIN64_STATIC_OUT`. One line per function: name, code length, prologue size, the
/// pushed registers (Win64 numbers, push order), the `lea rbp` displacement, the epilogue
/// offsets (lea, each pop, ret), the prologue bytes, the epilogue bytes (lea to ret) and the
/// UNWIND_INFO bytes.
#[cfg(win64_static)]
mod static_dump {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn write_the_coff_object() {
        let out = std::env::var_os("EP9_WIN64_STATIC_OUT")
            .expect("EP9_WIN64_STATIC_OUT: the output directory (set by win-static.sh)");
        let out = std::path::PathBuf::from(out);
        let functions = x64_functions();
        let mut lines = String::new();
        for f in &functions {
            let frame = Frame::decode(&f.name, &f.code);
            let info = f.unwind.win64.as_ref().expect("UNWIND_INFO");
            let epi = &frame.epilogue;
            let list = |v: &mut dyn Iterator<Item = u32>| {
                v.map(|x| x.to_string()).collect::<Vec<_>>().join(",")
            };
            lines.push_str(&format!(
                "{} {} {} {} {} {} {} {} {}\n",
                f.name,
                f.code.len(),
                frame.prolog_end,
                list(&mut frame.pushes.iter().map(|p| u32::from(p.1))),
                frame.fp_disp,
                list(&mut frame.epilogue.iter().copied()),
                hex(&f.code[..frame.prolog_end as usize]),
                hex(&f.code[epi[0] as usize..=*epi.last().expect("ret") as usize]),
                hex(info)
            ));
        }
        std::fs::write(out.join("functions.txt"), lines).expect("functions.txt");
        std::fs::write(out.join("win64-jit.obj"), coff_object(&functions)).expect("object");
    }
}

/// The real run on Windows x64 (D25).
#[cfg(windows)]
mod live {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::ops::Range;

    use ep9_vm::{Args, Cx, JsResult, Local};
    use spike_b::jit::JitModule;

    use super::*;

    /// `IMAGE_RUNTIME_FUNCTION_ENTRY` (x64).
    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct RuntimeFunction {
        begin: u32,
        end: u32,
        unwind_info: u32,
    }

    /// The x64 `CONTEXT` (winnt.h): 1232 bytes, 16-aligned; the integer registers at 0x78 in
    /// Win64 register-number order (rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8-r15), rip at
    /// 0xf8. The rest is not read here.
    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    struct Context {
        head: [u64; 15],
        gp: [u64; 16],
        rip: u64,
        rest: [u8; 0x3d0],
    }

    const _: () = assert!(size_of::<Context>() == 0x4d0);
    const _: () = assert!(std::mem::offset_of!(Context, gp) == 0x78);
    const _: () = assert!(std::mem::offset_of!(Context, rip) == 0xf8);

    impl Context {
        fn zeroed() -> Context {
            let mut c = Context {
                head: [0; 15],
                gp: [0; 16],
                rip: 0,
                rest: [0; 0x3d0],
            };
            // ContextFlags (offset 0x30): CONTEXT_AMD64 | CONTROL | INTEGER | FLOATING_POINT.
            c.head[6] = 0x0010_000b;
            c
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn RtlCaptureContext(context: *mut Context);
        fn RtlLookupFunctionEntry(
            control_pc: u64,
            image_base: *mut u64,
            history_table: *mut c_void,
        ) -> *const RuntimeFunction;
        fn RtlVirtualUnwind(
            handler_type: u32,
            image_base: u64,
            control_pc: u64,
            function_entry: *const RuntimeFunction,
            context: *mut Context,
            handler_data: *mut *mut c_void,
            establisher_frame: *mut u64,
            context_pointers: *mut c_void,
        ) -> *mut c_void;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    const UNW_FLAG_NHANDLER: u32 = 0;
    const RSP: u8 = 4;

    /// `RtlLookupFunctionEntry(pc)`: the image base and a copy of the entry.
    fn lookup(pc: usize) -> Option<(u64, RuntimeFunction, *const RuntimeFunction)> {
        let mut base = 0u64;
        // SAFETY: the lookup only reads the loaded images and the registered function tables;
        // `base` is a valid out pointer; no history table.
        let e = unsafe { RtlLookupFunctionEntry(pc as u64, &mut base, std::ptr::null_mut()) };
        if e.is_null() {
            None
        } else {
            // SAFETY: a non-null result points at a live RUNTIME_FUNCTION of a registered table
            // or a loaded image (it stays valid while that table or image is).
            Some((base, unsafe { *e }, e))
        }
    }

    /// `RtlVirtualUnwind` of one frame: the establisher frame; `ctx` becomes the caller's.
    ///
    /// # Safety
    /// `entry` came from [`lookup`] of `ctx.rip`, its table is still registered, and every
    /// stack slot the unwind reads (from `ctx.rsp` / the frame register) is readable.
    unsafe fn unwind(base: u64, entry: *const RuntimeFunction, ctx: &mut Context) -> u64 {
        let mut data: *mut c_void = std::ptr::null_mut();
        let mut establisher = 0u64;
        // SAFETY: the caller's contract; the out pointers are valid locals.
        unsafe {
            RtlVirtualUnwind(
                UNW_FLAG_NHANDLER,
                base,
                ctx.rip,
                entry,
                ctx,
                &mut data,
                &mut establisher,
                std::ptr::null_mut(),
            )
        };
        establisher
    }

    /// The sentinel each callee-saved register holds in the caller.
    fn saved(r: u8) -> u64 {
        0x5a00_0000_0000_0011 | (u64::from(r) << 8)
    }
    /// What a register holds inside the function body (the function's own use of it).
    fn clobbered(r: u8) -> u64 {
        0xc100_0000_0000_0022 | (u64::from(r) << 8)
    }
    const RET: u64 = 0x0000_7ff0_1234_5670;

    /// One machine state of a function at `ip` (an offset): the register file and the stack
    /// the frame model gives for it, as the code would have built it after its caller's call.
    struct State {
        what: String,
        ip: u32,
        /// Pushes done (the prologue) or not yet popped (the epilogue).
        live: usize,
        /// rbp holds the frame pointer.
        fp_set: bool,
    }

    /// Every state to unwind from: offset 0, after each push, after the `lea rbp`, each
    /// body instruction, the epilogue `lea rsp`, before each pop and at `ret`.
    fn states(f: &FunctionCode, fr: &Frame) -> Vec<State> {
        let n = fr.pushes.len();
        let mut v = vec![State {
            what: "entry".into(),
            ip: 0,
            live: 0,
            fp_set: false,
        }];
        for (i, &(end, r)) in fr.pushes.iter().enumerate() {
            v.push(State {
                what: format!("after push {r}"),
                ip: end,
                live: i + 1,
                fp_set: false,
            });
        }
        v.push(State {
            what: "after lea rbp".into(),
            ip: fr.prolog_end,
            live: n,
            fp_set: true,
        });
        for (&(off, op), &native) in f.ops.iter().zip(&f.native_offsets) {
            v.push(State {
                what: format!("body {} (bytecode {off})", op.info().name),
                ip: native,
                live: n,
                fp_set: true,
            });
        }
        // epilogue[0] = lea rsp; epilogue[1 + j] = pop j (j pops done before it); last = ret.
        for (j, &ip) in fr.epilogue.iter().enumerate() {
            let popped = j.saturating_sub(1);
            v.push(State {
                what: if j == 0 {
                    "epilogue lea rsp".into()
                } else if j == fr.epilogue.len() - 1 {
                    "ret".into()
                } else {
                    format!("epilogue pop {}", fr.pushes[n - j].1)
                },
                ip,
                live: n - popped,
                // rbp is pushed first, so it is popped last: it holds the frame pointer
                // until its pop.
                fp_set: fr.pushes[..n - popped].iter().any(|p| p.1 == RBP),
            });
        }
        v
    }

    /// Unwinds `f` (placed at `code`) from every state and checks the caller's registers.
    /// Returns the number of states checked, or the first mismatch.
    fn check_function(f: &FunctionCode, code: &Range<usize>) -> Result<usize, String> {
        let fr = Frame::decode(&f.name, &f.code);
        let n = fr.pushes.len();
        let mut checked = 0;
        for st in states(f, &fr) {
            let pc = code.start + st.ip as usize;
            let Some((base, rf, entry)) = lookup(pc) else {
                return Err(format!(
                    "{} {}: no RUNTIME_FUNCTION for {pc:#x}",
                    f.name, st.what
                ));
            };
            let (b, e) = (
                base as usize + rf.begin as usize,
                base as usize + rf.end as usize,
            );
            if (b..e) != *code {
                return Err(format!(
                    "{} {}: the entry covers {b:#x}..{e:#x}, the code is {code:x?}",
                    f.name, st.what
                ));
            }
            // A fake stack: s0 = rsp at the entry, [s0] = the return address, then the pushes.
            let mut stack = [0u64; 64];
            let s0 = stack.as_ptr() as usize + 32 * 8;
            stack[32] = RET;
            for (i, &(_, r)) in fr.pushes.iter().enumerate() {
                stack[31 - i] = saved(r);
            }
            let mut ctx = Context::zeroed();
            for r in 0..16u8 {
                ctx.gp[r as usize] = saved(r);
            }
            // Registers already pushed and not yet popped hold the body's values.
            let in_body = st.ip >= fr.prolog_end;
            if in_body {
                for &(_, r) in &fr.pushes[..st.live] {
                    ctx.gp[r as usize] = clobbered(r);
                }
            }
            let rsp = s0 - 8 * st.live;
            ctx.gp[RSP as usize] = rsp as u64;
            let frame_rbp = (s0 - 8 * n + fr.fp_disp as usize) as u64;
            if st.fp_set {
                ctx.gp[RBP as usize] = frame_rbp;
            }
            ctx.rip = pc as u64;
            let before = ctx;
            std::hint::black_box(&mut stack);
            // SAFETY: `entry` is the lookup of `ctx.rip`; the table lives while the module
            // does; the unwind reads only `stack` (rsp and rbp point into it).
            let establisher = unsafe { unwind(base, entry, &mut ctx) };
            let mut bad = Vec::new();
            if ctx.rip != RET {
                bad.push(format!("rip {:#x}, want {RET:#x}", ctx.rip));
            }
            if ctx.gp[RSP as usize] != (s0 + 8) as u64 {
                bad.push(format!(
                    "rsp {:#x}, want {:#x}",
                    ctx.gp[RSP as usize],
                    s0 + 8
                ));
            }
            for r in 0..16u8 {
                if r == RSP {
                    continue;
                }
                let pushed = fr.pushes.iter().any(|p| p.1 == r);
                let want = if pushed {
                    saved(r)
                } else {
                    before.gp[r as usize]
                };
                if ctx.gp[r as usize] != want {
                    bad.push(format!("r{r} = {:#x}, want {want:#x}", ctx.gp[r as usize]));
                }
            }
            if in_body && st.ip < fr.epilogue[0] && establisher != (s0 - 8 * n) as u64 {
                bad.push(format!(
                    "establisher frame {establisher:#x}, want {:#x}",
                    s0 - 8 * n
                ));
            }
            if !bad.is_empty() {
                return Err(format!(
                    "{} {} (offset {}): {}",
                    f.name,
                    st.what,
                    st.ip,
                    bad.join("; ")
                ));
            }
            checked += 1;
        }
        Ok(checked)
    }

    /// Places `functions` with `JitModule::new` and checks each one.
    fn check_module(functions: Vec<FunctionCode>) -> Result<Vec<String>, String> {
        let module = JitModule::new(functions.clone()).map_err(|e| e.to_string())?;
        let mut report = Vec::new();
        for (f, p) in functions.iter().zip(module.functions()) {
            assert_eq!(f.name, p.name);
            // The registered UNWIND_INFO is the emitted one.
            let (base, rf, _) = lookup(p.code.start).ok_or(format!("{}: no entry", f.name))?;
            let want = f.unwind.win64.as_ref().expect("UNWIND_INFO");
            let at = base as usize + rf.unwind_info as usize;
            // SAFETY: `at` is the UNWIND_INFO of a registered entry, inside the module's
            // chunk, which is mapped readable while `module` lives.
            let got = unsafe { std::slice::from_raw_parts(at as *const u8, want.len()) };
            if got != &want[..] {
                return Err(format!(
                    "{}: registered UNWIND_INFO {got:02x?}, emitted {want:02x?}",
                    f.name
                ));
            }
            let n = check_function(f, &p.code)?;
            report.push(format!("{}: {} states, {} bytes", f.name, n, f.code.len()));
        }
        // No entry past the last function's end inside the chunk (padding and UNWIND_INFO).
        let last = module.functions().last().expect("functions").code.end;
        if let Some((base, rf, _)) = lookup(last) {
            let b = base as usize + rf.begin as usize;
            if b < last {
                return Err(format!(
                    "{last:#x} (past the code) has an entry from {b:#x}"
                ));
            }
        }
        Ok(report)
    }

    #[test]
    fn every_state_of_every_function_unwinds_to_its_caller() {
        let functions = x64_functions();
        let report = check_module(functions).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.len(), 3, "{report:?}");
        eprintln!("{}", report.join("\n"));
    }

    /// Negative controls: the same code with a broken UNWIND_INFO must fail the check.
    #[test]
    fn a_broken_unwind_info_fails_the_check() {
        let drive = x64_functions().swap_remove(0);
        assert_eq!(drive.name, "drive");
        let info = drive.unwind.win64.clone().expect("UNWIND_INFO");
        // Codes are stored last prolog instruction first: [set_fpreg, push r13, push r15,
        // push rbp]; the register is the high nibble of each code's second byte.
        assert_eq!(info[..4], [1, 10, 4, 0x15], "{info:02x?}");
        let mut swapped = info.clone();
        swapped.swap(7, 9);
        let mut off = info.clone();
        off[3] = 0x25; // frame offset 32, not 16
        for (what, bad) in [("pushes swapped", swapped), ("frame offset", off)] {
            let mut f = drive.clone();
            f.unwind.win64 = Some(bad);
            let r = check_module(vec![f]);
            assert!(r.is_err(), "{what}: the broken UNWIND_INFO passed: {r:?}");
            eprintln!("{what}: {}", r.unwrap_err());
        }
    }

    // ---- the walk from a host function under running JIT code ----------------------------

    /// The frame that started the run: its rsp and the start of its function.
    #[derive(Clone, Copy, Debug, Default)]
    struct Outer {
        rsp: u64,
        begin: usize,
    }

    thread_local! {
        static WALK: RefCell<Result<Vec<String>, String>> = const { RefCell::new(Ok(Vec::new())) };
        static OUTER: RefCell<Outer> = const { RefCell::new(Outer { rsp: 0, begin: 0 }) };
        static JIT: RefCell<Vec<(String, Range<usize>)>> = const { RefCell::new(Vec::new()) };
    }

    /// The start of the function that contains `pc` (from its RUNTIME_FUNCTION).
    fn function_start(pc: usize) -> Option<usize> {
        lookup(pc).map(|(base, rf, _)| base as usize + rf.begin as usize)
    }

    /// Walks from here to the frame that started the run ([`OUTER`]); at each JIT frame,
    /// checks its entry and the unwind against the frame record in memory.
    #[inline(never)]
    fn walk_from_here() -> Result<Vec<String>, String> {
        let outer = OUTER.with(|o| *o.borrow());
        let jit = JIT.with(|j| j.borrow().clone());
        // SAFETY: GetModuleHandleW(null) returns the module of this process's exe; no input.
        let exe = unsafe { GetModuleHandleW(std::ptr::null()) } as u64;
        let sp = ep9_platform::stack_pointer();
        let mut ctx = Context::zeroed();
        // SAFETY: `ctx` is a 16-aligned CONTEXT that RtlCaptureContext fills for this thread.
        unsafe { RtlCaptureContext(&mut ctx) };
        // The CONTEXT layout: rsp near this frame's sp, rip in this function of the exe.
        let me = function_start(walk_from_here as fn() -> Result<Vec<String>, String> as usize);
        if ctx.gp[RSP as usize].abs_diff(sp as u64) > 4096
            || lookup(ctx.rip as usize).map(|l| l.0) != Some(exe)
        {
            return Err(format!(
                "CONTEXT layout: rsp {:#x} (sp {sp:#x}), rip {:#x}, start {me:x?}",
                ctx.gp[RSP as usize], ctx.rip
            ));
        }
        let mut seen = Vec::new();
        for _ in 0..512 {
            let ip = ctx.rip as usize;
            if ip == 0 {
                return Err(format!(
                    "the walk ended at ip 0 before the outer frame: {seen:?}"
                ));
            }
            if ctx.gp[RSP as usize] >= outer.rsp {
                // The first frame at or above the outer rsp must be the outer frame itself.
                let start = function_start(ip);
                if ctx.gp[RSP as usize] != outer.rsp || start != Some(outer.begin) {
                    return Err(format!(
                        "the walk reached rsp {:#x} in the function at {start:x?}, not the outer frame {outer:x?}; JIT frames {seen:?}",
                        ctx.gp[RSP as usize]
                    ));
                }
                return Ok(seen);
            }
            let Some((base, rf, entry)) = lookup(ip) else {
                // A leaf: the return address is at rsp.
                let rsp = ctx.gp[RSP as usize];
                // SAFETY: rsp of a live frame of this thread points at its return address.
                ctx.rip = unsafe { *(rsp as *const u64) };
                ctx.gp[RSP as usize] = rsp + 8;
                continue;
            };
            let Some((name, code)) = jit.iter().find(|(_, c)| c.contains(&ip)) else {
                // SAFETY: `entry` is the lookup of ctx.rip; every frame below the outer one is
                // live.
                unsafe { unwind(base, entry, &mut ctx) };
                continue;
            };
            let (b, e) = (
                base as usize + rf.begin as usize,
                base as usize + rf.end as usize,
            );
            if (b..e) != *code {
                return Err(format!(
                    "{name}: the entry for {ip:#x} covers {b:#x}..{e:#x}, not {code:x?}"
                ));
            }
            // In the body, rbp points at the frame record: the saved rbp, then the return
            // address.
            let fp = ctx.gp[RBP as usize] as usize;
            if !(ctx.gp[RSP as usize] as usize..outer.rsp as usize).contains(&fp) {
                return Err(format!("{name}: rbp {fp:#x} is not in the live stack"));
            }
            // SAFETY: fp lies between this frame's rsp and the outer frame (checked above), in
            // the live stack of this thread.
            let (saved_rbp, ret) = unsafe { (*(fp as *const u64), *((fp + 8) as *const u64)) };
            // SAFETY: `entry` is the lookup of ctx.rip; the frame is live.
            unsafe { unwind(base, entry, &mut ctx) };
            let mut bad = Vec::new();
            if ctx.rip != ret {
                bad.push(format!(
                    "rip {:#x}, the frame record says {ret:#x}",
                    ctx.rip
                ));
            }
            if ctx.gp[RSP as usize] != (fp + 16) as u64 {
                bad.push(format!(
                    "rsp {:#x}, want {:#x}",
                    ctx.gp[RSP as usize],
                    fp + 16
                ));
            }
            if ctx.gp[RBP as usize] != saved_rbp {
                bad.push(format!(
                    "rbp {:#x}, saved {saved_rbp:#x}",
                    ctx.gp[RBP as usize]
                ));
            }
            let back = ctx.rip as usize;
            if jit.iter().any(|(_, c)| c.contains(&back)) {
                bad.push(format!(
                    "returns into JIT code at {back:#x}, not its native caller"
                ));
            }
            match lookup(back) {
                Some((b, ..)) if b == exe => {}
                other => bad.push(format!(
                    "the caller {back:#x} is not in the test exe ({exe:#x}): {other:?}"
                )),
            }
            if !bad.is_empty() {
                return Err(format!("{name} at {ip:#x}: {}", bad.join("; ")));
            }
            seen.push(format!("{name} -> {back:#x}"));
        }
        Err(format!("no outer frame after 512 frames: {seen:?}"))
    }

    /// `probe()`: [`walk_from_here`] into `WALK`.
    fn probe<'s>(
        _cx: &mut Cx<'s>,
        _t: Local<'s>,
        _a: Args<'s>,
        _n: Local<'s>,
    ) -> JsResult<Local<'s>> {
        let r = walk_from_here();
        WALK.with(|w| *w.borrow_mut() = r);
        Ok(Local::undefined())
    }

    /// structural.rs `PATHS_DEFS`, first two functions.
    const PATHS_DEFS: &str = "\
function g(f, x) { return f(x); }
function h(f, x) { return f(x); }
";

    #[inline(never)]
    fn run_walk() -> Vec<String> {
        let mut s = Session::new(&Config::default()).expect("session");
        s.iso.register_host_function("probe", 0, probe);
        s.run(PATHS_DEFS, "defs.js").result.expect("defs");
        let placed: Vec<(String, Range<usize>)> = s
            .jit(&["g", "h"])
            .expect("jit")
            .iter()
            .map(|p| (p.name.clone(), p.code.clone()))
            .collect();
        JIT.with(|j| *j.borrow_mut() = placed);
        // This frame: its rsp (constant after the prologue) and its function's start.
        // RtlCaptureContext gives the context of its caller, so it is called from here.
        let mut here = Context::zeroed();
        // SAFETY: `here` is a 16-aligned CONTEXT that RtlCaptureContext fills for this thread.
        unsafe { RtlCaptureContext(&mut here) };
        let outer = Outer {
            rsp: here.gp[RSP as usize],
            begin: function_start(here.rip as usize).expect("an entry for run_walk"),
        };
        OUTER.with(|o| *o.borrow_mut() = outer);
        WALK.with(|w| *w.borrow_mut() = Err("probe did not run".into()));
        let r = s.run("g(h, probe);\n", "main.js");
        assert_eq!(r.result, Ok("undefined".to_string()), "{r:?}");
        WALK.with(|w| std::mem::replace(&mut *w.borrow_mut(), Ok(Vec::new())))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// T0 -> g (JIT) -> h (JIT) -> probe (host): the walk steps through both JIT frames, each
    /// unwind matches the frame record in memory and returns into this exe, and the walk
    /// reaches the frame that started the run.
    #[test]
    fn a_walk_from_running_jit_code_unwinds_each_jit_frame_to_its_caller() {
        // Debug ep9 frames are large; the structural checks use 64 MiB too.
        let seen = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(run_walk)
            .expect("thread")
            .join()
            .expect("walk thread");
        assert_eq!(seen.len(), 2, "JIT frames in the walk: {seen:?}");
        assert!(
            seen[0].starts_with("h ") && seen[1].starts_with("g "),
            "{seen:?}"
        );
        eprintln!("{}", seen.join("\n"));
    }
}
