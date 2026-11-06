//! Clippy reads the nearest clippy.toml only, so this crate's file must repeat every ban of the
//! root clippy.toml (H36), as ep9_builtins' does (its `tests/table.rs`).

use std::path::PathBuf;

#[test]
fn clippy_toml_repeats_the_root_bans() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ours = std::fs::read_to_string(dir.join("clippy.toml")).unwrap();
    let root = std::fs::read_to_string(dir.join("../../clippy.toml")).unwrap();
    let paths = |text: &str| -> Vec<String> {
        text.lines()
            .filter_map(|l| l.split("path = \"").nth(1))
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_string)
            .collect()
    };
    let root_paths = paths(&root);
    assert!(root_paths.len() > 50, "the root bans: {}", root_paths.len());
    let our_paths = paths(&ours);
    for p in &root_paths {
        assert!(
            our_paths.contains(p),
            "crates/ep9_bigint/clippy.toml lacks {p}"
        );
    }
    for p in [
        "dashu_base::EstimatedLog2::log2_bounds",
        "dashu_base::EstimatedLog2::log2_est",
    ] {
        assert!(our_paths.iter().any(|q| q == p), "no ban of {p}");
    }
}
