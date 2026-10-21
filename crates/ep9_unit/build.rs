#![allow(dead_code)] // m2-stub
//! The engine build id (H35-Q8): writes `$OUT_DIR/engine_build_id.bin`, 16 bytes.
//!
//! Stub (M0 month 2, P0): the 16 bytes are all zero, the same value as
//! `ENGINE_BUILD_ID` in `src/host.rs` today, and nothing reads the file yet. The BUILD-ID task
//! (P2) makes it the first 16 bytes of SHA-256 over the commit, `rustc -vV`, the target and the
//! features (with `ep9_bytecode::sha256`, the build dependency), adds the `EP9_BUILD_ID`
//! override, and points `ENGINE_BUILD_ID` at this file.

use std::path::PathBuf;

/// Length of the build id in bytes (`ENGINE_BUILD_ID: [u8; 16]`).
const BUILD_ID_LEN: usize = 16;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("engine_build_id.bin"), [0u8; BUILD_ID_LEN])
        .expect("write engine_build_id.bin");
    // The stub reads no input, so it reruns only when this file changes.
    println!("cargo::rerun-if-changed=build.rs");
}
