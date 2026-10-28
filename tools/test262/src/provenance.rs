//! Where a JSON summary came from (plan v9 section 7 step 3, P3-ORCH-14 and P3-ORCH-21): the
//! commit the ep9 tree is at when the runner runs, whether that tree had uncommitted changes,
//! and the host or CI job that ran it. The fields are read at run time, not at build time: a
//! binary built at one commit and run after a checkout of another names the other one. Run the
//! runner with `cargo run` (it rebuilds on a change) to make an m1-exit input.
//! `cargo xtask m1-exit` items 3 and 4 read the three fields and give OPEN "stale input" when
//! the commit is missing, the tree was dirty, or the paths the run depends on changed after the
//! commit.
//!
//! The runner reads the facts twice (CX5-C1): before the run starts and when it writes the
//! summary. The JSON names the first read; when the two differ in the commit, the dirty flag,
//! the source kind, a tree id or the manifest outcome ([`settle`]), it writes `"dirty": true` and
//! `"source_changed_during_run": true`, so m1-exit gives OPEN.
//!
//! - `"commit"`: `git rev-parse --verify HEAD` in the repo root (found at build time, as the
//!   default paths are). Left out when git cannot answer.
//! - `"dirty"`: `git status --porcelain --untracked-files=no` prints a line (a tracked file has
//!   an uncommitted change). Left out when git cannot answer.
//! - `"commit_source"`: `"git"`, or `"env"` when the repo root is no git tree (a copy of the
//!   tree, as on a sandbox box) and `EP9_SOURCE_COMMIT` (40 hex digits) and `EP9_SOURCE_DIRTY`
//!   (`0` or `1`) name them; the caller that copied the tree sets both. With no git tree and no
//!   `EP9_SOURCE_COMMIT`: `"sbx-commit"` when `<root>/.sbx-commit` holds one line of 40 hex
//!   digits (CX5-C6; `infra/x86-sandbox/sbx push-git` writes it after it extracts `git archive`
//!   of that commit, committed files only). Such a tree is clean only when its files match the
//!   export manifest `<root>/.sbx-manifest` (CX6-2 c, [`check_manifest`]): `"dirty": false` when
//!   the manifest is complete, of that commit, and every listed file has its listed git blob id,
//!   `"dirty": true` on a mismatch, and no dirty field (so m1-exit gives OPEN) when the manifest
//!   is missing, cut short or cannot be read.
//!   `"source_manifest"` names the outcome: `"checked"`, `"mismatch"`, `"missing"` or
//!   `"unreadable"` (only on this path).
//! - An explicit `EP9_SOURCE_DIRTY` other than `0` makes `"dirty": true` on every path (git, env,
//!   `.sbx-commit`), also with no `EP9_SOURCE_COMMIT` (CX6-2). `0` does not hide a dirty git tree.
//! - `"made_by"`: `host:<hostname>`, plus ` gh-run:<GITHUB_RUN_ID>/<GITHUB_JOB>` in a GitHub job.
//! - `"tree"`: `{"<path>": "<git tree or blob id at HEAD>", ...}` for [`TREE_PATHS`] (from git
//!   only). A pull_request CI job runs on GitHub's merge commit, which the local repository does
//!   not have: m1-exit then compares these ids with HEAD's.
//!
//! Two build facts (plan v10 section 2.6, [`write_runner_fields`]; m1-exit X4 checks both):
//!
//! - `"runner_commit"`: the commit this runner binary was built at (C2): `EP9_RUNNER_COMMIT`,
//!   which build.rs sets from `git rev-parse HEAD` and keeps fresh by watching the HEAD paths
//!   (`crates/ep9_unit/build_watch.rs`). Left out when git could not answer at build time. A
//!   runner built at one commit and run on a tree at another writes both: m1-exit gives OPEN.
//! - `"runner_dirty"`: whether a tracked file under the compiled paths (`runner_build.rs`:
//!   `crates`, `tools/test262`, the root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, the
//!   regress pin and patch set) differed from HEAD when this binary was built
//!   (`EP9_RUNNER_DIRTY`; gates5 high 1). build.rs reruns on the git index and on any change
//!   under those paths, so a binary built from an edit that was undone later still says `true`.
//!   Left out when git could not answer. `true` or no field: m1-exit gives OPEN.
//! - `"test262_head"`: the HEAD of the test262 checkout the run used (in:193 item 8), read before
//!   the run starts; left out when that directory is not the top of a git work tree. m1-exit
//!   compares it with the pin, `third_party/test262.commit`.
//! - `"test262_head_end"`: the same HEAD read again when the summary is written, and
//!   `"test262_dirty"`: whether `git status --porcelain --untracked-files=normal` of the checkout
//!   printed a line before the run or at its end (an edited test, or a new untracked one the
//!   run picks up), not counting the `.ep9-stamp` file of scripts/sync-lib.sh; gates5 high 2.
//!   Each is left out when git cannot answer. m1-exit gives OPEN when the dirty flag is `true`
//!   or missing, or the two HEADs differ. (A patch set under `third_party/patches/test262`
//!   would show as dirty; there is none.)
//!
//! **The export manifest** `.sbx-manifest` (written by `infra/x86-sandbox/sbx push-git`; the
//! format is the one of infra/x86-sandbox/README.md, section push-git):
//!
//! ```text
//! commit <full commit id>      line 1: the exported commit, equal to .sbx-commit
//! <blob id> <path>             one line per file of the archive (git ls-tree -r <commit>,
//!                              gitlinks skipped), sorted by path byte by byte (LC_ALL=C)
//! end <n>                      last line: n = the number of file lines, decimal
//! ```
//!
//! `<path>` is the raw path from the root, not quoted: everything after the first space. Every
//! line ends with `\n`. The tree is `"checked"` only when the manifest is complete (the end line
//! is there, last, with the right count; a write cut short has none) and of `.sbx-commit` (else
//! `"mismatch"`), and every listed file exists under the root with that git blob id (SHA-1 of
//! `blob <len>\0<bytes>`; a symbolic link hashes its target text). The manifest names no mode,
//! so the executable bit is not checked. Files that the manifest does not list are not checked,
//! as git status ignores untracked files. `git archive` converts nothing in this repository
//! (`.gitattributes`: `eol=lf`, no `export-subst`), so the extracted bytes are the blob bytes.
//! A manifest in another form, or with a path that is not UTF-8 or leaves the root, is
//! `"unreadable"`.
//!
//! The nesting log of m1-exit item 8 prints the same facts in one line (NESTING_CMD in
//! tools/xtask/src/m1_exit.rs).

