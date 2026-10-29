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

use ep9_value::encoding::DOUBLE_ENCODE_OFFSET;

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

    /// The address the word names if it is a native word (`frame::native_addr`), else
    /// `None`: for a word that may hold another Value, such as the FEEDBACK Call key half
    /// (the hole, undefined or a cell; `facts_export.rs`). Not a pointer read.
    #[inline(always)]
    pub(crate) fn addr(self) -> Option<usize> {
        let native = DOUBLE_ENCODE_OFFSET..DOUBLE_ENCODE_OFFSET << 1;
        native
            .contains(&self.word)
            .then(|| crate::frame::native_addr(self.word))
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

    /// `addr` gives the address of a native word and `None` for any other Value (M5-P3
    /// reviewer A F3: the FEEDBACK Call key read of `facts_export.rs` goes through it).
    #[test]
    fn addr_names_a_native_word_only() {
        let x = 7u64;
        let p: *const u64 = &x;
        assert_eq!(ExternalRef::new(p).addr(), Some(p.addr()));
        // Undefined, the hole, null, an int32, a cell-shaped word, a double Value, 0 and all ones.
        let double = 1.5f64.to_bits() + super::DOUBLE_ENCODE_OFFSET;
        let words = [
            frame::UNDEFINED,
            frame::HOLE,
            frame::NULL,
            frame::int32_bits(3),
        ];
        for w in words.into_iter().chain([0x1_0000, double, 0, u64::MAX]) {
            assert_eq!(ExternalRef::<u64>::from_word(w).addr(), None, "{w:#x}");
        }
    }
}
