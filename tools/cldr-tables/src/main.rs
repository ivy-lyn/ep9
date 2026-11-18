//! `cldr-tables [MODE] [--check] [OPTIONS]`: writes generated tables of `ep9_intl` (see the
//! library doc and README.md). `--check` writes nothing and exits 1 when a generated file
//! differs from the file on disk (the CI form). The modes ([`MODES`]):
//!
//! - `tables` (the default): the CLDR side tables, from the pinned CLDR JSON.
//!   Options: `--cldr DIR`, `--pin FILE`, `--out DIR`.
//! - `collator-search` (feature `datagen`): the search collations, from the pinned ICU export
//!   data (`collator_search` module). Options: `--icuexport ZIP`, `--pin FILE`, `--out DIR`.
//! - `t3`: T3, the flexible day periods and the numeric date patterns, from the pinned CLDR
//!   JSON (`t3` module). Options: `--cldr DIR`, `--pin FILE`, `--out DIR`.

use std::path::PathBuf;
use std::process::ExitCode;

/// A mode: its name, its path options and the function that runs it.
type Mode = (&'static str, &'static [&'static str], fn(Args) -> ExitCode);

/// The modes. Append a new mode at the end.
const MODES: &[Mode] = &[
    ("tables", &["--cldr", "--pin", "--out"], tables),
    (
        "collator-search",
        &["--icuexport", "--pin", "--out"],
        collator_search,
    ),
    ("t3", &["--cldr", "--pin", "--out"], t3),
];

/// The usage line.
const USAGE: &str = "usage: cldr-tables [tables] [--check] [--cldr DIR] [--pin FILE] [--out DIR]\n       \
                     cldr-tables collator-search [--check] [--icuexport ZIP] [--pin FILE] [--out DIR]\n       \
                     cldr-tables t3 [--check] [--cldr DIR] [--pin FILE] [--out DIR]";

/// The parsed options of a mode: `--check` and the path options it names.
struct Args {
    check: bool,
    paths: Vec<(&'static str, PathBuf)>,
}

impl Args {
    /// The value of the path option `name`, or `default`.
    fn path(&self, name: &str, default: PathBuf) -> PathBuf {
        self.paths
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map_or(default, |(_, p)| p.clone())
    }
}

/// Parses `--check` and the path options in `names`.
fn parse(args: impl Iterator<Item = String>, names: &[&'static str]) -> Result<Args, String> {
    let mut out = Args {
        check: false,
        paths: Vec::new(),
    };
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--check" {
            out.check = true;
        } else if let Some(name) = names.iter().find(|n| **n == arg) {
            let v = args.next().ok_or_else(|| format!("{name} needs a value"))?;
            out.paths.push((name, PathBuf::from(v)));
        } else {
            return Err(format!("unknown argument {arg}"));
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    let mode = match args.peek() {
        Some(a) if !a.starts_with("--") => args.next().unwrap_or_default(),
        _ => "tables".to_owned(),
    };
    let Some((_, names, run)) = MODES.iter().find(|(name, _, _)| *name == mode) else {
        eprintln!("cldr-tables: unknown mode {mode}\n{USAGE}");
        return ExitCode::from(2);
    };
    match parse(args, names) {
        Ok(a) => run(a),
        Err(e) => {
            eprintln!("cldr-tables: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Writes `files` into `out`, or with `check` compares them (the `--check` form).
fn write_or_check(
    out: &std::path::Path,
    files: &std::collections::BTreeMap<&'static str, String>,
    check: bool,
    what: &str,
    again: &str,
) -> Result<(), ExitCode> {
    if check {
        let stale = cldr_tables::stale(out, files);
        if stale.is_empty() {
            let total: usize = files.values().map(String::len).sum();
            println!(
                "cldr-tables --check: {} files current ({what}, {total} B)",
                files.len()
            );
            return Ok(());
        }
        eprintln!(
            "cldr-tables --check: {} differ from the {what} output: {}; run {again}",
            out.display(),
            stale.join(", ")
        );
        return Err(ExitCode::FAILURE);
    }
    for (name, text) in files {
        let path = out.join(name);
        let dir = path.parent().unwrap_or(out);
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("cldr-tables: {}: {e}", dir.display());
            return Err(ExitCode::from(2));
        }
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("cldr-tables: {}: {e}", path.display());
            return Err(ExitCode::from(2));
        }
        println!("wrote {} ({} B)", path.display(), text.len());
    }
    Ok(())
}

/// The `tables` mode: the CLDR side tables.
fn tables(a: Args) -> ExitCode {
    let (cldr, pin, out) = cldr_tables::default_paths();
    let (cldr, pin, out) = (
        a.path("--cldr", cldr),
        a.path("--pin", pin),
        a.path("--out", out),
    );
    let tables = match cldr_tables::read_pin(&pin).and_then(|p| cldr_tables::read(&cldr, &p)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cldr-tables: {e}");
            return ExitCode::from(2);
        }
    };
    let files = cldr_tables::render(&tables);
    let total: usize = files.values().map(String::len).sum();
    let what = format!("CLDR {}", tables.cldr_version);
    if let Err(code) = write_or_check(
        &out,
        &files,
        a.check,
        &what,
        "cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml",
    ) {
        return code;
    }
    if !a.check {
        println!(
            "cldr-tables: CLDR {} ({}): {} numbering systems, {} T1 records, {} T1 rows, T2 {} \
             units, {} texts, {} records, {} unit sets, {total} B",
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
    }
    ExitCode::SUCCESS
}

/// The `collator-search` mode: the search collations (feature `datagen`).
#[cfg(feature = "datagen")]
fn collator_search(a: Args) -> ExitCode {
    use cldr_tables::collator_search as cs;
    let (pin, dir) = cs::default_paths();
    let (_, _, out) = cldr_tables::default_paths();
    let (pin_file, out) = (a.path("--pin", pin), a.path("--out", out));
    let pin = match cs::read_pin(&pin_file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cldr-tables: {e}");
            return ExitCode::from(2);
        }
    };
    let zip = a.path("--icuexport", dir.join(&pin.file));
    let list = match cs::read(&pin, &zip) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "cldr-tables: {e}\n(run tools/cldr-tables/sync-icuexport.sh first; it checks the \
                 sha256 of icuexport.pin)"
            );
            return ExitCode::from(2);
        }
    };
    let text = cs::render(&pin, &list);
    let len = text.len();
    let files = std::collections::BTreeMap::from([(cs::OUTPUT_FILE, text)]);
    let what = format!("ICU export {}", pin.tag);
    if let Err(code) = write_or_check(
        &out,
        &files,
        a.check,
        &what,
        "cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml --features datagen -- \
         collator-search",
    ) {
        return code;
    }
    if !a.check {
        let ids: Vec<String> = list
            .iter()
            .map(|c| format!("{}/{}", c.locale, c.collation))
            .collect();
        println!(
            "cldr-tables collator-search: ICU export {} ({}): {} search collations ({}), {len} B",
            pin.tag,
            pin.sha256,
            list.len(),
            ids.join(" ")
        );
    }
    ExitCode::SUCCESS
}

/// The `collator-search` mode without the feature `datagen`.
#[cfg(not(feature = "datagen"))]
fn collator_search(_: Args) -> ExitCode {
    eprintln!(
        "cldr-tables: the collator-search mode needs the feature datagen (cargo run --locked \
         --manifest-path tools/cldr-tables/Cargo.toml --features datagen -- collator-search)"
    );
    ExitCode::from(2)
}

/// The `t3` mode: T3, the flexible day periods and the numeric date patterns.
fn t3(a: Args) -> ExitCode {
    use cldr_tables::t3;
    let (cldr, pin, out) = cldr_tables::default_paths();
    let (cldr, pin, out) = (
        a.path("--cldr", cldr),
        a.path("--pin", pin),
        a.path("--out", out),
    );
    let t = match cldr_tables::read_pin(&pin).and_then(|p| t3::read(&cldr, &p)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cldr-tables: {e}");
            return ExitCode::from(2);
        }
    };
    let text = t3::render(&t);
    let len = text.len();
    let files = std::collections::BTreeMap::from([(t3::OUTPUT_FILE, text)]);
    let what = format!("CLDR {} T3", t.cldr_version);
    if let Err(code) = write_or_check(
        &out,
        &files,
        a.check,
        &what,
        "cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml -- t3",
    ) {
        return code;
    }
    if !a.check {
        println!(
            "cldr-tables t3: CLDR {} ({}): {} rule locales, {} texts, {} name records, {} sets, \
             {} locales, {len} B",
            t.cldr_version,
            t.cldr_commit,
            t.rule_locales.len(),
            t.texts.len(),
            t.names.len(),
            t.sets.len(),
            t.locales.len()
        );
    }
    ExitCode::SUCCESS
}
