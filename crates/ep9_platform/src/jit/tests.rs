//! W^X, icache and unwind tests with hand-assembled code. The bytes were checked with
//! `clang -c` + `objdump -d` (aarch64, x86-64) and `llvm-mc` + `llvm-readobj --unwind` (Win64).

use std::cell::RefCell;
use std::io;
use std::sync::Barrier;

use super::{CodeMemory, WxMode};
use crate::unwind;
use crate::va::{
    CAGE_LIMIT, Cage, CageConfig, CodeChunk, CodeRange, CodeRangeConfig, DEFAULT_CODE_RANGE_SIZE,
    Reservation, TSRS_RESERVATION, allocation_granularity, page_size,
};

/// Owns the pages a test commits code in.
enum Space {
    Chunk(CodeChunk<'static>),
    Plain(Reservation),
}

impl Space {
    /// A chunk of the process code range for MapJit and for every mode off macOS arm64; a
    /// plain reservation for Protect on macOS arm64 (MAP_JIT pages cannot be mprotected).
    fn for_mode(mode: WxMode) -> Space {
        if mode.needs_map_jit() || !cfg!(all(target_vendor = "apple", target_arch = "aarch64")) {
            let range = CodeRange::process().expect("reserve the process code range");
            Space::Chunk(range.alloc(page_size()).expect("a chunk of the code range"))
        } else {
            Space::Plain(Reservation::reserve(1 << 20).expect("reserve 1 MiB"))
        }
    }

    /// The whole reservation: the process code range, or the plain one.
    fn range(&self) -> &Reservation {
        match self {
            Space::Chunk(c) => c.code_range().reservation(),
            Space::Plain(r) => r,
        }
    }

    /// Offset of this test's page in [`Space::range`].
    fn offset(&self) -> usize {
        match self {
            Space::Chunk(c) => c.offset(),
            Space::Plain(_) => 0,
        }
    }
}

fn commit(space: &Space, mode: WxMode) -> CodeMemory<'_> {
    // SAFETY: the page at the space's offset is this test's alone (its chunk, or a new plain
    // reservation), and this is its only code memory.
    unsafe { CodeMemory::commit(space.range(), space.offset(), page_size(), mode) }
        .expect("commit one page")
}

/// x86-64 Linux under Rosetta (for example Docker on an Apple silicon Mac). Rosetta keeps a
/// translation of the RX view of a dual mapping when the RW view changes (measured
/// 2026-10-21: the rewritten function still returned the old value), so the dual-map rewrite
/// check cannot pass there. Real x86-64 hardware snoops by physical address.
fn under_rosetta() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && std::fs::read_to_string("/proc/cpuinfo").is_ok_and(|s| s.contains("VirtualApple"))
}

/// `return imm` for the host.
fn ret_const(imm: u16) -> Vec<u8> {
    #[cfg(target_arch = "aarch64")]
    {
        // movz x0, #imm; ret
        let movz = 0xd280_0000u32 | (u32::from(imm) << 5);
        [movz, 0xd65f_03c0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }
    #[cfg(target_arch = "x86_64")]
    {
        // mov eax, imm32; ret
        let mut v = vec![0xb8];
        v.extend_from_slice(&u32::from(imm).to_le_bytes());
        v.push(0xc3);
        v
    }
}

fn write(mem: &mut CodeMemory<'_>, offset: usize, code: &[u8]) {
    let mut w = mem
        .open_write(offset, code.len())
        .expect("open a write window");
    w.bytes().copy_from_slice(code);
    w.close().expect("close the write window");
}

/// # Safety
/// `addr` holds a complete `ret_const` function.
unsafe fn call0(addr: usize) -> u64 {
    // SAFETY: the caller guarantees the address holds a function with this C signature.
    let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(addr) };
    f()
}

#[test]
fn modes_of_this_target() {
    let modes = WxMode::available();
    assert_eq!(modes[0], WxMode::DEFAULT);
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    assert_eq!(modes, [WxMode::MapJit, WxMode::Protect]);
    #[cfg(target_os = "linux")]
    assert_eq!(modes, [WxMode::Protect, WxMode::DualMap]);
    #[cfg(windows)]
    assert_eq!(modes, [WxMode::Protect]);
}

