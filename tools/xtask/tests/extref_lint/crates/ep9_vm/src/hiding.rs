// Code from a file that the lint does not parse.
fn a(c: CellPtr) -> u64 {
    include!("../hidden/word.inc") // FLAG
}
// A name joined from parts (nightly forms; the tree uses neither).
fn b(p: *const u8) -> u64 {
    concat_idents!(native, _of)(p) // FLAG
}
macro_rules! joined {
    ($a:ident, $b:ident) => {
        ${concat($a, $b)} // FLAG
    };
}
// A rename of include! hides the call by its new name (M7-P5 Codex T5).
use std::include as source; // FLAG
pub use core::concat_idents as join; // FLAG
fn d() -> u64 {
    source!("../hidden/word.inc")
}
// Data, not code: include_str! and include_bytes! are not findings.
const TEXT: &str = include_str!("../hidden/word.inc");
