//! `ep9 --facts-out <file>` (M3-P4 FIXES; H16-Q5): after the run, the feedback facts of the
//! main unit go to `<file>` as an encoded FactSnapshot v0 (`ep9_vm::abi::export_facts`, then
//! `FactSnapshot::encode`).
//!
//! The header fields are read at the offsets of `ep9_facts::offsets` (ep9_cli does not link
//! ep9_facts; the decode round trip is `crates/ep9_builtins/tests/facts_export.rs`):
//!
//! ```text
//!  0 magic "\x89EP9FS\r\n"   8 schema version u32   12 bytecode ABI u32   16 unit hash [u8; 32]
//! 48 source u8 (0 = runtime)  49 reserved [u8; 3]   52 shape count u32    56 record count u32
//! 60 total size u32          64 shapes, then records
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use ep9_frontend::{CompileOptions, SourceOpts, Strictness};

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn ep9(args: &[&str]) -> Out {
    let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
        .args(args)
        .env_remove("EP9_CLI_FAULT")
        .env_remove("RUST_BACKTRACE")
        .output()
        .expect("run ep9");
    Out {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A new empty directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ep9-facts-out-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create a temp dir");
    d
}

fn s(p: &Path) -> &str {
    p.to_str().expect("a UTF-8 path")
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// The content hash of `src` compiled as the CLI compiles it.
fn unit_hash(src: &str, opts: SourceOpts) -> [u8; 32] {
    ep9_frontend::compile(src, opts, CompileOptions::default())
        .expect("the source compiles")
        .content_hash()
}

/// Checks the header of an encoded snapshot of the unit `hash`. Returns its record count.
fn check_header(b: &[u8], hash: &[u8; 32]) -> u32 {
    assert!(b.len() >= 64, "{} bytes", b.len());
    assert_eq!(&b[..8], b"\x89EP9FS\r\n", "magic");
    assert_eq!(u32_at(b, 8), 1, "FACT_SCHEMA_VERSION");
    assert_eq!(u32_at(b, 12), ep9_bytecode::ABI_VERSION, "bytecode ABI");
    assert_eq!(&b[16..48], hash, "unit hash");
    assert_eq!(b[48], 0, "source: runtime");
    assert_eq!(&b[49..52], &[0, 0, 0], "reserved");
    assert_eq!(u32_at(b, 60) as usize, b.len(), "total size");
    u32_at(b, 56)
}

/// P runs 100 times (2 named stores), f runs 100 times (2 named loads, a global load, an add
/// and a global store); the top-level code runs once, below the default threshold of 8, but
/// gets its vector at entry (M4-P2 FACTS2, PERF3 step-2 item 3).
const SCRIPT: &str = "\
var g = 0;
function P(x) { this.x = x; this.y = 1; }
function f(o) { g = g + o.x; return o.y; }
for (var i = 0; i < 100; i++) f(new P(i));
print(g);
";

#[test]
fn a_script_run_writes_its_facts() {
    let d = temp_dir("script");
    let js = d.join("a.js");
    std::fs::write(&js, SCRIPT).unwrap();
    let hash = unit_hash(SCRIPT, SourceOpts::script(Strictness::Sloppy));

    let out = d.join("a.facts");
    let o = ep9(&["--facts-out", s(&out), s(&js)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!((o.stdout.as_str(), o.stderr.as_str()), ("4950\n", ""));
    let a = std::fs::read(&out).expect("the facts file");
    // Since M4-P2 FACTS2 the exporter writes every kind T0 records, and the top-level code has
    // its vector at entry: 7 records in P and f (2 StoreNamed; LoadGlobal, 2 LoadNamed,
    // BinaryOp, StoreGlobal) and 15 in the top-level code (2 CreateClosure, 2 StoreGlobal,
    // 5 LoadGlobal, CompareOp, 3 Call, BinaryOp, JumpLoop). Before FACTS2: the 6 IC sites.
    assert_eq!(
        check_header(&a, &hash),
        22,
        "22 sites in P, f and the top-level code"
    );

    // The same program gives the same bytes: in a second run, and with a GC before every
    // allocation (no address or ShapeId is in the output).
    let out2 = d.join("b.facts");
    let o = ep9(&[&format!("--facts-out={}", s(&out2)), s(&js)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(std::fs::read(&out2).unwrap(), a);
    let o = ep9(&["--gc-stress", "--facts-out", s(&out2), s(&js)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(std::fs::read(&out2).unwrap(), a);

    // With a vector at the first call, the same 22 sites have records (the top-level code
    // has its vector at entry either way); with none, the snapshot is empty.
    let o = ep9(&["--feedback-threshold", "0", "--facts-out", s(&out2), s(&js)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(check_header(&std::fs::read(&out2).unwrap(), &hash), 22);
    let o = ep9(&[
        "--feedback-threshold",
        "4294967295",
        "--facts-out",
        s(&out2),
        s(&js),
    ]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    let none = std::fs::read(&out2).unwrap();
    assert_eq!(check_header(&none, &hash), 0);
    assert_eq!(
        (u32_at(&none, 52), none.len()),
        (0, 64),
        "no shapes, header only"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn eval_code_and_the_entry_module_have_facts() {
    let d = temp_dir("module");
    let code = "function h(o) { return o.k; } for (let i = 0; i < 20; i++) h({k: i});";
    let out = d.join("e.facts");
    let o = ep9(&["-e", code, "--facts-out", s(&out)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    let hash = unit_hash(code, SourceOpts::script(Strictness::Sloppy));
    // 1 record in h (LoadNamed) and, since M4-P2 FACTS2 (a vector at entry, every kind), 8 in
    // the eval code: CreateClosure, CompareOp, LoadGlobal, Literal, DefineNamedOwn, Call,
    // BinaryOp, JumpLoop.
    assert_eq!(check_header(&std::fs::read(&out).unwrap(), &hash), 9);
    let o = ep9(&["--strict", "-e", code, "--facts-out", s(&out)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    let hash = unit_hash(code, SourceOpts::script(Strictness::Strict));
    assert_eq!(check_header(&std::fs::read(&out).unwrap(), &hash), 9);

    // The entry module's unit; the imported module is another unit, not in the file.
    let main = "import { v } from './dep.mjs';\n\
                function h(o) { return o.k; }\n\
                for (let i = 0; i < 20; i++) h({k: v});\n\
                print(h({k: v}));\n";
    std::fs::write(d.join("main.mjs"), main).unwrap();
    std::fs::write(
        d.join("dep.mjs"),
        "function d(o) { return o.z; } for (let i = 0; i < 20; i++) d({z: i}); export const v = 7;",
    )
    .unwrap();
    let o = ep9(&["--module", "--facts-out", s(&out), s(&d.join("main.mjs"))]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(o.stdout, "7\n");
    let hash = unit_hash(main, SourceOpts::module());
    // 1 record in h and 12 in the module body (M4-P2 FACTS2): CreateClosure, CompareOp,
    // 2 Literal, 2 DefineNamedOwn, 3 Call, BinaryOp, JumpLoop, LoadGlobal (`print`).
    assert_eq!(check_header(&std::fs::read(&out).unwrap(), &hash), 13);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn errors_and_exit_status() {
    let d = temp_dir("errors");
    // An uncaught exception: the file is still written, and the exit status stays 1.
    let out = d.join("t.facts");
    let o = ep9(&["-e", "throw 1", "--facts-out", s(&out)]);
    assert_eq!(o.code, Some(1));
    assert_eq!(o.stderr.trim_end(), "[eval]:1:1: Uncaught 1");
    let hash = unit_hash("throw 1", SourceOpts::script(Strictness::Sloppy));
    assert_eq!(check_header(&std::fs::read(&out).unwrap(), &hash), 0);

    // A compile error: no run, no file.
    let out = d.join("c.facts");
    let o = ep9(&["-e", "var = ;", "--facts-out", s(&out)]);
    assert_eq!(o.code, Some(1));
    assert!(!out.exists());

    // A file that cannot be written: an I/O error (exit 2) after a good run; after an
    // uncaught exception the run's own status (1) stays.
    let bad = d.join("no-such-dir").join("x.facts");
    let o = ep9(&["-e", "print(1)", "--facts-out", s(&bad)]);
    assert_eq!(o.code, Some(2));
    assert_eq!(o.stdout, "1\n");
    assert!(o.stderr.starts_with("ep9: cannot write "), "{}", o.stderr);
    let o = ep9(&["-e", "throw 1", "--facts-out", s(&bad)]);
    assert_eq!(o.code, Some(1));
    assert!(o.stderr.contains("ep9: cannot write "), "{}", o.stderr);

    // A usage error: the flag needs a file name.
    let o = ep9(&["-e", "1", "--facts-out"]);
    assert_eq!(o.code, Some(2));
    assert!(
        o.stderr.contains("--facts-out needs a value"),
        "{}",
        o.stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}
