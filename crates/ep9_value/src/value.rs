//! The NaN-box encoding (H6). It follows JSC: WebKit
//! `Source/JavaScriptCore/runtime/JSCJSValue.h` (commit e4be3948, lines 327-427) and
//! `runtime/PureNaN.h` (PNaN = 0x7ff8000000000000). Two ep9 differences:
//! - Empty/hole is the non-zero immediate 0x22, not 0 (JSC `ValueEmpty`). Zero stays the niche.
//! - Every NaN is purified on boxing, not only impure ones (06-object-model §1).

use core::fmt;
use core::num::NonZeroU64;
use core::ptr::{self, NonNull};

use encoding::*;

/// Encoding constants. JIT and AOT code use the same values; units record [`VERSION`].
pub mod encoding {
    /// Value-encoding version. The unit header records it (H35); a mismatch makes the loader
    /// ignore native code and run bytecode.
    pub const VERSION: u32 = 1;
    /// Doubles are stored as f64 bits + 2^49 (wrapping), as in JSC.
    pub const DOUBLE_ENCODE_OFFSET: u64 = 1 << 49;
    /// All bits set: int32. Some but not all set: double. None set: cell or immediate.
    pub const NUMBER_TAG: u64 = 0xFFFE_0000_0000_0000;
    /// Set in every non-number immediate. Never set in an 8-aligned cell pointer.
    pub const OTHER_TAG: u64 = 0x2;
    pub const BOOL_TAG: u64 = 0x4;
    pub const UNDEFINED_TAG: u64 = 0x8;
    /// ep9 only. JSC uses 0x10 for BigInt32 (not used by ep9), so this takes 0x20.
    pub const EMPTY_TAG: u64 = 0x20;
    /// `v & NOT_CELL_MASK == 0` means `v` is a cell. Not an aarch64 logical immediate.
    pub const NOT_CELL_MASK: u64 = NUMBER_TAG | OTHER_TAG;

    pub const VALUE_NULL: u64 = OTHER_TAG; // 0x02
    pub const VALUE_FALSE: u64 = OTHER_TAG | BOOL_TAG; // 0x06
    pub const VALUE_TRUE: u64 = OTHER_TAG | BOOL_TAG | 1; // 0x07
    pub const VALUE_UNDEFINED: u64 = OTHER_TAG | UNDEFINED_TAG; // 0x0a
    /// Array hole and "no value yet" (TDZ). Never visible to JS code.
    pub const VALUE_EMPTY: u64 = OTHER_TAG | EMPTY_TAG; // 0x22

    /// The one NaN that boxing produces (JSC PNaN).
    pub const PURE_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;
    /// Cell pointers are below this address: the cage is below 2^47 (H7).
    pub const CELL_ADDRESS_LIMIT: u64 = 1 << 47;
    /// Cells are 8-byte aligned, so bits 0-2 of a cell pointer are 0.
    pub const CELL_ALIGN: u64 = 8;
}

/// A JS value in 8 bytes (H6). `Copy`, no destructor, never the bit pattern 0.
///
/// `Value` has no `PartialEq`: bit equality is not JS equality (strings, int32 1 vs double 1.0).
/// Compare [`Value::to_bits`] when encoding identity is what you mean.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Value(NonZeroU64);

/// A decoded [`Value`], for `match` and `Debug`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unpacked {
    Cell(NonNull<u8>),
    Int32(i32),
    Double(f64),
    Bool(bool),
    Undefined,
    Null,
    Empty,
}

/// Builds a constant. Panics at compile time if `bits` is 0.
const fn imm(bits: u64) -> Value {
    match NonZeroU64::new(bits) {
        Some(n) => Value(n),
        None => panic!("Value bits are never 0"),
    }
}

impl Value {
    pub const UNDEFINED: Value = imm(VALUE_UNDEFINED);
    pub const NULL: Value = imm(VALUE_NULL);
    pub const FALSE: Value = imm(VALUE_FALSE);
    pub const TRUE: Value = imm(VALUE_TRUE);
    pub const EMPTY: Value = imm(VALUE_EMPTY);

    // ---- constructors ----

