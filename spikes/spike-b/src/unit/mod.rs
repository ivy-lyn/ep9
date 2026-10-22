//! `spike-b unit`: SP-Unit (M3-P1; moved from M0 month 2 by M2-D21-FAIL). The 11 Spike B
//! opcodes as AOT unit text on placement (b), behind the frozen [`crate::emit`] `Emitter` and
//! `Finalize` traits: build, load, call, unwind, GC scan and fall back on a mismatch (09 §4,
//! M0 exit item 11; H34, H35).
//!
//! ```text
//! source --ep9_frontend--> CompiledUnit --UnitEmitter (Emitter)--> FunctionCode per function
//!   --UnitFinalizer (Finalize)--> unit bytes: ep9_unit header v0 | text | rodata |
//!                                 rw_template | bytecode | source | unwind
//! load (per isolate): ep9_unit::verdict -> LOAD | BYTECODE | SOURCE | LOAD ERROR
//!   LOAD: text mapped once per process (0 relocations: the file's bytes run as they are)
//!         -> RW copy per isolate (link cells: strong root range; IC cells: weak root range)
//!         -> cx.unit_data[slot] = the copy -> install_native_b per JS function
//!   unit code: x27 / r14 = cx.unit_data[slot] on entry (saved and restored), then
//!              [x27 + ...]: the runtime table (an external reference), IC cells, counters
//! ```
//!
//! | Module | Contents |
//! |---|---|
//! | [`emit`] | `UnitEmitter` (the frozen `Emitter`), the RW layout |
//! | [`a64`], [`x64`] | the code generators (placement (b) code + the unit-data register) |
//! | [`format`](mod@format) | `UnitFinalizer` (the frozen `Finalize`), the unit reader, the spike host |
//! | [`build`] | source -> unit bytes |
//! | [`load`] | the loader: process text registry, unwind self-test, RW copies, install, fallback |
//! | [`scan`] | the relocation scan of unit text |
//! | [`checks`] | the gate checks of `spike-b unit` (gate.sh unit) and `tests/unit.rs` |
//!
//! `spike-b unit` with no argument runs every check; `spike-b unit <name>` runs one.

pub mod a64;
pub mod build;
pub mod checks;
pub mod emit;
pub mod format;
pub mod load;
pub mod scan;
pub mod x64;

use std::time::Instant;

/// Entry of `spike-b unit [<check>]` (`main.rs` dispatches here). Returns the exit status:
/// 0 if every selected check passed, 1 if one failed, 2 on a usage error.
pub fn main(args: &[String]) -> i32 {
    let selected: Vec<&checks::Check> = match args.first().map(String::as_str) {
        None | Some("all") => checks::CHECKS.iter().collect(),
        Some(name) => match checks::CHECKS.iter().find(|c| c.name == name) {
            Some(c) => vec![c],
            None => {
                let names: Vec<&str> = checks::CHECKS.iter().map(|c| c.name).collect();
                eprintln!(
                    "spike-b unit: unknown check {name:?} (all, {})",
                    names.join(", ")
                );
                return 2;
            }
        },
    };
    let scale = checks::Scale::Gate;
    let mut failed = 0;
    if let Some(r) = checks::refusal() {
        for c in &selected {
            println!(
                "check ({}) SKIP: {}: {r} ({})",
                c.name,
                c.title,
                std::env::consts::ARCH
            );
        }
        println!("unit: SKIP (no unit code generator for this target)");
        return 0;
    }
    for c in selected {
        let start = Instant::now();
        match checks::on_thread(c.run, scale) {
            Ok(lines) => {
                println!(
                    "check ({}) PASS: {} [{:.1?}]",
                    c.name,
                    c.title,
                    start.elapsed()
                );
                for l in lines {
                    println!("    {l}");
                }
            }
            Err(e) => {
                failed += 1;
                println!("check ({}) FAIL: {}", c.name, c.title);
                for l in e.lines() {
                    println!("    {l}");
                }
            }
        }
    }
    println!(
        "unit: {} on {} ({}, {})",
        if failed == 0 { "PASS" } else { "FAIL" },
        std::env::consts::ARCH,
        std::env::consts::OS,
        crate::contracts_line()
    );
    i32::from(failed != 0)
}
