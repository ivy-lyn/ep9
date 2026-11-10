// The module file is outside src/, so the lint never reads it.
#[path = "../hidden/mod.rs"] // FLAG
mod hidden;
#[cfg_attr(feature = "x", path = "../hidden/mod.rs")] // FLAG
mod hidden_cfg;
// A #[path] module file that the lint scans.
#[path = "inner/ok.rs"]
mod ok;
// A test module is not read (test code).
#[cfg(test)]
#[path = "../hidden/mod.rs"]
mod hidden_tests;
