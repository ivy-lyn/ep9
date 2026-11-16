//! The INTL-V2-FP3 source guard (H36, the fp-check item of INTL W3): no `ep9_intl` source
//! calls the ICU4X unit converter (`icu_experimental::units`: its `Convertible::mul_add` is
//! `f64` math, `self * factor + addend`, that the fp-check symbol scan does not see) or the
//! `f64` functions `log10`, `powi`, `powf` and `mul_add`. ECMA-402 converts no unit, and the
//! exponent of a value comes from its `Decimal` magnitude with integer arithmetic.
//!
//! ```text
//! crates/ep9_intl/src/**/*.rs ─ every line that is not a `//` comment line
//!   ─ fails on: units::converter  UnitsConverter  ConverterFactory  log10(  powi(  powf(  mul_add(
//! control: planted lines fail it; comment lines and `mul_add_small(` (digits.rs) do not
//! ```

use std::path::{Path, PathBuf};

/// The banned texts (DL INTL-V2-FP3 item 3).
const BANNED: [&str; 7] = [
    "units::converter",
    "UnitsConverter",
    "ConverterFactory",
    "log10(",
    "powi(",
    "powf(",
    "mul_add(",
];

/// The (line number, banned text) of each hit in the source text `text`. A line whose first
/// non-blank characters are `//` (a comment or doc comment line) is not read.
fn hits(text: &str) -> Vec<(usize, &'static str)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for b in BANNED {
            if line.contains(b) {
                out.push((n + 1, b));
            }
        }
    }
    out
}

/// The `.rs` files under `dir`, sorted.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).expect("a source directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// No `ep9_intl` source line names the unit converter or a banned `f64` function.
#[test]
fn no_unit_conversion_and_no_float_exponent_in_the_sources() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = rust_files(&src);
    // The scan reads the whole tree (lib.rs, the services, number_format/, cldr/gen/).
    assert!(files.len() >= 30, "{} files", files.len());
    assert!(files.iter().any(|f| f.ends_with("number_format/unit.rs")));
    let mut found = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("a source file");
        for (line, b) in hits(&text) {
            found.push(format!("{}:{line}: {b}", file.display()));
        }
    }
    assert!(
        found.is_empty(),
        "DL INTL-V2-FP3: no ICU4X unit conversion and no f64 log10/pow/mul_add in ep9_intl:\n{}",
        found.join("\n")
    );
}

/// The control: a planted line of each banned text fails the scan; comment lines and the
/// integer helper `mul_add_small` do not.
#[test]
fn the_scan_finds_planted_lines() {
    let planted = "\
use icu_experimental::units::converter::UnitsConverter;
fn f(x: f64) -> f64 {
    let e = x.log10().floor();
    let _ = (x.powi(2), x.powf(0.5), x.mul_add(2.0, 1.0));
    let _ = ConverterFactory::new();
    e
}
";
    assert_eq!(
        hits(planted),
        [
            (1, "units::converter"),
            (1, "UnitsConverter"),
            (3, "log10("),
            (4, "powi("),
            (4, "powf("),
            (4, "mul_add("),
            (5, "ConverterFactory"),
        ]
    );
    let clean = "\
// x.log10() in a comment line
    /// `UnitsConverter` in a doc line
//! mul_add( in a module doc line
let y = mul_add_small(&mut digits, 10, 3);
";
    assert_eq!(hits(clean), []);
}
