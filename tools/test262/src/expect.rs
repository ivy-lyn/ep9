//! Expectation files: the sorted list of known failing paths, one per line.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

/// Read an expectation file. Blank lines and `#` lines are ignored.
pub fn read(path: &Path) -> io::Result<BTreeSet<String>> {
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// The header of a parse expectation file.
pub fn parse_header(triple: &str) -> String {
    format!(
        "# ep9-test262 parse: known failures on {triple}. Paths are relative to the test262 root.\n\
         # Regenerate: cargo run --release -p ep9_test262 -- parse --update-expectations\n"
    )
}

/// Write the failing paths, sorted, after `header` (`#` lines).
pub fn write(path: &Path, header: &str, failing: &BTreeSet<String>) -> io::Result<()> {
    let mut text = header.to_string();
    for p in failing {
        text.push_str(p);
        text.push('\n');
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

/// Difference between this run and the expectation file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Failing now, not listed.
    pub new_failures: Vec<String>,
    /// Listed, but passing now (or not present any more in a full run).
    pub now_passing: Vec<String>,
}

impl Diff {
    pub fn is_clean(&self) -> bool {
        self.new_failures.is_empty() && self.now_passing.is_empty()
    }
}

/// `checked` is every path this run looked at. In a filtered run (`full == false`), listed paths
/// outside `checked` are ignored. In a full run they count as stale.
pub fn diff(
    expected: &BTreeSet<String>,
    failing: &BTreeSet<String>,
    checked: &BTreeSet<String>,
    full: bool,
) -> Diff {
    Diff {
        new_failures: failing.difference(expected).cloned().collect(),
        now_passing: expected
            .iter()
            .filter(|p| !failing.contains(*p) && (full || checked.contains(*p)))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn diff_rules() {
        let expected = set(&["a", "b", "gone"]);
        let failing = set(&["a", "c"]);
        let checked = set(&["a", "b", "c"]);
        let full = diff(&expected, &failing, &checked, true);
        assert_eq!(full.new_failures, ["c"]);
        assert_eq!(full.now_passing, ["b", "gone"]);
        let filtered = diff(&expected, &failing, &checked, false);
        assert_eq!(filtered.now_passing, ["b"]);
        assert!(diff(&expected, &expected, &expected, true).is_clean());
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("ep9-expect-{}", std::process::id()));
        let path = dir.join("x.txt");
        let failing = set(&["test/b.js", "test/a.js"]);
        write(&path, "# t\n", &failing).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("test/a.js\ntest/b.js\n"), "{text}");
        assert_eq!(read(&path).unwrap(), failing);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
