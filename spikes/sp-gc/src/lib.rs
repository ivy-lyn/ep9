//! SP-GC spike (M0 week 2; 03-gc section 2, R05 probe).
//!
//! ```text
//! mutator frames ──call──> safepoint ──> ep9_platform trampoline (spill callee-saved regs)
//!                                          └─> gc(sp): scan [sp, stack top) word by word
//!                                                 in cage? ─> VO lookup (interior, one-past-end)
//!                                                 ─> mark ─> precise roots ─> trace children
//!                                          checker: truth log (ids) ─> live set ─> all marked?
//! ```
//!
//! - `heap`: fake objects, VO bitmap, conservative scan, precise trace, sweep.
//! - `mutator`: random frames, the truth log and the 10^6-safepoint stress.
//! - `asm`: naked helpers for the deterministic probes and the no-spill control.
//! - `probes` (tests): the probe cases S1-S7, one or more tests per case.

pub mod asm;
pub mod heap;
pub mod mutator;
pub mod value;

#[cfg(test)]
mod probes;
