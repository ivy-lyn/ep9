//! Access to shared memory outside the Rust memory model (TypedArray TA-4 R2 ATOMICS; route
//! TA-SHARED-MIXED-WIDTH, plan v13 V13-D8; design `docs/plan/notes/typedarray-design.md` §5.6).
//!
//! The bytes of a SharedArrayBuffer are shared by isolates on other threads. JS lets two
//! threads access the same bytes with two widths at the same time (an `Int8Array` and an
//! `Int32Array` on one block, a DataView and a TypedArray, `Atomics` and a plain access). In
//! the Rust abstract model a mixed-size atomic access is undefined behavior, and a plain Rust
//! access that races another access is a data race. So every access to SAB bytes goes through
//! this module: each one is one machine instruction (or one LL/SC or `lock cmpxchg` loop) in
//! inline `asm!`, which the Rust model does not see. The hardware gives single-copy atomicity
//! for an aligned access of up to 8 bytes on both ep9 targets.
//!
//! ```text
//! op (per width 1, 2, 4, 8)    aarch64 (ARMv8.0, no LSE)        x86-64
//! load_racy                     ldrb / ldrh / ldr                 movzx / mov
//! store_racy                    strb / strh / str                 mov
//! load_seqcst                   ldarb / ldarh / ldar              movzx / mov
//! store_seqcst                  stlrb / stlrh / stlr              xchg
//! exchange                      ldaxr + stlxr loop                xchg
//! compare_exchange              ldaxr, cmp, stlxr loop (clrex)    lock cmpxchg
//! fetch_add, fetch_sub          ldaxr, add / sub, stlxr loop      lock xadd (sub: of -x)
//! fetch_and, fetch_or, fetch_xor ldaxr, and / orr / eor, stlxr    lock cmpxchg loop
//! copy_racy                     racy byte and 8-byte word loads and stores, direction-safe
//! ```
//!
//! | Rule | Statement |
//! |---|---|
//! | The mapping | The SeqCst ops use the mapping of Rust's (LLVM's) own SeqCst atomics (aarch64 `ldar`/`stlr` and `ldaxr`/`stlxr` loops; x86-64 `mov` load, `xchg` store, `lock` RMW), so they form one SeqCst order with the Rust atomics of the same process (`SharedBlock::len` in `ep9_vm`). A later JIT lowering of `Atomics` must use the same mapping (SpiderMonkey `js/src/jit/AtomicOperations.h` keeps its C++ and JIT forms equal in the same way) |
//! | Racy ops | `load_racy` and `store_racy` are the Unordered access of ECMA-262 §29: one plain machine load or store of the width, no barrier. An aligned one does not tear; the caller decides the width (a TypedArray element: its size; a DataView or a copy: bytes) |
//! | No `nomem` | Every `asm!` block lets the compiler see a memory access (no `nomem`; the loads are `readonly` only when they are racy), so the compiler does not move other memory accesses across a SeqCst op |
//! | Alignment | Every access of width `n` needs an address that is a multiple of `n` (an unaligned `ldaxr` faults; an unaligned `lock` op is slow and may split). The callers' contract |
//! | Miri | Under `cfg(miri)` each op is the Rust atomic of the same width and order (Miri cannot run `asm!`); so Miri checks the callers' bounds and alignment, not the mixed-width race, and a test that mixes widths is `cfg_attr(miri, ignore)` |
//! | Targets | aarch64 and x86-64 only: another target is a `compile_error!` (ep9 builds no other) |
//! | Endianness | Values are native integers; both targets are little endian |

#[cfg(not(any(miri, target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!(
    "ep9_platform::shared_mem has asm for aarch64 and x86-64 only (V13-D8); add the target's \
     mapping first"
);

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
    impl Sealed for u64 {}
}

/// An integer width of a shared access: `u8`, `u16`, `u32` or `u64`.
///
/// # Safety (every method)
/// `p` is aligned to `size_of::<Self>()` and points at memory that stays valid (mapped and
/// readable, and writable for a method that writes) for the whole call. Other threads may
/// access the same bytes at the same time with any width through this module.
pub trait Word: Copy + Eq + core::fmt::Debug + sealed::Sealed {
    /// A plain load (Unordered): one machine load of the width.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn load_racy(p: *const Self) -> Self;
    /// A plain store (Unordered): one machine store of the width.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn store_racy(p: *mut Self, v: Self);
    /// A SeqCst load.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn load_seqcst(p: *const Self) -> Self;
    /// A SeqCst store.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn store_seqcst(p: *mut Self, v: Self);
    /// A SeqCst exchange: stores `v`, returns the old value.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn exchange(p: *mut Self, v: Self) -> Self;
    /// A SeqCst compare-exchange: stores `new` if the value is `expected`; returns the old
    /// value either way.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn compare_exchange(p: *mut Self, expected: Self, new: Self) -> Self;
    /// A SeqCst wrapping add; returns the old value.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn fetch_add(p: *mut Self, v: Self) -> Self;
    /// A SeqCst wrapping sub; returns the old value.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn fetch_sub(p: *mut Self, v: Self) -> Self;
    /// A SeqCst bitwise and; returns the old value.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn fetch_and(p: *mut Self, v: Self) -> Self;
    /// A SeqCst bitwise or; returns the old value.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn fetch_or(p: *mut Self, v: Self) -> Self;
    /// A SeqCst bitwise xor; returns the old value.
    ///
    /// # Safety
    /// See [`Word`].
    unsafe fn fetch_xor(p: *mut Self, v: Self) -> Self;
}

