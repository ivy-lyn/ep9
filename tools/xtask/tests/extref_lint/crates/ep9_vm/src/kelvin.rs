// U+212A KELVIN SIGN: rustc reads FEEDBAC\u{212A}_INFO as FEEDBACK_INFO (identifiers are NFC).
fn a(c: CellPtr) -> u64 {
    rd(c, layout::FEEDBACK_INFO) // FLAG
}
// Another non-ASCII name is not a protected name.
fn b(c: CellPtr) -> u64 {
    rd(c, layout::FEEDBACK_Å)
}