use crate::json;
use std::path::Path;
use std::process::Command;

/// The paths whose ids the JSON names: the paths whose change makes an m1-exit item 3 or 4 input
/// stale (`T262_FRESH_PATHS` in tools/xtask/src/m1_exit.rs; an m1-exit test checks that the two
/// lists are the same). The last two are the regress fork (D41): its pin and its patch set.
pub const TREE_PATHS: [&str; 9] = [
    "crates",
    "tools/bench",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "tools/test262",
    "third_party/test262.commit",
    "third_party/regress.commit",
    "third_party/patches/regress",
];

/// The source facts of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub commit: Option<String>,
    pub dirty: Option<bool>,
    /// "git", "env" or "sbx-commit".
    pub from: &'static str,
    pub made_by: String,
    /// (path, id at HEAD) for each of [`TREE_PATHS`] that HEAD has (git only).
    pub tree: Vec<(&'static str, String)>,
    /// The export manifest check of a `.sbx-commit` tree ([`check_manifest`]); None on the
    /// other paths.
    pub manifest: Option<Manifest>,
    /// The facts read when the summary was written differ from the ones read before the run
    /// ([`settle`]).
    pub changed: bool,
}

/// The file `infra/x86-sandbox/sbx push-git` writes in the root of the tree it extracts.
pub const SBX_COMMIT: &str = ".sbx-commit";

/// The export manifest next to [`SBX_COMMIT`] (CX6-2 c; module doc).
pub const SBX_MANIFEST: &str = ".sbx-manifest";

/// The outcome of the export manifest check of a `.sbx-commit` tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manifest {
    /// Every listed file has its listed id (and mode).
    Checked,
    /// A listed file is missing or differs.
    Mismatch,
    /// No manifest file.
    Missing,
    /// The manifest cannot be read or has a line in another form.
    Unreadable,
}

impl Manifest {
    pub fn name(self) -> &'static str {
        match self {
            Manifest::Checked => "checked",
            Manifest::Mismatch => "mismatch",
            Manifest::Missing => "missing",
            Manifest::Unreadable => "unreadable",
        }
    }
}

/// The trimmed stdout of `git -C <root> <args>`, or None when git fails.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `EP9_SOURCE_DIRTY`: unset is None, `0` is `false`, any other value is `true` (CX6-2).
fn dirty_env(env: &dyn Fn(&str) -> Option<String>) -> Option<bool> {
    env("EP9_SOURCE_DIRTY").map(|d| d.trim() != "0")
}

/// The facts of the tree at `root`; `env` reads an environment variable (a parameter, so the
/// tests do not touch the process environment).
pub fn source(root: &Path, env: &dyn Fn(&str) -> Option<String>) -> Source {
    let made_by = made_by(env);
    // An explicit dirty declaration wins on every path (CX6-2).
    let declared_dirty = dirty_env(env) == Some(true);
    // A work tree of its own: `--show-toplevel` must be `root`, so a copy of the tree inside
    // another repository does not take that repository's commit.
    let top = git(root, &["rev-parse", "--show-toplevel"]);
    let same = top.as_deref().is_some_and(|t| {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        canon(Path::new(t)) == canon(root)
    });
    if same {
        let commit = git(root, &["rev-parse", "--verify", "HEAD^{commit}"]).filter(|c| full_sha(c));
        let dirty = if declared_dirty {
            Some(true)
        } else {
            git(root, &["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty())
        };
        let tree = if commit.is_some() {
            TREE_PATHS
                .iter()
                .filter_map(|p| Some((*p, git(root, &["rev-parse", &format!("HEAD:{p}")])?)))
                .collect()
        } else {
            Vec::new()
        };
        return Source {
            commit,
            dirty,
            from: "git",
            made_by,
            tree,
            changed: false,
            manifest: None,
        };
    }
    if env("EP9_SOURCE_COMMIT").is_none()
        && let Ok(text) = std::fs::read_to_string(root.join(SBX_COMMIT))
    {
        // One commit's committed files (`git archive`): clean when they match the export
        // manifest, unless the caller says dirty. A bad line names nothing.
        let commit = Some(text.trim().to_ascii_lowercase()).filter(|c| full_sha(c));
        let manifest = check_manifest(root, commit.as_deref());
        return Source {
            dirty: if declared_dirty {
                Some(true)
            } else if commit.is_none() {
                None
            } else {
                match manifest {
                    Manifest::Checked => Some(false),
                    Manifest::Mismatch => Some(true),
                    Manifest::Missing | Manifest::Unreadable => None,
                }
            },
            commit,
            from: "sbx-commit",
            made_by,
            tree: Vec::new(),
            changed: false,
            manifest: Some(manifest),
        };
    }
    let commit = env("EP9_SOURCE_COMMIT")
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| full_sha(c));
    Source {
        commit,
        dirty: dirty_env(env),
        from: "env",
        made_by,
        tree: Vec::new(),
        changed: false,
        manifest: None,
    }
}

/// The export manifest check (module doc): `<root>/.sbx-manifest` against the files under
/// `root`, for the commit `commit` of `.sbx-commit` (None when that file names none).
pub fn check_manifest(root: &Path, commit: Option<&str>) -> Manifest {
    let bytes = match std::fs::read(root.join(SBX_MANIFEST)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Manifest::Missing,
        Err(_) => return Manifest::Unreadable,
    };
    let Some((of, entries)) = manifest_entries(&bytes) else {
        return Manifest::Unreadable;
    };
    if entries.is_empty() {
        return Manifest::Unreadable;
    }
    // The manifest of another commit names other files (gates5 high 3).
    if commit != Some(of.as_str()) {
        return Manifest::Mismatch;
    }
    for e in &entries {
        if !file_matches(root, e) {
            return Manifest::Mismatch;
        }
    }
    Manifest::Checked
}

/// One file line of the manifest.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    path: String,
}