    #[inline(always)]
    pub const fn int32(i: i32) -> Value {
        // The tag makes the bits non-zero, so `imm` never panics here.
        imm(NUMBER_TAG | i as u32 as u64)
    }

    /// Boxes `d` in the double form. Every NaN becomes [`PURE_NAN_BITS`] first. Without that,
    /// a NaN with top bits 0xFFFC..=0xFFFF would encode as an int32, a cell pointer or 0.
    #[inline(always)]
    pub const fn double(d: f64) -> Value {
        let bits = if d.is_nan() {
            PURE_NAN_BITS
        } else {
            d.to_bits()
        };
        let enc = bits.wrapping_add(DOUBLE_ENCODE_OFFSET);
        // SAFETY: `enc` is 0 only for bits 0xFFFE_0000_0000_0000, which is a NaN. NaNs were
        // replaced by PURE_NAN_BITS above, so `enc` is not 0.
        Value(unsafe { NonZeroU64::new_unchecked(enc) })
    }

    /// Boxes a JS number: int32 when `d` is an integer in i32 range and not -0, else double.
    #[inline(always)]
    pub const fn number(d: f64) -> Value {
        let i = d as i32; // saturates; NaN gives 0
        if i as f64 == d && (i != 0 || d.to_bits() == 0) {
            Value::int32(i)
        } else {
            Value::double(d)
        }
    }

    #[inline(always)]
    pub const fn bool(b: bool) -> Value {
        if b { Value::TRUE } else { Value::FALSE }
    }

    /// Boxes a cell pointer. The address is stored as is (no tag).
    ///
    /// # Safety
    /// `p` must point to a live GC cell inside the cage: 8-byte aligned and below 2^47
    /// ([`CELL_ADDRESS_LIMIT`]). The GC, the barrier and JIT code trust every cell `Value`.
    #[inline(always)]
    pub unsafe fn from_cell_ptr<T>(p: NonNull<T>) -> Value {
        // Expose the provenance so `as_cell_ptr` can rebuild a usable pointer from the bits.
        let addr = p.as_ptr().expose_provenance() as u64;
        debug_assert!(
            is_cell_address(addr),
            "cell pointer outside the cage or misaligned"
        );
        // SAFETY: `NonNull` addresses are not 0.
        Value(unsafe { NonZeroU64::new_unchecked(addr) })
    }

    /// Rebuilds a `Value` from raw bits (VM stack, heap slots, snapshot, JIT code).
    ///
    /// # Safety
    /// `bits` must satisfy [`Value::is_valid_bits`], and a cell pattern must point to a live
    /// GC cell, as for [`Value::from_cell_ptr`].
    #[inline(always)]
    pub const unsafe fn from_bits_unchecked(bits: u64) -> Value {
        debug_assert!(Value::is_valid_bits(bits));
        // SAFETY: valid bits are never 0 (caller contract).
        Value(unsafe { NonZeroU64::new_unchecked(bits) })
    }

    // ---- raw access ----

    #[inline(always)]
    pub const fn to_bits(self) -> u64 {
        self.0.get()
    }

    /// True if `bits` is a pattern some constructor can produce: a cell address, an int32, a
    /// double that is not an impure NaN, or one of the 5 immediates.
    pub const fn is_valid_bits(bits: u64) -> bool {
        if bits & NUMBER_TAG == NUMBER_TAG {
            // int32: bits 32-48 must be 0.
            return bits >> 32 == NUMBER_TAG >> 32;
        }
        if bits & NUMBER_TAG != 0 {
            let d = bits.wrapping_sub(DOUBLE_ENCODE_OFFSET);
            return !f64::from_bits(d).is_nan() || d == PURE_NAN_BITS;
        }
        matches!(
            bits,
            VALUE_NULL | VALUE_FALSE | VALUE_TRUE | VALUE_UNDEFINED | VALUE_EMPTY
        ) || is_cell_address(bits)
    }

    // ---- type tests (JSC JSCJSValue.h lines 712-760) ----

    #[inline(always)]
    pub const fn is_cell(self) -> bool {
        self.to_bits() & NOT_CELL_MASK == 0
    }