#[test]
fn write_call_rewrite() {
    for &mode in WxMode::available() {
        if mode == WxMode::DualMap && under_rosetta() {
            // Rosetta keeps its translation of the RX view when the RW view changes.
            eprintln!("dual-map rewrite skipped: Rosetta does not see writes through the alias");
            continue;
        }
        let space = Space::for_mode(mode);
        let mut mem = commit(&space, mode);
        assert_eq!(mem.mode(), mode);
        assert!(space.range().contains(mem.exec_base()));
        // Two functions in one page; rewrite each several times. A missing icache flush would
        // run stale bytes on arm64.
        for round in 0..8u16 {
            write(&mut mem, 0, &ret_const(100 + round));
            write(&mut mem, 64, &ret_const(200 + round));
            // SAFETY: both offsets hold a complete ret_const function, written just now.
            let (a, b) = unsafe { (call0(mem.exec_addr(0)), call0(mem.exec_addr(64))) };
            assert_eq!(
                (a, b),
                (u64::from(100 + round), u64::from(200 + round)),
                "{mode}"
            );
        }
    }
}

/// Pages never used before read zero. A private range, not the process one: there a page that
/// another test's `MapJit` memory used holds unspecified bytes (`CodeMemory::commit`).
#[test]
fn fresh_pages_read_zero_through_a_window() {
    let fresh = CodeRange::reserve(&CodeRangeConfig {
        size: 1 << 20,
        ..CodeRangeConfig::default()
    })
    .expect("reserve a private code range");
    let ps = page_size();
    for &mode in WxMode::available() {
        let plain;
        let mut chunk;
        let mut mem = if fresh.is_map_jit() == mode.needs_map_jit() {
            chunk = fresh.alloc(ps).unwrap();
            chunk.commit(mode).unwrap()
        } else {
            plain = Reservation::reserve(1 << 20).unwrap();
            // SAFETY: a new reservation; this is its only code memory.
            unsafe { CodeMemory::commit(&plain, 0, ps, mode) }.unwrap()
        };
        let mut w = mem.open_write(0, ps).unwrap();
        assert!(w.bytes().iter().all(|&b| b == 0), "{mode}");
    }
}

#[test]
fn nested_map_jit_windows() {
    super::map_jit_nest(true, |on| assert!(on));
    let mut toggles = Vec::new();
    super::map_jit_nest(true, |on| toggles.push(on));
    super::map_jit_nest(false, |on| toggles.push(on));
    assert!(toggles.is_empty(), "inner windows must not toggle");
    super::map_jit_nest(false, |on| assert!(!on));
}

