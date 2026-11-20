//! `ep9-typecheck`: the tsrs type oracle, v0 (crate docs in `lib.rs`).
//!
//! ```text
//!  ep9-typecheck <tsconfig.json | dir> --out <file.ep9h>   check the project, write the file
//!  ep9-typecheck --verify <file.ep9h>                      read the file back, print a summary
//! ```
//!
//! Exit status: 0 done (a file with type errors is data, not a failure); 1 tsrs made no program,
//! a file could not be read, written or verified, or `--out` is refused (the file does not
//! change); 2 bad arguments.
//!
//! `--out` is refused when it names a known input of the project (the config, a config that it
//! extends, or a source file), and also when it names any other file that exists and is not a
//! regular file that starts with the `.ep9h` magic (`ep9_hints::MAGIC`): a link, a directory, or
//! a file that the check can read but that is not in the inputs list (a config named by
//! `references` and the configs that it extends, a `package.json` that module resolution reads).
//! No input of the checker starts with that magic, so only an output of an earlier run is
//! replaced. A `--out` file that does not exist is written.
//!
//! The output goes to a new temporary file next to `--out` first (made with `create_new`, so it
//! never follows a link or writes into a file that is already there). Then a hard link makes an
//! absent `--out` (it fails when `--out` was made after the check), or a rename replaces an
//! earlier `.ep9h` output. So a failed write never leaves a part of a file.

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

/// The first bytes of the file at `path` are the `.ep9h` magic.
fn starts_with_magic(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == ep9_hints::MAGIC
}

/// Writes `bytes` to `out`: refused when `out` is one of `inputs`, or when `out` exists and is
/// not a regular file (not a link) that starts with the `.ep9h` magic; else a new temporary file
/// in the same directory ([`stage`]), then [`publish`]: a hard link to an absent `out`, or a
/// rename over an earlier `.ep9h` output (it replaces the file on Windows too).
fn write_out(out: &str, bytes: &[u8], inputs: &[String]) -> Result<(), String> {
    let path = Path::new(out);
    if inputs.iter().any(|i| same_file(path, Path::new(i))) {
        return Err(format!(
            "--out {out} is an input of the project; it is not written"
        ));
    }
    let replace = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("cannot read {out}: {e}; it is not written")),
        Ok(m) if m.file_type().is_file() && starts_with_magic(path) => true,
        Ok(_) => {
            return Err(format!(
                "--out {out} exists and is not a .ep9h file; it is not written"
            ));
        }
    };
    let tmp = stage(out, bytes)?;
    publish(&tmp, path, replace).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!("--out {out} was made during the run; it is not written")
        } else {
            format!("cannot write {out}: {e}")
        }
    })
}

/// The most temporary names that [`stage`] tries.
const STAGE_TRIES: u32 = 8;

/// Writes `bytes` to a new file `<out>.tmp<pid>.<n>` and returns its name. `create_new` makes
/// the file or fails (`O_CREAT | O_EXCL` on unix, `CREATE_NEW` on Windows): an entry that is
/// already at that name (a file, or a link, also a dangling one) is never opened, and the next
/// `n` is tried. On a write error only the file that this run made is removed.
fn stage(out: &str, bytes: &[u8]) -> Result<String, String> {
    use std::io::Write;
    for n in 0..STAGE_TRIES {
        let tmp = format!("{out}.tmp{}.{n}", std::process::id());
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(mut f) => {
                return match f.write_all(bytes).and_then(|()| f.sync_all()) {
                    Ok(()) => Ok(tmp),
                    Err(e) => {
                        drop(f);
                        let _ = std::fs::remove_file(&tmp);
                        Err(format!("cannot write {out}: {e}"))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot write {out}: {e}")),
        }
    }
    Err(format!("cannot write {out}: no free temporary name"))
}

/// Puts the staged file `tmp` at `out`. `replace` false (`out` was absent at the check): a hard
/// link, which fails with `AlreadyExists` when `out` exists now, so a file made after the check
/// is never replaced; then `tmp` is removed. `replace` true: a rename over the earlier `.ep9h`
/// output (`tmp` is removed only when the rename fails).
fn publish(tmp: &str, out: &Path, replace: bool) -> std::io::Result<()> {
    let r = if replace {
        std::fs::rename(tmp, out)
    } else {
        std::fs::hard_link(tmp, out)
    };
    if !replace || r.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    r
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A new empty scratch directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ep9-typecheck-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A link at the first temporary name to an input is not followed: the input does not
    /// change, the link stays, and `--out` gets the new bytes (through the next name).
    #[cfg(unix)]
    #[test]
    fn stage_never_follows_a_link_at_the_temporary_name() {
        let dir = scratch("stage-link");
        let input = dir.join("in.ts");
        std::fs::write(&input, b"input").unwrap();
        let out = dir.join("out.ep9h");
        let out = out.to_str().unwrap();
        let link = format!("{out}.tmp{}.0", std::process::id());
        std::os::unix::fs::symlink(&input, &link).unwrap();
        write_out(out, b"new", &[]).unwrap();
        assert_eq!(std::fs::read(&input).unwrap(), b"input");
        assert_eq!(std::fs::read(out).unwrap(), b"new");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!Path::new(&format!("{out}.tmp{}.1", std::process::id())).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A `--out` file made after the check (absent at the check) is not replaced, and the
    /// temporary file is removed.
    #[test]
    fn publish_never_replaces_a_file_made_after_the_check() {
        let dir = scratch("publish-race");
        let out = dir.join("out.ep9h");
        let tmp = stage(out.to_str().unwrap(), b"new").unwrap();
        std::fs::write(&out, b"other").unwrap();
        let e = publish(&tmp, &out, false).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&out).unwrap(), b"other");
        assert!(!Path::new(&tmp).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An earlier output is replaced by the rename, and no temporary file is left.
    #[test]
    fn publish_replaces_an_earlier_output() {
        let dir = scratch("publish-replace");
        let out = dir.join("out.ep9h");
        std::fs::write(&out, b"old").unwrap();
        let tmp = stage(out.to_str().unwrap(), b"new").unwrap();
        publish(&tmp, &out, true).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert!(!Path::new(&tmp).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
