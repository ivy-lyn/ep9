// The real exports: each const has the name of a protected layout export.
pub const FUNCTION_INFO: usize = fun::INFO;
pub const CONTEXT_SCOPE: u32 = ctxf::SCOPE as u32;
pub const FEEDBACK_INFO: u32 = fv::INFO as u32;
pub const NATIVE_WORD_OFFSET: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET;
// A field that is not a native word is not protected.
pub const FUNCTION_CONTEXT: usize = fun::CONTEXT;
// Renames by value: the new names are not in the token rules.
pub const FI: usize = fun::INFO; // FLAG
pub static SCOPE_WORD: u32 = ctxf::SCOPE as u32; // FLAG
pub const OFF: u64 = ep9_value::encoding::DOUBLE_ENCODE_OFFSET; // FLAG
pub const MODULE_WORD: usize = modf::UNIT; // FLAG
