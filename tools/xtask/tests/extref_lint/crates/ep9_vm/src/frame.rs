// The four encoders (ENCODERS).
pub fn native(addr: usize) -> u64 {
    addr as u64 + DOUBLE_ENCODE_OFFSET
}
pub fn native_of<T>(p: *const T) -> u64 {
    native(p as usize)
}
pub fn native_addr(bits: u64) -> usize {
    (bits - DOUBLE_ENCODE_OFFSET) as usize
}
pub fn native_ptr<T>(bits: u64) -> *mut T {
    native_addr(bits) as *mut T
}
// Wrappers: new names for the encoding.
pub fn word_of<T>(p: *const T) -> u64 {
    native_of(p) // FLAG
}
pub fn decode(bits: u64) -> usize {
    (bits - DOUBLE_ENCODE_OFFSET) as usize // FLAG
}
#[macro_export]
macro_rules! nw {
    ($p:expr) => {
        $crate::frame::native_of($p) // FLAG
    };
}
pub const ENC: fn(usize) -> u64 = native; // FLAG
pub const UNDEFINED: u64 = 0x0a;
