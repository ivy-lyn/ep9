//! Passes the target triple to the report (`target` field).
fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=EP9_BENCH_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
