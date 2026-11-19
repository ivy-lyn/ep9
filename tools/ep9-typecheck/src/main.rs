//! `ep9-typecheck`: the tsrs type oracle, v0 (crate docs in `lib.rs`).
//!
//! ```text
//!  ep9-typecheck <tsconfig.json | dir> --out <file.ep9h>   check the project, write the file
//!  ep9-typecheck --verify <file.ep9h>                      read the file back, print a summary
//! ```
//!
//! Exit status: 0 done (a file with type errors is data, not a failure); 1 tsrs made no program,
//! or a file could not be read, written or verified; 2 bad arguments.

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
    std::fs::write(out, &bytes).map_err(|e| format!("cannot write {out}: {e}"))?;
    Ok(format!(
        "ep9-typecheck: {total} files ({with_errors} with type errors), 0 records, {} bytes -> {out}\n",
        bytes.len()
    ))
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
