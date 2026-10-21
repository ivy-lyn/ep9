//! SP-JIT-mem (M0 week 3; docs/plan/09-milestones.md section 4; H28, H27).
//!
//! For each W^X mode of the host ([`WxMode::available`]):
//!
//! ```text
//!   main -> run_mode -> call_generated -> [outer] -> [inner] -> callback (Rust)
//!                                          generated code, unwind data registered
//!   call ......... the generated code returns cb(x + 1) + x
//!   icache ....... rewrite an immediate in `inner` 1000 times; each call sees the new value
//!   bt-std ....... std::backtrace from the callback: callback, inner, outer, call_generated, main
//!   bt-raw ....... the system unwinder gives the exact return addresses into inner and outer
//!   callee-saved . the walk restores x19/rbx = 0xbeef in the outer frame (inner changed it)
//!   panic ........ a panic in the callback unwinds through both frames to catch_unwind
//!   wx-toggle .... cost of a write-window open + close (no write), with a write + flush, and
//!                  of the flush alone
//! ```
//!
//! Output: one `RESULT <check>/<mode> PASS|FAIL|SKIP <detail>` line per check,
//! `MEASURE wx-toggle/<mode> ...` per mode, then `SUMMARY PASS|FAIL`. Exit code 1 on any FAIL.
//! Flags: `--iters N` (bench iterations per batch), `--no-bench`.

mod code;
mod walk;

use std::cell::RefCell;
use std::process::ExitCode;
use std::time::Instant;

use ep9_platform::jit::{CodeMemory, WxMode, flush_icache};
use ep9_platform::{CodeRange, Reservation};

type Callback = extern "C-unwind" fn(u64) -> u64;
type InnerFn = unsafe extern "C-unwind" fn(u64, Callback) -> u64;
type OuterFn = unsafe extern "C-unwind" fn(u64, Callback, InnerFn) -> u64;

const CODE_BYTES: usize = 64 << 10;
#[cfg(windows)]
const INNER_INFO_OFF: usize = 512;
#[cfg(windows)]
const OUTER_INFO_OFF: usize = 520;

struct Report {
    failed: bool,
}

impl Report {
    fn line(&mut self, check: &str, mode: WxMode, status: &str, detail: impl AsRef<str>) {
        if status == "FAIL" {
            self.failed = true;
        }
        println!("RESULT {check}/{mode} {status} {}", detail.as_ref());
    }

    fn check(&mut self, check: &str, mode: WxMode, ok: bool, detail: impl AsRef<str>) {
        self.line(check, mode, if ok { "PASS" } else { "FAIL" }, detail);
    }
}

// ---- callbacks and what they capture ----------------------------------------------------

#[derive(Default)]
struct Capture {
    std_bt: String,
    raw: Vec<usize>,
    regs: Vec<(usize, u64)>,
}

thread_local! {
    static CAPTURE: RefCell<Capture> = RefCell::new(Capture::default());
}

extern "C-unwind" fn cb_plain(x: u64) -> u64 {
    x * 10
}

extern "C-unwind" fn cb_capture(x: u64) -> u64 {
    let std_bt = format!("{:#}", std::backtrace::Backtrace::force_capture());
    let raw = ep9_platform::unwind::native_return_addresses(256);
    let regs = walk::frames_with_saved_reg(256);
    CAPTURE.with(|c| {
        *c.borrow_mut() = Capture { std_bt, raw, regs };
    });
    x * 10
}

struct Marker(u64);

extern "C-unwind" fn cb_panic(x: u64) -> u64 {
    // A real panic: it runs the hook (one stderr line) and the full unwind machinery.
    std::panic::panic_any(Marker(x))
}

/// The only Rust frame between the checks and the generated code.
#[inline(never)]
fn call_generated(outer: OuterFn, x: u64, cb: Callback, inner: InnerFn) -> u64 {
    // SAFETY: `outer` and `inner` point to the generated functions of this signature, written
    // and registered by `run_mode`.
    std::hint::black_box(unsafe { outer(std::hint::black_box(x), cb, inner) })
}

