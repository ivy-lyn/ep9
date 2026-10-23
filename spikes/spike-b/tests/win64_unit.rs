//! Win64 unwind data of SP-Unit x86-64 unit text (M3-P2 WIN-STATIC-2; H28, H35): what
//! `tests/win64_unwind.rs` does for placement (a) and (b) JIT code, for the functions of a unit
//! blob. The input is units A and B (`unit::checks::unit_a`, `unit_b`) built for x86-64 by
//! `unit::build::build`: per unit, its JS functions, the unwind self-test probe and the cell
//! reader, with the UNWIND_INFO of each in the unit's unwind section.
//!
//! | Build | What runs | Who runs it |
//! |---|---|---|
//! | every host | the static checks: the unwind section has one Win64 entry per function of the function table, with its text range; each UNWIND_INFO is well formed and its codes equal the codes the machine code's prologue needs; a model of the Win64 unwinder (prologue codes, epilogue emulation) unwinds every prologue, body and epilogue state to the caller; 2 broken UNWIND_INFOs fail it; `llvm-readobj --unwind` reads the same codes (skipped without llvm-tools) | `cargo test` in `gate.sh structural`; the named CI step on windows-x64 |
//! | Windows (`cfg(windows)`) | [`live`]: units A and B loaded by the spike loader (`unit::load`, which registers the function table, `RtlAddGrowableFunctionTable`); per function, `RtlLookupFunctionEntry` finds its RUNTIME_FUNCTION and the registered UNWIND_INFO is the unit's; `RtlVirtualUnwind` from every state returns to the caller; a unit with a broken UNWIND_INFO fails | the CI step `cargo test --test win64_unit` on windows-x64 |
//!
//! The expected results come from the machine code (the prologue and epilogue instructions,
//! decoded from the text bytes and run on a small register and stack model), not from the
//! UNWIND_INFO, so a wrong UNWIND_INFO fails.
//!
//! ```text
//! JS function  push rbp ; push r15 ; push r13 ; push r14 ; push rbx ; lea rbp, [rsp + 32]
//!              ... lea rsp, [rbp - 32] ; pop rbx ; pop r14 ; pop r13 ; pop r15 ; pop rbp
//!              rex.w jmp qword ptr [rdi + rax*8]
//! probe        push rbp ; mov rbp, rsp ; push rbx ; sub rsp, 8 ; ... add rsp, 8 ; pop rbx ;
//!              pop rbp ; ret
//! cell reader  push r14 ; ... pop r14 ; ret
//! ```

use std::ops::Range;

use spike_b::emit::{Arch, Emitter, FunctionCode};
use spike_b::harness::compile;
use spike_b::unit::build::{UnitSpec, build, function_input};
use spike_b::unit::checks::{unit_a, unit_b};
use spike_b::unit::emit::UnitEmitter;
use spike_b::unit::format::{FnKind, UnitView, rehash, spike_host};

// ---- the units -------------------------------------------------------------------------------

/// One function of an x86-64 unit: its code, its UNWIND_INFO from the unwind section, and the
/// body offsets the states use (each emitted step, for a JS function).
#[derive(Clone, Debug)]
struct UnitFn {
    name: String,
    kind: FnKind,
    /// Its range in the unit's text section.
    text: Range<u32>,
    code: Vec<u8>,
    info: Vec<u8>,
    steps: Vec<u32>,
}

/// The x86-64 bytes of `spec`.
fn x64_unit(spec: &UnitSpec) -> Vec<u8> {
    build(spec, Arch::X86_64).unwrap_or_else(|e| panic!("unit {}: {e}", spec.name))
}

/// The functions of the x86-64 unit `bytes` (built from `spec`), with the checks that tie the
/// unwind section to the function table: one Win64 entry per function, in table order, with
/// the function's text range. The emitter is run again on the same inputs to get each JS
/// function's step offsets; its code and UNWIND_INFO must equal the unit's.
fn unit_functions(spec: &UnitSpec, bytes: &[u8]) -> Vec<UnitFn> {
    let view = UnitView::parse(bytes).unwrap_or_else(|e| panic!("unit {}: {e}", spec.name));
    assert_eq!(
        view.arch,
        ep9_unit::target_arch::X86_64,
        "unit {}",
        spec.name
    );
    let w = &view.unwind.win64;
    assert_eq!(
        w.len(),
        view.functions.len(),
        "unit {}: Win64 entries vs functions",
        spec.name
    );
    let mut prev_end = 0;
    for (e, f) in w.iter().zip(&view.functions) {
        assert_eq!(
            e.text, f.text,
            "unit {} {}: Win64 entry range",
            spec.name, f.name
        );
        assert!(
            f.text.start < f.text.end
                && f.text.start >= prev_end
                && f.text.start.is_multiple_of(16)
                && f.text.end as usize <= view.text.len(),
            "unit {} {}: text {:?} (previous end {prev_end}, text {})",
            spec.name,
            f.name,
            f.text,
            view.text.len()
        );
        prev_end = f.text.end;
    }
    // The emitter's view of the same functions.
    let cu = compile(&spec.source).unwrap_or_else(|e| panic!("unit {}: {e}", spec.name));
    let mut em = UnitEmitter::new(Arch::X86_64, view.rw.layout).expect("emitter");
    let mut codes: Vec<FunctionCode> = spec
        .native
        .iter()
        .map(|n| {
            let input = function_input(&cu, n).expect("input");
            em.emit(&input).unwrap_or_else(|r| panic!("{n}: {r}"))
        })
        .collect();
    codes.push(em.probe().expect("probe"));
    codes.push(em.cell_reader().expect("cell reader"));
    assert_eq!(codes.len(), view.functions.len(), "unit {}", spec.name);
    view.functions
        .iter()
        .zip(w)
        .zip(codes)
        .map(|((f, e), c)| {
            let code = view.text[f.text.start as usize..f.text.end as usize].to_vec();
            assert_eq!(
                code, c.code,
                "unit {} {}: text vs emitter",
                spec.name, f.name
            );
            assert_eq!(
                Some(&e.info),
                c.unwind.win64.as_ref(),
                "unit {} {}: unwind section vs emitter",
                spec.name,
                f.name
            );
            UnitFn {
                name: f.name.clone(),
                kind: f.kind,
                text: f.text.clone(),
                code,
                info: e.info.clone(),
                steps: c.native_offsets,
            }
        })
        .collect()
}

