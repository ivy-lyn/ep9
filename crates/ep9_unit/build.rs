//! The engine build id (H35-Q8): writes `$OUT_DIR/engine_build_id.bin` (16 bytes), which
//! `ENGINE_BUILD_ID` in `src/host.rs` includes, and `$OUT_DIR/engine_build_id.txt`, the text
//! the id was hashed from.
//!
//! ```text
//! EP9_BUILD_ID=<32 hex digits> set  -> id = those 16 bytes          text: "override <hex>\n"
//! otherwise                         -> id = SHA-256(text)[..16]     text:
//!     ep9 engine build id v1
//!     commit <git rev-parse HEAD, or "unknown" without git or outside a checkout>
//!     target <TARGET>
//!     features <ep9_unit's own CARGO_FEATURE_* names, sorted, comma-separated>
//!     rustc
//!     <`$RUSTC -vV`, line by line>
//! ```
//!
//! A build script sees only its own package's features, so the id cannot name the features of
//! other crates (H35-Q8). It reruns when `EP9_BUILD_ID` changes, when this file changes, and
//! when HEAD moves: it watches the HEAD file, and the branch ref HEAD names (the loose ref
//! file; while the ref is packed, the nearest existing directory above it inside `refs/`, and
//! `packed-refs`). `git rev-parse --git-path` finds them, also in a worktree, where `.git` is a
//! file. The paths are chosen in `build_watch.rs` (tested by `tests/build_watch.rs`). Cargo
//! reruns build scripts on a toolchain change by itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use ep9_bytecode::sha256::sha256;

#[path = "build_watch.rs"]
mod build_watch;

use build_watch::git;

/// Length of the build id in bytes (`ENGINE_BUILD_ID: [u8; 16]`).
const BUILD_ID_LEN: usize = 16;

/// The override variable: exactly 32 hex digits.
const OVERRIDE_VAR: &str = "EP9_BUILD_ID";

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=build_watch.rs");
    println!("cargo::rerun-if-env-changed={OVERRIDE_VAR}");

    let (id, text) = match std::env::var(OVERRIDE_VAR) {
        Ok(hex) => (
            parse_override(&hex),
            format!("override {}\n", hex.to_ascii_lowercase()),
        ),
        Err(std::env::VarError::NotPresent) => {
            let text = input_text(&dir);
            let digest = sha256(text.as_bytes());
            let mut id = [0u8; BUILD_ID_LEN];
            id.copy_from_slice(&digest[..BUILD_ID_LEN]);
            (id, text)
        }
        Err(e) => panic!("{OVERRIDE_VAR}: {e}"),
    };
    write_if_changed(&out.join("engine_build_id.bin"), &id);
    write_if_changed(&out.join("engine_build_id.txt"), text.as_bytes());
}

/// The 16 bytes of an `EP9_BUILD_ID` value; anything but 32 hex digits fails the build.
fn parse_override(hex: &str) -> [u8; BUILD_ID_LEN] {
    let b = hex.as_bytes();
    if b.len() != BUILD_ID_LEN * 2 {
        bad_override(hex);
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => bad_override(hex),
    };
    let mut id = [0u8; BUILD_ID_LEN];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = nibble(b[2 * i]) << 4 | nibble(b[2 * i + 1]);
    }
    id
}

fn bad_override(hex: &str) -> ! {
    panic!(
        "{OVERRIDE_VAR} must be {} hex digits, got {hex:?}",
        BUILD_ID_LEN * 2
    )
}

/// The hashed text (module doc), with the rerun lines for the git inputs.
fn input_text(dir: &Path) -> String {
    let commit = git(dir, &["rev-parse", "HEAD"]);
    // The rerun lines: the HEAD paths; while git cannot answer, a path that never exists, so
    // cargo reruns this script on every build until git answers (`watch_git`, month-5 input
    // item 10). The id files are written only when their bytes change (`write_if_changed`).
    // (Five lines, so the lines cited below, docs/plan/13-execution-modes.md, do not move.)
    watch_git(dir, commit.is_some());
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = Command::new(&rustc)
        .arg("-vV")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_else(|| panic!("`{} -vV` failed", Path::new(&rustc).display()));
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    let mut features: Vec<String> = std::env::vars()
        .filter_map(|(k, _)| k.strip_prefix("CARGO_FEATURE_").map(str::to_string))
        .collect();
    features.sort();
    let mut text = format!(
        "ep9 engine build id v1\ncommit {}\ntarget {target}\nfeatures {}\nrustc\n",
        commit.as_deref().unwrap_or("unknown"),
        features.join(",")
    );
    for line in version.lines() {
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

/// The rerun lines of the git inputs: the HEAD paths when git answers. Else the script watches
/// `NO_GIT_WATCH` in `OUT_DIR`, a path it never creates: cargo then reruns the script on every
/// build, so the first build after git works again names the commit (month-5 input item 10;
/// before, an id with "commit unknown" stayed cached until a clean build).
fn watch_git(dir: &Path, git_answers: bool) {
    if git_answers {
        for p in build_watch::head_watch_paths(dir) {
            println!("cargo::rerun-if-changed={}", p.display());
        }
    } else {
        let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
        println!(
            "cargo::rerun-if-changed={}",
            out.join(NO_GIT_WATCH).display()
        );
    }
}

/// The watched path, in `OUT_DIR`, while git cannot answer: it never exists.
const NO_GIT_WATCH: &str = "ep9-no-git.rerun";

/// Writes `bytes` to `path` unless the file already holds them (a rerun with the same input
/// keeps the file's time, so the crate that includes it is not rebuilt for nothing).
fn write_if_changed(path: &Path, bytes: &[u8]) {
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return;
    }
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