// ---- std::backtrace parsing ---------------------------------------------------------------

/// `(address, name)` per frame line of `{:#}` output: `  12:   0x1234 - crate[hash]::f`.
fn parse_std_backtrace(s: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let t = line.trim_start();
        let Some((idx, rest)) = t.split_once(':') else {
            continue;
        };
        if idx.is_empty() || !idx.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Some((addr, name)) = rest.trim().split_once(" - ") else {
            continue;
        };
        let Some(hex) = addr.trim().strip_prefix("0x") else {
            continue;
        };
        let Ok(a) = usize::from_str_radix(hex, 16) else {
            continue;
        };
        out.push((a, strip_hashes(name.trim())));
    }
    out
}

/// The Rust `main` of this binary. With `debug = "line-tables-only"` the symbolizer gives the
/// short name (`main`); with the symbol table it gives `sp_jit_mem::main`. `_main` is the C one.
fn is_main(name: &str) -> bool {
    name == "main" || name == "sp_jit_mem::main"
}

/// `sp_jit_mem[1a2b]::main` -> `sp_jit_mem::main`; drops a trailing `::h0123..` too.
fn strip_hashes(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    let mut skip = false;
    for ch in name.chars() {
        match ch {
            '[' => skip = true,
            ']' if skip => skip = false,
            _ if !skip => s.push(ch),
            _ => {}
        }
    }
    if let Some(i) = s.rfind("::h")
        && s[i + 3..].len() == 16
        && s[i + 3..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        s.truncate(i);
    }
    s
}

// ---- one mode -------------------------------------------------------------------------------

fn under_rosetta() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && std::fs::read_to_string("/proc/cpuinfo").is_ok_and(|s| s.contains("VirtualApple"))
}

fn write_at(mem: &mut CodeMemory<'_>, offset: usize, bytes: &[u8]) {
    let mut w = mem
        .open_write(offset, bytes.len())
        .expect("open a write window");
    w.bytes().copy_from_slice(bytes);
    w.close().expect("close the write window");
}

