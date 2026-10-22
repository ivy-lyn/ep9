//! Worker stack size, from measured stack use per depth unit (`probe.rs`, H23).
//!
//! ```text
//! WORKER_STACK_BYTES = BASE + SAFETY x max( PRESCAN_CAP x PARSE_PER_UNIT,     oxc_parser (layer 0 bound)
//!                                           AST_CAP     x PASS_PER_LEVEL )    depth pass, oxc_semantic,
//!                                                                             scope pass (layer 2 bound)
//! ```
//!
//! Measured on aarch64-apple-darwin, 2026-10-21, `probe::measure_forms` (n = 500 -> 1500), worst
//! form per column. The other targets are covered by `SAFETY` and checked by the depth tests,
//! which run every form at the caps. CI runs them in debug on all 4 CI targets (`cargo test
//! --workspace`); the release run is local only.
//!
//! | Bytes per unit | debug | release | worst form |
//! |---|---|---|---|
//! | oxc_parser, per pre-scan unit | 4,240 | 2,288 | debug: nested functions; release: nested objects |
//! | oxc regex parser, per group | 16,864 | 720 | regex groups (weighted 4 units, `prescan::REGEX_WEIGHT`) |
//! | layer-2 depth pass, per AST level | 800 | 80 | debug: arrays; release: class heritage, assignment |
//! | oxc_semantic, per AST level | 1,440 | 256 | arrays / class heritage |
//! | scope pass (`scope::analyze`), per AST level | 2,016 | 240 | class heritage |
//!
//! The reservation is virtual: a thread only commits the pages it touches. Normal code touches
//! well under 1 MiB of it.

use crate::depth::{AST_CAP, PRESCAN_CAP};

/// Fixed cost: thread start, the bases of oxc_parser (about 6 KiB) and oxc_semantic (about
/// 40 KiB), our pass frames, and slack.
const BASE: usize = 2 << 20;

/// Margin for targets that were not measured (Windows x64: shadow space and larger frames).
const SAFETY_NUM: usize = 3;
const SAFETY_DEN: usize = 2;

#[cfg(debug_assertions)]
const PARSE_PER_UNIT: usize = 4_608;
#[cfg(not(debug_assertions))]
const PARSE_PER_UNIT: usize = 2_560;

/// The largest per-AST-level cost of the passes that recurse over the AST after layer 2:
/// the depth pass, oxc_semantic and the scope pass (debug worst: scope pass, 2,016 B).
/// In both profiles the parse bound is larger, so this constant does not set the stack size.
#[cfg(debug_assertions)]
const PASS_PER_LEVEL: usize = 2_560;
#[cfg(not(debug_assertions))]
const PASS_PER_LEVEL: usize = 512;

const fn max(a: usize, b: usize) -> usize {
    if a > b { a } else { b }
}

/// Stack of the front-end worker thread.
pub(crate) const WORKER_STACK_BYTES: usize = BASE
    + max(
        PRESCAN_CAP as usize * PARSE_PER_UNIT,
        AST_CAP as usize * PASS_PER_LEVEL,
    ) / SAFETY_DEN
        * SAFETY_NUM;

// Checked in every build: the stack fits both bounds with the margin, and stays a sane
// reservation (under 256 MiB even in debug).
const _: () = {
    let parse = PRESCAN_CAP as usize * PARSE_PER_UNIT;
    let passes = AST_CAP as usize * PASS_PER_LEVEL;
    assert!(WORKER_STACK_BYTES >= BASE + parse + parse / 2);
    assert!(WORKER_STACK_BYTES >= BASE + passes + passes / 2);
    assert!(WORKER_STACK_BYTES < 256 << 20);
};
