#![allow(dead_code)] // m5-stub
//! `ExternalRef<T>` (owner M5-P3 M1A-CODE; C-26, H25-Q2, H25:55): the one type of a native
//! word in a heap cell. From M1a every such field has this type, and `cargo xtask
//! extref-lint` rejects a bare `native_of` or `native_ptr` outside it (and the VM stack code)
//! and a native word copied from one cell field into another.
//!
//! ```text
//!  heap cell word (u64, the native-word encoding) <-> ExternalRef<T> <-> *const T
//! ```
//!
//! M5-P0 stub: the type and its signatures; nothing uses it yet.

use core::marker::PhantomData;

/// A native pointer to a `T` as it is stored in a heap cell word.
#[repr(transparent)]
pub(crate) struct ExternalRef<T> {
    word: u64,
    _t: PhantomData<*const T>,
}

impl<T> Clone for ExternalRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ExternalRef<T> {}

impl<T> ExternalRef<T> {
    /// The reference to `p`. M5-P0 stub: the word is the address.
    pub(crate) fn new(p: *const T) -> ExternalRef<T> {
        ExternalRef {
            word: p as usize as u64,
            _t: PhantomData,
        }
    }

    /// The pointer.
    pub(crate) fn get(self) -> *const T {
        self.word as usize as *const T
    }

    /// The word a heap cell stores.
    pub(crate) fn to_word(self) -> u64 {
        self.word
    }

    /// The reference a heap cell word holds.
    pub(crate) fn from_word(word: u64) -> ExternalRef<T> {
        ExternalRef {
            word,
            _t: PhantomData,
        }
    }
}
