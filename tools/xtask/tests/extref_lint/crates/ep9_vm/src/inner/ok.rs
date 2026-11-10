pub fn ok(c: CellPtr, fi: &FunctionInfo) {
    init(c, fv::INFO, ExternalRef::new(fi).to_word());
}