// ---- cfg(miri): the Rust atomics of the same width and order ---------------------------------

#[cfg(miri)]
macro_rules! word_miri {
    ($t:ty, $a:ty) => {
        impl Word for $t {
            unsafe fn load_racy(p: *const $t) -> $t {
                // SAFETY: aligned and valid (the trait contract).
                unsafe { <$a>::from_ptr(p.cast_mut()) }.load(Relaxed)
            }
            unsafe fn store_racy(p: *mut $t, v: $t) {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.store(v, Relaxed)
            }
            unsafe fn load_seqcst(p: *const $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p.cast_mut()) }.load(SeqCst)
            }
            unsafe fn store_seqcst(p: *mut $t, v: $t) {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.store(v, SeqCst)
            }
            unsafe fn exchange(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.swap(v, SeqCst)
            }
            unsafe fn compare_exchange(p: *mut $t, expected: $t, new: $t) -> $t {
                // SAFETY: as `load_racy`.
                let a = unsafe { <$a>::from_ptr(p) };
                match a.compare_exchange(expected, new, SeqCst, SeqCst) {
                    Ok(v) | Err(v) => v,
                }
            }
            unsafe fn fetch_add(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.fetch_add(v, SeqCst)
            }
            unsafe fn fetch_sub(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.fetch_sub(v, SeqCst)
            }
            unsafe fn fetch_and(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.fetch_and(v, SeqCst)
            }
            unsafe fn fetch_or(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.fetch_or(v, SeqCst)
            }
            unsafe fn fetch_xor(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `load_racy`.
                unsafe { <$a>::from_ptr(p) }.fetch_xor(v, SeqCst)
            }
        }
    };
}

#[cfg(miri)]
use core::sync::atomic::{
    AtomicU8, AtomicU16, AtomicU32, AtomicU64,
    Ordering::{Relaxed, SeqCst},
};

#[cfg(miri)]
word_miri!(u8, AtomicU8);
#[cfg(miri)]
word_miri!(u16, AtomicU16);
#[cfg(miri)]
word_miri!(u32, AtomicU32);
#[cfg(miri)]
word_miri!(u64, AtomicU64);

// ---- aarch64 (ARMv8.0 LL/SC; no LSE requirement) ---------------------------------------------
//
// `$r` is the register type of the value (`u32` for 1, 2 and 4 bytes, `u64` for 8), `$m` the
// register modifier (`w` or `x`) and `$s` the size suffix of the load and store mnemonics
// (`b`, `h` or empty). Each input is widened to `$r` in Rust, so its upper bits are 0 (a
// register operand narrower than the register has undefined upper bits; the compare of
// `compare_exchange` reads the whole `w` register). Each output is the zero-extended value of
// the width (`ldrb`, `ldarb` and `ldaxrb` zero-extend), cut back to `$t`.

