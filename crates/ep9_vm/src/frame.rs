//! VM register stack and the canonical frame layout (H14, D21: VM register stack + native
//! control stack, Hermes model).
//!
//! The VM stack grows up. Each frame is an argument area, an 8-word header and the register
//! file. `fp` points at `r0`:
//!
//! ```text
//! low   ... caller registers ...                      <- caller top = caller fp + caller regs
//!       a(n-1)            fp - 10 - (n-1)    n = max(argc, parameter_count)
//!       ...                                  args reversed: a0 is next to the header
//!       a0                fp - 10            missing args are undefined (padding)
//!       this              fp - 9             receiver
//!       caller fp         fp - 8   native    header slot 0
//!       return pc         fp - 7   native    pc after the call, or a VM stub (exit, construct)
//!       callee            fp - 6   Value     the function object
//!       argc              fp - 5   int32     actual count; bit 30 = [[Construct]] call
//!       function info     fp - 4   native    FunctionInfo of this frame
//!       feedback          fp - 3   Value     undefined until H16 (lazy FeedbackVector)
//!       context           fp - 2   Value     current context
//!       bytecode offset   fp - 1   int32     saved at runtime calls (stack traces, H29)
//!       r0 .. r(m-1)      fp + 0.. Value     undefined on push
//! high  ... next frame ...                            <- top = fp + register_count
//! ```
//!
//! Every slot below the top holds a valid `Value` (03-gc §2 "VM-stack rule"), so the GC scans
//! `[base, top)` as one precise slice. Words that are not JS values use two encodings:
//!
//! | Encoding | Bits | Value kind | Used for |
//! |---|---|---|---|
//! | int32 | `0xFFFE_0000_0000_0000 \| n` | int32 | argc, bytecode offset |
//! | native word | `addr + 2^49` | a subnormal double | caller fp, return pc, function info |
//!
//! A native word never looks like a cell, so the GC ignores it. Decoding is one subtract.

use core::cell::Cell;
use core::ptr;

use ep9_value::Value;
use ep9_value::encoding::{DOUBLE_ENCODE_OFFSET, NUMBER_TAG};

/// Header size in words.
pub const HEADER: usize = 8;

/// Header slot offsets from `fp`, in words.
pub mod slot {
    /// Caller frame pointer (native word; null native word for the first frame).
    pub const CALLER_FP: isize = -8;
    /// Return pc (native word).
    pub const RETURN_PC: isize = -7;
    /// Callee function object (Value).
    pub const CALLEE: isize = -6;
    /// Actual argument count (int32), plus [`super::CONSTRUCT_BIT`].
    pub const ARGC: isize = -5;
    /// FunctionInfo pointer (native word).
    pub const FUNCTION_INFO: isize = -4;
    /// Feedback vector (Value; undefined in v0).
    pub const FEEDBACK: isize = -3;
    /// Current context (Value).
    pub const CONTEXT: isize = -2;
    /// Bytecode offset of the current instruction (int32), saved at runtime calls.
    pub const BYTECODE_OFFSET: isize = -1;
    /// The receiver (`this`).
    pub const RECEIVER: isize = -9;
    /// Parameter `a0`; `a<k>` is at `PARAM0 - k`.
    pub const PARAM0: isize = -10;
}

/// Set in the argc slot of a frame entered through `[[Construct]]`.
pub const CONSTRUCT_BIT: u32 = 1 << 30;

/// Encodes an address as a native word (a non-cell Value). Addresses on the 4 targets are
/// below 2^48 (aarch64 Linux uses 48-bit VA), so `addr + 2^49` is a subnormal double pattern.
#[inline(always)]
pub fn native(addr: usize) -> u64 {
    debug_assert!(
        addr < 1 << 49,
        "address {addr:#x} too large for a native word"
    );
    addr as u64 + DOUBLE_ENCODE_OFFSET
}

/// Encodes a pointer as a native word and exposes its provenance, so [`native_ptr`] can rebuild
/// a usable pointer.
#[inline(always)]
pub fn native_of<T>(p: *const T) -> u64 {
    native(p.expose_provenance())
}

/// Decodes a native word to an address.
#[inline(always)]
pub fn native_addr(bits: u64) -> usize {
    debug_assert!((DOUBLE_ENCODE_OFFSET..DOUBLE_ENCODE_OFFSET << 1).contains(&bits));
    (bits - DOUBLE_ENCODE_OFFSET) as usize
}

