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
//! file, or its directory and `packed-refs` while the ref is packed). `git rev-parse
//! --git-path` finds both, also in a worktree, where `.git` is a file. Cargo reruns build
//! scripts on a toolchain change by itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use ep9_bytecode::sha256::sha256;

/// Length of the build id in bytes (`ENGINE_BUILD_ID: [u8; 16]`).
const BUILD_ID_LEN: usize = 16;

/// The override variable: exactly 32 hex digits.
const OVERRIDE_VAR: &str = "EP9_BUILD_ID";

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    println!("cargo::rerun-if-changed=build.rs");
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
    std::fs::write(out.join("engine_build_id.bin"), id).expect("write engine_build_id.bin");
    std::fs::write(out.join("engine_build_id.txt"), text).expect("write engine_build_id.txt");
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
    if commit.is_some() {
        watch_head(dir);
    }
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

/// Rerun when HEAD moves: the HEAD file, then the ref it names.
fn watch_head(dir: &Path) {
    let Some(head) = git_path(dir, "HEAD") else {
        return;
    };
    println!("cargo::rerun-if-changed={}", head.display());
    let Ok(content) = std::fs::read_to_string(&head) else {
        return;
    };
    // A detached HEAD holds the commit itself; a branch HEAD reads "ref: refs/heads/<name>".
    let Some(name) = content.trim().strip_prefix("ref: ") else {
        return;
    };
    let Some(loose) = git_path(dir, name) else {
        return;
    };
    if loose.is_file() {
        println!("cargo::rerun-if-changed={}", loose.display());
        return;
    }
    // A packed ref: a commit writes the loose file (a new file in its directory) and leaves
    // packed-refs alone, so watch the directory too. A path that does not exist would make
    // cargo rerun this script on every build.
    if let Some(parent) = loose.parent().filter(|p| p.is_dir()) {
        println!("cargo::rerun-if-changed={}", parent.display());
    }
    if let Some(packed) = git_path(dir, "packed-refs").filter(|p| p.is_file()) {
        println!("cargo::rerun-if-changed={}", packed.display());
    }
}

/// `git rev-parse --git-path <path>`, absolute.
fn git_path(dir: &Path, path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(git(dir, &["rev-parse", "--git-path", path])?);
    Some(if p.is_absolute() { p } else { dir.join(p) })
}

/// The trimmed stdout of `git -C <dir> <args>`, or None if git is missing or fails.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !s.trim().is_empty()).then(|| s.trim().to_string())
}