#[cfg(all(not(miri), target_arch = "aarch64"))]
macro_rules! word_aarch64 {
    ($t:ty, $r:ty, $m:literal, $s:literal) => {
        impl Word for $t {
            #[inline]
            unsafe fn load_racy(p: *const $t) -> $t {
                let v: $r;
                // SAFETY: aligned and valid (the trait contract); a plain load.
                unsafe {
                    core::arch::asm!(
                        concat!("ldr", $s, " {v:", $m, "}, [{p}]"),
                        p = in(reg) p,
                        v = lateout(reg) v,
                        options(nostack, readonly, preserves_flags),
                    )
                };
                v as $t
            }
            #[inline]
            unsafe fn store_racy(p: *mut $t, v: $t) {
                // SAFETY: as `load_racy`; a plain store.
                unsafe {
                    core::arch::asm!(
                        concat!("str", $s, " {v:", $m, "}, [{p}]"),
                        p = in(reg) p,
                        v = in(reg) <$r>::from(v),
                        options(nostack, preserves_flags),
                    )
                };
            }
            #[inline]
            unsafe fn load_seqcst(p: *const $t) -> $t {
                let v: $r;
                // SAFETY: as `load_racy`; a load-acquire (RCsc: orders against `stlr`).
                unsafe {
                    core::arch::asm!(
                        concat!("ldar", $s, " {v:", $m, "}, [{p}]"),
                        p = in(reg) p,
                        v = lateout(reg) v,
                        options(nostack, preserves_flags),
                    )
                };
                v as $t
            }
            #[inline]
            unsafe fn store_seqcst(p: *mut $t, v: $t) {
                // SAFETY: as `load_racy`; a store-release (RCsc).
                unsafe {
                    core::arch::asm!(
                        concat!("stlr", $s, " {v:", $m, "}, [{p}]"),
                        p = in(reg) p,
                        v = in(reg) <$r>::from(v),
                        options(nostack, preserves_flags),
                    )
                };
            }
            #[inline]
            unsafe fn exchange(p: *mut $t, v: $t) -> $t {
                let old: $r;
                // SAFETY: as `load_racy`; an LL/SC loop of acquire-release exclusives.
                unsafe {
                    core::arch::asm!(
                        "2:",
                        concat!("ldaxr", $s, " {old:", $m, "}, [{p}]"),
                        concat!("stlxr", $s, " {st:w}, {v:", $m, "}, [{p}]"),
                        "cbnz {st:w}, 2b",
                        p = in(reg) p,
                        v = in(reg) <$r>::from(v),
                        old = out(reg) old,
                        st = out(reg) _,
                        options(nostack),
                    )
                };
                old as $t
            }
            #[inline]
            unsafe fn compare_exchange(p: *mut $t, expected: $t, new: $t) -> $t {
                let old: $r;
                // SAFETY: as `exchange`; the failure path drops the exclusive monitor.
                unsafe {
                    core::arch::asm!(
                        "2:",
                        concat!("ldaxr", $s, " {old:", $m, "}, [{p}]"),
                        concat!("cmp {old:", $m, "}, {e:", $m, "}"),
                        "b.ne 3f",
                        concat!("stlxr", $s, " {st:w}, {n:", $m, "}, [{p}]"),
                        "cbnz {st:w}, 2b",
                        "b 4f",
                        "3:",
                        "clrex",
                        "4:",
                        p = in(reg) p,
                        e = in(reg) <$r>::from(expected),
                        n = in(reg) <$r>::from(new),
                        old = out(reg) old,
                        st = out(reg) _,
                        options(nostack),
                    )
                };
                old as $t
            }
            #[inline]
            unsafe fn fetch_add(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `exchange`.
                unsafe { word_aarch64!(@rmw $t, $r, $m, $s, "add", p, v) }
            }
            #[inline]
            unsafe fn fetch_sub(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `exchange`.
                unsafe { word_aarch64!(@rmw $t, $r, $m, $s, "sub", p, v) }
            }
            #[inline]
            unsafe fn fetch_and(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `exchange`.
                unsafe { word_aarch64!(@rmw $t, $r, $m, $s, "and", p, v) }
            }
            #[inline]
            unsafe fn fetch_or(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `exchange`.
                unsafe { word_aarch64!(@rmw $t, $r, $m, $s, "orr", p, v) }
            }
            #[inline]
            unsafe fn fetch_xor(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `exchange`.
                unsafe { word_aarch64!(@rmw $t, $r, $m, $s, "eor", p, v) }
            }
        }
    };
    // One read-modify-write loop: `new = old <op> v`, then the store-release exclusive.
    (@rmw $t:ty, $r:ty, $m:literal, $s:literal, $op:literal, $p:ident, $v:ident) => {{
        let old: $r;
        core::arch::asm!(
            "2:",
            concat!("ldaxr", $s, " {old:", $m, "}, [{p}]"),
            concat!($op, " {new:", $m, "}, {old:", $m, "}, {v:", $m, "}"),
            concat!("stlxr", $s, " {st:w}, {new:", $m, "}, [{p}]"),
            "cbnz {st:w}, 2b",
            p = in(reg) $p,
            v = in(reg) <$r>::from($v),
            old = out(reg) old,
            new = out(reg) _,
            st = out(reg) _,
            options(nostack),
        );
        old as $t
    }};
}

#[cfg(all(not(miri), target_arch = "aarch64"))]
word_aarch64!(u8, u32, "w", "b");
#[cfg(all(not(miri), target_arch = "aarch64"))]
word_aarch64!(u16, u32, "w", "h");
#[cfg(all(not(miri), target_arch = "aarch64"))]
word_aarch64!(u32, u32, "w", "");
#[cfg(all(not(miri), target_arch = "aarch64"))]
word_aarch64!(u64, u64, "x", "");

