//! Exports the target triple, which names the default expectation file.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=EP9_TARGET_TRIPLE={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
