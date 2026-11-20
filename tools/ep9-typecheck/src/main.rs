//! `ep9-typecheck`: the tsrs type oracle, v0 (crate docs in `lib.rs`).
//!
//! ```text
//!  ep9-typecheck <tsconfig.json | dir> --out <file.ep9h>   check the project, write the file
//!  ep9-typecheck --verify <file.ep9h>                      read the file back, print a summary
//! ```
//!
//! Exit status: 0 done (a file with type errors is data, not a failure); 1 tsrs made no program,
//! a file could not be read, written or verified, or `--out` names an input of the project (the
//! config or a source file; the file does not change); 2 bad arguments.
//!
//! The output goes to a temporary file next to `--out` first, then a rename replaces `--out`,
//! so a failed write never leaves a part of a file.

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: ep9-typecheck <tsconfig.json | dir> --out <file.ep9h>\n       \
                     ep9-typecheck --verify <file.ep9h>";

/// The most error lines that a run prints.
const MAX_ERROR_LINES: usize = 20;

enum Mode {
    Check { project: String, out: String },
    Verify { file: String },
}

fn parse(args: &[String]) -> Result<Mode, String> {
    match args {
        [flag, file] if flag == "--verify" => Ok(Mode::Verify { file: file.clone() }),
        [project, flag, out] | [flag, out, project]
            if flag == "--out" && !project.starts_with('-') =>
        {
            Ok(Mode::Check {
                project: project.clone(),
                out: out.clone(),
            })
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn run_check(project: &str, out: &str) -> Result<String, String> {
    let checked = ep9_typecheck::oracle::check(project)?;
    for line in checked.errors.iter().take(MAX_ERROR_LINES) {
        eprintln!("ep9-typecheck: {line}");
    }
    if checked.errors.len() > MAX_ERROR_LINES {
        eprintln!(
            "ep9-typecheck: ... {} more errors",
            checked.errors.len() - MAX_ERROR_LINES
        );
    }
    let total = checked.files.len();
    let with_errors = checked.files.iter().filter(|f| f.has_type_errors).count();
    let bytes = ep9_typecheck::write_v0(checked.files, checked.ts_flags)?;
    write_out(out, &bytes, &checked.inputs)?;
    Ok(format!(
        "ep9-typecheck: {total} files ({with_errors} with type errors), 0 records, {} bytes -> {out}\n",
        bytes.len()
    ))
}

/// `a` and `b` are one file (after links): the same device and inode on unix, else the same
/// canonical path. A path that does not exist is no file.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}

/// Writes `bytes` to `out`: refused when `out` is one of `inputs`; else a temporary file in the
/// same directory, then a rename over `out` (it replaces the file on Windows too).
fn write_out(out: &str, bytes: &[u8], inputs: &[String]) -> Result<(), String> {
    let path = Path::new(out);
    if inputs.iter().any(|i| same_file(path, Path::new(i))) {
        return Err(format!(
            "--out {out} is an input of the project; it is not written"
        ));
    }
    let tmp = format!("{out}.tmp{}", std::process::id());
    if let Err(e) = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("cannot write {out}: {e}"));
    }
    Ok(())
}

fn run_verify(file: &str) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {file}: {e}"))?;
    let hints = ep9_typecheck::verify(&bytes).map_err(|e| format!("{file}: {e}"))?;
    Ok(ep9_typecheck::summary(&hints))
}

fn run(args: &[String]) -> u8 {
    let result = match parse(args) {
        Ok(Mode::Check { project, out }) => run_check(&project, &out),
        Ok(Mode::Verify { file }) => run_verify(&file),
        Err(usage) => {
            eprintln!("{usage}");
            return 2;
        }
    };
    match result {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("ep9-typecheck: {e}");
            1
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The checker recurses deeply: a large stack, as the tsrs command line uses.
    let status = std::thread::Builder::new()
        .name("ep9-typecheck".to_owned())
        .stack_size(512 << 20)
        .spawn(move || run(&args))
        .map_or(1, |t| t.join().unwrap_or(1));
    ExitCode::from(status)
}
