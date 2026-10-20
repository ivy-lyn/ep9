//! `ep9-test262 index`: the front matter of every test as JSON.
//!
//! Output: `{"test262": ..., "count": N, "errors": E, "tests": [{path, flags, includes,
//! features, locale, negative}, ...]}`, one test per line, sorted by path. A file whose front
//! matter does not parse gets `{"path": ..., "error": ...}` and the exit code is 1.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::{frontmatter, json, pool, suite};

pub struct Options {
    pub test262: PathBuf,
    pub out: Option<PathBuf>,
    pub threads: usize,
}

pub fn run(opts: &Options) -> ExitCode {
    let root: &Path = &opts.test262;
    let files = match suite::all_dirs(root).and_then(|dirs| {
        let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
        suite::discover(root, &dirs)
    }) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("ep9-test262: cannot list tests in {}: {e}", root.display());
            return ExitCode::from(2);
        }
    };
    let entries = pool::par_map(&files, opts.threads, 8 << 20, |file| {
        let meta = std::fs::read_to_string(&file.path)
            .map_err(|e| format!("cannot read as UTF-8: {e}"))
            .and_then(|src| frontmatter::parse(&src));
        match meta {
            Ok(meta) => (frontmatter::to_json(&file.rel, &meta), None),
            Err(e) => {
                let mut out = String::from("{\"path\":");
                json::string(&mut out, &file.rel);
                out.push_str(",\"error\":");
                json::string(&mut out, &e);
                out.push('}');
                (out, Some(format!("{}: {e}", file.rel)))
            }
        }
    });
    let errors: Vec<&String> = entries.iter().filter_map(|(_, e)| e.as_ref()).collect();
    let mut out = String::from("{\"test262\":");
    json::string(&mut out, &root.display().to_string());
    out.push_str(&format!(
        ",\"count\":{},\"errors\":{},\"tests\":[",
        entries.len(),
        errors.len()
    ));
    for (i, (entry, _)) in entries.iter().enumerate() {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        out.push_str(entry);
    }
    out.push_str("\n]}\n");

    let written = match &opts.out {
        Some(path) => {
            std::fs::write(path, &out).map_err(|e| format!("cannot write {}: {e}", path.display()))
        }
        None => {
            use std::io::Write as _;
            std::io::stdout()
                .lock()
                .write_all(out.as_bytes())
                .map_err(|e| format!("cannot write stdout: {e}"))
        }
    };
    if let Err(e) = written {
        eprintln!("ep9-test262: {e}");
        return ExitCode::from(2);
    }
    eprintln!(
        "ep9-test262 index: {} files, {} front matter errors",
        entries.len(),
        errors.len()
    );
    for e in &errors {
        eprintln!("  {e}");
    }
    if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