fn run_mode(mode: WxMode, rep: &mut Report, iters: Option<usize>) {
    // MapJit and every mode off macOS arm64 use a chunk of the process code range. Protect on
    // macOS arm64 needs a plain range (MAP_JIT pages cannot be mprotected), so it runs outside.
    let in_range =
        mode.needs_map_jit() || !cfg!(all(target_vendor = "apple", target_arch = "aarch64"));
    // Both outlive `mem` (declared before it).
    let mut chunk;
    let plain;
    let (mut mem, place) = if in_range {
        let range = CodeRange::process().expect("reserve the process code range");
        chunk = range.alloc(CODE_BYTES).expect("a chunk of the code range");
        let mem = chunk.commit(mode).expect("commit code memory");
        let place = format!(
            "a chunk of the process code range {:#x}..{:#x}",
            range.base(),
            range.end()
        );
        (mem, place)
    } else {
        plain = Reservation::reserve(1 << 20).expect("reserve 1 MiB");
        // SAFETY: the range is new; this is its only code memory.
        let mem =
            unsafe { CodeMemory::commit(&plain, 0, CODE_BYTES, mode) }.expect("commit code memory");
        let place = format!(
            "a plain reservation {:#x}..{:#x}",
            plain.base(),
            plain.end()
        );
        (mem, place)
    };
    let (inner_addr, outer_addr) = (
        mem.exec_addr(code::INNER_OFF),
        mem.exec_addr(code::OUTER_OFF),
    );
    let (inner_ret, outer_ret) = (inner_addr + code::INNER_RET, outer_addr + code::OUTER_RET);
    println!(
        "mode {mode}: code at {:#x}, in {place}; inner {inner_addr:#x}, outer {outer_addr:#x}",
        mem.exec_base(),
    );

    write_at(&mut mem, code::INNER_OFF, &code::inner());
    write_at(&mut mem, code::OUTER_OFF, &code::outer());

    #[cfg(unix)]
    // SAFETY: the FDEs describe the two functions exactly; `reg` drops before `mem`.
    let reg = unsafe {
        ep9_platform::unwind::EhFrameRegistration::register(code::eh_frame(inner_addr, outer_addr))
    }
    .expect("register .eh_frame");
    #[cfg(windows)]
    let reg = {
        use ep9_platform::unwind::FunctionTableRegistration;
        use ep9_platform::unwind::win64::RuntimeFunction;
        let info = code::unwind_info();
        write_at(&mut mem, INNER_INFO_OFF, &info);
        write_at(&mut mem, OUTER_INFO_OFF, &info);
        let entries = [
            RuntimeFunction {
                begin: code::INNER_OFF as u32,
                end: (code::INNER_OFF + code::inner().len()) as u32,
                unwind_info: INNER_INFO_OFF as u32,
            },
            RuntimeFunction {
                begin: code::OUTER_OFF as u32,
                end: (code::OUTER_OFF + code::outer().len()) as u32,
                unwind_info: OUTER_INFO_OFF as u32,
            },
        ];
        let range = mem.exec_base()..mem.exec_base() + mem.len();
        // SAFETY: the UNWIND_INFO describes both prologs exactly; `reg` drops before `mem`.
        unsafe { FunctionTableRegistration::register(range, Box::new(entries)) }
            .expect("register the function table")
    };

    // SAFETY: the offsets hold the generated functions with these signatures.
    let (inner, outer): (InnerFn, OuterFn) = unsafe {
        (
            std::mem::transmute::<usize, InnerFn>(inner_addr),
            std::mem::transmute::<usize, OuterFn>(outer_addr),
        )
    };

    // call
    let got = call_generated(outer, 5, cb_plain, inner);
    // SAFETY: `inner` is the generated function; the callback is plain Rust.
    let got_inner = unsafe { inner(5, cb_plain) };
    rep.check(
        "call",
        mode,
        got == 65 && got_inner == 65,
        format!("outer(5) = {got}, inner(5) = {got_inner}, want 65"),
    );

    // icache: rewrite the immediate of `add x0, x0, #k` / `lea reg, [reg + k]`.
    if mode == WxMode::DualMap && under_rosetta() {
        rep.line(
            "icache",
            mode,
            "SKIP",
            "Rosetta keeps its translation of the RX view after a write through the RW view",
        );
    } else {
        let rounds = 1000u32;
        let mut bad = 0;
        let mut first_bad = String::new();
        for r in 0..rounds {
            let k = 1 + r % code::IMM_MAX;
            let (off, bytes) = code::inner_imm_patch(k);
            write_at(&mut mem, code::INNER_OFF + off, &bytes);
            let got = call_generated(outer, 5, cb_plain, inner);
            let want = (5 + u64::from(k)) * 10 + 5;
            if got != want {
                if bad == 0 {
                    first_bad = format!(" (first: k={k} got {got} want {want})");
                }
                bad += 1;
            }
        }
        let (off, bytes) = code::inner_imm_patch(1);
        write_at(&mut mem, code::INNER_OFF + off, &bytes);
        rep.check(
            "icache",
            mode,
            bad == 0,
            format!("{rounds} rewrites, {bad} stale calls{first_bad}"),
        );
    }

    // Backtraces and the saved register, all captured inside one callback.
    let got = call_generated(outer, 7, cb_capture, inner);
    let cap = CAPTURE.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let frames = parse_std_backtrace(&cap.std_bt);
    let names: Vec<&str> = frames.iter().map(|(_, n)| n.as_str()).collect();
    let pos = frames.iter().position(|&(a, _)| a == inner_ret);
    let bt_ok = got == 87
        && pos.is_some_and(|i| {
            i >= 1
                && names[i - 1].ends_with("cb_capture")
                && frames.get(i + 1).is_some_and(|f| f.0 == outer_ret)
                && names
                    .get(i + 2)
                    .is_some_and(|n| n.ends_with("call_generated"))
                && names[i..].iter().any(|n| is_main(n))
        });
    let to_main = pos.and_then(|i| names[i..].iter().position(|n| is_main(n)).map(|j| j + 1));
    let detail = match (pos, to_main) {
        (Some(i), Some(m)) => format!(
            "result {got} (want 87); {} frames; generated frames at {i} and {} ({:?}, {:?}); {m} frames from inner to main",
            frames.len(),
            i + 1,
            names[i],
            names.get(i + 1).copied().unwrap_or("-"),
        ),
        _ => format!("generated frame {inner_ret:#x} not found or order wrong"),
    };
    rep.check("bt-std", mode, bt_ok, detail);
    if !bt_ok {
        println!("std backtrace (frame list):");
        for (i, (a, n)) in frames.iter().enumerate() {
            println!("  {i:3} {a:#x} {n}");
        }
    }

    let raw = &cap.raw;
    let rpos = raw.iter().position(|&ip| ip == inner_ret);
    let raw_ok =
        rpos.is_some_and(|i| i >= 1 && raw.get(i + 1) == Some(&outer_ret) && raw.len() - i >= 5);
    rep.check(
        "bt-raw",
        mode,
        raw_ok,
        match rpos {
            Some(i) => format!(
                "{} frames; return addresses {inner_ret:#x} at {i}, {outer_ret:#x} at {}",
                raw.len(),
                i + 1
            ),
            None => format!("{inner_ret:#x} not in {raw:x?}"),
        },
    );

    let inner_reg = cap.regs.iter().find(|f| f.0 == inner_ret).map(|f| f.1);
    let outer_reg = cap.regs.iter().find(|f| f.0 == outer_ret).map(|f| f.1);
    rep.check(
        "callee-saved",
        mode,
        inner_reg == Some(7) && outer_reg == Some(code::MARK),
        format!(
            "x19/rbx in inner frame = {inner_reg:x?} (want 7), in outer frame = {outer_reg:x?} (want {:#x})",
            code::MARK
        ),
    );

    // panic: unwinds from the callback through inner and outer into catch_unwind.
    let r = std::panic::catch_unwind(|| call_generated(outer, 41, cb_panic, inner));
    let caught = match r {
        Err(p) => p.downcast::<Marker>().map(|m| m.0).ok(),
        Ok(_) => None,
    };
    let after = call_generated(outer, 5, cb_plain, inner);
    rep.check(
        "panic",
        mode,
        caught == Some(42) && after == 65,
        format!("caught payload {caught:?} (want 42); next call = {after} (want 65)"),
    );

    if let Some(n) = iters {
        bench(&mut mem, mode, n);
    }

    // Negative control: without the registration the walk must not restore x19/rbx = MARK in
    // the outer frame. This shows that the checks above see the registered data, not a
    // frame-pointer fallback of the unwinder.
    drop(reg);
    control(mode, rep, outer, inner, inner_ret, outer_ret);
}

