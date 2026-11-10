impl<T> ExternalRef<T> {
    pub(crate) fn new(p: *const T) -> ExternalRef<T> {
        ExternalRef { w: crate::frame::native_of(p), _t: PhantomData }
    }
    pub(crate) fn get(self) -> *const T {
        crate::frame::native_ptr(self.w)
    }
    pub(crate) fn addr(self) -> Option<usize> {
        self.w.checked_sub(DOUBLE_ENCODE_OFFSET)
    }
    // A wrapper of the encoding in the type file.
    pub(crate) fn raw_word(p: *const T) -> u64 {
        crate::frame::native_of(p) // FLAG
    }
}