// ---- x86-64 (TSO: a plain `mov` load is SeqCst-ready; the SeqCst store is `xchg`) -------------
//
// `$r` is the register type (`u32` for 1, 2 and 4 bytes, `u64` for 8), `$m` the modifier of
// the width (`l` = al, `x` = ax, `e` = eax, `r` = rax), `$ptr` the operand size (`byte ptr`
// ...), `$ld` the zero-extending load (`movzx` or `mov`), `$lm` the modifier of its target and
// `$acc` the accumulator register that `cmpxchg` uses (`eax` or `rax`; the instruction reads
// its low bytes of the width). Inputs are widened to `$r` in Rust (upper bits 0); outputs are cut back to `$t`.

#[cfg(all(not(miri), target_arch = "x86_64"))]
macro_rules! word_x86 {
    ($t:ty, $r:ty, $m:literal, $ptr:literal, $ld:literal, $lm:literal, $acc:tt) => {
        impl Word for $t {
            #[inline]
            unsafe fn load_racy(p: *const $t) -> $t {
                let v: $r;
                // SAFETY: aligned and valid (the trait contract); a plain load.
                unsafe {
                    core::arch::asm!(
                        concat!($ld, " {v:", $lm, "}, ", $ptr, " [{p}]"),
                        p = in(reg) p,
                        v = lateout(reg) v,
                        options(nostack, readonly, preserves_flags),
                    )
                };
                v as $t
            }
            #[inline]
            unsafe fn store_racy(p: *mut $t, v: $t) {
                // SAFETY: as `load_racy`; a plain store.
                unsafe {
                    core::arch::asm!(
                        concat!("mov ", $ptr, " [{p}], {v:", $m, "}"),
                        p = in(reg) p,
                        v = in(reg) <$r>::from(v),
                        options(nostack, preserves_flags),
                    )
                };
            }
            #[inline]
            unsafe fn load_seqcst(p: *const $t) -> $t {
                let v: $r;
                // SAFETY: as `load_racy`. A TSO load is acquire; with the `xchg` store it is
                // SeqCst (the mapping of Rust's SeqCst load). No `readonly`: the compiler keeps
                // other memory accesses on their side.
                unsafe {
                    core::arch::asm!(
                        concat!($ld, " {v:", $lm, "}, ", $ptr, " [{p}]"),
                        p = in(reg) p,
                        v = lateout(reg) v,
                        options(nostack, preserves_flags),
                    )
                };
                v as $t
            }
            #[inline]
            unsafe fn store_seqcst(p: *mut $t, v: $t) {
                // SAFETY: as `load_racy`; `xchg` with memory is locked (a full barrier).
                unsafe {
                    core::arch::asm!(
                        concat!("xchg ", $ptr, " [{p}], {v:", $m, "}"),
                        p = in(reg) p,
                        v = inout(reg) <$r>::from(v) => _,
                        options(nostack, preserves_flags),
                    )
                };
            }
            #[inline]
            unsafe fn exchange(p: *mut $t, v: $t) -> $t {
                let mut x = <$r>::from(v);
                // SAFETY: as `store_seqcst`; the register gets the old value.
                unsafe {
                    core::arch::asm!(
                        concat!("xchg ", $ptr, " [{p}], {v:", $m, "}"),
                        p = in(reg) p,
                        v = inout(reg) x,
                        options(nostack, preserves_flags),
                    )
                };
                x as $t
            }
            #[inline]
            unsafe fn compare_exchange(p: *mut $t, expected: $t, new: $t) -> $t {
                let old: $r;
                // SAFETY: as `load_racy`. `lock cmpxchg` compares the accumulator of the width
                // with the memory: equal, it stores `new`; not equal, it loads the memory into
                // the accumulator (its upper bits stay those of `expected`: 0).
                unsafe {
                    core::arch::asm!(
                        concat!("lock cmpxchg ", $ptr, " [{p}], {n:", $m, "}"),
                        p = in(reg) p,
                        n = in(reg) <$r>::from(new),
                        inout($acc) <$r>::from(expected) => old,
                        options(nostack),
                    )
                };
                old as $t
            }
            #[inline]
            unsafe fn fetch_add(p: *mut $t, v: $t) -> $t {
                let mut x = <$r>::from(v);
                // SAFETY: as `load_racy`; `lock xadd` leaves the old value in the register.
                unsafe {
                    core::arch::asm!(
                        concat!("lock xadd ", $ptr, " [{p}], {v:", $m, "}"),
                        p = in(reg) p,
                        v = inout(reg) x,
                        options(nostack),
                    )
                };
                x as $t
            }
            #[inline]
            unsafe fn fetch_sub(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `fetch_add` (old - v = old + (-v) modulo the width).
                unsafe { <$t as Word>::fetch_add(p, v.wrapping_neg()) }
            }
            #[inline]
            unsafe fn fetch_and(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `compare_exchange`.
                unsafe { word_x86!(@loop $t, $m, $ptr, $ld, $acc, "and", p, v) }
            }
            #[inline]
            unsafe fn fetch_or(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `compare_exchange`.
                unsafe { word_x86!(@loop $t, $m, $ptr, $ld, $acc, "or", p, v) }
            }
            #[inline]
            unsafe fn fetch_xor(p: *mut $t, v: $t) -> $t {
                // SAFETY: as `compare_exchange`.
                unsafe { word_x86!(@loop $t, $m, $ptr, $ld, $acc, "xor", p, v) }
            }
        }
    };
    // and, or, xor with the old value: x86 has no locked form that returns it, so a `lock
    // cmpxchg` loop (SpiderMonkey `GenerateAtomicOperations.py`, the fetch ops). The
    // accumulator holds the old value; a failed `cmpxchg` reloads it.
    (@loop $t:ty, $m:literal, $ptr:literal, $ld:literal, $acc:tt, $op:literal, $p:ident,
     $v:ident) => {{
        // The accumulator (`rax`, zero-extended by the first load; a failed `cmpxchg` writes
        // only its low bytes of the width, or all of `eax` for 4 bytes) holds the old value;
        // the scratch `t` gets `old <op> v` in 64 bits, of which `cmpxchg` stores the width.
        let old: u64;
        core::arch::asm!(
            concat!($ld, " ", $acc, ", ", $ptr, " [{p}]"),
            "2:",
            "mov {t:r}, rax",
            concat!($op, " {t:r}, {v:r}"),
            concat!("lock cmpxchg ", $ptr, " [{p}], {t:", $m, "}"),
            "jne 2b",
            p = in(reg) $p,
            v = in(reg) u64::from($v),
            t = out(reg) _,
            out("rax") old,
            options(nostack),
        );
        old as $t
    }};
}

