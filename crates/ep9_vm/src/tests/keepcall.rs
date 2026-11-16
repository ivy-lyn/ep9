//! Form D keep-calls (M9-P1 X86-KEEPCALL; `interp/keepcall.rs`; plan v14 V14-D7).
//!
//! - Every target: the T0 context store (`keepcall::context_set`, the store of
//!   `StaContextSlot` and `StaCurrentContextSlot`) stores in range, refuses out of range, and
//!   keeps the barrier: an old (BLACK) context that gets a young cell is logged (GREY), and the
//!   heap verifier finds no store without the barrier.
//! - x86-64: the same through `keepcall::context_set_keep` (the keep-call form that the
//!   release build uses; the unit tests unwind, so `context_set` is the plain call there), the
//!   decision table of `slot_set_keep` against `Slot::set`, and the GPR-fill tests of the
//!   naked wrappers (DL M8-P3-D3 X3): a wrapper keeps rcx, rdx, rsi, rdi and r8-r11, gives
//!   its target the 3 arguments with rsp 8 mod 16 at the target's entry, and a `keep_call!`
//!   caller keeps its values across a target that changes every caller-saved register.

use ep9_heap::gc_state;

use crate::interp::keepcall;
use crate::object::{self, attr};
use crate::tests::{config, isolate, on_big_stack};
use crate::{Dispatch, Isolate, atoms, props};

/// A context with `len` slots, held by the global property `name`, and old: a full GC has run
/// since it was made, so its header is BLACK. The scope word points nowhere: nothing here
/// reads it.
fn old_context(iso: &mut Isolate, name: &str, len: usize) -> u64 {
    let c = {
        let rt = &iso.rt;
        let c = object::new_context(rt, crate::frame::UNDEFINED, core::ptr::null(), len);
        let k = atoms::intern_str(rt, name);
        props::own_define(rt, rt.global(), k, c, attr::DEFAULT);
        c
    };
    iso.collect_garbage();
    assert_eq!(
        gc_state_of(c),
        gc_state::BLACK,
        "a full GC leaves a live cell old"
    );
    c
}

fn gc_state_of(c: u64) -> u8 {
    // SAFETY: `c` is a live cell: a global property holds it.
    unsafe { object::cell(c).header() }.gc_state()
}

/// Runs `set` (a context store) on an old context: an int32, a young cell, then out of range.
fn context_store_keeps_the_barrier(set: fn(&crate::runtime::Runtime, u64, u32, u64) -> bool) {
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        let c = old_context(&mut iso, "keepCtx", 2);
        let rt = &iso.rt;
        // Not a cell: no barrier, the host stays BLACK.
        let n = crate::frame::int32_bits(7);
        assert!(set(rt, c, 1, n));
        assert_eq!(object::context_get(c, 1), Some(n));
        assert_eq!(gc_state_of(c), gc_state::BLACK);
        // A young cell into an old host: the slow path stores and logs it (GREY).
        let young = object::new_plain_object(rt);
        assert_eq!(gc_state_of(young), gc_state::WHITE);
        assert!(set(rt, c, 0, young));
        assert_eq!(object::context_get(c, 0), Some(young));
        assert_eq!(gc_state_of(c), gc_state::GREY, "the barrier slow path ran");
        // Out of range: refused, nothing stored.
        assert!(!set(rt, c, 2, young));
        assert_eq!(object::context_get(c, 2), None);
        // The generational invariant: no old cell holds a young one outside the logs.
        iso.verify_heap().unwrap();
        // A minor GC keeps the young cell (the context logged it) and the slot still holds it.
        iso.rt.heap.collect_minor();
        assert_eq!(object::context_get(c, 0), Some(young));
        iso.verify_heap().unwrap();
    });
}

/// The context store that `ops::ctx_store` calls on this build.
#[test]
fn the_t0_context_store_keeps_the_barrier() {
    context_store_keeps_the_barrier(keepcall::context_set);
}

/// The keep-call form of the context store (x86-64 release builds), run by `cargo test`.
#[cfg(target_arch = "x86_64")]
#[test]
fn the_keep_call_context_store_keeps_the_barrier() {
    context_store_keeps_the_barrier(keepcall::context_set_keep);
}

#[cfg(target_arch = "x86_64")]
mod x64 {
    use core::sync::atomic::Ordering::Relaxed;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use ep9_heap::{BarrierState, Slot, gc_state, threshold};
    use ep9_value::{ShapeId, Value, Word0};

