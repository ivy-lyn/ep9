//! The C-20 measure (doc 13 section 9, "Unit-data slot rule"; H35-Q9): the cost of the
//! unit-data entry of a unit function under two slot rules, on hand-written AArch64 stub code
//! (no JS runs; plan v12 rule 71).
//!
//! ```text
//!  fixed slot (H35-Q9 as built, the C-20 pick): the slot is fixed when the unit is built
//!      ldr x9,  [x28, #CX_UNIT_DATA]      the isolate's unit-data directory
//!      ldr x27, [x9, #8 * slot]           the unit's RW copy
//!  slot word (the doc 13 option "a map-time slot word in an RW page"):
//!      ldr w12, [slot word]               per process, written at map time
//!      ldr x9,  [x28, #CX_UNIT_DATA]
//!      ldr x27, [x9, x12, lsl #3]
//!  then the first RW access: ldr x11, [x27]
//! ```
//!
//! Two forms per rule: "chain" makes each entry depend on the last RW read (the latency of a
//! cold entry), "stream" has no dependency between entries (the throughput of a call-heavy
//! loop). Each form runs `N` entries 7 times; the report prints the median ns per entry.
//! Wall time on a loaded Mac is a report number, never a gate reading.
//!
//! `cargo run --release -p ep9_unit --example unit_data_entry`

#[cfg(target_arch = "aarch64")]
mod a64 {
    use core::arch::asm;

    /// The isolate word that holds the directory (the `Cx` field), the directory, an RW copy
    /// whose first word points back at the isolate word (so "chain" can follow it), and the
    /// slot word.
    #[repr(C, align(64))]
    pub struct World {
        pub cx: [u64; 4],
        pub dir: [u64; 64],
        pub rw: [u64; 8],
        pub slot_word: u32,
    }

    pub const SLOT: usize = 5;

    pub fn world() -> Box<World> {
        let mut w = Box::new(World {
            cx: [0; 4],
            dir: [0; 64],
            rw: [0; 8],
            slot_word: SLOT as u32,
        });
        let dir = w.dir.as_ptr() as u64;
        let cx = w.cx.as_ptr() as u64;
        w.cx[2] = dir; // CX_UNIT_DATA at +16 in this stub world
        w.dir[SLOT] = w.rw.as_ptr() as u64;
        w.rw[0] = cx;
        w
    }

    /// Fixed slot, chain form: `n` entries, each starting at the isolate word the last RW
    /// read gave.
    pub fn fixed_chain(w: &World, n: u64) -> u64 {
        let mut cx = w.cx.as_ptr() as u64;
        // SAFETY: the loads read `w` only: cx -> dir -> rw -> cx, all inside the box.
        unsafe {
            asm!(
                "2:",
                "ldr x9, [{cx}, #16]",
                "ldr x10, [x9, #{off}]",
                "ldr {cx}, [x10]",
                "subs {n}, {n}, #1",
                "b.ne 2b",
                cx = inout(reg) cx,
                n = inout(reg) n => _,
                off = const SLOT * 8,
                out("x9") _, out("x10") _,
                options(nostack, readonly),
            );
        }
        cx
    }

    /// Slot word, chain form.
    pub fn word_chain(w: &World, n: u64) -> u64 {
        let mut cx = w.cx.as_ptr() as u64;
        let sw = &w.slot_word as *const u32 as u64;
        // SAFETY: as above, plus the slot word inside the box.
        unsafe {
            asm!(
                "2:",
                "ldr w12, [{sw}]",
                "ldr x9, [{cx}, #16]",
                "ldr x10, [x9, x12, lsl #3]",
                "ldr {cx}, [x10]",
                "subs {n}, {n}, #1",
                "b.ne 2b",
                cx = inout(reg) cx,
                n = inout(reg) n => _,
                sw = in(reg) sw,
                out("x9") _, out("x10") _, out("x12") _,
                options(nostack, readonly),
            );
        }
        cx
    }

    /// Fixed slot, stream form: every entry starts at the same isolate word.
    pub fn fixed_stream(w: &World, n: u64) -> u64 {
        let cx = w.cx.as_ptr() as u64;
        let mut acc = 0u64;
        // SAFETY: as above.
        unsafe {
            asm!(
                "2:",
                "ldr x9, [{cx}, #16]",
                "ldr x10, [x9, #{off}]",
                "ldr x11, [x10]",
                "add {acc}, {acc}, x11",
                "subs {n}, {n}, #1",
                "b.ne 2b",
                cx = in(reg) cx,
                acc = inout(reg) acc,
                n = inout(reg) n => _,
                off = const SLOT * 8,
                out("x9") _, out("x10") _, out("x11") _,
                options(nostack, readonly),
            );
        }
        acc
    }

    /// Slot word, stream form.
    pub fn word_stream(w: &World, n: u64) -> u64 {
        let cx = w.cx.as_ptr() as u64;
        let sw = &w.slot_word as *const u32 as u64;
        let mut acc = 0u64;
        // SAFETY: as above.
        unsafe {
            asm!(
                "2:",
                "ldr w12, [{sw}]",
                "ldr x9, [{cx}, #16]",
                "ldr x10, [x9, x12, lsl #3]",
                "ldr x11, [x10]",
                "add {acc}, {acc}, x11",
                "subs {n}, {n}, #1",
                "b.ne 2b",
                cx = in(reg) cx,
                sw = in(reg) sw,
                acc = inout(reg) acc,
                n = inout(reg) n => _,
                out("x9") _, out("x10") _, out("x11") _, out("x12") _,
                options(nostack, readonly),
            );
        }
        acc
    }
}

/// One measured form: it runs `n` entries and returns the value that proves they ran.
#[cfg(target_arch = "aarch64")]
type Form = fn(&a64::World, u64) -> u64;

#[cfg(target_arch = "aarch64")]
fn main() {
    use std::time::Instant;
    const N: u64 = 200_000_000;
    const REPS: usize = 7;
    let w = a64::world();
    let forms: [(&str, Form); 4] = [
        ("fixed slot, chain", a64::fixed_chain),
        ("slot word, chain", a64::word_chain),
        ("fixed slot, stream", a64::fixed_stream),
        ("slot word, stream", a64::word_stream),
    ];
    // Both chain forms end at the isolate word; both stream forms add the same word N times.
    let want_chain = w.cx.as_ptr() as u64;
    println!("C-20 unit-data entry measure: N = {N} entries, {REPS} runs, median ns per entry");
    for (name, f) in forms {
        let mut ns: Vec<f64> = Vec::new();
        for _ in 0..REPS {
            let t = Instant::now();
            let r = std::hint::black_box(f(&w, N));
            ns.push(t.elapsed().as_nanos() as f64 / N as f64);
            if name.ends_with("chain") {
                assert_eq!(r, want_chain);
            } else {
                assert_eq!(r, want_chain.wrapping_mul(N));
            }
        }
        ns.sort_by(f64::total_cmp);
        println!(
            "{name:<20} {:.3} ns (min {:.3}, max {:.3})",
            ns[REPS / 2],
            ns[0],
            ns[REPS - 1]
        );
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn main() {
    println!("SKIP: the C-20 measure is AArch64 stub code (an x86-64 run is a request file)");
}