#[cfg(all(not(miri), target_arch = "x86_64"))]
word_x86!(u8, u32, "l", "byte ptr", "movzx", "e", "eax");
#[cfg(all(not(miri), target_arch = "x86_64"))]
word_x86!(u16, u32, "x", "word ptr", "movzx", "e", "eax");
#[cfg(all(not(miri), target_arch = "x86_64"))]
word_x86!(u32, u32, "e", "dword ptr", "mov", "e", "eax");
#[cfg(all(not(miri), target_arch = "x86_64"))]
word_x86!(u64, u64, "r", "qword ptr", "mov", "r", "rax");

// ---- the size-dispatched forms (the callers know the width at run time) -----------------------

/// A read-modify-write op of [`rmw`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rmw {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Exchange,
}

/// Runs `$body` with `W` bound to the word type of `$size` bytes (1, 2, 4 or 8; a panic for
/// another size: a caller bug).
macro_rules! by_size {
    ($size:expr, $W:ident => $body:expr) => {
        match $size {
            1 => {
                type $W = u8;
                $body
            }
            2 => {
                type $W = u16;
                $body
            }
            4 => {
                type $W = u32;
                $body
            }
            8 => {
                type $W = u64;
                $body
            }
            n => panic!("shared_mem: no access of {n} bytes"),
        }
    };
}

/// [`Word::load_racy`] of `size` bytes (1, 2, 4 or 8) at `p`, zero-extended.
///
/// # Safety
/// As [`Word`], for the word of `size` bytes at `p`.
#[inline]
// The 8-byte arm of `by_size!` widens a `u64` to `u64`.
#[allow(clippy::useless_conversion)]
pub unsafe fn load_racy(p: *const u8, size: usize) -> u64 {
    // SAFETY: the caller's contract.
    by_size!(size, W => u64::from(unsafe { W::load_racy(p.cast::<W>()) }))
}

/// [`Word::store_racy`] of the low `size` bytes of `x` at `p`.
///
/// # Safety
/// As [`load_racy`].
#[inline]
pub unsafe fn store_racy(p: *mut u8, size: usize, x: u64) {
    // SAFETY: the caller's contract.
    by_size!(size, W => unsafe { W::store_racy(p.cast::<W>(), x as W) })
}

