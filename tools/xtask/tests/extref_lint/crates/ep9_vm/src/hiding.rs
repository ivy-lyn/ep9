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
// Data, not code: include_str! and include_bytes! are not findings.
const TEXT: &str = include_str!("../hidden/word.inc");