    use crate::interp::keepcall::test_target::{SEEN, ep9_keep_test_clobber};
    use crate::interp::keepcall::{self, keep_call};

    /// The values that the fill helpers put in the registers that a wrapper must keep.
    const RCX: u64 = 0x6969_0000_0000_0C0C;
    const RDX: u64 = 0x6969_0000_0000_0D0D;
    const RSI: u64 = 0x6969_0000_0000_050E;
    const RDI: u64 = 0x6969_0000_0000_0D1D;
    const R11: u64 = 0x6969_0000_0000_1111;

    /// Defines `$name(out, a, b, c)`: puts `a`, `b`, `c` in r8, r9, r10 and the values above
    /// in rcx, rdx, rsi, rdi and r11, calls `$wrapper`, then writes rcx, rdx, rsi, rdi and
    /// r8-r11 to `out[0..8]`. SysV ABI on every OS (rbx keeps `out`).
    macro_rules! fill_and_call {
        ($name:ident => $wrapper:path) => {
            #[unsafe(naked)]
            unsafe extern "sysv64" fn $name(out: *mut [u64; 8], a: u64, b: u64, c: u64) {
                core::arch::naked_asm!(
                    "push rbx",
                    "mov rbx, rdi",
                    "mov r8, rsi",
                    "mov r9, rdx",
                    "mov r10, rcx",
                    "mov rcx, {rcx}",
                    "mov rdx, {rdx}",
                    "mov rsi, {rsi}",
                    "mov rdi, {rdi}",
                    "mov r11, {r11}",
                    // Entry rsp is 8 mod 16; the push makes it 16-aligned, as an `asm!` call.
                    "call {w}",
                    "mov qword ptr [rbx], rcx",
                    "mov qword ptr [rbx + 8], rdx",
                    "mov qword ptr [rbx + 16], rsi",
                    "mov qword ptr [rbx + 24], rdi",
                    "mov qword ptr [rbx + 32], r8",
                    "mov qword ptr [rbx + 40], r9",
                    "mov qword ptr [rbx + 48], r10",
                    "mov qword ptr [rbx + 56], r11",
                    "pop rbx",
                    "ret",
                    w = sym $wrapper,
                    rcx = const RCX,
                    rdx = const RDX,
                    rsi = const RSI,
                    rdi = const RDI,
                    r11 = const R11,
                )
            }
        };
    }

    fill_and_call!(fill_and_call_clobber => ep9_keep_test_clobber);
    fill_and_call!(fill_and_call_barrier => keepcall::ep9_keep_barrier_slow);

    fn kept(a: u64, b: u64, c: u64) -> [u64; 8] {
        [RCX, RDX, RSI, RDI, a, b, c, R11]
    }

    /// `clobber` writes the one static `SEEN`, and libtest runs tests on parallel threads. Each
    /// test that calls `clobber` holds this lock, so no other call writes `SEEN` between a
    /// test's call and its read.
    static CLOBBER: Mutex<()> = Mutex::new(());

    fn clobber_lock() -> MutexGuard<'static, ()> {
        CLOBBER.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The wrapper keeps rcx, rdx, rsi, rdi and r8-r11 although its target changes every
    /// register it may; the target gets the 3 arguments, with rsp 8 mod 16 at its entry.
    #[test]
    fn a_wrapper_keeps_every_register_it_saves() {
        let (a, b, c) = (
            0x1234_5678_9ABC_DEF0,
            0x0FED_CBA9_8765_4321,
            0x5555_AAAA_5555_AAAA,
        );
        let mut out = [0u64; 8];
        let _clobber = clobber_lock();
        // SAFETY: the helper and the wrapper keep the SysV contract; `clobber` changes only
        // registers that its C ABI lets it change and writes only `SEEN` and its shadow space.
        unsafe { fill_and_call_clobber(&raw mut out, a, b, c) };
        assert_eq!(out, kept(a, b, c));
        let seen: Vec<u64> = SEEN.iter().map(|s| s.load(Relaxed)).collect();
        assert_eq!(
            seen,
            [a, b, c, 8],
            "arguments, then rsp mod 16 at the target"
        );
    }

