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
//! M5-P0 stub: the type and its signatures; nothing uses it yet. Since M5-P1 (the phase fixer,
//! M5-P0-ORCH-12, MODES-CHECK-w18 MC-w18-13b) `new` and `get` use the native-word encoding
//! (`frame::native_of`, `frame::native_ptr`): the word is a double Value, never a raw address.

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
    /// The reference to `p`: its native word (`frame::native_of`, which exposes the
    /// provenance).
    pub(crate) fn new(p: *const T) -> ExternalRef<T> {
        ExternalRef {
            word: crate::frame::native_of(p),
            _t: PhantomData,
        }
    }

    /// The pointer (`frame::native_ptr` of the word).
    pub(crate) fn get(self) -> *const T {
        crate::frame::native_ptr::<T>(self.word).cast_const()
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

#[cfg(test)]
mod tests {
    use super::ExternalRef;
    use crate::frame;

    /// The cell word is the native-word encoding of the address (address +
    /// `DOUBLE_ENCODE_OFFSET`, a double Value), not the raw address, and it round-trips.
    #[test]
    fn the_word_is_the_native_word_encoding() {
        let x = 7u64;
        let p: *const u64 = &x;
        let r = ExternalRef::new(p);
        assert_eq!(r.to_word(), frame::native_of(p));
        assert_ne!(r.to_word(), p.addr() as u64);
        assert_eq!(frame::native_addr(r.to_word()), p.addr());
        let back = ExternalRef::<u64>::from_word(r.to_word()).get();
        assert_eq!(back, p);
        // SAFETY: `back` is `p`, which points at the live local `x`.
        assert_eq!(unsafe { *back }, 7);
    }
}