#[cfg(unix)]
fn control(
    mode: WxMode,
    rep: &mut Report,
    outer: OuterFn,
    inner: InnerFn,
    inner_ret: usize,
    outer_ret: usize,
) {
    let got = call_generated(outer, 7, cb_walk_only, inner);
    let regs = CAPTURE.with(|c| std::mem::take(&mut c.borrow_mut().regs));
    let at_inner = regs.iter().any(|f| f.0 == inner_ret);
    let outer_reg = regs.iter().find(|f| f.0 == outer_ret).map(|f| f.1);
    let missed = outer_reg != Some(code::MARK);
    rep.check(
        "control-no-unwind-info",
        mode,
        got == 87 && missed,
        format!(
            "without registration: {} frames, inner frame {}, outer frame {} (must not give {:#x})",
            regs.len(),
            if at_inner { "seen" } else { "not seen" },
            match outer_reg {
                Some(v) => format!("seen with x19/rbx = {v:#x}"),
                None => "not seen".into(),
            },
            code::MARK
        ),
    );
}

/// Windows: without a function table the walk treats the generated frame as a leaf and reads
/// a return address from the shadow space, so it would follow garbage. No control there.
#[cfg(windows)]
fn control(
    mode: WxMode,
    rep: &mut Report,
    _outer: OuterFn,
    _inner: InnerFn,
    _inner_ret: usize,
    _outer_ret: usize,
) {
    rep.line(
        "control-no-unwind-info",
        mode,
        "SKIP",
        "an unregistered x64 frame looks like a leaf; the walk would read garbage",
    );
}