    /// The production wrapper of `barrier_slow`: the store lands, and the 8 registers are kept.
    /// The host is a header on this stack, outside every heap, so the slow path only stores
    /// (H11-Q6).
    #[test]
    fn the_barrier_wrapper_stores_and_keeps_every_register_it_saves() {
        let host = Word0::new(ShapeId::new(8), gc_state::BLACK, 0, 0);
        let slot = Slot::new(Value::UNDEFINED);
        let target = 0u64;
        let v = (&raw const target).addr() as u64;
        assert!(
            ep9_heap::barrier::is_cell_bits(v),
            "a stack address has the cell pattern"
        );
        let (a, b) = (
            (&raw const host).addr() as u64,
            (&raw const slot).addr() as u64,
        );
        let mut out = [0u64; 8];
        // SAFETY: the wrapper calls `barrier_slow(&host, &slot, v)`: live references and the
        // bits of a cell pattern, which `barrier_slow` stores and does not read.
        unsafe { fill_and_call_barrier(&raw mut out, a, b, v) };
        assert_eq!(out, kept(a, b, v));
        assert_eq!(slot.bits(), v, "the slow path does the store");
    }

    /// A Rust caller of `keep_call!` gets its integers and doubles back unchanged after a
    /// target that changes every caller-saved register. This test does not prove the clobber
    /// list of the `asm!` block. In the debug test job (opt-level 0), LLVM keeps these values
    /// in stack slots, not in registers, across the call, so that run does not check the
    /// clobber list. The release-tests job (`.github/workflows/ci.yml` `release-tests`,
    /// opt-level 3, on linux-x64 and windows-x64) builds this test optimized, where LLVM may
    /// keep the values in caller-saved registers across the call: a partial check only, as no
    /// assert shows which registers held them. The proof of the clobber list is the review of
    /// the `asm!` operands against the SysV and Win64 caller-saved sets (`interp/keepcall.rs`,
    /// "Clobbers") and the review of the release disasm of the keep-call handlers (M9-P1
    /// X86-KEEPCALL and its JIT review). `cargo xtask disasm-check` counts only the entry
    /// saves; it does not check the clobber list.
    #[test]
    fn a_keep_call_caller_keeps_its_values() {
        let _clobber = clobber_lock();
        let ints: [u64; 12] = core::hint::black_box(core::array::from_fn(|i| 0x100 + i as u64));
        let floats: [f64; 6] = core::hint::black_box(core::array::from_fn(|i| 0.5 + i as f64));
        let [i0, i1, i2, i3, i4, i5, i6, i7, i8, i9, i10, i11] = ints;
        let [f0, f1, f2, f3, f4, f5] = floats;
        // SAFETY: `clobber` keeps its C ABI and does not unwind.
        unsafe { keep_call!(ep9_keep_test_clobber; i0, i1, i2) };
        let after = core::hint::black_box((
            [i0, i1, i2, i3, i4, i5, i6, i7, i8, i9, i10, i11],
            [f0, f1, f2, f3, f4, f5],
        ));
        assert_eq!(after, (ints, floats));
    }

    /// `slot_set_keep` against `Slot::set`, on hosts outside every heap: every gc_state x the
    /// stage thresholds x a cell and non-cell values. The stored bits and the host header
    /// agree. This test cannot see which path ran: on a host outside every heap the slow path
    /// only stores, as the fast path does. `the_keep_call_context_store_keeps_the_barrier`
    /// proves that the slow path runs (the old host becomes GREY).
    #[test]
    fn the_keep_store_matches_slot_set() {
        let target = 0u64;
        // SAFETY: a stack address has the cell pattern; the slow path stores it and never
        // reads it.
        let cell = unsafe { Value::from_bits_unchecked((&raw const target).addr() as u64) };
        let values = [
            cell,
            Value::int32(-1),
            Value::double(0.5),
            Value::NULL,
            Value::TRUE,
        ];
        for th in [threshold::OFF, threshold::GEN, 2, threshold::MARKING] {
            let state = BarrierState::new(th);
            for g in 0..=u8::MAX {
                for v in values {
                    let (h1, h2) = (
                        Word0::new(ShapeId::new(8), g, 0, 0),
                        Word0::new(ShapeId::new(8), g, 0, 0),
                    );
                    let (s1, s2) = (Slot::new(Value::UNDEFINED), Slot::new(Value::UNDEFINED));
                    s1.set(&h1, v, &state);
                    keepcall::slot_set_keep(&s2, &h2, v, &state);
                    assert_eq!(s1.bits(), s2.bits(), "threshold {th} gc_state {g}");
                    assert_eq!(s2.bits(), v.to_bits());
                    assert_eq!(h1.gc_state(), h2.gc_state());
                }
            }
        }
    }
}
