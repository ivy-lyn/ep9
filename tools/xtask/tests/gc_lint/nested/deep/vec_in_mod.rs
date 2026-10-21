//! gc-lint case: a finding in a nested directory; its path prints with `/` on every OS.

pub mod inner {
    pub type Locals<'s> = std::vec::Vec<crate::abi::Local<'s>>; // FLAG
}