/// [`Word::load_seqcst`] of `size` bytes at `p`, zero-extended.
///
/// # Safety
/// As [`load_racy`].
#[inline]
// The 8-byte arm of `by_size!` widens a `u64` to `u64`.
#[allow(clippy::useless_conversion)]
pub unsafe fn load_seqcst(p: *const u8, size: usize) -> u64 {
    // SAFETY: the caller's contract.
    by_size!(size, W => u64::from(unsafe { W::load_seqcst(p.cast::<W>()) }))
}

/// [`Word::store_seqcst`] of the low `size` bytes of `x` at `p`.
///
/// # Safety
/// As [`load_racy`].
#[inline]
pub unsafe fn store_seqcst(p: *mut u8, size: usize, x: u64) {
    // SAFETY: the caller's contract.
    by_size!(size, W => unsafe { W::store_seqcst(p.cast::<W>(), x as W) })
}

/// The SeqCst read-modify-write `op` with the low `size` bytes of `x` at `p`; the old value,
/// zero-extended.
///
/// # Safety
/// As [`load_racy`].
#[inline]
// The 8-byte arm of `by_size!` widens a `u64` to `u64`.
#[allow(clippy::useless_conversion)]
pub unsafe fn rmw(p: *mut u8, size: usize, op: Rmw, x: u64) -> u64 {
    by_size!(size, W => {
        let (p, x) = (p.cast::<W>(), x as W);
        // SAFETY: the caller's contract.
        u64::from(unsafe {
            match op {
                Rmw::Add => W::fetch_add(p, x),
                Rmw::Sub => W::fetch_sub(p, x),
                Rmw::And => W::fetch_and(p, x),
                Rmw::Or => W::fetch_or(p, x),
                Rmw::Xor => W::fetch_xor(p, x),
                Rmw::Exchange => W::exchange(p, x),
            }
        })
    })
}

/// [`Word::compare_exchange`] of `size` bytes at `p` with the low bytes of `expected` and
/// `new`; the old value, zero-extended.
///
/// # Safety
/// As [`load_racy`].
#[inline]
// The 8-byte arm of `by_size!` widens a `u64` to `u64`.
#[allow(clippy::useless_conversion)]
pub unsafe fn compare_exchange(p: *mut u8, size: usize, expected: u64, new: u64) -> u64 {
    // SAFETY: the caller's contract.
    by_size!(size, W => u64::from(unsafe {
        W::compare_exchange(p.cast::<W>(), expected as W, new as W)
    }))
}