#[cfg(unix)]
extern "C-unwind" fn cb_walk_only(x: u64) -> u64 {
    let regs = walk::frames_with_saved_reg(256);
    CAPTURE.with(|c| c.borrow_mut().regs = regs);
    x * 10
}

// ---- W^X toggle cost --------------------------------------------------------------------

/// Median of 5 batches, in ns per operation.
fn median_ns(n: usize, mut op: impl FnMut()) -> f64 {
    let mut v: Vec<f64> = (0..5)
        .map(|_| {
            let t = Instant::now();
            for _ in 0..n {
                op();
            }
            t.elapsed().as_nanos() as f64 / n as f64
        })
        .collect();
    v.sort_by(f64::total_cmp);
    v[2]
}

fn bench(mem: &mut CodeMemory<'_>, mode: WxMode, iters: usize) {
    let n = match mode {
        WxMode::Protect => (iters / 10).max(1),
        _ => iters,
    };
    let off = code::INNER_OFF;
    let (patch_off, patch) = code::inner_imm_patch(1);
    let pair = median_ns(n, || {
        let w = mem.open_write(off, 16).expect("open");
        w.close().expect("close");
    });
    let write_flush = median_ns(n, || {
        let mut w = mem.open_write(off + patch_off, patch.len()).expect("open");
        w.bytes().copy_from_slice(&patch);
        w.close().expect("close");
    });
    let addr = mem.exec_addr(off);
    let flush = median_ns(n, || flush_icache(addr, 16));
    println!(
        "MEASURE wx-toggle/{mode} pair_ns={pair:.1} write_flush_ns={write_flush:.1} flush_ns={flush:.1} iters={n}"
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut iters = Some(100_000usize);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--no-bench" => iters = None,
            "--iters" => {
                i += 1;
                iters = Some(args.get(i).and_then(|s| s.parse().ok()).expect("--iters N"));
            }
            other => {
                eprintln!("unknown argument {other}; flags: --iters N, --no-bench");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    // Keep the expected panic of the `panic` check to one short line.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().downcast_ref::<Marker>().is_some() {
            eprintln!("(expected panic from the callback: unwinding through generated code)");
        } else {
            default_hook(info);
        }
    }));

    println!(
        "SP-JIT-mem: {} {}, page {} B, modes {:?}{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        ep9_platform::page_size(),
        WxMode::available()
            .iter()
            .map(|m| m.name())
            .collect::<Vec<_>>(),
        if under_rosetta() {
            " (under Rosetta)"
        } else {
            ""
        },
    );
    let mut rep = Report { failed: false };
    for &mode in WxMode::available() {
        run_mode(mode, &mut rep, iters);
    }
    println!("SUMMARY {}", if rep.failed { "FAIL" } else { "PASS" });
    if rep.failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_std_backtrace_lines() {
        let s = "   3:        0x1023d4b64 - bt[e0602aae86402061]::inner\n                               at /x.rs:2:40\n  10:     0x7ff6 - <unknown>\n  11: 0x10 - sp_jit_mem::main::h0123456789abcdef\n";
        assert_eq!(
            parse_std_backtrace(s),
            [
                (0x1023d4b64, "bt::inner".to_string()),
                (0x7ff6, "<unknown>".to_string()),
                (0x10, "sp_jit_mem::main".to_string()),
            ]
        );
    }

    #[test]
    fn code_sizes_fit_their_slots() {
        assert!(code::inner().len() <= code::OUTER_OFF);
        assert!(code::OUTER_OFF + code::outer().len() <= 256);
    }
}
