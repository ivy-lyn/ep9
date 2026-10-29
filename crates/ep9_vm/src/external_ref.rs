//! `ExternalRef<T>` (C-26, H25-Q1; M5-P3 M1A-CODE): the one type of a native word in a heap
//! cell. Every heap-cell field that holds an off-heap address (the H25 inventory: FUNCTION
//! +24, FEEDBACK `fv::INFO`, CONTEXT `ctxf::SCOPE`, MODULE `modf::UNIT`, and the FEEDBACK Call
//! key half, a copy of a FUNCTION +24 word) is written from [`ExternalRef::new`] or from the
//! word of another `ExternalRef`, and read through [`ExternalRef::from_word`]. `cargo xtask
//! extref-lint` (H25-Q2) rejects a bare `native_of`, `native_ptr` or `native_addr` outside
//! this file and the VM stack code, and a read or a write of one of those fields that does
//! not go through this type (so a native word cannot be copied from one cell field into
//! another as a bare `u64`).
//!
//! ```text
//!  *const T --new--> ExternalRef<T> --to_word--> heap cell word (u64: addr + 2^49, a double)
//!  *const T <--get-- ExternalRef<T> <--from_word-- heap cell word
//! ```
//!
//! Today the word is the native-word encoding of H14 (`frame::native_of`): a subnormal double
//! Value, never a cell, so the GC skips it. A sandbox build (D6) would change this file only:
//! the word would be an index into a per-isolate pointer table. Every method is
//! `#[inline(always)]`, so the T0 handlers that read a FunctionInfo compile as before.

use core::marker::PhantomData;

/// A native pointer to a `T` as it is stored in a heap cell word.
#[repr(transparent)]
pub(crate) struct ExternalRef<T> {
    word: u64,
    _t: PhantomData<*const T>,
}

impl<T> Clone for ExternalRef<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ExternalRef<T> {}

impl<T> ExternalRef<T> {
    /// The reference to `p`: its native word (`frame::native_of`, which exposes the
    /// provenance).
    #[inline(always)]
    pub(crate) fn new(p: *const T) -> ExternalRef<T> {
        ExternalRef {
            word: crate::frame::native_of(p),
            _t: PhantomData,
        }
    }

    /// The pointer (`frame::native_ptr` of the word).
    #[inline(always)]
    pub(crate) fn get(self) -> *const T {
        crate::frame::native_ptr::<T>(self.word).cast_const()
    }

    /// The word a heap cell stores.
    #[inline(always)]
    pub(crate) fn to_word(self) -> u64 {
        self.word
    }

    /// The reference a heap cell word holds. The word must be one that [`to_word`] gave
    /// (a read of an inventory field); `get` of any other word gives a wrong pointer.
    ///
    /// [`to_word`]: ExternalRef::to_word
    #[inline(always)]
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
