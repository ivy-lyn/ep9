//! `cldr-tables [--check] [--cldr DIR] [--pin FILE] [--out DIR]`: writes the CLDR side tables
//! of `ep9_intl` (see the library doc and README.md). `--check` writes nothing and exits 1 when
//! a generated file differs from the file on disk (the CI form).

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let (mut cldr, mut pin, mut out) = cldr_tables::default_paths();
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| match args.next() {
            Some(v) => Ok(PathBuf::from(v)),
            None => Err(format!("{name} needs a value")),
        };
        let r = match arg.as_str() {
            "--check" => {
                check = true;
                Ok(())
            }
            "--cldr" => value("--cldr").map(|v| cldr = v),
            "--pin" => value("--pin").map(|v| pin = v),
            "--out" => value("--out").map(|v| out = v),
            _ => Err(format!("unknown argument {arg}")),
        };
        if let Err(e) = r {
            eprintln!(
                "cldr-tables: {e}\nusage: cldr-tables [--check] [--cldr DIR] [--pin FILE] [--out DIR]"
            );
            return ExitCode::from(2);
        }
    }

    let tables = match cldr_tables::read_pin(&pin).and_then(|p| cldr_tables::read(&cldr, &p)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cldr-tables: {e}");
            return ExitCode::from(2);
        }
    };
    let files = cldr_tables::render(&tables);
    let total: usize = files.values().map(String::len).sum();

    if check {
        let stale = cldr_tables::stale(&out, &files);
        if stale.is_empty() {
            println!(
                "cldr-tables --check: {} files current (CLDR {}, {total} B)",
                files.len(),
                tables.cldr_version
            );
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "cldr-tables --check: {} differ from the CLDR {} output: {}; run cargo run --locked \
             --manifest-path tools/cldr-tables/Cargo.toml",
            out.display(),
            tables.cldr_version,
            stale.join(", ")
        );
        return ExitCode::FAILURE;
    }

    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("cldr-tables: {}: {e}", out.display());
        return ExitCode::from(2);
    }
    for (name, text) in &files {
        let path = out.join(name);
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("cldr-tables: {}: {e}", path.display());
            return ExitCode::from(2);
        }
        println!("wrote {} ({} B)", path.display(), text.len());
    }
    println!(
        "cldr-tables: CLDR {} ({}): {} numbering systems, {} T1 records, {} T1 rows, T2 {} units, \
         {} texts, {} records, {} unit sets, {total} B",
        tables.cldr_version,
        tables.cldr_commit,
        tables.numbering_systems.len(),
        tables.t1_symbols.len(),
        tables.t1_rows.len(),
        tables.unit_ids.len(),
        tables.t2_texts.len(),
        tables.t2_records.len(),
        tables.t2_sets.len()
    );
    ExitCode::SUCCESS
}
