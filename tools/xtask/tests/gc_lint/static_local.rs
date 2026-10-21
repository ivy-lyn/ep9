//! gc-lint case: a `static` that holds a `Local` (by value or through a reference).

use std::sync::Mutex;

use crate::abi::Local;

pub static LAST: Mutex<Option<Local<'static>>> = Mutex::new(None); // FLAG
pub static NONE: &[Local<'static>] = &[]; // FLAG
pub static COUNT: Mutex<u32> = Mutex::new(0);