/// Units A and B with their functions.
fn units() -> Vec<(UnitSpec, Vec<u8>, Vec<UnitFn>)> {
    [unit_a(), unit_b()]
        .into_iter()
        .map(|spec| {
            let bytes = x64_unit(&spec);
            let fns = unit_functions(&spec, &bytes);
            (spec, bytes, fns)
        })
        .collect()
}

// ---- UNWIND_INFO -----------------------------------------------------------------------------

const RSP: u8 = 4;
const RBP: u8 = 5;
const REG_NAMES: [&str; 16] = [
    "RAX", "RCX", "RDX", "RBX", "RSP", "RBP", "RSI", "RDI", "R8", "R9", "R10", "R11", "R12", "R13",
    "R14", "R15",
];

/// An unwind code (Microsoft "x64 exception handling", UNWIND_CODE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Code {
    PushNonvol(u8),
    /// `sub rsp, n` (ALLOC_SMALL or ALLOC_LARGE): n bytes.
    Alloc(u32),
    SetFpreg,
    /// `mov [rsp + off], reg`.
    SaveNonvol(u8, u32),
}

/// A decoded UNWIND_INFO.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Info {
    prolog_size: u8,
    frame_reg: u8,
    /// The frame offset in bytes (the field times 16).
    frame_offset: u32,
    /// (prologue offset, code), in stored order: last prologue instruction first.
    codes: Vec<(u8, Code)>,
}

/// Decodes UNWIND_INFO bytes. Only the forms the spike's code generators use: version 1, no
/// flags (no handler, no chained info).
fn decode_info(b: &[u8]) -> Result<Info, String> {
    let [vf, prolog_size, count, fr, ..] = *b else {
        return Err(format!("{} bytes, no header", b.len()));
    };
    if vf & 7 != 1 || vf >> 3 != 0 {
        return Err(format!(
            "version {} flags {:#x}, want 1 and 0",
            vf & 7,
            vf >> 3
        ));
    }
    let slots = count as usize;
    let len = 4 + 2 * slots.next_multiple_of(2);
    if b.len() != len {
        return Err(format!("{} bytes for {count} slots, want {len}", b.len()));
    }
    let slot = |i: usize| -> Result<[u8; 2], String> {
        if i >= slots {
            return Err(format!("slot {i} past the {count} slots"));
        }
        Ok([b[4 + 2 * i], b[5 + 2 * i]])
    };
    let mut codes = Vec::new();
    let mut i = 0;
    while i < slots {
        let [off, op] = slot(i)?;
        let info = op >> 4;
        let (code, n) = match op & 15 {
            0 => (Code::PushNonvol(info), 1),
            1 if info == 0 => {
                let [lo, hi] = slot(i + 1)?;
                (Code::Alloc(8 * u32::from(u16::from_le_bytes([lo, hi]))), 2)
            }
            1 if info == 1 => {
                let [a, b2] = slot(i + 1)?;
                let [c, d] = slot(i + 2)?;
                (Code::Alloc(u32::from_le_bytes([a, b2, c, d])), 3)
            }
            2 => (Code::Alloc(8 * (u32::from(info) + 1)), 1),
            3 => (Code::SetFpreg, 1),
            4 => {
                let [lo, hi] = slot(i + 1)?;
                (
                    Code::SaveNonvol(info, 8 * u32::from(u16::from_le_bytes([lo, hi]))),
                    2,
                )
            }
            k => return Err(format!("unwind op {k} (info {info}) at slot {i}")),
        };
        codes.push((off, code));
        i += n;
    }
    if codes.windows(2).any(|w| w[0].0 < w[1].0) {
        return Err(format!(
            "codes are not in prologue order, last first: {codes:?}"
        ));
    }
    if codes.first().map_or(0, |c| c.0) != prolog_size {
        return Err(format!(
            "prolog size {prolog_size}, the last code ends at {:?}",
            codes.first()
        ));
    }
    let set_fp = codes.iter().any(|c| c.1 == Code::SetFpreg);
    if set_fp != (fr & 15 != 0) {
        return Err(format!(
            "frame register {} with SET_FPREG {set_fp}",
            fr & 15
        ));
    }
    Ok(Info {
        prolog_size,
        frame_reg: fr & 15,
        frame_offset: 16 * u32::from(fr >> 4),
        codes,
    })
}

// ---- the machine code of a frame -------------------------------------------------------------

/// A prologue or epilogue instruction of the spike's x86-64 code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Insn {
    Push(u8),
    Pop(u8),
    /// `mov rbp, rsp`.
    MovRbpRsp,
    /// `lea rbp, [rsp + d]`.
    LeaRbp(u8),
    SubRsp(u32),
    AddRsp(u32),
    /// `lea rsp, [frame register + d]`.
    LeaRsp(i32),
    Ret,
    /// `jmp qword ptr [mem]` with ModRM mod 00 (the (b) exit `rex.w jmp [rdi + rax*8]`, or
    /// `jmp [rip + disp32]`).
    JmpMem,
}

/// The exit of placement (b) and unit code: `rex.w jmp qword ptr [rdi + rax*8]`.
const EXIT_JMP: [u8; 4] = [0x48, 0xFF, 0x24, 0xC7];

impl Insn {
    /// The bytes of the forms the expected epilogue uses.
    fn bytes(self) -> Vec<u8> {
        match self {
            Insn::Pop(r) if r < 8 => vec![0x58 + r],
            Insn::Pop(r) => vec![0x41, 0x58 + r - 8],
            Insn::AddRsp(n) if n < 128 => vec![0x48, 0x83, 0xC4, n as u8],
            Insn::LeaRsp(d) if (-128..128).contains(&d) => vec![0x48, 0x8D, 0x65, d as u8],
            Insn::Ret => vec![0xC3],
            Insn::JmpMem => EXIT_JMP.to_vec(),
            i => panic!("no encoding for {i:?} here"),
        }
    }
}

/// Decodes one prologue instruction at the start of `c`: `push r64`, `mov rbp, rsp`,
/// `lea rbp, [rsp + disp8]`, `sub rsp, imm8`.
fn prologue_insn(c: &[u8]) -> Option<(Insn, usize)> {
    match c {
        [0x41, b @ 0x50..=0x57, ..] => Some((Insn::Push(b - 0x50 + 8), 2)),
        [b @ 0x50..=0x57, ..] => Some((Insn::Push(b - 0x50), 1)),
        [0x48, 0x89, 0xE5, ..] => Some((Insn::MovRbpRsp, 3)),
        [0x48, 0x8D, 0x6C, 0x24, d, ..] => Some((Insn::LeaRbp(*d), 5)),
        [0x48, 0x83, 0xEC, n, ..] => Some((Insn::SubRsp(u32::from(*n)), 4)),
        _ => None,
    }
}