/// The commit line's id and the file lines of a manifest (module doc), or None when it is not
/// complete or a line is in another form: the text ends with `\n`, the first line is
/// `commit <40 hex digits>`, the last line is `end <n>` with n (decimal, no sign or leading
/// zero) the number of file lines between them (gates5 high 3: a manifest cut short has no end
/// line, or one with another count).
fn manifest_entries(bytes: &[u8]) -> Option<(String, Vec<Entry>)> {
    let text = std::str::from_utf8(bytes).ok()?.strip_suffix('\n')?;
    let lines: Vec<&str> = text.split('\n').collect();
    let (first, rest) = lines.split_first()?;
    let (last, files) = rest.split_last()?;
    let commit = first.strip_prefix("commit ").filter(|c| full_sha(c))?;
    let n = last.strip_prefix("end ")?;
    if n != files.len().to_string() {
        return None;
    }
    let mut out = Vec::with_capacity(files.len());
    for l in files {
        let (id, path) = l.split_once(' ')?;
        // A path that leaves the root is no entry of a tree.
        if !full_sha(id)
            || path.is_empty()
            || path.starts_with('/')
            || path
                .split('/')
                .any(|c| c.is_empty() || c == "." || c == "..")
        {
            return None;
        }
        out.push(Entry {
            id: id.to_ascii_lowercase(),
            path: path.to_string(),
        });
    }
    Some((commit.to_ascii_lowercase(), out))
}

/// The file of `e` under `root` has the entry's blob id: a symbolic link hashes its target
/// text, a regular file its bytes (the manifest names no mode, so the executable bit is not
/// checked).
fn file_matches(root: &Path, e: &Entry) -> bool {
    let p = root.join(&e.path);
    let Ok(meta) = std::fs::symlink_metadata(&p) else {
        return false;
    };
    let bytes = if meta.file_type().is_symlink() {
        match std::fs::read_link(&p) {
            Ok(t) => t.to_string_lossy().into_owned().into_bytes(),
            Err(_) => return false,
        }
    } else if meta.is_file() {
        match std::fs::read(&p) {
            Ok(b) => b,
            Err(_) => return false,
        }
    } else {
        return false;
    };
    hex(&blob_id(&bytes)) == e.id
}

/// The git blob id of `bytes`: SHA-1 of `blob <len>\0<bytes>`.
fn blob_id(bytes: &[u8]) -> [u8; 20] {
    let mut data = format!("blob {}\0", bytes.len()).into_bytes();
    data.extend_from_slice(bytes);
    sha1(&data)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-1 (FIPS 180-4), for git object ids only (not for security).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// The commit this runner was built at (`EP9_RUNNER_COMMIT` of build.rs; module doc), or None
/// when git could not answer at build time.
pub fn runner_commit() -> Option<&'static str> {
    Some(env!("EP9_RUNNER_COMMIT")).filter(|c| full_sha(c))
}

/// Whether the compiled sources had uncommitted changes when this runner was built
/// (`EP9_RUNNER_DIRTY` of build.rs; module doc), or None when git could not answer.
pub fn runner_dirty() -> Option<bool> {
    match env!("EP9_RUNNER_DIRTY") {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// The HEAD commit of the test262 checkout at `dir` (module doc): None unless `dir` is the top
/// of its own git work tree (a checkout copied without `.git` into the ep9 tree must not take
/// the ep9 commit).
pub fn test262_head(dir: &Path) -> Option<String> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon(Path::new(&top)) != canon(dir) {
        return None;
    }
    git(dir, &["rev-parse", "--verify", "HEAD^{commit}"])
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| full_sha(c))
}

/// The test262 checkout as one read sees it (gates5 high 2; module doc): its HEAD
/// ([`test262_head`]) and whether it has changes ([`test262_dirty`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub head: Option<String>,
    pub dirty: Option<bool>,
}

/// One read of the test262 checkout at `dir` (before the run, and again at its end).
pub fn test262_checkout(dir: &Path) -> Checkout {
    Checkout {
        head: test262_head(dir),
        dirty: test262_dirty(dir),
    }
}

/// The file scripts/sync-lib.sh writes in each checkout it syncs (`<pin> <patch hash>`): not
/// a change of the test262 tree.
const SYNC_STAMP: &str = ".ep9-stamp";

/// Whether the test262 checkout at `dir` has a change (module doc): a tracked file that
/// differs from HEAD or an untracked file, other than [`SYNC_STAMP`]. None unless `dir` is the
/// top of its own git work tree, or when git fails.
pub fn test262_dirty(dir: &Path) -> Option<bool> {
    let top = git(dir, &["rev-parse", "--show-toplevel"])?;
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon(Path::new(&top)) != canon(dir) {
        return None;
    }
    let status = git(dir, &["status", "--porcelain", "--untracked-files=normal"])?;
    Some(
        status
            .lines()
            .any(|l| !l.is_empty() && l != format!("?? {SYNC_STAMP}")),
    )
}

