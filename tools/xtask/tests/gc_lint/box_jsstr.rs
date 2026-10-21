//! gc-lint case: `Box<[JsStr]>`, views into string cells kept in heap memory.

use crate::string::JsStr;

pub struct Pieces<'a> {
    pub count: usize,
    pub parts: Box<[JsStr<'a>]>, // FLAG
}