/// Copies `n` bytes from `src` to `dst` with racy loads and stores (CopyDataBlockBytes on
/// shared bytes: each byte is an Unordered access, so the copy may tear). The ranges may
/// overlap: the result is as if through a temporary copy (the direction follows the
/// overlap). When both addresses have the same offset in an 8-byte word, the middle goes by
/// 8-byte words.
///
/// # Safety
/// `[src, src + n)` is valid to read and `[dst, dst + n)` valid to write for the whole call.
pub unsafe fn copy_racy(dst: *mut u8, src: *const u8, n: usize) {
    const W: usize = 8;
    let words = dst.addr() % W == src.addr() % W;
    let byte = |i: usize| {
        // SAFETY: `i < n`: inside both ranges (the caller's contract); a byte is aligned.
        unsafe { u8::store_racy(dst.add(i), u8::load_racy(src.add(i))) }
    };
    let word = |i: usize| {
        // SAFETY: `i + 8 <= n`, and `src + i`, `dst + i` are multiples of 8 (the callers
        // below).
        unsafe {
            let x = u64::load_racy(src.add(i).cast::<u64>());
            u64::store_racy(dst.add(i).cast::<u64>(), x);
        }
    };
    // Forward unless `dst` starts inside `[src, src + n)` after `src`.
    let forward = dst.addr() <= src.addr() || dst.addr() >= src.addr() + n;
    let head = if words { (W - src.addr() % W) % W } else { n }.min(n);
    if forward {
        let mut i = 0;
        while i < head {
            byte(i);
            i += 1;
        }
        while words && i + W <= n {
            word(i);
            i += W;
        }
        while i < n {
            byte(i);
            i += 1;
        }
    } else {
        // Backward: the tail bytes after the last aligned word, the words, then the head.
        let mut end = n;
        if words {
            let tail = (src.addr() + n) % W;
            let stop = end.saturating_sub(tail).max(head);
            while end > stop {
                end -= 1;
                byte(end);
            }
            while end >= head + W {
                end -= W;
                word(end);
            }
        }
        while end > 0 {
            end -= 1;
            byte(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `N` 8-aligned words that threads share; every access goes through this module.
    #[repr(align(8))]
    struct Cells<const N: usize>(core::cell::UnsafeCell<[u64; N]>);

    // SAFETY: the tests touch the words only through the ops of this module (atomic on both
    // paths) or after every thread that touched them was joined.
    unsafe impl<const N: usize> Sync for Cells<N> {}

    impl<const N: usize> Cells<N> {
        fn new() -> Self {
            Cells(core::cell::UnsafeCell::new([0; N]))
        }
        fn ptr(&self) -> *mut u8 {
            self.0.get().cast()
        }
        /// Word `i`, read racily.
        fn word(&self, i: usize) -> u64 {
            assert!(i < N);
            // SAFETY: word `i` is inside the cells and 8-aligned.
            unsafe { load_racy(self.ptr().add(8 * i), 8) }
        }
    }

    /// The mask of the low `size` bytes.
    fn mask(size: usize) -> u64 {
        if size == 8 {
            u64::MAX
        } else {
            (1u64 << (8 * size)) - 1
        }
    }

    /// Each op of each width on a raw page: the values, the wrap at both ends, the old value,
    /// and no access past the width (the neighbour words stay 0).
    #[test]
    fn each_op_of_each_width_gives_the_value_and_the_old_value() {
        let cells = Cells::<3>::new();
        for size in [1usize, 2, 4, 8] {
            let m = mask(size);
            let q = cells.ptr().wrapping_add(8);
            // SAFETY: `q` is word 1 (8-aligned, so aligned to `size`); one thread.
            unsafe {
                store_racy(q, 8, 0);
                store_racy(q, size, 0x1122_3344_5566_7788);
                assert_eq!(load_racy(q, size), 0x1122_3344_5566_7788 & m, "{size}");
                assert_eq!(load_racy(q, 8), 0x1122_3344_5566_7788 & m, "{size}: width");
                store_seqcst(q, size, 5);
                assert_eq!(load_seqcst(q, size), 5);
                assert_eq!(rmw(q, size, Rmw::Add, 3), 5);
                assert_eq!(load_racy(q, size), 8);
                assert_eq!(rmw(q, size, Rmw::Sub, 10), 8);
                assert_eq!(load_racy(q, size), m - 1, "wrap below 0 ({size})");
                assert_eq!(rmw(q, size, Rmw::Add, 3), m - 1);
                assert_eq!(load_racy(q, size), 1, "wrap above the max ({size})");
                assert_eq!(rmw(q, size, Rmw::Or, 0b1100), 1);
                assert_eq!(rmw(q, size, Rmw::And, 0b0110), 0b1101);
                assert_eq!(rmw(q, size, Rmw::Xor, 0b0011), 0b0100);
                assert_eq!(rmw(q, size, Rmw::Exchange, 42), 0b0111);
                assert_eq!(
                    compare_exchange(q, size, 41, 7),
                    42,
                    "no store if not equal"
                );
                assert_eq!(load_racy(q, size), 42);
                assert_eq!(compare_exchange(q, size, 42, 7), 42);
                assert_eq!(load_racy(q, size), 7);
                // The bits above the width of an argument do not count.
                assert_eq!(compare_exchange(q, size, 7 | !m, u64::MAX), 7, "{size}");
                assert_eq!(load_racy(q, 8), m, "{size}: only the width's bytes");
                assert_eq!(rmw(q, size, Rmw::Xor, u64::MAX), m, "{size}");
                assert_eq!(load_racy(q, 8), 0, "{size}");
            }
            assert_eq!([cells.word(0), cells.word(2)], [0, 0], "{size}");
        }
    }

    /// A 1-byte op at each of the 8 offsets of a word leaves the other 7 bytes.
    #[test]
    fn a_byte_op_touches_one_byte() {
        let cells = Cells::<1>::new();
        let p = cells.ptr();
        for i in 0..8u8 {
            let at = p.wrapping_add(usize::from(i));
            // SAFETY: inside the word; a byte is aligned.
            unsafe {
                assert_eq!(rmw(at, 1, Rmw::Add, u64::from(0x80 + i)), 0);
                let x = compare_exchange(at, 1, u64::from(0x80 + i), u64::from(0x10 + i));
                assert_eq!(x, u64::from(0x80 + i));
            }
        }
        let want = u64::from_le_bytes([0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17]);
        assert_eq!(cells.word(0), want);
    }

    /// `copy_racy` on overlapping ranges, at many offset pairs and lengths (both directions,
    /// with and without the word path), gives what `copy_within` gives.
    #[test]
    fn copy_racy_is_a_memmove() {
        const N: usize = 48;
        let lens: &[usize] = if cfg!(miri) {
            &[0, 1, 9, 23]
        } else {
            &[0, 1, 7, 8, 9, 17, 23, 31]
        };
        let offs: &[usize] = if cfg!(miri) {
            &[0, 3, 8]
        } else {
            &[0, 1, 3, 7, 8, 9, 15]
        };
        for &len in lens {
            for &from in offs {
                for &to in offs {
                    let cells = Cells::<6>::new();
                    let p = cells.ptr();
                    let mut want = [0u8; N];
                    for (i, w) in want.iter_mut().enumerate() {
                        *w = i as u8 + 1;
                        // SAFETY: byte `i` of the N bytes.
                        unsafe { store_racy(p.add(i), 1, u64::from(*w)) };
                    }
                    want.copy_within(from..from + len, to);
                    // SAFETY: both ranges are inside the N bytes (15 + 31 <= 48).
                    unsafe { copy_racy(p.add(to), p.add(from), len) };
                    let got: Vec<u8> = (0..N)
                        // SAFETY: as above.
                        .map(|i| unsafe { load_racy(p.add(i), 1) } as u8)
                        .collect();
                    assert_eq!(got, want, "len {len} from {from} to {to}");
                }
            }
        }
    }

    /// Two threads, one width: a writer of 0 and all bits (racy and SeqCst stores) and a
    /// reader (racy and SeqCst loads); each read is one of the two values (no tear).
    #[test]
    fn two_threads_one_width_see_no_tear() {
        let rounds = if cfg!(miri) { 50 } else { 200_000 };
        for size in [2usize, 4, 8] {
            let cells = Cells::<1>::new();
            let all = mask(size);
            std::thread::scope(|s| {
                s.spawn(|| {
                    for r in 0..rounds {
                        // SAFETY: the word lives through the scope; aligned.
                        unsafe {
                            if r % 2 == 0 {
                                store_racy(cells.ptr(), size, all);
                            } else {
                                store_seqcst(cells.ptr(), size, 0);
                            }
                        }
                    }
                });
                for _ in 0..rounds {
                    // SAFETY: as above.
                    let (a, b) =
                        unsafe { (load_racy(cells.ptr(), size), load_seqcst(cells.ptr(), size)) };
                    assert!(a == 0 || a == all, "{size}: {a:#x}");
                    assert!(b == 0 || b == all, "{size}: {b:#x}");
                }
            });
        }
    }

    /// Two threads, mixed widths on one word (1-byte stores and xors against 4-byte loads and
    /// a 4-byte RMW: a JS mix that the Rust model calls undefined): every 4-byte value is made
    /// of whole bytes that the byte writer left (0 or 0xff each). Under Miri the ops are Rust
    /// atomics, and a mixed-size race is UB there: ignored (the asm path is the one under
    /// test).
    #[test]
    #[cfg_attr(
        miri,
        ignore = "a mixed-size race is UB in the Rust model that Miri runs"
    )]
    fn two_threads_mixed_widths_on_one_word() {
        let cells = Cells::<1>::new();
        let rounds = 200_000usize;
        let whole = |x: u64| (0..4).all(|k| matches!((x >> (8 * k)) & 0xff, 0 | 0xff));
        std::thread::scope(|s| {
            s.spawn(|| {
                let p = cells.ptr();
                for r in 0..rounds {
                    let x = if r % 8 < 4 { 0xff } else { 0 };
                    // SAFETY: inside the word; a byte is aligned.
                    unsafe {
                        store_racy(p.add(r % 4), 1, x);
                        rmw(p.add(4 + r % 4), 1, Rmw::Xor, 0xff);
                    }
                }
            });
            let p = cells.ptr();
            for _ in 0..rounds {
                // SAFETY: inside the word; aligned to 4.
                let (a, b, c) = unsafe {
                    (
                        load_racy(p, 4),
                        load_seqcst(p.add(4), 4),
                        rmw(p.add(4), 4, Rmw::Or, 0),
                    )
                };
                assert!(whole(a) && whole(b) && whole(c), "{a:#x} {b:#x} {c:#x}");
            }
        });
    }

    /// Four threads add 1 to one counter of each width: no update is lost (the count modulo
    /// the width).
    #[test]
    fn concurrent_adds_lose_nothing() {
        let per = if cfg!(miri) { 20 } else { 20_000 };
        for size in [1usize, 2, 4, 8] {
            let cells = Cells::<1>::new();
            std::thread::scope(|s| {
                for _ in 0..4 {
                    s.spawn(|| {
                        for _ in 0..per {
                            // SAFETY: the word lives through the scope; aligned.
                            unsafe { rmw(cells.ptr(), size, Rmw::Add, 1) };
                        }
                    });
                }
            });
            assert_eq!(cells.word(0), (4 * per) as u64 & mask(size), "{size}");
        }
    }
}
