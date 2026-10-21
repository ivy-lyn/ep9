//! Host (Rust) functions callable from JS: the names of the week-3 API. The builtin ABI v1
//! (H20) lives in [`crate::abi`]; this module keeps the old names for one release.
//!
//! | Old name | Now |
//! |---|---|
//! | `HostFn` | [`crate::abi::HostFn`]: `for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>` |
//! | `HostCx<'a>` | an alias of [`crate::abi::Cx`] (deprecated name; removed after one release) |
//! | `Args<'a>` | [`crate::abi::Args`]: one word, the host frame pointer |

pub use crate::abi::HostFn;

/// The week-3 name of [`crate::abi::Cx`]. Kept for one release (H20); new code names `Cx`.
pub type HostCx<'s> = crate::abi::Cx<'s>;
