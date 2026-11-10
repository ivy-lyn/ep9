pub const DOUBLE_ENCODE_OFFSET: u64 = 1 << 49;
// The Value encoding boxes a double with the offset: allowed in this crate.
pub fn box_double(bits: u64) -> u64 {
    bits.wrapping_add(DOUBLE_ENCODE_OFFSET)
}
// A rename of the offset by value.
pub const SHIFT: u64 = DOUBLE_ENCODE_OFFSET; // FLAG
pub static SHIFT_STATIC: u64 = DOUBLE_ENCODE_OFFSET; // FLAG