/// Appends `,"runner_commit":...`, `,"runner_dirty":...`, `,"test262_head":...` (HEAD of
/// `t262` read before the run), `,"test262_head_end":...` (read now, `end`) and
/// `,"test262_dirty":...` (a change in either read). Each is left out when unknown, so m1-exit
/// reads "no runner commit", "no runner dirty flag", "no test262 HEAD" or "no test262 dirty
/// flag" and gives OPEN.
pub fn write_runner_fields(
    out: &mut String,
    runner: Option<&str>,
    runner_dirty: Option<bool>,
    t262: &Checkout,
    end: &Checkout,
) {
    if let Some(c) = runner {
        out.push_str(",\"runner_commit\":");
        json::string(out, c);
    }
    if let Some(d) = runner_dirty {
        out.push_str(if d {
            ",\"runner_dirty\":true"
        } else {
            ",\"runner_dirty\":false"
        });
    }
    if let Some(h) = &t262.head {
        out.push_str(",\"test262_head\":");
        json::string(out, h);
    }
    if let Some(h) = &end.head {
        out.push_str(",\"test262_head_end\":");
        json::string(out, h);
    }
    if let (Some(a), Some(b)) = (t262.dirty, end.dirty) {
        out.push_str(if a || b {
            ",\"test262_dirty\":true"
        } else {
            ",\"test262_dirty\":false"
        });
    }
}

/// The facts of a run (CX5-C1): `before` (read before the run started) when `after` (read when
/// the summary is written) names the same commit, dirty flag, source kind, tree ids and manifest
/// outcome; else
/// `before` with `dirty` true and `changed` set, because the tests may have run on either tree.
pub fn settle(before: Source, after: &Source) -> Source {
    let same = before.commit == after.commit
        && before.dirty == after.dirty
        && before.from == after.from
        && before.tree == after.tree
        && before.manifest == after.manifest;
    if same {
        before
    } else {
        Source {
            dirty: Some(true),
            changed: true,
            ..before
        }
    }
}

/// `host:<hostname>[ gh-run:<run id>/<job>]`.
pub fn made_by(env: &dyn Fn(&str) -> Option<String>) -> String {
    let host = Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| env("COMPUTERNAME"))
        .or_else(|| env("HOSTNAME"))
        .unwrap_or_else(|| "unknown".into());
    let mut s = format!("host:{host}");
    if let Some(run) = env("GITHUB_RUN_ID").filter(|r| !r.is_empty()) {
        let job = env("GITHUB_JOB").unwrap_or_default();
        s.push_str(&format!(" gh-run:{run}/{job}"));
    }
    s
}

