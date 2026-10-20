//! The H6 NaN-box subset that probe S6 needs (06-object-model section 1). Spike-local; the
//! real encoding lives in `ep9_value`.
//!
//! | Kind | 64-bit pattern |
//! |---|---|
//! | cell | the raw pointer, below 2^47 |
//! | int32 | `0xFFFE_0000_IIII_IIII` |
//! | double | raw bits + 2^49 (NaN purified first), so it is never below 2^49 |
//! | immediates | small constants with bit 1 set (null = 0x2, undefined = 0xA) |

pub const NUMBER_TAG: u64 = 0xFFFE_0000_0000_0000;
pub const DOUBLE_OFFSET: u64 = 1 << 49;
pub const NOT_CELL_MASK: u64 = NUMBER_TAG | 0x2;
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;
/// The tag the R05 probe used for its S6 case: a high-tagged pointer.
pub const PROBE_HIGH_TAG: u64 = 0xFFF9 << 48;

pub fn cell(addr: usize) -> u64 {
    addr as u64
}

pub fn int32(i: i32) -> u64 {
    NUMBER_TAG | i as u32 as u64
}

pub fn double(d: f64) -> u64 {
    let bits = if d.is_nan() {
        CANONICAL_NAN
    } else {
        d.to_bits()
    };
    bits + DOUBLE_OFFSET
}

pub fn is_cell(v: u64) -> bool {
    v & NOT_CELL_MASK == 0
}

pub fn as_int32(v: u64) -> Option<i32> {
    (v & NUMBER_TAG == NUMBER_TAG).then_some(v as u32 as i32)
}

pub fn as_double(v: u64) -> Option<f64> {
    (v & NUMBER_TAG != NUMBER_TAG && v >= DOUBLE_OFFSET).then(|| f64::from_bits(v - DOUBLE_OFFSET))
}