fn pop_at(c: &[u8]) -> Option<(u8, usize)> {
    match c {
        [0x41, b @ 0x58..=0x5F, ..] => Some((b - 0x58 + 8, 2)),
        [b @ 0x58..=0x5F, ..] => Some((b - 0x58, 1)),
        _ => None,
    }
}

/// The frame a function's machine code builds and tears down.
#[derive(Clone, Debug)]
struct Frame {
    /// Each prologue instruction and the offset after it.
    prologue: Vec<(Insn, u32)>,
    /// Each epilogue instruction and its offset; the last one is the exit.
    epilogue: Vec<(Insn, u32)>,
}

impl Frame {
    fn prolog_end(&self) -> u32 {
        self.prologue.last().map_or(0, |p| p.1)
    }

    fn pushes(&self) -> Vec<u8> {
        self.prologue
            .iter()
            .filter_map(|p| match p.0 {
                Insn::Push(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// The register that holds the frame pointer, and the rsp offset from it.
    fn frame_pointer(&self) -> Option<(u8, u32)> {
        self.prologue.iter().find_map(|p| match p.0 {
            Insn::MovRbpRsp => Some((RBP, 0)),
            Insn::LeaRbp(d) => Some((RBP, u32::from(d))),
            _ => None,
        })
    }

    /// Decodes the frame of `code`: the prologue from offset 0, then the one epilogue that
    /// undoes it (the stack adjust, the pops in reverse order, `ret` or the (b) exit jump).
    /// Panics if the code has another shape (an emitter change then needs this test changed
    /// with it).
    fn decode(name: &str, code: &[u8]) -> Frame {
        let mut prologue = Vec::new();
        let mut at = 0;
        while let Some((i, len)) = prologue_insn(&code[at..]) {
            at += len;
            prologue.push((i, at as u32));
        }
        let f = Frame {
            prologue,
            epilogue: Vec::new(),
        };
        let alloc = f.prologue.iter().find_map(|p| match p.0 {
            Insn::SubRsp(n) => Some(n),
            _ => None,
        });
        let mut want = match (alloc, f.frame_pointer()) {
            (Some(n), _) => vec![Insn::AddRsp(n)],
            (None, Some((_, d))) if d > 0 => vec![Insn::LeaRsp(-(d as i32))],
            _ => Vec::new(),
        };
        want.extend(f.pushes().iter().rev().map(|&r| Insn::Pop(r)));
        assert!(
            !f.pushes().is_empty(),
            "{name}: no push at the start: {:02x?}",
            &code[..code.len().min(16)]
        );
        let mut found = Vec::new();
        for exit in [Insn::Ret, Insn::JmpMem] {
            let mut seq = want.clone();
            seq.push(exit);
            let bytes: Vec<u8> = seq.iter().flat_map(|i| i.bytes()).collect();
            for start in (0..code.len()).filter(|&s| code[s..].starts_with(&bytes)) {
                let mut at = start as u32;
                let mut epi = Vec::new();
                for &i in &seq {
                    epi.push((i, at));
                    at += i.bytes().len() as u32;
                }
                found.push(epi);
            }
        }
        assert_eq!(
            found.len(),
            1,
            "{name}: epilogues of shape {want:?}: {found:?}"
        );
        Frame {
            epilogue: found.pop().expect("one"),
            ..f
        }
    }

    /// The unwind codes this prologue needs, in stored order (last instruction first).
    fn codes(&self) -> Vec<(u8, Code)> {
        let mut v: Vec<(u8, Code)> = self
            .prologue
            .iter()
            .map(|&(i, end)| {
                let c = match i {
                    Insn::Push(r) => Code::PushNonvol(r),
                    Insn::MovRbpRsp | Insn::LeaRbp(_) => Code::SetFpreg,
                    Insn::SubRsp(n) => Code::Alloc(n),
                    i => unreachable!("{i:?} in a prologue"),
                };
                (end as u8, c)
            })
            .collect();
        v.reverse();
        v
    }
}

// ---- the register and stack model ------------------------------------------------------------

/// The integer registers (Win64 numbers) and rip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Regs {
    gp: [u64; 16],
    rip: u64,
}

/// A fake stack: 64 words; its addresses are real (the live unwinder reads it).
struct Stack {
    words: Box<[u64; 64]>,
}

impl Stack {
    fn new() -> Stack {
        Stack {
            words: Box::new([0; 64]),
        }
    }

    fn base(&self) -> u64 {
        self.words.as_ptr() as u64
    }

    /// rsp at the function's entry: the return address is there.
    fn entry_rsp(&self) -> u64 {
        self.base() + 32 * 8
    }

    fn index(&self, addr: u64) -> Result<usize, String> {
        let off = addr.wrapping_sub(self.base());
        if !off.is_multiple_of(8) || off / 8 >= 64 {
            return Err(format!("read at {addr:#x}, outside the fake stack"));
        }
        Ok((off / 8) as usize)
    }

    fn read(&self, addr: u64) -> Result<u64, String> {
        Ok(self.words[self.index(addr)?])
    }

    fn write(&mut self, addr: u64, v: u64) {
        let i = self.index(addr).expect("a write inside the fake stack");
        self.words[i] = v;
    }
}

/// The sentinel each register holds in the caller.
fn saved(r: u8) -> u64 {
    0x5a00_0000_0000_0011 | (u64::from(r) << 8)
}
/// What a pushed register holds in the body (the function's own use of it).
fn clobbered(r: u8) -> u64 {
    0xc100_0000_0000_0022 | (u64::from(r) << 8)
}
const RET: u64 = 0x0000_7ff0_1234_5670;

/// One machine state of a function: the offset and the registers there.
#[derive(Clone, Debug)]
struct State {
    what: String,
    ip: u32,
    regs: Regs,
    /// The establisher frame the unwinder must give (body states only).
    establisher: Option<u64>,
}

/// Runs `insn` on the model (prologue and epilogue forms only).
fn step(insn: Insn, r: &mut Regs, s: &mut Stack, frame_reg: u8) {
    let rsp = &mut r.gp[RSP as usize];
    match insn {
        Insn::Push(x) => {
            *rsp -= 8;
            let at = *rsp;
            let v = r.gp[x as usize];
            s.write(at, v);
        }
        Insn::Pop(x) => {
            let v = s.read(*rsp).expect("pop");
            *rsp += 8;
            r.gp[x as usize] = v;
        }
        Insn::MovRbpRsp => r.gp[RBP as usize] = *rsp,
        Insn::LeaRbp(d) => r.gp[RBP as usize] = *rsp + u64::from(d),
        Insn::SubRsp(n) => *rsp -= u64::from(n),
        Insn::AddRsp(n) => *rsp += u64::from(n),
        Insn::LeaRsp(d) => {
            r.gp[RSP as usize] = r.gp[frame_reg as usize].wrapping_add_signed(i64::from(d))
        }
        Insn::Ret | Insn::JmpMem => unreachable!("the exit is not run"),
    }
}

/// Every state to unwind from, with the stack the code builds: offset 0, after each prologue
/// instruction, each body step (and the first instruction after the prologue), the first
/// out-of-line instruction after the exit, and each epilogue instruction up to the exit.
fn states(f: &UnitFn, fr: &Frame, s: &mut Stack) -> Vec<State> {
    let mut r = Regs {
        gp: std::array::from_fn(|i| saved(i as u8)),
        rip: 0,
    };
    r.gp[RSP as usize] = s.entry_rsp();
    s.write(s.entry_rsp(), RET);
    let fp = fr.frame_pointer();
    let frame_reg = fp.map_or(0, |p| p.0);
    let mut v = vec![State {
        what: "entry".into(),
        ip: 0,
        regs: r,
        establisher: None,
    }];
    let mut establisher = None;
    for &(i, end) in &fr.prologue {
        step(i, &mut r, s, frame_reg);
        if matches!(i, Insn::MovRbpRsp | Insn::LeaRbp(_)) {
            establisher = Some(r.gp[RSP as usize]);
        }
        v.push(State {
            what: format!("after {i:?}"),
            ip: end,
            regs: r,
            establisher: None,
        });
    }
    // The body: every pushed register but the frame register holds the body's value.
    for x in fr.pushes() {
        if fp.is_none_or(|p| p.0 != x) {
            r.gp[x as usize] = clobbered(x);
        }
    }
    let establisher = establisher.unwrap_or(r.gp[RSP as usize]);
    let epi_start = fr.epilogue[0].1;
    let exit_end = {
        let (i, at) = *fr.epilogue.last().expect("exit");
        at + i.bytes().len() as u32
    };
    let mut body: Vec<(String, u32)> = vec![("body start".into(), fr.prolog_end())];
    body.extend(f.steps.iter().map(|&o| (format!("step at {o}"), o)));
    if (exit_end as usize) < f.code.len() {
        body.push(("out of line, after the exit".into(), exit_end));
    }
    for (what, ip) in body {
        assert!(
            ip >= fr.prolog_end() && (ip < epi_start || ip >= exit_end),
            "{}: {what} at {ip} is in the prologue or the epilogue",
            f.name
        );
        v.push(State {
            what,
            ip,
            regs: r,
            establisher: Some(establisher),
        });
    }
    for &(i, at) in &fr.epilogue {
        v.push(State {
            what: format!("epilogue {i:?}"),
            ip: at,
            regs: r,
            establisher: None,
        });
        if !matches!(i, Insn::Ret | Insn::JmpMem) {
            step(i, &mut r, s, frame_reg);
        }
    }
    v
}

/// An unwinder: from `regs` at offset `ip` of `f` (its rip already set by the caller for the
/// live one), to the caller's registers; returns the establisher frame.
type Unwinder<'a> = dyn FnMut(&UnitFn, u32, &mut Regs, &Stack) -> Result<u64, String> + 'a;

/// Unwinds `f` from every state and checks the caller's registers. Returns the number of
/// states checked, or the first mismatch.
fn check_function(f: &UnitFn, unwind: &mut Unwinder<'_>) -> Result<usize, String> {
    let fr = Frame::decode(&f.name, &f.code);
    let pushes = fr.pushes();
    let mut stack = Stack::new();
    let all = states(f, &fr, &mut stack);
    for st in &all {
        let mut r = st.regs;
        let establisher = unwind(f, st.ip, &mut r, &stack)?;
        let mut bad = Vec::new();
        if r.rip != RET {
            bad.push(format!("rip {:#x}, want {RET:#x}", r.rip));
        }
        let want_rsp = stack.entry_rsp() + 8;
        if r.gp[RSP as usize] != want_rsp {
            bad.push(format!("rsp {:#x}, want {want_rsp:#x}", r.gp[RSP as usize]));
        }
        for x in 0..16u8 {
            if x == RSP {
                continue;
            }
            let want = if pushes.contains(&x) {
                saved(x)
            } else {
                st.regs.gp[x as usize]
            };
            if r.gp[x as usize] != want {
                bad.push(format!(
                    "{} = {:#x}, want {want:#x}",
                    REG_NAMES[x as usize], r.gp[x as usize]
                ));
            }
        }
        if let Some(e) = st.establisher
            && e != establisher
        {
            bad.push(format!("establisher frame {establisher:#x}, want {e:#x}"));
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
    }
    Ok(all.len())
}

// ---- a model of the Win64 unwinder -----------------------------------------------------------

/// The epilogue that starts at `c` as the Win64 unwinder reads it, or `None`: an optional
/// `add rsp, imm` or `lea rsp, [frame register + disp]`, pops, then `ret` or a `jmp` through
/// memory with ModRM mod 00 (Microsoft "x64 prolog and epilog": the legal epilogue forms).
fn epilogue_at(c: &[u8], frame_reg: u8) -> Option<Vec<Insn>> {
    let mut v = Vec::new();
    let mut p = 0;
    match c {
        [0x48, 0x83, 0xC4, n, ..] => {
            v.push(Insn::AddRsp(u32::from(*n)));
            p = 4;
        }
        [0x48, 0x81, 0xC4, a, b, c2, d, ..] => {
            v.push(Insn::AddRsp(u32::from_le_bytes([*a, *b, *c2, *d])));
            p = 7;
        }
        [0x48, 0x8D, m, rest @ ..]
            if frame_reg != 0 && frame_reg < 8 && (m >> 3) & 7 == RSP && m & 7 == frame_reg =>
        {
            match (m >> 6, rest) {
                (1, [d, ..]) => {
                    v.push(Insn::LeaRsp(i32::from(*d as i8)));
                    p = 4;
                }
                (2, [a, b, c2, d, ..]) => {
                    v.push(Insn::LeaRsp(i32::from_le_bytes([*a, *b, *c2, *d])));
                    p = 7;
                }
                _ => return None,
            }
        }
        _ => {}
    }
    while let Some((x, len)) = pop_at(&c[p..]) {
        v.push(Insn::Pop(x));
        p += len;
    }
    match &c[p..] {
        [0xC3, ..] => v.push(Insn::Ret),
        [0x48 | 0x49, 0xFF, m, ..] if (m >> 3) & 7 == 4 && m >> 6 == 0 => v.push(Insn::JmpMem),
        [0xFF, 0x25, ..] => v.push(Insn::JmpMem),
        _ => return None,
    }
    Some(v)
}

/// The model unwinder: `RtlVirtualUnwind` as Microsoft documents it for code without a
/// handler or chained info. Past the prologue, an epilogue at `ip` is emulated forward;
/// otherwise each code whose prologue instruction has run (its offset <= `ip`) is undone in
/// stored order, then the return address is popped.
fn model_unwind(f: &UnitFn, info: &[u8], ip: u32, r: &mut Regs, s: &Stack) -> Result<u64, String> {
    let info = decode_info(info).map_err(|e| format!("{}: {e}", f.name))?;
    let mut establisher = r.gp[RSP as usize];
    let pop = |r: &mut Regs, x: Option<u8>| -> Result<(), String> {
        let v = s.read(r.gp[RSP as usize])?;
        r.gp[RSP as usize] += 8;
        match x {
            Some(x) => r.gp[x as usize] = v,
            None => r.rip = v,
        }
        Ok(())
    };
    if ip >= u32::from(info.prolog_size)
        && let Some(epi) = epilogue_at(&f.code[ip as usize..], info.frame_reg)
    {
        for i in epi {
            match i {
                Insn::AddRsp(n) => r.gp[RSP as usize] += u64::from(n),
                Insn::LeaRsp(d) => {
                    r.gp[RSP as usize] =
                        r.gp[info.frame_reg as usize].wrapping_add_signed(i64::from(d))
                }
                Insn::Pop(x) => pop(r, Some(x))?,
                Insn::Ret | Insn::JmpMem => pop(r, None)?,
                i => unreachable!("{i:?} in an epilogue"),
            }
        }
        return Ok(establisher);
    }
    for &(off, c) in &info.codes {
        if u32::from(off) > ip {
            continue;
        }
        match c {
            Code::PushNonvol(x) => pop(r, Some(x))?,
            Code::Alloc(n) => r.gp[RSP as usize] += u64::from(n),
            Code::SetFpreg => {
                let frame = r.gp[info.frame_reg as usize] - u64::from(info.frame_offset);
                r.gp[RSP as usize] = frame;
                establisher = frame;
            }
            Code::SaveNonvol(..) => return Err(format!("{}: SAVE_NONVOL is not modelled", f.name)),
        }
    }
    pop(r, None)?;
    Ok(establisher)
}

// ---- the static checks (every host) ----------------------------------------------------------

/// Each function of units A and B has its Win64 entry; each UNWIND_INFO is well formed, its
/// prolog size is the end of the decoded prologue, and its codes are the codes that prologue
/// needs (register, order, offsets, frame register and offset).
#[test]
fn every_unit_function_has_the_unwind_info_of_its_prologue() {
    let mut report = Vec::new();
    for (spec, _, fns) in units() {
        assert_eq!(
            fns.iter().filter(|f| f.kind == FnKind::Js).count(),
            spec.native.len(),
            "unit {}",
            spec.name
        );
        for f in &fns {
            let fr = Frame::decode(&f.name, &f.code);
            let info = decode_info(&f.info).unwrap_or_else(|e| panic!("{}: {e}", f.name));
            assert_eq!(
                u32::from(info.prolog_size),
                fr.prolog_end(),
                "{}: prolog size",
                f.name
            );
            assert_eq!(info.codes, fr.codes(), "{}: unwind codes", f.name);
            let (reg, off) = fr.frame_pointer().unwrap_or((0, 0));
            assert_eq!(
                (info.frame_reg, info.frame_offset),
                (reg, off),
                "{}: frame register and offset",
                f.name
            );
            if f.kind == FnKind::Js {
                assert_eq!(
                    fr.epilogue.last().map(|e| e.0),
                    Some(Insn::JmpMem),
                    "{}: unit JS code exits by the (b) exit jump",
                    f.name
                );
            }
            report.push(format!(
                "unit {} {}: {} bytes, prolog {}, {} codes, exit {:?}",
                spec.name,
                f.name,
                f.code.len(),
                info.prolog_size,
                info.codes.len(),
                fr.epilogue.last().expect("exit").0
            ));
        }
    }
    eprintln!("{}", report.join("\n"));
}

/// The model unwinder takes every state of every function of units A and B back to the
/// caller with the caller's registers.
#[test]
fn the_model_unwinds_every_state_of_every_unit_function() {
    let mut total = 0;
    for (spec, _, fns) in units() {
        for f in &fns {
            let n = check_function(f, &mut |f, ip, r, s| model_unwind(f, &f.info, ip, r, s))
                .unwrap_or_else(|e| panic!("unit {}: {e}", spec.name));
            total += n;
            eprintln!("unit {} {}: {n} states", spec.name, f.name);
        }
    }
    assert!(total > 0);
}

/// The broken UNWIND_INFOs of the negative controls, for a unit JS function: two push codes
/// swapped, and the frame offset 16 bytes off.
fn broken_infos(f: &UnitFn) -> Vec<(&'static str, Vec<u8>)> {
    let info = &f.info;
    // Stored codes: [set_fpreg, push rbx, push r14, push r13, push r15, push rbp]; the
    // register is the high nibble of each code's second byte.
    assert_eq!(info[..4], [1, 13, 6, 0x25], "{}: {info:02x?}", f.name);
    let mut swapped = info.clone();
    swapped.swap(7, 9);
    let mut off = info.clone();
    off[3] = 0x15;
    vec![("pushes swapped", swapped), ("frame offset", off)]
}

/// `bytes` (a unit) with the UNWIND_INFO of `f` replaced by `new` (same length), rehashed.
/// The unwind section stores each entry as text start, text end, length, then the
/// UNWIND_INFO (unit::format); `id` and `drive` have equal UNWIND_INFO bytes, so the search
/// includes the text range.
fn patched(bytes: &[u8], f: &UnitFn, new: &[u8]) -> Vec<u8> {
    assert_eq!(f.info.len(), new.len());
    let mut entry = Vec::new();
    entry.extend_from_slice(&f.text.start.to_le_bytes());
    entry.extend_from_slice(&f.text.end.to_le_bytes());
    entry.extend_from_slice(&(f.info.len() as u32).to_le_bytes());
    entry.extend_from_slice(&f.info);
    let mut b = bytes.to_vec();
    let at: Vec<usize> = (0..=b.len() - entry.len())
        .filter(|&i| b[i..i + entry.len()] == *entry)
        .collect();
    assert_eq!(at.len(), 1, "{}: unwind entry found at {at:?}", f.name);
    let info = at[0] + 12;
    b[info..info + new.len()].copy_from_slice(new);
    rehash(&mut b);
    b
}

/// The negative controls of the Windows run (`live`) load unit A at slots 10 and 11 with a
/// broken UNWIND_INFO for `drive`: the patched unit keeps a LOAD verdict on an x86-64 spike
/// host (its hash is rewritten), and only `drive`'s UNWIND_INFO changes.
#[test]
fn a_patched_unit_carries_the_broken_unwind_info() {
    for (k, slot) in [(0usize, 10u32), (1, 11)] {
        let spec = UnitSpec { slot, ..unit_a() };
        let bytes = x64_unit(&spec);
        let fns = unit_functions(&spec, &bytes);
        let drive = fns.iter().find(|f| f.name == "drive").expect("drive");
        let (what, bad) = broken_infos(drive).swap_remove(k);
        let p = patched(&bytes, drive, &bad);
        // The spike host of an x86-64 machine (on Windows x64, `spike_host()` itself).
        let mut host = spike_host();
        host.target_arch = ep9_unit::target_arch::X86_64;
        for (which, b) in [("unpatched", &bytes), ("patched", &p)] {
            let v = ep9_unit::verdict(b, &host);
            assert_eq!(v.outcome(), ep9_unit::Outcome::Load, "{what} {which}: {v}");
        }
        let view = UnitView::parse(&p).expect("patched unit");
        for (e, f) in view.unwind.win64.iter().zip(&fns) {
            let want = if f.name == "drive" { &bad } else { &f.info };
            assert_eq!(e.info, *want, "{what}: {}", f.name);
        }
    }
}

/// A [`check_function`] error that names wrong caller registers at a state (not a lookup or
/// decode error, and not the entry state, where no code applies).
fn is_register_mismatch(e: &str) -> bool {
    e.contains(" (offset ") && !e.contains(" entry (offset ")
}

/// Negative controls: the model fails `drive` (unit A) with a broken UNWIND_INFO.
#[test]
fn a_broken_unwind_info_fails_the_model() {
    let (_, _, fns) = units().into_iter().next().expect("unit A");
    let drive = fns.iter().find(|f| f.name == "drive").expect("drive");
    for (what, bad) in broken_infos(drive) {
        let r = check_function(drive, &mut |f, ip, r, s| model_unwind(f, &bad, ip, r, s));
        match r {
            Err(e) if is_register_mismatch(&e) => eprintln!("{what}: {e}"),
            r => panic!("{what}: want wrong caller registers, got {r:?}"),
        }
    }
}

/// The toolchain's `llvm-readobj` (rustup component llvm-tools), if it is there.
fn llvm_readobj() -> Option<std::path::PathBuf> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new(&rustc)
            .args(args)
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let sysroot = out(&["--print", "sysroot"])?;
    let vv = out(&["-vV"])?;
    let host = vv.lines().find_map(|l| l.strip_prefix("host: "))?;
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let p = std::path::Path::new(&sysroot)
        .join("lib/rustlib")
        .join(host)
        .join("bin")
        .join(format!("llvm-readobj{exe}"));
    p.is_file().then_some(p)
}

/// A COFF x86-64 object of one unit: `.text` = the unit's text section as it is, `.xdata` =
/// each function's UNWIND_INFO (4-aligned), `.pdata` = one RUNTIME_FUNCTION each, with
/// ADDR32NB relocations against a symbol per function and against `.xdata`.
fn coff_object(text: &[u8], fns: &[UnitFn]) -> Vec<u8> {
    const IMAGE_REL_AMD64_ADDR32NB: u16 = 3;
    let mut xdata = Vec::new();
    let mut pdata = Vec::new();
    let mut relocs = Vec::new();
    for (i, f) in fns.iter().enumerate() {
        xdata.resize(xdata.len().next_multiple_of(4), 0);
        let info_at = xdata.len() as u32;
        xdata.extend_from_slice(&f.info);
        let base = pdata.len() as u32;
        // In-place addends: begin = sym + 0, end = sym + len, unwind = .xdata + offset.
        pdata.extend_from_slice(&0u32.to_le_bytes());
        pdata.extend_from_slice(&(f.code.len() as u32).to_le_bytes());
        pdata.extend_from_slice(&info_at.to_le_bytes());
        for (off, sym) in [(0, 3 + i as u32), (4, 3 + i as u32), (8, 1)] {
            relocs.extend_from_slice(&(base + off).to_le_bytes());
            relocs.extend_from_slice(&sym.to_le_bytes());
            relocs.extend_from_slice(&IMAGE_REL_AMD64_ADDR32NB.to_le_bytes());
        }
    }
    let text_at = 20 + 3 * 40;
    let xdata_at = text_at + text.len();
    let pdata_at = xdata_at + xdata.len();
    let reloc_at = pdata_at + pdata.len();
    let sym_at = reloc_at + relocs.len();
    let mut o = Vec::new();
    o.extend_from_slice(&0x8664u16.to_le_bytes()); // IMAGE_FILE_MACHINE_AMD64
    o.extend_from_slice(&3u16.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&(sym_at as u32).to_le_bytes());
    o.extend_from_slice(&((3 + fns.len()) as u32).to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    let section =
        |o: &mut Vec<u8>, name: &[u8], size: usize, at: usize, rel: (usize, usize), ch: u32| {
            let mut n = [0u8; 8];
            n[..name.len()].copy_from_slice(name);
            o.extend_from_slice(&n);
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&(size as u32).to_le_bytes());
            o.extend_from_slice(&(at as u32).to_le_bytes());
            o.extend_from_slice(&(rel.0 as u32).to_le_bytes());
            o.extend_from_slice(&0u32.to_le_bytes());
            o.extend_from_slice(&(rel.1 as u16).to_le_bytes());
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
        (reloc_at, relocs.len() / 10),
        0x4030_0040,
    );
    o.extend_from_slice(text);
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
    symbol(&mut o, ".text", 0, 1, 0, 3);
    symbol(&mut o, ".xdata", 0, 2, 0, 3);
    symbol(&mut o, ".pdata", 0, 3, 0, 3);
    for f in fns {
        symbol(&mut o, &coff_name(f), f.text.start, 1, 0x20, 2);
    }
    o.extend_from_slice(&4u32.to_le_bytes()); // empty string table
    o
}

/// The symbol name of `f` in the object (`<probe>` and `<cell>` lose their brackets).
fn coff_name(f: &UnitFn) -> String {
    f.name.trim_matches(|c| c == '<' || c == '>').to_string()
}

/// The `llvm-readobj --unwind` lines this test expects for `f` (from [`decode_info`]): the
/// start, the end, the prolog size, the frame register and every unwind code.
fn readobj_lines(f: &UnitFn) -> Vec<String> {
    let info = decode_info(&f.info).expect("info");
    let name = coff_name(f);
    let mut v = vec![
        format!("StartAddress: {name} (0x{:X})", 0),
        format!("EndAddress: {name} +0x{:X}", f.code.len()),
        format!("PrologSize: {}", info.prolog_size),
    ];
    if info.frame_reg != 0 {
        v.push(format!(
            "FrameRegister: {} (0x{:X})",
            REG_NAMES[info.frame_reg as usize], info.frame_reg
        ));
        v.push(format!("FrameOffset: 0x{:X}", info.frame_offset / 16));
    } else {
        v.push("FrameRegister: -".into());
        v.push("FrameOffset: -".into());
    }
    for &(off, c) in &info.codes {
        v.push(match c {
            Code::PushNonvol(x) => {
                format!("0x{off:02X}: PUSH_NONVOL reg={}", REG_NAMES[x as usize])
            }
            Code::Alloc(n) if n <= 128 => format!("0x{off:02X}: ALLOC_SMALL size={n}"),
            Code::Alloc(n) => format!("0x{off:02X}: ALLOC_LARGE size={n}"),
            Code::SetFpreg => format!(
                "0x{off:02X}: SET_FPREG reg={}, offset=0x{:X}",
                REG_NAMES[info.frame_reg as usize], info.frame_offset
            ),
            Code::SaveNonvol(x, o) => format!(
                "0x{off:02X}: SAVE_NONVOL reg={}, offset=0x{o:X}",
                REG_NAMES[x as usize]
            ),
        });
    }
    v
}

/// An independent decoder: `llvm-readobj --unwind` on a COFF object of each unit reads the
/// UNWIND_INFO of every function as this test reads it (the register names, the scaled frame
/// offset, the code offsets and the function ranges). Skipped when llvm-tools is not
/// installed (`rustup component add llvm-tools`; CI installs it).
#[test]
fn llvm_readobj_reads_the_same_unwind_info() {
    let Some(readobj) = llvm_readobj() else {
        eprintln!("llvm-readobj cross-check: SKIP (no llvm-readobj in the toolchain's llvm-tools)");
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("win64_unit");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    for (spec, bytes, fns) in units() {
        let view = UnitView::parse(&bytes).expect("unit");
        let obj = dir.join(format!("unit-{}.obj", spec.name));
        std::fs::write(&obj, coff_object(view.text, &fns)).expect("object");
        let out = std::process::Command::new(&readobj)
            .arg("--unwind")
            .arg(&obj)
            .output()
            .expect("llvm-readobj runs");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && err.trim().is_empty(),
            "llvm-readobj: {err}"
        );
        let text = String::from_utf8_lossy(&out.stdout);
        // One block per RuntimeFunction, in object order; StartAddress offsets are section
        // offsets, so they are replaced by 0 (the symbol names the function).
        let blocks: Vec<Vec<String>> = text
            .split("RuntimeFunction {")
            .skip(1)
            .map(|b| {
                b.lines()
                    .map(str::trim)
                    .filter(|l| {
                        [
                            "StartAddress:",
                            "EndAddress:",
                            "PrologSize:",
                            "FrameRegister:",
                            "FrameOffset:",
                        ]
                        .iter()
                        .any(|k| l.starts_with(k))
                            || l.starts_with("0x")
                    })
                    .map(|l| {
                        if let Some(rest) = l.strip_prefix("StartAddress: ") {
                            let name = rest.split(' ').next().unwrap_or("");
                            format!("StartAddress: {name} (0x0)")
                        } else if let Some(rest) = l.strip_prefix("EndAddress: ") {
                            rest.split(" (")
                                .next()
                                .map_or(l.to_string(), |s| format!("EndAddress: {s}"))
                        } else {
                            l.to_string()
                        }
                    })
                    .collect()
            })
            .collect();
        assert_eq!(blocks.len(), fns.len(), "unit {}:\n{text}", spec.name);
        for (f, got) in fns.iter().zip(&blocks) {
            assert_eq!(
                *got,
                readobj_lines(f),
                "unit {} {}:\n{text}",
                spec.name,
                f.name
            );
        }
        eprintln!(
            "unit {}: llvm-readobj --unwind = this test's decoding ({} functions)",
            spec.name,
            fns.len()
        );
    }
}

/// The real run on Windows x64: the spike loader maps the unit text and registers its
/// function table, and the system unwinder reads it.
#[cfg(windows)]
mod live {
    use std::ffi::c_void;

    use spike_b::harness::Config;
    use spike_b::unit::checks::on_thread;
    use spike_b::unit::load::{LoadKind, LoadOptions, Mapped, UnitSession, load, mapped_units};

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
    /// Win64 register-number order, rip at 0xf8. The rest is not read here.
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

    #[link(name = "kernel32")]
    unsafe extern "system" {
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
    }

    const UNW_FLAG_NHANDLER: u32 = 0;

    /// `RtlLookupFunctionEntry(pc)`: the image base, a copy of the entry and the entry.
    fn lookup(pc: usize) -> Option<(u64, RuntimeFunction, *const RuntimeFunction)> {
        let mut base = 0u64;
        // SAFETY: the lookup only reads the loaded images and the registered function tables;
        // `base` is a valid out pointer; no history table.
        let e = unsafe { RtlLookupFunctionEntry(pc as u64, &mut base, std::ptr::null_mut()) };
        if e.is_null() {
            None
        } else {
            // SAFETY: a non-null result points at a live RUNTIME_FUNCTION of a registered table
            // (the unit's tables are never deleted).
            Some((base, unsafe { *e }, e))
        }
    }

    /// The system unwinder for the function `f` placed at `code`: checks the entry that
    /// `RtlLookupFunctionEntry` finds and its registered UNWIND_INFO, then runs
    /// `RtlVirtualUnwind` on `regs` at offset `ip`.
    fn system_unwind(
        code: &Range<usize>,
        f: &UnitFn,
        ip: u32,
        regs: &mut Regs,
        stack: &Stack,
    ) -> Result<u64, String> {
        let pc = code.start + ip as usize;
        let (base, rf, entry) =
            lookup(pc).ok_or_else(|| format!("{}: no RUNTIME_FUNCTION for {pc:#x}", f.name))?;
        let (b, e) = (
            base as usize + rf.begin as usize,
            base as usize + rf.end as usize,
        );
        if (b..e) != *code {
            return Err(format!(
                "{} at {ip}: the entry covers {b:#x}..{e:#x}, the code is {code:x?}",
                f.name
            ));
        }
        let at = base as usize + rf.unwind_info as usize;
        // SAFETY: `at` is the UNWIND_INFO of a registered entry, in the unit's chunk, which
        // stays mapped for the process.
        let got = unsafe { std::slice::from_raw_parts(at as *const u8, f.info.len()) };
        if got != &f.info[..] {
            return Err(format!(
                "{}: registered UNWIND_INFO {got:02x?}, the unit's {:02x?}",
                f.name, f.info
            ));
        }
        let mut ctx = Context {
            head: [0; 15],
            gp: regs.gp,
            rip: pc as u64,
            rest: [0; 0x3d0],
        };
        // ContextFlags (offset 0x30): CONTEXT_AMD64 | CONTROL | INTEGER | FLOATING_POINT.
        ctx.head[6] = 0x0010_000b;
        let mut data: *mut c_void = std::ptr::null_mut();
        let mut establisher = 0u64;
        std::hint::black_box(&stack.words);
        // SAFETY: `entry` is the lookup of ctx.rip; the table stays registered; rsp and rbp
        // point into `stack`, which lives across the call; the unwind reads only those words.
        unsafe {
            RtlVirtualUnwind(
                UNW_FLAG_NHANDLER,
                base,
                ctx.rip,
                entry,
                &mut ctx,
                &mut data,
                &mut establisher,
                std::ptr::null_mut(),
            )
        };
        regs.gp = ctx.gp;
        regs.rip = ctx.rip;
        Ok(establisher)
    }

    /// Loads `bytes` in a new session (it must give LOAD) and returns its mapping.
    fn loaded(bytes: &[u8]) -> Result<&'static Mapped, String> {
        let mut us = UnitSession::new(&Config::default())?;
        let l = load(&mut us, bytes, &spike_host(), &LoadOptions::default());
        match &l.kind {
            LoadKind::Native { .. } => {}
            k => return Err(format!("{} ({k:?}), want LOAD", k.name())),
        }
        let hash = UnitView::parse(bytes)?.header.content_hash;
        mapped_units()
            .into_iter()
            .find(|m| m.hash == hash)
            .ok_or_else(|| "the unit is not mapped".into())
    }

    /// Checks every function of the mapped unit with the system unwinder.
    fn check_mapped(m: &Mapped, fns: &[UnitFn]) -> Result<Vec<String>, String> {
        let mut report = Vec::new();
        for f in fns {
            let e = m
                .functions
                .iter()
                .find(|e| e.text == f.text)
                .ok_or_else(|| format!("{}: not in the mapping", f.name))?;
            let code = m.code(e);
            let n = check_function(f, &mut |f, ip, r, s| system_unwind(&code, f, ip, r, s))?;
            report.push(format!("{}: {n} states", f.name));
        }
        // No entry past the text (the UNWIND_INFO after it is data).
        if let Some((base, rf, _)) = lookup(m.text.end)
            && (base as usize + rf.begin as usize) < m.text.end
        {
            return Err(format!("{:#x} (past the text) has an entry", m.text.end));
        }
        Ok(report)
    }

    fn every_state(_: spike_b::unit::checks::Scale) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for (spec, bytes, fns) in units() {
            let m = loaded(&bytes).map_err(|e| format!("unit {}: {e}", spec.name))?;
            for l in check_mapped(m, &fns).map_err(|e| format!("unit {}: {e}", spec.name))? {
                out.push(format!("unit {} {l}", spec.name));
            }
        }
        Ok(out)
    }

    /// Units A and B, loaded: every state of every function unwinds to its caller through
    /// `RtlVirtualUnwind`, with the registered UNWIND_INFO.
    #[test]
    fn every_state_of_every_loaded_unit_function_unwinds_to_its_caller() {
        let out = on_thread(every_state, spike_b::unit::checks::Scale::Test)
            .unwrap_or_else(|e| panic!("{e}"));
        eprintln!("{}", out.join("\n"));
    }

    fn broken(_: spike_b::unit::checks::Scale) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        // Unit A at slots 10 and 11, so neither variant meets unit A's own mapping.
        for (k, slot) in [(0usize, 10u32), (1, 11)] {
            let spec = UnitSpec { slot, ..unit_a() };
            let bytes = x64_unit(&spec);
            let fns = unit_functions(&spec, &bytes);
            let drive = fns.iter().find(|f| f.name == "drive").expect("drive");
            let (what, bad) = broken_infos(drive).swap_remove(k);
            let m = loaded(&patched(&bytes, drive, &bad))?;
            // The registered UNWIND_INFO is now the broken one (system_unwind compares it
            // with `f.info`), and the system unwinder must give wrong caller registers.
            let mut f = drive.clone();
            f.info = bad;
            let e = m
                .functions
                .iter()
                .find(|e| e.text == f.text)
                .ok_or("drive is not in the mapping")?;
            let code = m.code(e);
            match check_function(&f, &mut |f, ip, r, s| system_unwind(&code, f, ip, r, s)) {
                Err(e) if is_register_mismatch(&e) => out.push(format!("{what}: {e}")),
                r => return Err(format!("{what}: want wrong caller registers, got {r:?}")),
            }
        }
        Ok(out)
    }

    /// Negative controls: a unit whose `drive` has a broken UNWIND_INFO loads, and the system
    /// unwinder then gives wrong caller registers.
    #[test]
    fn a_broken_unwind_info_fails_the_system_unwinder() {
        let out =
            on_thread(broken, spike_b::unit::checks::Scale::Test).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("{}", out.join("\n"));
    }
}