    #[inline(always)]
    pub const fn is_number(self) -> bool {
        self.to_bits() & NUMBER_TAG != 0
    }

    #[inline(always)]
    pub const fn is_int32(self) -> bool {
        self.to_bits() & NUMBER_TAG == NUMBER_TAG
    }

    #[inline(always)]
    pub const fn is_double(self) -> bool {
        self.is_number() && !self.is_int32()
    }

    #[inline(always)]
    pub const fn is_bool(self) -> bool {
        self.to_bits() & !1 == VALUE_FALSE
    }

    #[inline(always)]
    pub const fn is_undefined(self) -> bool {
        self.to_bits() == VALUE_UNDEFINED
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.to_bits() == VALUE_NULL
    }

    /// One mask and compare: undefined (0x0a) and null (0x02) differ only in UNDEFINED_TAG.
    #[inline(always)]
    pub const fn is_undefined_or_null(self) -> bool {
        self.to_bits() & !UNDEFINED_TAG == VALUE_NULL
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.to_bits() == VALUE_EMPTY
    }

    // ---- payload access ----

    #[inline(always)]
    pub const fn as_int32(self) -> Option<i32> {
        if self.is_int32() {
            Some(self.to_bits() as u32 as i32)
        } else {
            None
        }
    }

    /// The f64 of a double-form value. `None` for int32 and non-numbers.
    #[inline(always)]
    pub const fn as_double(self) -> Option<f64> {
        if self.is_double() {
            Some(f64::from_bits(
                self.to_bits().wrapping_sub(DOUBLE_ENCODE_OFFSET),
            ))
        } else {
            None
        }
    }

    /// The JS number, from either form.
    #[inline(always)]
    pub const fn as_number(self) -> Option<f64> {
        match self.as_int32() {
            Some(i) => Some(i as f64),
            None => self.as_double(),
        }
    }

    #[inline(always)]
    pub const fn as_bool(self) -> Option<bool> {
        if self.is_bool() {
            Some(self.to_bits() & 1 == 1)
        } else {
            None
        }
    }

    /// The cell pointer, with the provenance that [`Value::from_cell_ptr`] exposed.
    #[inline(always)]
    pub fn as_cell_ptr(self) -> Option<NonNull<u8>> {
        if self.is_cell() {
            let p = ptr::with_exposed_provenance_mut::<u8>(self.to_bits() as usize);
            // SAFETY: Value bits are never 0, so `p` is not null.
            Some(unsafe { NonNull::new_unchecked(p) })
        } else {
            None
        }
    }

    pub fn unpack(self) -> Unpacked {
        if let Some(i) = self.as_int32() {
            return Unpacked::Int32(i);
        }
        if let Some(d) = self.as_double() {
            return Unpacked::Double(d);
        }
        if let Some(p) = self.as_cell_ptr() {
            return Unpacked::Cell(p);
        }
        match self.to_bits() {
            VALUE_UNDEFINED => Unpacked::Undefined,
            VALUE_NULL => Unpacked::Null,
            VALUE_FALSE => Unpacked::Bool(false),
            VALUE_TRUE => Unpacked::Bool(true),
            VALUE_EMPTY => Unpacked::Empty,
            bits => unreachable!("invalid Value bits {bits:#x}"),
        }
    }
}

/// True if `addr` can be a cell: not 0, 8-byte aligned, below 2^47.
pub(crate) const fn is_cell_address(addr: u64) -> bool {
    addr != 0 && addr.is_multiple_of(CELL_ALIGN) && addr < CELL_ADDRESS_LIMIT
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unpack() {
            Unpacked::Cell(p) => write!(f, "Value(cell {p:p})"),
            Unpacked::Int32(i) => write!(f, "Value(int32 {i})"),
            Unpacked::Double(d) => write!(f, "Value(double {d:?})"),
            Unpacked::Bool(b) => write!(f, "Value({b})"),
            Unpacked::Undefined => f.write_str("Value(undefined)"),
            Unpacked::Null => f.write_str("Value(null)"),
            Unpacked::Empty => f.write_str("Value(empty)"),
        }
    }
}
