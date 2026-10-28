//! Exports the target triple, which names the default expectation file, and the commit this
//! runner is built at (`EP9_RUNNER_COMMIT`, C2; plan v10 section 2.6), which the exec JSON
//! writes as `"runner_commit"` (src/provenance.rs).
//!
//! The commit is `git rev-parse HEAD` of this checkout, or empty when git cannot answer. The
//! script reruns when HEAD moves: it watches the HEAD paths that `ep9_unit`'s build script
//! watches, chosen by the same file (`crates/ep9_unit/build_watch.rs`, tested there). While git
//! cannot answer it watches a path in `OUT_DIR` that never exists, so cargo reruns it on every
//! build until git answers (the ep9_unit fix of month-5 input item 10).

use std::path::PathBuf;

#[path = "../../crates/ep9_unit/build_watch.rs"]
mod build_watch;

/// The watched path, in `OUT_DIR`, while git cannot answer: it never exists.
const NO_GIT_WATCH: &str = "ep9-no-git.rerun";

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=EP9_TARGET_TRIPLE={target}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../crates/ep9_unit/build_watch.rs");

    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let commit = build_watch::git(&dir, &["rev-parse", "--verify", "HEAD^{commit}"])
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| c.len() == 40 && c.bytes().all(|b| b.is_ascii_hexdigit()));
    match &commit {
        Some(_) => {
            for p in build_watch::head_watch_paths(&dir) {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
        None => println!(
            "cargo:rerun-if-changed={}",
            out.join(NO_GIT_WATCH).display()
        ),
    }
    println!(
        "cargo:rustc-env=EP9_RUNNER_COMMIT={}",
        commit.as_deref().unwrap_or("")
    );
}
