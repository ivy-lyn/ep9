// The module file is outside src/, so the lint never reads it.
#[path = "../hidden/mod.rs"] // FLAG
mod hidden;
#[cfg_attr(feature = "x", path = "../hidden/mod.rs")] // FLAG
mod hidden_cfg;
// Two cfg_attr paths: the first is scanned, the second is not; each is checked (M7-P5
// Codex T6).
#[cfg_attr(windows, path = "inner/ok.rs")] // FLAG
#[cfg_attr(unix, path = "../hidden/mod.rs")]
mod two;
// A #[path] module file that the lint scans.
#[path = "inner/ok.rs"]
mod ok;
// A test module is not read (test code).
#[cfg(test)]
#[path = "../hidden/mod.rs"]
mod hidden_tests;
