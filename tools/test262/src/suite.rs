//! Test file discovery and paths.

use std::io;
use std::path::{Path, PathBuf};

/// One test file. `rel` is relative to the test262 root, with `/` separators
/// (`test/language/...js`). Expectation files and reports use `rel`.
#[derive(Clone, Debug)]
pub struct TestFile {
    pub rel: String,
    pub path: PathBuf,
}

/// The top directory under `test/` (`language`, `built-ins`, ...).
pub fn top_dir(rel: &str) -> &str {
    rel.strip_prefix("test/")
        .and_then(|r| r.split('/').next())
        .unwrap_or("")
}

/// Every `*.js` file under `root/test/<dir>` for each dir, except `*_FIXTURE*` files
/// (modules that tests import; INTERPRETING.md says they are not tests). Sorted by `rel`.
pub fn discover(root: &Path, dirs: &[&str]) -> io::Result<Vec<TestFile>> {
    let mut out = Vec::new();
    for dir in dirs {
        let base = root.join("test").join(dir);
        if !base.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not a directory", base.display()),
            ));
        }
        walk(&base, &format!("test/{dir}"), &mut out)?;
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// The directories directly under `root/test`, sorted.
pub fn all_dirs(root: &Path) -> io::Result<Vec<String>> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(root.join("test"))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<TestFile>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = format!("{rel}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(&entry.path(), &child_rel, out)?;
        } else if kind.is_file() && name.ends_with(".js") && !name.contains("_FIXTURE") {
            out.push(TestFile {
                rel: child_rel,
                path: entry.path(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn top_dir_of_rel_path() {
        assert_eq!(super::top_dir("test/built-ins/Array/x.js"), "built-ins");
        assert_eq!(super::top_dir("harness/assert.js"), "");
    }
}
