//! The `ep9_vm` build check (route SHSTK-ENGINE, M7-P4; MODES-CHECK MC-w28-9.1, doc 13 C-19):
//! a build of the engine does not opt in to a hardware shadow stack (x86-64 CET, AArch64 GCS).
//! `ep9_vm` holds `deopt::swap_returns`: lazy deopt replaces a native return address in a frame
//! record (the H29 Contract), and a shadow stack faults on that return. So every build that
//! links `ep9_vm` (the `ep9` binary with or without `jit`, the test262 runner, `ep9_unit`) gets
//! the same refusal as `crates/ep9_unit/build.rs`, from the same rule file
//! (`crates/ep9_unit/build_flags.rs`, by `#[path]`: no `ep9_unit` edge, no Cargo.toml key).
//!
//! ```text
//! CARGO_ENCODED_RUSTFLAGS (split at 0x1f) --shadow_stack_opt_in--> None: build on
//!                                                                  Some(flag): panic, name it
//! ```

#[path = "../ep9_unit/build_flags.rs"]
mod build_flags;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../ep9_unit/build_flags.rs");
    let rustflags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let flags: Vec<&str> = rustflags.split('\x1f').filter(|f| !f.is_empty()).collect();
    if let Some(flag) = build_flags::shadow_stack_opt_in(&flags) {
        panic!(
            "ep9 does not opt in to hardware shadow stacks (CET, GCS) until the H29 A4 part \
             proves a lazy-deopt form for native return addresses (doc 13 C-19, MC-w28-9.1, \
             SHSTK-ENGINE); remove the flag {flag:?}"
        );
    }
}