/// The process environment, for [`source`].
pub fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Appends `,"commit":...,"dirty":...,"commit_source":...,"made_by":...` (a missing commit or
/// dirty flag is left out, so m1-exit reads "no commit" and gives OPEN), then
/// `,"source_changed_during_run":true` when [`settle`] saw a change, and `,"tree":{...}`.
pub fn write_fields(out: &mut String, s: &Source) {
    if let Some(c) = &s.commit {
        out.push_str(",\"commit\":");
        json::string(out, c);
    }
    if let Some(d) = s.dirty {
        out.push_str(if d {
            ",\"dirty\":true"
        } else {
            ",\"dirty\":false"
        });
    }
    out.push_str(",\"commit_source\":");
    json::string(out, s.from);
    if let Some(m) = s.manifest {
        out.push_str(",\"source_manifest\":");
        json::string(out, m.name());
    }
    out.push_str(",\"made_by\":");
    json::string(out, &s.made_by);
    if s.changed {
        out.push_str(",\"source_changed_during_run\":true");
    }
    if !s.tree.is_empty() {
        out.push_str(",\"tree\":{");
        for (i, (p, id)) in s.tree.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json::string(out, p);
            out.push(':');
            json::string(out, id);
        }
        out.push('}');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git_in(root: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@test.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ep9-t262-src-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// A clean tree: its HEAD, not dirty; a changed tracked file makes it dirty; an untracked
    /// file does not.
    #[test]
    fn a_git_tree_names_its_head_and_dirty_state() {
        let d = tmp("git");
        std::fs::write(d.join("a.txt"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        let s = source(&d, &no_env);
        assert_eq!(s.from, "git");
        assert_eq!(s.commit.as_deref().map(str::len), Some(40), "{s:?}");
        assert_eq!(s.dirty, Some(false));
        std::fs::write(d.join("new.txt"), "untracked\n").unwrap();
        assert_eq!(source(&d, &no_env).dirty, Some(false));
        std::fs::write(d.join("a.txt"), "changed\n").unwrap();
        assert_eq!(source(&d, &no_env).dirty, Some(true));
        let mut out = String::new();
        write_fields(&mut out, &source(&d, &no_env));
        assert!(out.starts_with(",\"commit\":\""), "{out}");
        assert!(out.contains(",\"dirty\":true,\"commit_source\":\"git\",\"made_by\":\"host:"));
        // The temp tree has none of the nine paths; with one, its id is named.
        assert!(!out.contains("\"tree\""), "{out}");
        std::fs::write(d.join("Cargo.lock"), "lock\n").unwrap();
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "lock"]);
        let s = source(&d, &no_env);
        assert_eq!(s.tree.len(), 1);
        assert_eq!(s.tree[0].0, "Cargo.lock");
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(
            out.ends_with(&format!(",\"tree\":{{\"Cargo.lock\":\"{}\"}}", s.tree[0].1)),
            "{out}"
        );
        // The root Cargo.toml is a tree path too (M4-P5 C1: its `[profile.release]`).
        std::fs::write(d.join("Cargo.toml"), "[workspace]\n").unwrap();
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "manifest"]);
        let s = source(&d, &no_env);
        let names: Vec<&str> = s.tree.iter().map(|(p, _)| *p).collect();
        assert_eq!(names, ["Cargo.toml", "Cargo.lock"], "{s:?}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// pre medium 1 (D41): the regress fork pin and its patch set are tree paths, so a CI
    /// merge-commit run names their ids and m1-exit compares them with HEAD's.
    #[test]
    fn the_regress_fork_is_a_tree_path() {
        let d = tmp("regress");
        std::fs::create_dir_all(d.join("third_party/patches/regress")).unwrap();
        std::fs::write(d.join("third_party/regress.commit"), "b\n").unwrap();
        std::fs::write(d.join("third_party/patches/regress/0001.patch"), "p\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        let s = source(&d, &no_env);
        let names: Vec<&str> = s.tree.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            names,
            ["third_party/regress.commit", "third_party/patches/regress"],
            "{s:?}"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    /// No git tree: the commit comes only from EP9_SOURCE_COMMIT and EP9_SOURCE_DIRTY; without
    /// them both fields are left out of the JSON. A subdir of another repository is not that
    /// repository's tree.
    #[test]
    fn a_copy_without_git_uses_the_env_or_names_nothing() {
        let d = tmp("copy");
        let s = source(&d, &no_env);
        assert_eq!((s.commit.clone(), s.dirty, s.from), (None, None, "env"));
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(!out.contains("\"commit\""), "{out}");
        assert!(!out.contains("\"dirty\""), "{out}");
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let env = |k: &str| match k {
            "EP9_SOURCE_COMMIT" => Some(sha.to_string()),
            "EP9_SOURCE_DIRTY" => Some("0".to_string()),
            "GITHUB_RUN_ID" => Some("42".to_string()),
            "GITHUB_JOB" => Some("test".to_string()),
            _ => None,
        };
        let s = source(&d, &env);
        assert_eq!(s.commit.as_deref(), Some(sha));
        assert_eq!(s.dirty, Some(false));
        assert!(s.made_by.ends_with(" gh-run:42/test"), "{}", s.made_by);
        // A short or bad sha is no commit.
        let bad = |k: &str| (k == "EP9_SOURCE_COMMIT").then(|| "abc123".to_string());
        assert_eq!(source(&d, &bad).commit, None);
        // Inside another repository: not its commit.
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["commit", "-q", "--allow-empty", "-m", "outer"]);
        let sub = d.join("copy");
        std::fs::create_dir_all(&sub).unwrap();
        let s = source(&sub, &no_env);
        assert_eq!((s.commit, s.from), (None, "env"));
        std::fs::remove_dir_all(d).unwrap();
    }

    /// CX5-C1 (Codex round 1): the facts are read before the run and again when the summary
    /// is written. A commit made between the two reads (here, in place of a long run), or
    /// another env commit, gives the first commit with dirty true and the change flag; the
    /// same facts give the first read unchanged.
    #[test]
    fn a_source_change_during_the_run_is_dirty() {
        let d = tmp("during");
        std::fs::write(d.join("Cargo.lock"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        let before = source(&d, &no_env);
        let same = settle(before.clone(), &source(&d, &no_env));
        assert_eq!(same, before);
        let mut out = String::new();
        write_fields(&mut out, &same);
        assert!(!out.contains("source_changed_during_run"), "{out}");
        // The checkout moves on while the tests run.
        std::fs::write(d.join("Cargo.lock"), "b\n").unwrap();
        git_in(&d, &["commit", "-q", "-am", "b"]);
        let after = source(&d, &no_env);
        assert_ne!(after.commit, before.commit);
        let s = settle(before.clone(), &after);
        assert_eq!(
            (s.commit.clone(), s.dirty, s.changed),
            (before.commit.clone(), Some(true), true)
        );
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(out.contains(",\"dirty\":true,"), "{out}");
        assert!(out.contains(",\"source_changed_during_run\":true"), "{out}");
        // Only the dirty flag changes (an edit that is not committed): a change too.
        std::fs::write(d.join("Cargo.lock"), "c\n").unwrap();
        let s = settle(after.clone(), &source(&d, &no_env));
        assert!(s.changed, "{s:?}");
        // A git-free copy: the env seam names another commit at the end.
        let copy = tmp("during-copy");
        let at = |sha: &'static str| {
            move |k: &str| match k {
                "EP9_SOURCE_COMMIT" => Some(sha.to_string()),
                "EP9_SOURCE_DIRTY" => Some("0".to_string()),
                _ => None,
            }
        };
        let a = "0123456789abcdef0123456789abcdef01234567";
        let b = "89abcdef0123456789abcdef0123456789abcdef";
        let first = source(&copy, &at(a));
        assert!(!settle(first.clone(), &source(&copy, &at(a))).changed);
        let s = settle(first, &source(&copy, &at(b)));
        assert_eq!(
            (s.commit.as_deref(), s.dirty, s.changed),
            (Some(a), Some(true), true)
        );
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(copy).unwrap();
    }

    /// CX6-2 (Codex round 2): an explicit `EP9_SOURCE_DIRTY` other than `0` marks the run dirty
    /// on every path: git, env and `.sbx-commit`, also with no `EP9_SOURCE_COMMIT`. `0` does not
    /// hide a dirty git tree.
    #[test]
    fn an_explicit_dirty_env_is_dirty_on_every_path() {
        let dirty =
            |v: &'static str| move |k: &str| (k == "EP9_SOURCE_DIRTY").then(|| v.to_string());
        // git: a clean tree.
        let d = tmp("dirty-env-git");
        std::fs::write(d.join("a.txt"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "a"]);
        for (v, want) in [("1", true), ("yes", true), ("0", false)] {
            let s = source(&d, &dirty(v));
            assert_eq!((s.from, s.dirty), ("git", Some(want)), "{v}: {s:?}");
        }
        std::fs::write(d.join("a.txt"), "changed\n").unwrap();
        assert_eq!(source(&d, &dirty("0")).dirty, Some(true));
        // .sbx-commit, no EP9_SOURCE_COMMIT.
        let sbx = tmp("dirty-env-sbx");
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(sbx.join(".sbx-commit"), format!("{sha}\n")).unwrap();
        write_manifest_of(&sbx, sha, &[("a.txt", b"a\n")]);
        for (v, want) in [("1", true), ("2", true), ("0", false)] {
            let s = source(&sbx, &dirty(v));
            assert_eq!(
                (s.commit.as_deref(), s.from, s.dirty),
                (Some(sha), "sbx-commit", Some(want)),
                "{v}: {s:?}"
            );
        }
        // env: no git tree, no .sbx-commit, no EP9_SOURCE_COMMIT.
        let copy = tmp("dirty-env-copy");
        for (v, want) in [("1", Some(true)), ("true", Some(true)), ("0", Some(false))] {
            let s = source(&copy, &dirty(v));
            assert_eq!(
                (s.commit.as_deref(), s.from, s.dirty),
                (None, "env", want),
                "{v}"
            );
        }
        std::fs::remove_dir_all(d).unwrap();
        std::fs::remove_dir_all(sbx).unwrap();
        std::fs::remove_dir_all(copy).unwrap();
    }

    /// Writes `files` under `root` and the `.sbx-manifest` of `commit` that lists them.
    fn write_manifest_of(root: &Path, commit: &str, files: &[(&str, &[u8])]) {
        for (name, bytes) in files {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        std::fs::write(root.join(SBX_MANIFEST), sbx_manifest_text(commit, files)).unwrap();
    }

    /// CX5-C6 (Codex round 1): `sbx push-git` extracts `git archive` of one commit (committed
    /// files only) and writes that commit to `<root>/.sbx-commit`. With no git tree and no
    /// EP9_SOURCE_COMMIT, the runner names it, `commit_source` "sbx-commit", not dirty when
    /// the files match `.sbx-manifest` (CX6-2 c). A line that is not 40 hex digits names
    /// nothing; the env goes first.
    #[test]
    fn a_sbx_push_git_tree_names_its_sbx_commit() {
        let d = tmp("sbx");
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        std::fs::write(d.join(".sbx-commit"), format!("{sha}\n")).unwrap();
        write_manifest_of(&d, sha, &[("a.txt", b"a\n")]);
        let s = source(&d, &no_env);
        assert_eq!(
            (s.commit.as_deref(), s.dirty, s.from, s.manifest),
            (
                Some(sha),
                Some(false),
                "sbx-commit",
                Some(Manifest::Checked)
            ),
            "{s:?}"
        );
        let mut out = String::new();
        write_fields(&mut out, &s);
        assert!(
            out.starts_with(&format!(
                ",\"commit\":\"{sha}\",\"dirty\":false,\"commit_source\":\"sbx-commit\",\"source_manifest\":\"checked\""
            )),
            "{out}"
        );
        // The env goes first.
        let other = "0123456789abcdef0123456789abcdef01234567";
        let env = |k: &str| match k {
            "EP9_SOURCE_COMMIT" => Some(other.to_string()),
            "EP9_SOURCE_DIRTY" => Some("1".to_string()),
            _ => None,
        };
        let s = source(&d, &env);
        assert_eq!(
            (s.commit.as_deref(), s.dirty, s.from),
            (Some(other), Some(true), "env")
        );
        // Not a full id: no commit, no dirty flag.
        std::fs::write(d.join(".sbx-commit"), "abc123\n").unwrap();
        let s = source(&d, &no_env);
        assert_eq!((s.commit.as_deref(), s.dirty), (None, None), "{s:?}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// The text of an `.sbx-manifest` as `sbx push-git` writes it (module doc): the commit
    /// line, one `<blob id> <path>` line per file sorted by path, the end line.
    fn sbx_manifest_text(commit: &str, files: &[(&str, &[u8])]) -> String {
        let mut entries: Vec<String> = files
            .iter()
            .map(|(name, bytes)| format!("{} {name}\n", hex(&blob_id(bytes))))
            .collect();
        entries.sort_by(|a, b| a.as_bytes()[41..].cmp(&b.as_bytes()[41..]));
        format!(
            "commit {commit}\n{}end {}\n",
            entries.concat(),
            entries.len()
        )
    }

    /// gates5 high 3: the manifest is complete (its last line is `end <n>`, n the number of
    /// entry lines) and of the exported commit (its first line names `.sbx-commit`); else the
    /// tree is not "checked". A manifest cut after an unchanged file (an interrupted write)
    /// hid an edited file before.
    #[test]
    fn the_sbx_manifest_must_be_complete_and_of_its_commit() {
        let d = tmp("sbx-complete");
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        let files: [(&str, &[u8]); 2] = [("a.txt", b"a\n"), ("engine.rs", b"// engine\n")];
        for (n, b) in files {
            std::fs::write(d.join(n), b).unwrap();
        }
        std::fs::write(d.join(SBX_COMMIT), format!("{sha}\n")).unwrap();
        let full = sbx_manifest_text(sha, &files);
        let state = |m: &str| {
            std::fs::write(d.join(SBX_MANIFEST), m).unwrap();
            let s = source(&d, &no_env);
            (s.dirty, s.manifest)
        };
        assert_eq!(state(&full), (Some(false), Some(Manifest::Checked)));
        std::fs::write(d.join("engine.rs"), "// edited\n").unwrap();
        assert_eq!(state(&full), (Some(true), Some(Manifest::Mismatch)));
        // Cut after the first (unchanged) file: no end line.
        let cut: String = full.lines().take(2).map(|l| format!("{l}\n")).collect();
        assert_eq!(state(&cut), (None, Some(Manifest::Unreadable)));
        // An end line with another count, in another form, or not last; no final newline.
        let body = &full[..full.rfind("end ").unwrap()];
        for bad in [
            format!("{body}end 1\n"),
            format!("{body}end 3\n"),
            format!("{body}end 02\n"),
            format!("{body}end +2\n"),
            format!("{body}end 2"),
            format!("{body}end 2\nend 2\n"),
            format!("{body}end 2\n{}", &full[full.find('\n').unwrap() + 1..]),
        ] {
            assert_eq!(state(&bad), (None, Some(Manifest::Unreadable)), "{bad:?}");
        }
        // The manifest of another commit.
        std::fs::write(d.join("engine.rs"), "// engine\n").unwrap();
        let other = sbx_manifest_text(&"0".repeat(40), &files);
        assert_eq!(state(&other), (Some(true), Some(Manifest::Mismatch)));
        // No commit line.
        let headless = &full[full.find('\n').unwrap() + 1..];
        assert_eq!(state(headless), (None, Some(Manifest::Unreadable)));
        assert_eq!(state(&full), (Some(false), Some(Manifest::Checked)));
        std::fs::remove_dir_all(d).unwrap();
    }

    /// SHA-1 known answers (FIPS 180-4 examples; one and two blocks) and git blob ids.
    #[test]
    fn sha1_and_blob_ids() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            hex(&blob_id(b"hello\n")),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(
            hex(&blob_id(b"")),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    /// The manifest form of `sbx push-git` (module doc): raw paths (a space, a quote, a
    /// non-ASCII letter), the commit line first and the end line last. A line in another form,
    /// or a path that leaves the root, makes the manifest unreadable.
    #[test]
    fn manifest_entries_read_the_sbx_form() {
        let id = "ce013625030ba8dba906f756967f9e9ca394464a";
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        let (of, e) = manifest_entries(
            format!("commit {sha}\n{id} a.txt\n{id} d/\u{e9} \"q\".sh\nend 2\n").as_bytes(),
        )
        .unwrap();
        assert_eq!(of, sha);
        let paths: Vec<&str> = e.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "d/\u{e9} \"q\".sh"]);
        for bad in [
            format!("commit {sha}\n{id}\ta.txt\nend 1\n"),
            format!("commit {sha}\nabc a\nend 1\n"),
            format!("commit {sha}\n{id} ../a\nend 1\n"),
            format!("commit {sha}\n{id} /etc/a\nend 1\n"),
            format!("commit {sha}\n{id} a//b\nend 1\n"),
            format!("commit {sha}\n{id} \nend 1\n"),
            format!("commit abc\n{id} a\nend 1\n"),
            format!("100644 blob {id}\ta\n"),
        ] {
            assert_eq!(manifest_entries(bad.as_bytes()), None, "{bad:?}");
        }
    }

    /// CX6-2 (c): a git-free copy of a real commit (as `git archive` makes it) checks against
    /// the manifest of that commit, made as `sbx push-git` makes it (from `git ls-tree -r -z`:
    /// blobs only, sorted by path); an edited or missing file, or another symlink target, is a
    /// mismatch and dirty; an untracked file or a new mode is not checked; no manifest or a bad
    /// one gives no dirty field.
    #[test]
    fn the_export_manifest_check() {
        let repo = tmp("manifest-repo");
        std::fs::create_dir_all(repo.join("d")).unwrap();
        let files: [(&str, &[u8]); 4] = [
            ("a.txt", b"a\n"),
            ("d/\u{e9}.txt", b"accent\n"),
            ("big.bin", &[7u8; 5000]),
            ("sp ace.txt", b"space\n"),
        ];
        for (n, b) in files {
            std::fs::write(repo.join(n), b).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", repo.join("link")).unwrap();
        git_in(&repo, &["init", "-q"]);
        git_in(&repo, &["add", "-A"]);
        git_in(&repo, &["commit", "-q", "-m", "a"]);
        let sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        // The manifest as sbx_manifest of infra/x86-sandbox/sbx writes it.
        let o = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["ls-tree", "-r", "-z", "--full-tree", "HEAD"])
            .output()
            .unwrap();
        assert!(o.status.success());
        let mut entries: Vec<(String, String)> = String::from_utf8(o.stdout)
            .unwrap()
            .split('\0')
            .filter(|r| !r.is_empty())
            .map(|r| {
                let (meta, path) = r.split_once('\t').unwrap();
                let id = meta.split(' ').nth(2).unwrap();
                (path.to_string(), id.to_string())
            })
            .collect();
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut manifest = format!("commit {sha}\n");
        for (path, id) in &entries {
            manifest.push_str(&format!("{id} {path}\n"));
        }
        manifest.push_str(&format!("end {}\n", entries.len()));
        // The copy: the files only, no .git.
        let copy = tmp("manifest-copy");
        std::fs::create_dir_all(copy.join("d")).unwrap();
        for (n, b) in files {
            std::fs::write(copy.join(n), b).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", copy.join("link")).unwrap();
        std::fs::write(copy.join(SBX_COMMIT), format!("{sha}\n")).unwrap();
        let state = || {
            let s = source(&copy, &no_env);
            (s.dirty, s.manifest)
        };
        assert_eq!(state(), (None, Some(Manifest::Missing)));
        std::fs::write(copy.join(SBX_MANIFEST), &manifest).unwrap();
        assert_eq!(state(), (Some(false), Some(Manifest::Checked)));
        // An untracked file is not checked.
        std::fs::write(copy.join("new.txt"), "x").unwrap();
        assert_eq!(state(), (Some(false), Some(Manifest::Checked)));
        // An edited file.
        std::fs::write(copy.join("big.bin"), [7u8; 4999]).unwrap();
        assert_eq!(state(), (Some(true), Some(Manifest::Mismatch)));
        let mut out = String::new();
        write_fields(&mut out, &source(&copy, &no_env));
        assert!(
            out.contains(
                ",\"dirty\":true,\"commit_source\":\"sbx-commit\",\"source_manifest\":\"mismatch\""
            ),
            "{out}"
        );
        std::fs::write(copy.join("big.bin"), [7u8; 5000]).unwrap();
        assert_eq!(state(), (Some(false), Some(Manifest::Checked)));
        // A missing file.
        std::fs::remove_file(copy.join("d/\u{e9}.txt")).unwrap();
        assert_eq!(state(), (Some(true), Some(Manifest::Mismatch)));
        std::fs::write(copy.join("d/\u{e9}.txt"), b"accent\n").unwrap();
        #[cfg(unix)]
        {
            std::fs::remove_file(copy.join("link")).unwrap();
            std::os::unix::fs::symlink("big.bin", copy.join("link")).unwrap();
            assert_eq!(state(), (Some(true), Some(Manifest::Mismatch)));
            std::fs::remove_file(copy.join("link")).unwrap();
            std::os::unix::fs::symlink("a.txt", copy.join("link")).unwrap();
            // The manifest names no mode.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(copy.join("a.txt"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert_eq!(state(), (Some(false), Some(Manifest::Checked)));
        // A manifest in another form names no dirty state; an explicit dirty env still wins.
        std::fs::write(copy.join(SBX_MANIFEST), "not a manifest\n").unwrap();
        assert_eq!(state(), (None, Some(Manifest::Unreadable)));
        let dirty = |k: &str| (k == "EP9_SOURCE_DIRTY").then(|| "1".to_string());
        assert_eq!(source(&copy, &dirty).dirty, Some(true));
        // A manifest that changes during the run is a source change (CX5-C1).
        std::fs::write(copy.join(SBX_MANIFEST), &manifest).unwrap();
        let before = source(&copy, &no_env);
        std::fs::write(copy.join("a.txt"), "edited\n").unwrap();
        assert!(settle(before, &source(&copy, &no_env)).changed);
        std::fs::remove_dir_all(repo).unwrap();
        std::fs::remove_dir_all(copy).unwrap();
    }

    /// gates5 high 2: the test262 checkout is dirty with an edited tracked file or a new
    /// untracked one (the runner would run it), not with the sync stamp; a dirty read before or
    /// at the end makes `test262_dirty` true; a directory that is not the top of its own work
    /// tree names nothing.
    #[test]
    fn test262_dirty_reads_the_checkout() {
        let d = tmp("t262-dirty");
        assert_eq!(test262_dirty(&d), None);
        std::fs::create_dir_all(d.join("test")).unwrap();
        std::fs::write(d.join("test/a.js"), "a\n").unwrap();
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["add", "-A"]);
        git_in(&d, &["commit", "-q", "-m", "t"]);
        assert_eq!(test262_dirty(&d), Some(false));
        std::fs::write(d.join(SYNC_STAMP), "pin hash\n").unwrap();
        assert_eq!(test262_dirty(&d), Some(false));
        assert_eq!(test262_dirty(&d.join("test")), None);
        let clean = test262_checkout(&d);
        std::fs::write(d.join("test/new.js"), "n\n").unwrap();
        assert_eq!(test262_dirty(&d), Some(true));
        std::fs::remove_file(d.join("test/new.js")).unwrap();
        std::fs::write(d.join("test/a.js"), "edited\n").unwrap();
        let dirty = test262_checkout(&d);
        assert_eq!(dirty.dirty, Some(true));
        for (a, b) in [(&clean, &dirty), (&dirty, &clean)] {
            let mut out = String::new();
            write_runner_fields(&mut out, None, None, a, b);
            assert!(out.ends_with(",\"test262_dirty\":true"), "{out}");
        }
        // Unknown at one end: no flag (m1-exit gives OPEN).
        let unknown = Checkout {
            head: clean.head.clone(),
            dirty: None,
        };
        let mut out = String::new();
        write_runner_fields(&mut out, None, None, &clean, &unknown);
        assert!(!out.contains("test262_dirty"), "{out}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// C2 and item 8: the runner's build commit (from build.rs, in this repository: a commit
    /// id) and the HEAD of a test262 checkout (only the top of its own work tree), both written
    /// by `write_runner_fields` and left out when unknown.
    #[test]
    fn runner_commit_and_test262_head() {
        let rc = runner_commit().expect("this test runs in the ep9 git tree");
        assert!(full_sha(rc), "{rc}");
        let d = tmp("t262-head");
        assert_eq!(test262_head(&d), None);
        git_in(&d, &["init", "-q"]);
        git_in(&d, &["commit", "-q", "--allow-empty", "-m", "t"]);
        let head = test262_head(&d).unwrap();
        assert!(full_sha(&head), "{head}");
        // A directory inside it is not a checkout of its own.
        std::fs::create_dir_all(d.join("sub")).unwrap();
        assert_eq!(test262_head(&d.join("sub")), None);
        let co = test262_checkout(&d);
        assert_eq!(
            (co.head.as_deref(), co.dirty),
            (Some(head.as_str()), Some(false))
        );
        let mut out = String::new();
        write_runner_fields(&mut out, Some(rc), Some(true), &co, &co);
        assert_eq!(
            out,
            format!(
                ",\"runner_commit\":\"{rc}\",\"runner_dirty\":true,\"test262_head\":\"{head}\",\"test262_head_end\":\"{head}\",\"test262_dirty\":false"
            )
        );
        // gates5 high 1: in this repository build.rs names the dirty flag too.
        assert!(runner_dirty().is_some());
        let none = Checkout {
            head: None,
            dirty: None,
        };
        let mut out = String::new();
        write_runner_fields(&mut out, None, None, &none, &none);
        assert_eq!(out, "");
        std::fs::remove_dir_all(d).unwrap();
    }
}