#[test]
fn rejects_bad_requests() {
    let mode = WxMode::DEFAULT;
    let space = Space::for_mode(mode);
    let ps = page_size();
    let r = space.range();
    // SAFETY: each commit below fails its checks before it touches memory.
    unsafe {
        assert!(
            CodeMemory::commit(r, 1, ps, mode).is_err(),
            "unaligned offset"
        );
        assert!(CodeMemory::commit(r, 0, 0, mode).is_err(), "empty");
        assert!(
            CodeMemory::commit(r, r.size(), ps, mode).is_err(),
            "outside"
        );
    }
    let mut mem = commit(&space, mode);
    assert!(mem.open_write(0, 0).is_err());
    assert!(mem.open_write(ps - 4, 8).is_err());
    assert!(mem.open_write(ps, 1).is_err());
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    {
        assert!(r.is_map_jit());
        // MAP_JIT pages are RWX already; plain commit and decommit must refuse them.
        assert_eq!(
            r.commit(0, ps).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        // SAFETY: the call is refused before it touches memory.
        let e = unsafe { r.decommit(0, ps) }.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
        // SAFETY: the calls are refused before they touch memory.
        unsafe {
            assert!(CodeMemory::commit(r, ps, ps, WxMode::Protect).is_err());
            let plain = Reservation::reserve(1 << 20).unwrap();
            assert!(CodeMemory::commit(&plain, 0, ps, WxMode::MapJit).is_err());
        }
    }
    #[cfg(not(all(target_vendor = "apple", target_arch = "aarch64")))]
    {
        assert!(!r.is_map_jit());
        // SAFETY: refused before it touches memory.
        let e = unsafe { CodeMemory::commit(r, ps, ps, WxMode::MapJit) }.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
    }
}

/// Page protections from /proc/self/maps: the window must flip RX <-> RW (Protect), and the
/// two views of a dual mapping must never change.
#[cfg(target_os = "linux")]
#[test]
fn linux_page_protections() {
    fn perms(addr: usize) -> String {
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
        for line in maps.lines() {
            let mut it = line.split_whitespace();
            let range = it.next().unwrap();
            let perms = it.next().unwrap();
            let (a, b) = range.split_once('-').unwrap();
            let a = usize::from_str_radix(a, 16).unwrap();
            let b = usize::from_str_radix(b, 16).unwrap();
            if a <= addr && addr < b {
                return perms.to_string();
            }
        }
        panic!("{addr:#x} is not mapped");
    }
    let space = Space::for_mode(WxMode::Protect);
    let mut mem = commit(&space, WxMode::Protect);
    let x = mem.exec_base();
    assert_eq!(perms(x), "r-xp");
    let w = mem.open_write(0, 16).unwrap();
    assert_eq!(perms(x), "rw-p");
    w.close().unwrap();
    assert_eq!(perms(x), "r-xp");
    drop(mem);
    assert_eq!(
        perms(x),
        "---p",
        "release makes the pages inaccessible again"
    );

    let space = Space::for_mode(WxMode::DualMap);
    let mut mem = commit(&space, WxMode::DualMap);
    let (x, wr) = (mem.exec_base(), mem.write);
    assert_ne!(x, wr);
    assert!(
        !space.range().contains(wr),
        "the RW view lies outside the code range"
    );
    assert_eq!((perms(x), perms(wr)), ("r-xs".into(), "rw-s".into()));
    let w = mem.open_write(0, 16).unwrap();
    assert_eq!((perms(x), perms(wr)), ("r-xs".into(), "rw-s".into()));
    w.close().unwrap();
    drop(mem);
    assert_eq!(perms(x), "---p");
}

/// Page protections from VirtualQuery.
#[cfg(windows)]
#[test]
fn windows_page_protections() {
    use windows_sys::Win32::System::Memory::{
        MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_READWRITE, VirtualQuery,
    };
    fn protect(addr: usize) -> u32 {
        // SAFETY: MEMORY_BASIC_INFORMATION is plain data; all-zero is valid.
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: VirtualQuery writes at most the given size into `mbi`.
        let n = unsafe {
            VirtualQuery(
                addr as *const _,
                &mut mbi,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        assert_ne!(n, 0);
        mbi.Protect
    }
    let space = Space::for_mode(WxMode::Protect);
    let mut mem = commit(&space, WxMode::Protect);
    let x = mem.exec_base();
    assert_eq!(protect(x), PAGE_EXECUTE_READ);
    let w = mem.open_write(0, 16).unwrap();
    assert_eq!(protect(x), PAGE_READWRITE);
    w.close().unwrap();
    assert_eq!(protect(x), PAGE_EXECUTE_READ);
}

// ---------------------------------------------------------------------------------------------
// A generated function with a real frame that calls back into Rust:
//   u64 f(u64 x, u64 (*cb)(u64)) { return cb(x + 1) + x; }
// It keeps `x` in a callee-saved register across the call, so the unwind info must describe
// a saved register, not only the return address.

/// Offset of the return address of the call to `cb`.
#[cfg(target_arch = "aarch64")]
const RET_OFF: usize = 24;
#[cfg(all(target_arch = "x86_64", not(windows)))]
const RET_OFF: usize = 18;
#[cfg(all(target_arch = "x86_64", windows))]
const RET_OFF: usize = 14;

fn frame_code() -> Vec<u8> {
    #[cfg(target_arch = "aarch64")]
    {
        [
            0xa9be_7bfdu32, //  0 stp x29, x30, [sp, #-32]!
            0x9100_03fd,    //  4 mov x29, sp
            0xa901_53f3,    //  8 stp x19, x20, [sp, #16]
            0xaa00_03f3,    // 12 mov x19, x0
            0x9100_0400,    // 16 add x0, x0, #1
            0xd63f_0020,    // 20 blr x1
            0x8b13_0000,    // 24 add x0, x0, x19
            0xa941_53f3,    // 28 ldp x19, x20, [sp, #16]
            0xa8c2_7bfd,    // 32 ldp x29, x30, [sp], #32
            0xd65f_03c0,    // 36 ret
        ]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect()
    }
    #[cfg(all(target_arch = "x86_64", not(windows)))]
    {
        #[rustfmt::skip]
        let v = vec![
            0x55,                   //  0 push rbp
            0x48, 0x89, 0xe5,       //  1 mov rbp, rsp
            0x53,                   //  4 push rbx
            0x48, 0x83, 0xec, 0x08, //  5 sub rsp, 8
            0x48, 0x89, 0xfb,       //  9 mov rbx, rdi
            0x48, 0x8d, 0x7f, 0x01, // 12 lea rdi, [rdi+1]
            0xff, 0xd6,             // 16 call rsi
            0x48, 0x01, 0xd8,       // 18 add rax, rbx
            0x48, 0x83, 0xc4, 0x08, // 21 add rsp, 8
            0x5b,                   // 25 pop rbx
            0x5d,                   // 26 pop rbp
            0xc3,                   // 27 ret
        ];
        v
    }
    #[cfg(all(target_arch = "x86_64", windows))]
    {
        #[rustfmt::skip]
        let v = vec![
            0x53,                   //  0 push rbx
            0x48, 0x83, 0xec, 0x20, //  1 sub rsp, 32 (shadow space)
            0x48, 0x89, 0xcb,       //  5 mov rbx, rcx
            0x48, 0x8d, 0x49, 0x01, //  8 lea rcx, [rcx+1]
            0xff, 0xd2,             // 12 call rdx
            0x48, 0x01, 0xd8,       // 14 add rax, rbx
            0x48, 0x83, 0xc4, 0x20, // 17 add rsp, 32
            0x5b,                   // 21 pop rbx
            0xc3,                   // 22 ret
        ];
        v
    }
}

/// The function's CFI after the CIE rules (every instruction boundary of prolog and epilogue).
#[cfg(unix)]
fn frame_cfi() -> unwind::cfi::CfiProgram {
    use unwind::cfi::reg;
    let mut p = unwind::cfi::CfiProgram::new();
    #[cfg(target_arch = "aarch64")]
    {
        use reg::aarch64::*;
        p.advance_loc(1)
            .def_cfa_offset(32)
            .offset(FP, 4)
            .offset(LR, 3)
            .advance_loc(1)
            .def_cfa_register(FP)
            .advance_loc(1)
            .offset(X19, 2)
            .offset(X20, 1)
            .advance_loc(5)
            .restore(X19)
            .restore(X20)
            .advance_loc(1)
            .def_cfa(SP, 0)
            .restore(FP)
            .restore(LR);
    }
    #[cfg(target_arch = "x86_64")]
    {
        use reg::x86_64::*;
        p.advance_loc(1)
            .def_cfa_offset(16)
            .offset(RBP, 2)
            .advance_loc(3)
            .def_cfa_register(RBP)
            .advance_loc(1)
            .offset(RBX, 3)
            .advance_loc(21)
            .restore(RBX)
            .advance_loc(1)
            .def_cfa(RSP, 8)
            .restore(RBP);
    }
    p
}

/// Unwind info at this offset of the code memory (Windows only).
#[cfg(windows)]
const UNWIND_INFO_OFF: usize = 64;

/// The registration for the frame function at offset 0 of `mem`.
#[cfg(unix)]
fn register_frame(mem: &mut CodeMemory<'_>) -> unwind::EhFrameRegistration {
    let mut b = unwind::cfi::EhFrameBuilder::new(&unwind::cfi::Cie::host());
    b.add_fde(mem.exec_base(), frame_code().len(), &frame_cfi());
    // SAFETY: the FDE describes the code at exec_base exactly; the caller drops the
    // registration before `mem`.
    unsafe { unwind::EhFrameRegistration::register(b.finish()) }.expect("register the FDE")
}

#[cfg(windows)]
fn register_frame(mem: &mut CodeMemory<'_>) -> unwind::FunctionTableRegistration {
    use unwind::win64::{RuntimeFunction, UnwindInfoBuilder, reg};
    let info = UnwindInfoBuilder::new()
        .push_nonvol(1, reg::RBX)
        .alloc(5, 32)
        .encode();
    write(mem, UNWIND_INFO_OFF, &info);
    let entry = RuntimeFunction {
        begin: 0,
        end: frame_code().len() as u32,
        unwind_info: UNWIND_INFO_OFF as u32,
    };
    let range = mem.exec_base()..mem.exec_base() + mem.len();
    // SAFETY: the UNWIND_INFO describes the prolog of the code at offset 0 exactly; the caller
    // drops the registration before `mem`.
    unsafe { unwind::FunctionTableRegistration::register(range, Box::new([entry])) }
        .expect("register the function table")
}

type Callback = extern "C-unwind" fn(u64) -> u64;
type FrameFn = unsafe extern "C-unwind" fn(u64, Callback) -> u64;

thread_local! {
    static WALK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

extern "C-unwind" fn walk_cb(x: u64) -> u64 {
    let ips = unwind::native_return_addresses(128);
    WALK.with(|w| *w.borrow_mut() = ips);
    x * 10
}

struct Marker(u64);

extern "C-unwind" fn panic_cb(x: u64) -> u64 {
    // resume_unwind skips the panic hook, so the test output stays clean.
    std::panic::resume_unwind(Box::new(Marker(x)))
}

#[inline(never)]
fn call_frame(f: FrameFn, x: u64, cb: Callback) -> u64 {
    // SAFETY: `f` is the frame function, written and registered by the caller.
    std::hint::black_box(unsafe { f(x, cb) })
}

#[test]
fn unwind_through_generated_frame() {
    for &mode in WxMode::available() {
        let space = Space::for_mode(mode);
        let mut mem = commit(&space, mode);
        check_unwind(&mut mem, mode);
    }
}

/// Writes the frame function at offset 0 of `mem`, registers its unwind info, calls it, walks
/// through it and unwinds a panic through it.
fn check_unwind(mem: &mut CodeMemory<'_>, mode: WxMode) {
    write(mem, 0, &frame_code());
    let reg = register_frame(mem);
    // SAFETY: offset 0 holds the frame function with this signature.
    let f: FrameFn = unsafe { std::mem::transmute(mem.exec_addr(0)) };

    // 1. A plain call: cb(5 + 1) + 5 = 65.
    assert_eq!(call_frame(f, 5, walk_cb), 65, "{mode}");

    // 2. The system unwinder walks from the callback through the generated frame.
    let ips = WALK.with(|w| w.take());
    let ret = mem.exec_addr(RET_OFF);
    let at = ips.iter().position(|&ip| ip == ret).unwrap_or_else(|| {
        panic!("{mode}: walk misses the generated frame (return address {ret:#x}): {ips:x?}")
    });
    assert!(at >= 1, "{mode}: the callback frame must come first");
    let call_frame_addr = call_frame as fn(FrameFn, u64, Callback) -> u64 as usize;
    let next = ips[at + 1..].first().copied().unwrap_or(0);
    assert!(
        next > call_frame_addr && next - call_frame_addr < 4096,
        "{mode}: the frame after the generated one must return into call_frame \
         ({call_frame_addr:#x}), got {next:#x}: {ips:x?}"
    );
    assert!(
        ips.len() - at >= 4,
        "{mode}: the walk stops right after the generated frame: {ips:x?}"
    );

    // 3. A Rust panic unwinds through the generated frame and is caught here.
    let r = std::panic::catch_unwind(|| call_frame(f, 41, panic_cb));
    let payload = r.expect_err("the callback panics");
    let m = payload.downcast::<Marker>().expect("our payload");
    assert_eq!(m.0, 42, "{mode}");

    // 4. The function still works after the unwind.
    assert_eq!(call_frame(f, 1, walk_cb), 21, "{mode}");
    drop(reg);
}

// ---------------------------------------------------------------------------------------------
// The process code range (H07-Q6).

/// Every thread gets the same range: 128 MiB, below 2^47, outside the tsrs range.
#[test]
fn process_code_range_is_one_per_process() {
    let here = CodeRange::process().unwrap() as *const CodeRange as usize;
    let there: Vec<usize> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..2)
            .map(|_| s.spawn(|| CodeRange::process().unwrap() as *const CodeRange as usize))
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(there, [here, here], "two threads, one code range");
    let r = CodeRange::process().unwrap();
    r.self_check().unwrap();
    assert_eq!(r.size(), DEFAULT_CODE_RANGE_SIZE);
    assert!(r.end() <= CAGE_LIMIT, "{:#x} ends above 2^47", r.end());
    let k = r.range();
    assert!(
        k.end <= TSRS_RESERVATION.start || k.start >= TSRS_RESERVATION.end,
        "{k:x?} overlaps the tsrs range"
    );
    // Unit tests build `jit`: on macOS arm64 the range is one MAP_JIT mapping.
    assert_eq!(
        r.is_map_jit(),
        cfg!(all(target_vendor = "apple", target_arch = "aarch64"))
    );
}

/// Two cages without a code range, reserved on two threads, each run code in their own chunk
/// of the process code range at the same time.
#[test]
fn cages_on_two_threads_take_chunks() {
    let range = CodeRange::process().unwrap();
    let both = Barrier::new(2);
    let got: Vec<_> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..2u16)
            .map(|t| {
                let both = &both;
                s.spawn(move || {
                    let cage = Cage::reserve(&CageConfig {
                        cage_size: allocation_granularity(),
                        code_range_size: 0,
                        ..CageConfig::default()
                    })
                    .expect("reserve a cage");
                    assert!(cage.code_range().is_none(), "no per-cage code range");
                    let mut chunk = range.alloc(page_size()).expect("a chunk");
                    let mut mem = chunk.commit(WxMode::DEFAULT).expect("commit the chunk");
                    write(&mut mem, 0, &ret_const(300 + t));
                    // Both chunks are committed and hold code now.
                    both.wait();
                    // SAFETY: offset 0 holds a ret_const function, written just now.
                    let v = unsafe { call0(mem.exec_addr(0)) };
                    let at = mem.exec_base();
                    both.wait();
                    (at, v, cage.heap().range())
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let ps = page_size();
    for (t, (at, v, heap)) in got.iter().enumerate() {
        assert_eq!(*v, 300 + t as u64);
        assert!(range.contains(*at) && range.contains(at + ps - 1));
        assert!(!heap.contains(at), "code lies outside the cage");
    }
    let (a, b) = (got[0].0, got[1].0);
    assert!(
        a + ps <= b || b + ps <= a,
        "the chunks overlap: {a:#x} {b:#x}"
    );
}

/// Code in a chunk from the safe entry (`CodeChunk::commit`) runs and unwinds.
#[test]
fn process_chunk_code_is_callable_and_unwinds() {
    let range = CodeRange::process().unwrap();
    let mut chunk = range.alloc(page_size()).unwrap();
    let mut mem = chunk.commit(WxMode::DEFAULT).unwrap();
    assert!(range.contains(mem.exec_base()));
    check_unwind(&mut mem, WxMode::DEFAULT);
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    {
        drop(mem);
        let e = chunk.commit(WxMode::Protect).unwrap_err();
        assert_eq!(
            e.kind(),
            io::ErrorKind::InvalidInput,
            "MAP_JIT takes MapJit only"
        );
    }
}
