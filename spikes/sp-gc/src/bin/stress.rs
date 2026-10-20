//! SP-GC random-safepoint stress.
//!
//! ```text
//! stress [--abi aapcs64|sysv64|win64] [--no-spill] [--scan full|exact-only|no-end]
//!        [--safepoints N] [--seed S] [--expect-misses]
//! stress --list-abis      # prints the trampolines built for this target
//! ```
//!
//! Exit 0 when the result is the expected one: 0 missed live objects, or (with
//! `--expect-misses`, for the negative controls) at least one.

use std::process::ExitCode;
use std::time::Instant;

use ep9_platform::SpillAbi;
use sp_gc::heap::ScanOptions;
use sp_gc::mutator::{Config, SHAPES, Spill, stress};

fn main() -> ExitCode {
    let mut abi = SpillAbi::HOST;
    let mut spill = true;
    let mut scan = ScanOptions::FULL;
    let mut scan_name = "full";
    let mut n = 1_000_000u64;
    let mut seed = 0x5EED_0000_0000_0001u64;
    let mut expect_misses = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| panic!("{a} needs a value"));
        match a.as_str() {
            "--abi" => {
                let v = val();
                abi = *SpillAbi::available()
                    .iter()
                    .find(|x| x.name() == v)
                    .unwrap_or_else(|| panic!("ABI {v} is not built for this target"));
            }
            "--no-spill" => spill = false,
            "--scan" => {
                scan_name = match val().as_str() {
                    "full" => "full",
                    "exact-only" => "exact-only",
                    "no-end" => "no-end",
                    other => panic!("unknown scan mode {other}"),
                };
                scan = match scan_name {
                    "full" => ScanOptions::FULL,
                    "exact-only" => ScanOptions {
                        interior: false,
                        one_past_end: false,
                    },
                    _ => ScanOptions {
                        interior: true,
                        one_past_end: false,
                    },
                };
            }
            "--safepoints" => n = val().parse().expect("--safepoints N"),
            "--seed" => seed = val().parse().expect("--seed S"),
            "--expect-misses" => expect_misses = true,
            "--list-abis" => {
                let names: Vec<&str> = SpillAbi::available().iter().map(|a| a.name()).collect();
                println!("{}", names.join(" "));
                return ExitCode::SUCCESS;
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let mode = if spill {
        Spill::Trampoline(abi)
    } else {
        Spill::NoSpill(abi)
    };
    let t = Instant::now();
    let (s, h) = stress(Config::stress(mode, scan, seed), n);
    let secs = t.elapsed().as_secs_f64();
    let by_shape: Vec<String> = SHAPES
        .iter()
        .zip(s.missed_by_shape)
        .filter(|(_, m)| *m > 0)
        .map(|(name, m)| format!("{name}={m}"))
        .collect();
    println!(
        "sp-gc stress: target={}-{} abi={} spill={} scan={} seed={seed:#x}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        abi.name(),
        if spill { "trampoline" } else { "none" },
        scan_name
    );
    println!(
        "  safepoints={} frames={} max_depth={} live_checks={} max_live={} MISSED={} [{}]",
        s.safepoints,
        s.frames,
        s.max_depth,
        s.checked,
        s.max_live,
        s.missed,
        by_shape.join(" ")
    );
    println!(
        "  roots found: spill_area_only={} stack_only={} both={} precise={} traced_only={} | exact={} interior={} one_past_end={}",
        s.roots_spill_only,
        s.roots_stack_only,
        s.roots_spill_and_stack,
        s.roots_precise,
        s.roots_traced_only,
        s.roots_exact,
        s.roots_interior,
        s.roots_end
    );
    println!(
        "  heap: allocated={} freed={} sweeps={} live_at_end={} used={} KiB scanned_words={} time={secs:.2}s",
        h.allocated,
        h.freed,
        h.sweeps,
        h.live_at_end,
        h.heap_used >> 10,
        h.scanned_words
    );
    let hist: Vec<String> = s
        .depth_hist
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(d, n)| format!("{d}:{n}"))
        .collect();
    println!("  safepoints by frame depth: {}", hist.join(" "));
    for m in &s.first_misses {
        println!("  miss: {m}");
    }
    let ok = if expect_misses {
        s.missed > 0
    } else {
        s.missed == 0
    };
    println!(
        "  RESULT: {} ({})",
        if ok { "PASS" } else { "FAIL" },
        if expect_misses {
            "negative control: misses expected"
        } else {
            "0 missed live objects expected"
        }
    );
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
