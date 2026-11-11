// The real exports: each const has the name of a protected layout export.
pub const FUNCTION_INFO: usize = fun::INFO;
pub const CONTEXT_SCOPE: u32 = ctxf::SCOPE as u32;
pub const FEEDBACK_INFO: u32 = fv::INFO as u32;
pub const NATIVE_WORD_OFFSET: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET;
// A field that is not a native word is not protected.
pub const FUNCTION_CONTEXT: usize = fun::CONTEXT;
// Renames by value: H25:54 allows layout.rs, so no finding (the stricter rule of V11IN-22
// waits for route H25-STRICT-V13).
pub const FI: usize = fun::INFO;
pub static SCOPE_WORD: u32 = ctxf::SCOPE as u32;
pub const OFF: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET;
pub const MODULE_WORD: usize = modf::UNIT;
// Only the top-level items are exports: `layout::inner::FEEDBACK_INFO` is not an export name,
// so a use of it would not be seen (M7-P5 Codex T7).
pub mod inner {
    pub const FEEDBACK_INFO: u32 = fv::INFO as u32; // FLAG
    pub const OFFSET: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET; // FLAG
}
// A function or method body reads or writes the word: that is not an export, so the field rule
// holds (H25:55; M7-P5 Codex R3 finding 3).
pub fn raw_info(c: usize) -> u64 {
    unsafe { ((c + crate::object::fun::INFO) as *const u64).read() } // FLAG
}
pub unsafe fn raw_info_write(c: usize, w: u64) {
    unsafe { ((c + crate::object::fun::INFO) as *mut u64).write(w) } // FLAG
}
pub struct Cell(usize);
impl Cell {
    pub fn raw_scope(&self) -> u64 {
        unsafe { ((self.0 + ctxf::SCOPE) as *const u64).read() } // FLAG
    }
}
// The offset in a function body: H25:54 allows it in layout.rs.
pub fn offset() -> u64 {
    ep9_value::encoding::DOUBLE_ENCODE_OFFSET
}
