//! The shadow-stack build rule (MC-w28-9.1; `build_flags.rs`, which `build.rs` runs on
//! `CARGO_ENCODED_RUSTFLAGS`): each opt-in form is refused, and the flags that do not turn on
//! a shadow stack pass.

#[path = "../build_flags.rs"]
mod build_flags;

use build_flags::shadow_stack_opt_in;

#[test]
fn every_shadow_stack_opt_in_is_found() {
    for flags in [
        &["-Zcf-protection=return"][..],
        &["-Zcf-protection=full"],
        &["-Z", "cf-protection=full"],
        &["-Zbranch-protection=bti,pac-ret,gcs"],
        &["-Z", "branch-protection=gcs"],
        &["-Clink-arg=/CETCOMPAT"],
        &["-C", "link-arg=/cetcompat"],
        &["-Clink-args=-Wl,-z,shstk"],
        &["-Clink-arg=-zshstk"],
        &["-Clink-arg=-z", "-Clink-arg=shstk"],
        &["-Copt-level=3", "-Clink-args=-z shstk"],
    ] {
        assert!(shadow_stack_opt_in(flags).is_some(), "{flags:?}");
    }
}

#[test]
fn other_flags_pass() {
    for flags in [
        &[][..],
        &["-Ctarget-cpu=native"],
        &["-Zcf-protection=branch"],
        &["-Zcf-protection=none"],
        &["-Zbranch-protection=bti,pac-ret"],
        &["-Clink-arg=-zstack-size=8388608"],
        &["-Clink-arg=/STACK:8388608"],
        &["-Cforce-frame-pointers=yes", "-Dwarnings"],
    ] {
        assert_eq!(shadow_stack_opt_in(flags), None, "{flags:?}");
    }
}