/// Decodes a native word to a pointer with exposed provenance.
#[inline(always)]
pub fn native_ptr<T>(bits: u64) -> *mut T {
    ptr::with_exposed_provenance_mut(native_addr(bits))
}

/// int32 Value bits for `n`.
#[inline(always)]
pub const fn int32_bits(n: i32) -> u64 {
    NUMBER_TAG | n as u32 as u64
}

/// The low 32 bits of an int32 word (no check).
#[inline(always)]
pub const fn int32_of(bits: u64) -> i32 {
    bits as u32 as i32
}

pub const UNDEFINED: u64 = Value::UNDEFINED.to_bits();
pub const NULL: u64 = Value::NULL.to_bits();
pub const TRUE: u64 = Value::TRUE.to_bits();
pub const FALSE: u64 = Value::FALSE.to_bits();
pub const HOLE: u64 = Value::EMPTY.to_bits();

/// The VM register stack of one isolate. The memory is zeroed on allocation and written before
/// it is read: the GC scans only `[base, top)`, and every slot there is written at frame push.
pub struct VmStack {
    base: *mut u64,
    slots: usize,
    /// The topmost frame at the last runtime entry, host call or entry from Rust; null when no
    /// frame is live. The GC root source reads it. Interpreter code writes it before every GC
    /// point (`Runtime::enter`).
    fp: Cell<*mut u64>,
}

impl VmStack {
    /// Allocates `slots` words of zeroed memory (the OS commits pages on first touch).
    pub fn new(slots: usize) -> VmStack {
        assert!(slots >= 64, "VM stack too small");
        let layout = std::alloc::Layout::array::<u64>(slots).expect("VM stack size");
        // SAFETY: the layout has a non-zero size (slots >= 64).
        let base = unsafe { std::alloc::alloc_zeroed(layout) }.cast::<u64>();
        if base.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        VmStack {
            base,
            slots,
            fp: Cell::new(ptr::null_mut()),
        }
    }

    pub fn base(&self) -> *mut u64 {
        self.base
    }

    /// One past the last slot.
    pub fn end(&self) -> *mut u64 {
        self.base.wrapping_add(self.slots)
    }

    pub fn fp(&self) -> *mut u64 {
        self.fp.get()
    }

    pub fn set_fp(&self, fp: *mut u64) {
        self.fp.set(fp);
    }
}

impl Drop for VmStack {
    fn drop(&mut self) {
        let layout = std::alloc::Layout::array::<u64>(self.slots).expect("VM stack size");
        // SAFETY: `base` came from `alloc_zeroed` with this layout and is freed once.
        unsafe { std::alloc::dealloc(self.base.cast(), layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_words_are_valid_non_cell_values() {
        for addr in [0usize, 8, 0x1234_5678, (1 << 47) - 8, (1 << 48) - 1] {
            let bits = native(addr);
            assert!(Value::is_valid_bits(bits), "{addr:#x}");
            // SAFETY: valid bits (checked above); not a cell pattern.
            let v = unsafe { Value::from_bits_unchecked(bits) };
            assert!(!v.is_cell() && v.is_double(), "{addr:#x}");
            assert_eq!(native_addr(bits), addr);
        }
        let x = 5u64;
        let p: *mut u64 = native_ptr(native_of(&x));
        // SAFETY: `p` is `&x` rebuilt with its exposed provenance.
        assert_eq!(unsafe { *p }, 5);
    }

    #[test]
    fn header_slots_and_int32_words() {
        assert_eq!(slot::CALLER_FP, -(HEADER as isize));
        assert_eq!(slot::BYTECODE_OFFSET, -1);
        assert_eq!(slot::RECEIVER, -(HEADER as isize) - 1);
        assert_eq!(slot::PARAM0, slot::RECEIVER - 1);
        assert_eq!(int32_of(int32_bits(-7)), -7);
        assert!(Value::is_valid_bits(int32_bits(i32::MIN)));
        assert_eq!(
            // SAFETY: int32 bits are always valid.
            unsafe { Value::from_bits_unchecked(int32_bits(42)) }.as_int32(),
            Some(42)
        );
    }

    #[test]
    fn stack_bounds() {
        let s = VmStack::new(128);
        assert!(s.fp().is_null());
        // SAFETY: slot 3 is inside the 128-slot stack; the memory is zeroed.
        assert_eq!(unsafe { *s.base().add(3) }, 0);
    }
}
