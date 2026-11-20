//! Steps 1-4 of `docs/plan/05-aot.md` section 6 on tsrs: the config, the host, the program and
//! the full check. The calls follow the R13 recipe (`docs/research/13-tsrs-type-oracle.md`,
//! Findings 1) and `tsrs_testrunner` `types_dump.rs`.
//!
//! The check is the one of `get_diagnostics_of_any_program`, but with no early stop: that
//! function skips the semantic check of every file when one file has a syntax error or the
//! program has a global error, so a file would read as clean when it was not checked. Here each
//! kind of diagnostic is read for all files. An error diagnostic sets `has_type_errors` on its
//! file; an error with no file, or with a file that is not in the table (a lib file), sets it on
//! every file, because then no type of the program can be trusted (S3).
//!
//! tsrs keeps process-wide state (a fixed address reserve, arenas that live until exit; R13), so
//! [`check`] runs once per process.

use std::collections::HashMap;
use std::sync::Arc;

use ep9_hints::TsFlags;
use tsrs_ast::Diagnostic;
use tsrs_compiler::{Context, ProgramOptions, new_cached_fs_compiler_host, new_program};
use tsrs_core::{CompilerOptions, P, Tristate, tspath};
use tsrs_diagnostics::Category;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{FS, bundled::embed as bundled, osvfs};

use crate::{FileFacts, text_hash};

/// The result of [`check`].
#[derive(Clone, Debug)]
pub struct Checked {
    /// One entry per source file that is not a bundled default lib file, in program order.
    pub files: Vec<FileFacts>,
    /// The three TS flags of H3, as the program's options give them.
    pub ts_flags: TsFlags,
    /// Error lines for the log: `TS<code> <path>: <message>`; `(program)` when no file is named.
    pub errors: Vec<String>,
    /// The absolute paths of the known files that the check read: the config file, each config
    /// that it extends (all steps of an `extends` chain) and each source file of the table.
    /// `--out` must not name one of them. The list is not complete (a config named by
    /// `references`, a `package.json` of module resolution are not in it): `main.rs` also
    /// refuses an `--out` file that exists and does not start with the `.ep9h` magic.
    pub inputs: Vec<String>,
}

struct Host {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for Host {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn relative(dir: &str, file_name: &str) -> String {
    tspath::get_relative_path_from_directory(
        dir,
        file_name,
        &tspath::ComparePathsOptions {
            use_case_sensitive_file_names: true,
            ..Default::default()
        },
    )
}

/// Runs steps 1-4 on `project`: a tsconfig file, or a directory that holds `tsconfig.json`.
/// An error means that tsrs could not make a program (no config, or a config it cannot read).
pub fn check(project: &str) -> Result<Checked, String> {
    // 1 config
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let cwd = std::env::current_dir().map_err(|e| format!("no current directory: {e}"))?;
    let cwd = tspath::normalize_path(&cwd.to_string_lossy());
    let host: &'static Host = Box::leak(Box::new(Host {
        fs: Arc::clone(&fs),
        cwd: cwd.clone(),
    }));
    let mut config_path = tspath::get_normalized_absolute_path(project, &cwd);
    if fs.directory_exists(&config_path) {
        config_path = tspath::combine_paths(&config_path, &["tsconfig.json"]);
    }
    if !fs.file_exists(&config_path) {
        return Err(format!("no config file {project}"));
    }
    let (config, config_errors) = tsoptions::get_parsed_command_line_of_config_file(
        &config_path,
        Some(&CompilerOptions::default()),
        None,
        host,
        None,
    );
    let Some(config) = config else {
        let text: Vec<String> = config_errors.iter().map(|d| d.localize()).collect();
        return Err(format!("cannot read {project}: {}", text.join("; ")));
    };
    let config_dir = tspath::get_directory_path(&config_path);
    // Every step of an `extends` chain: tsrs adds the extended files of each extended config.
    let mut inputs = vec![config_path.clone()];
    inputs.extend(config.extended_source_files());

    // 2 host, 3 program
    let compiler_host =
        new_cached_fs_compiler_host(&cwd, fs, &bundled::lib_path(), None, None, None);
    let mut opts = ProgramOptions::new(P::new(config), compiler_host);
    opts.single_threaded = Tristate::True;
    let program = new_program(opts);

    let options = program.options();
    let ts_flags = TsFlags {
        use_define_for_class_fields: options.get_use_define_for_class_fields(),
        experimental_decorators: options.experimental_decorators.is_true(),
        emit_decorator_metadata: options.emit_decorator_metadata.is_true(),
    };

    // The file table: every source file but the bundled default lib files. The hash is of the
    // text that tsrs parsed (the decoded text), read before the check.
    let mut files = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for &sf in program.source_files() {
        if program.is_source_file_default_library(sf.path()) {
            continue;
        }
        index.insert(sf.file_name().to_owned(), files.len());
        inputs.push(sf.file_name().to_owned());
        files.push(FileFacts {
            path: relative(&config_dir, sf.file_name()),
            text_hash: text_hash(sf.text()),
            has_type_errors: false,
        });
    }

    // 4 the full check, with no early stop (module docs).
    let ctx = Context::default();
    let mut diagnostics: Vec<P<Diagnostic>> = config_errors;
    diagnostics.extend(program.get_config_file_parsing_diagnostics());
    diagnostics.extend(program.get_syntactic_diagnostics(&ctx, None));
    diagnostics.extend(program.get_program_diagnostics());
    diagnostics.extend(program.get_bind_diagnostics(&ctx, None));
    diagnostics.extend(program.get_global_diagnostics(&ctx));
    diagnostics.extend(program.get_semantic_diagnostics(&ctx, None));
    diagnostics.extend(program.get_global_diagnostics(&ctx));

    let mut errors = Vec::new();
    let mut program_error = false;
    for d in diagnostics
        .iter()
        .filter(|d| d.category() == Category::Error)
    {
        let file = d.file().map(|f| f.file_name().to_owned());
        let at = match file.as_deref().map(|f| (f, index.get(f))) {
            Some((_, Some(&i))) => {
                files[i].has_type_errors = true;
                files[i].path.clone()
            }
            Some((f, None)) => {
                program_error = true;
                f.to_owned()
            }
            None => {
                program_error = true;
                "(program)".to_owned()
            }
        };
        errors.push(format!("TS{} {at}: {}", d.code(), d.localize()));
    }
    if program_error {
        for f in &mut files {
            f.has_type_errors = true;
        }
    }
    errors.sort();
    errors.dedup();
    Ok(Checked {
        files,
        ts_flags,
        errors,
        inputs,
    })
}
