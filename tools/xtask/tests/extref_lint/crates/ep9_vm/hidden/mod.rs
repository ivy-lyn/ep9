// Never read by the lint: a bare native word in a heap cell.
pub fn sneak(c: CellPtr, fi: &FunctionInfo) {
    init(c, fun::INFO, native_of(fi));
}
