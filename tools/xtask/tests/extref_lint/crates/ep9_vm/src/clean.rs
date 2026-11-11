// Negative controls: none of these is a finding.
fn a(c: CellPtr, fi: &FunctionInfo) {
    init(c, fv::INFO, ExternalRef::new(fi).to_word());
}
// A module name alone, or a field name alone, in a macro call.
fn b(fun: u64) -> String {
    format!("{fun} {}", INFO)
}
// `if !(..)` is not a macro call.
fn c(fun: bool, INFO: bool) -> bool {
    if !(fun && INFO) { true } else { false }
}
// A field that holds a Value, through a macro argument pair.
fn d(c: CellPtr) -> u64 {
    rd2!(fun, CONTEXT, c)
}
