// The macro joins `$m::$f`: no token of this file is the path `fun::INFO`.
macro_rules! rd2 {
    ($m:ident, $f:ident, $c:expr) => {
        rd($c, $m::$f)
    };
}
fn a(c: CellPtr) -> u64 {
    rd2!(fun, INFO, c) // FLAG
}
macro_rules! enc2 {
    ($m:ident, $f:ident, $a:expr) => {
        $m::$f($a)
    };
}
fn b(a: usize) -> u64 {
    enc2!(frame, native, a) // FLAG
}
// A call nested in a macro body.
macro_rules! outer {
    ($c:expr) => {
        rd2!(ctxf, SCOPE, $c) // FLAG
    };
}
// The pair through ExternalRef in the same statement passes (the field rule).
fn c(c: CellPtr) -> u64 {
    ExternalRef::<FunctionInfo>::from_word(rd2!(fun, INFO, c)).addr()
}
