//! Proof that a test run built ep9_vm without the `jit` feature (MC-w22-12; plan v11 §2.9).
//!
//! The CI job "no-jit units (linux-x64)" sets `EP9_EXPECT_NO_JIT=1`. A cargo command can turn
//! `jit` back on by feature unification (one package of the run that enables `ep9_vm/jit`), and
//! the unit tests would still pass, only on the wrong build. This test fails then.
//!
//! ```text
//! EP9_EXPECT_NO_JIT   what the test does
//! (unset)             nothing: other runs build with or without jit (feature matrix, workspace)
//! 1                   asserts: no `jit` cfg, no "jit" in build_info().features, the default
//!                     preset is jitless with ics_only feedback, the jit preset is not available
//! anything else       fails: a typo must not turn the check off
//! ```
//!
//! Proof it can fail: `EP9_EXPECT_NO_JIT=1 cargo test -p ep9_vm --features jit --test
//! no_jit_build` fails on the first assert.

use ep9_vm::{ExecOptions, Feedback, Preset, build_info};

#[test]
fn the_no_jit_build_has_no_jit() {
    match std::env::var("EP9_EXPECT_NO_JIT") {
        Err(std::env::VarError::NotPresent) => return,
        Ok(v) if v == "1" => {}
        other => panic!("EP9_EXPECT_NO_JIT must be 1 or unset, got {other:?}"),
    }
    let info = build_info();
    if cfg!(feature = "jit") {
        panic!("EP9_EXPECT_NO_JIT=1, but ep9_vm was built with the `jit` feature ({info})");
    }
    assert!(
        !info.features.contains(&"jit"),
        "EP9_EXPECT_NO_JIT=1, but build_info() lists jit ({info})"
    );
    assert_eq!(info.preset, "jitless", "{info}");
    let d = ExecOptions::default();
    assert_eq!(d.preset(), Preset::JITLESS, "{info}");
    assert_eq!(d.feedback_mode(), Feedback::ICS_ONLY, "{info}");
    assert!(!Preset::JIT.available(), "{info}");
}
