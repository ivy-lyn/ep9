//! `ep9 --facts-out <file>` (M3-P4 FIXES; H16-Q5): after the run, the feedback facts of the
//! main unit go to `<file>` as an encoded FactSnapshot, FACT_SCHEMA 2 since M6-P0
//! (`ep9_vm::abi::export_facts`, then `FactSnapshot::encode`).
//!
//! The header fields are read at the offsets of `ep9_facts::offsets` (ep9_cli does not link
//! ep9_facts; the decode round trip is `crates/ep9_builtins/tests/facts_export.rs`):
//!
//! ```text
//!  0 magic "\x89EP9FS\r\n"   8 schema version u32   12 bytecode ABI u32   16 unit hash [u8; 32]
//! 48 source u8 (0 = runtime)  49 reserved [u8; 3]   52 shape count u32    56 record count u32
//! 60 total size u32          64 shapes, then records
//! ```
//!
//! The sites are counted by kind, read past the shape table (`kinds`, by the layout of
//! `ep9_facts::codec`). What the file holds depends on the build's preset (`ics_only`):
//!
//! ```text
//! ep9_cli build                  preset    feedback    the export holds
//! default (`jit` feature)        jit       full        every kind T0 records
//! `--no-default-features`        jitless   ics_only    the IC and object-model kinds only
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use ep9_bytecode::FeedbackSlotKind as K;
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
    // `ep9_facts::FACT_SCHEMA_VERSION`, read from its source line: ep9_cli does not link
    // ep9_facts (above), and a literal fails on every bump (MC-P5-FACTTOP).
    let version: u32 = include_str!("../../ep9_facts/src/lib.rs")
        .lines()
        .find_map(|l| l.strip_prefix("pub const FACT_SCHEMA_VERSION: u32 = "))
        .and_then(|v| v.strip_suffix(';')?.parse().ok())
        .expect("FACT_SCHEMA_VERSION in crates/ep9_facts/src/lib.rs");
    assert_eq!(u32_at(b, 8), version, "FACT_SCHEMA_VERSION");
    assert_eq!(u32_at(b, 12), ep9_bytecode::ABI_VERSION, "bytecode ABI");
    assert_eq!(&b[16..48], hash, "unit hash");
    assert_eq!(b[48], 0, "source: runtime");
    assert_eq!(&b[49..52], &[0, 0, 0], "reserved");
    assert_eq!(u32_at(b, 60) as usize, b.len(), "total size");
    u32_at(b, 56)
}

/// Whether the isolates of the `ep9` binary run feedback `ics_only`. The CLI makes them with
/// `Isolate::new`, so with `ExecOptions::default()`: the `jit` preset (feedback `full`) with
/// the `jit` feature, else the `jitless` preset (feedback `ics_only`, MODES-D11;
/// `crates/ep9_vm/src/exec.rs`). This test links the same `ep9_vm` build as the binary; the
/// second line of `ep9 --version` checks that.
fn ics_only() -> bool {
    static ICS_ONLY: OnceLock<bool> = OnceLock::new();
    *ICS_ONLY.get_or_init(|| {
        let o = ep9(&["--version"]);
        let build = ep9_vm::build_info().to_string();
        assert_eq!(
            o.stdout.lines().nth(1),
            Some(build.as_str()),
            "{}",
            o.stderr
        );
        ep9_vm::ExecOptions::default().feedback_mode() == ep9_vm::Feedback::ICS_ONLY
    })
}

/// The record kinds: only the T0 record hooks write their facts, an `ics_only` isolate runs
/// none, so its export leaves them out (H16, the 2026-11-01 rewrite row;
/// `crates/ep9_vm/src/facts_export.rs` `keep`). The Call slot of a `Call*` site is left out
/// too; that of a `Construct` site stays (an IC).
const RECORD_KINDS: [K; 11] = [
    K::LoadKeyed,
    K::StoreKeyed,
    K::DefineKeyedOwn,
    K::HasKeyed,
    K::StoreInArrayLiteral,
    K::BinaryOp,
    K::CompareOp,
    K::TypeOf,
    K::ForIn,
    K::JumpLoop,
    K::CreateClosure,
];

/// A reader past the header: it skips each field by the `ep9_facts::codec` layout.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip(&mut self, n: usize) {
        self.at += n;
        assert!(
            self.at <= self.b.len(),
            "a field past the end at {}",
            self.at
        );
    }
    fn u8(&mut self) -> u8 {
        self.skip(1);
        self.b[self.at - 1]
    }
    fn len(&mut self) -> usize {
        self.skip(4);
        u32_at(self.b, self.at - 4) as usize
    }
    /// A FunctionRef: tag 0 Js (unit hash, function u32), 1 Builtin (u16).
    fn function(&mut self) {
        match self.u8() {
            0 => self.skip(32 + 4),
            1 => self.skip(2),
            t => panic!("function tag {t}"),
        }
    }
    /// A ShapeDesc of the shape table.
    fn shape(&mut self) {
        match self.u8() {
            0 | 3 => {}
            1 => self.skip(4),
            2 => self.function(),
            t => panic!("proto tag {t}"),
        }
        self.skip(1 + 1 + 1 + 2); // kind, elements, flags, capacity
        for _ in 0..self.len() {
            match self.u8() {
                0 => {
                    let n = self.len();
                    self.skip(n)
                }
                1 => {
                    let n = self.len();
                    self.skip(2 * n)
                }
                2 => self.skip(1),
                3 => {}
                t => panic!("key tag {t}"),
            }
            self.skip(1); // attrs, with the rep/const hint
        }
    }
    fn targets(&mut self) {
        self.skip(1); // state
        for _ in 0..self.len() {
            self.function();
        }
    }
    fn property(&mut self) {
        self.skip(1); // state
        for _ in 0..self.len() {
            self.skip(4); // receiver shape
            match self.u8() {
                0 => self.skip(2),
                1 => self.skip(1 + 2),
                2 | 8 => self.skip(1),
                3 | 4 => {
                    self.skip(1);
                    self.function();
                }
                5 => self.skip(4 + 2),
                6 | 7 | 9 => {}
                t => panic!("access tag {t}"),
            }
        }
        self.skip(2); // values
    }
    fn payload(&mut self, k: K) {
        match k {
            K::Call => {
                self.skip(4);
                self.targets();
                self.skip(2);
            }
            K::LoadNamed | K::StoreNamed | K::DefineNamedOwn => self.property(),
            K::LoadKeyed
            | K::StoreKeyed
            | K::DefineKeyedOwn
            | K::HasKeyed
            | K::StoreInArrayLiteral => {
                self.property();
                self.skip(1 + 1 + 2); // keys, hint, kinds
            }
            K::LoadGlobal | K::LoadGlobalInsideTypeof | K::StoreGlobal => self.skip(1 + 2),
            K::BinaryOp | K::ForIn | K::CreateClosure => self.skip(1),
            K::CompareOp | K::TypeOf => self.skip(2),
            K::InstanceOf => self.targets(),
            K::Literal => self.skip(4 + 4 + 1),
            K::CloneObject => {
                self.skip(1);
                let n = self.len();
                self.skip(8 * n);
            }
            K::JumpLoop => self.skip(4),
        }
    }
}

/// The sites of the snapshot `b` by kind name, with their counts.
fn kinds(b: &[u8]) -> BTreeMap<&'static str, u32> {
    let mut r = Reader { b, at: 64 };
    for _ in 0..u32_at(b, 52) {
        r.shape();
    }
    let mut kinds = BTreeMap::new();
    for _ in 0..u32_at(b, 56) {
        r.skip(4 + 4); // function, slot
        let k = r.u8();
        let k = K::from_u8(k).unwrap_or_else(|| panic!("kind {k}"));
        // The record flags; a runtime snapshot has no confidence byte (FACT_SCHEMA 2).
        let flags = r.u8();
        assert_eq!(flags & !0b11, 0, "record flags {flags:#x}");
        r.payload(k);
        *kinds.entry(k.name()).or_insert(0) += 1;
    }
    assert_eq!(r.at, b.len(), "the records end the file");
    kinds
}

/// The sites of one run by kind: in a `full` export and in an `ics_only` one.
struct Sites {
    full: &'static [(K, u32)],
    ics_only: &'static [(K, u32)],
}

/// Checks the snapshot `b` of the unit `hash`: its header, and its sites by kind for the
/// build's preset. An `ics_only` file has no record kind. Returns the record count.
fn check_facts(b: &[u8], hash: &[u8; 32], sites: &Sites) -> u32 {
    let n = check_header(b, hash);
    let got = kinds(b);
    let want = if ics_only() {
        sites.ics_only
    } else {
        sites.full
    };
    let want: BTreeMap<&str, u32> = want.iter().map(|&(k, n)| (k.name(), n)).collect();
    assert_eq!(got, want, "sites by kind, ics_only {}", ics_only());
    if ics_only() {
        for k in RECORD_KINDS {
            assert!(
                !got.contains_key(k.name()),
                "{} in an ics_only file",
                k.name()
            );
        }
    }
    n
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

/// SCRIPT: 7 sites in P and f (2 StoreNamed; LoadGlobal, 2 LoadNamed, BinaryOp, StoreGlobal)
/// and 15 in the top-level code (2 CreateClosure, 2 StoreGlobal, 5 LoadGlobal, CompareOp,
/// 3 Call, BinaryOp, JumpLoop). `ics_only` leaves out 8: the 2 BinaryOp, CompareOp, JumpLoop,
/// the 2 CreateClosure and the Call slots of `f(..)` and `print(g)`; that of `new P(i)` stays.
const SCRIPT_SITES: Sites = Sites {
    full: &[
        (K::StoreNamed, 2),
        (K::LoadNamed, 2),
        (K::LoadGlobal, 6),
        (K::StoreGlobal, 3),
        (K::Call, 3),
        (K::BinaryOp, 2),
        (K::CompareOp, 1),
        (K::JumpLoop, 1),
        (K::CreateClosure, 2),
    ],
    ics_only: &[
        (K::StoreNamed, 2),
        (K::LoadNamed, 2),
        (K::LoadGlobal, 6),
        (K::StoreGlobal, 3),
        (K::Call, 1),
    ],
};

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
    // Since M4-P2 FACTS2 a `full` export holds every kind T0 records, and the top-level code
    // has its vector at entry: 22 sites (SCRIPT_SITES). Before FACTS2: the 6 IC sites. Since
    // M5-P4 (MC-w21-1) an `ics_only` export (the jitless build) holds 14: no record kind.
    assert_eq!(
        check_facts(&a, &hash, &SCRIPT_SITES),
        if ics_only() { 14 } else { 22 },
        "the sites in P, f and the top-level code"
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

    // With a vector at the first call, the same 22 (`ics_only`: 14) sites have records (the
    // top-level code has its vector at entry either way); with none, the snapshot is empty.
    let o = ep9(&["--feedback-threshold", "0", "--facts-out", s(&out2), s(&js)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    let b = std::fs::read(&out2).unwrap();
    assert_eq!(
        check_facts(&b, &hash, &SCRIPT_SITES),
        if ics_only() { 14 } else { 22 }
    );
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
    // BinaryOp, JumpLoop. `ics_only` leaves out 5: CreateClosure, CompareOp, the Call slot of
    // `h(..)`, BinaryOp, JumpLoop.
    let eval_sites = Sites {
        full: &[
            (K::LoadNamed, 1),
            (K::LoadGlobal, 1),
            (K::Literal, 1),
            (K::DefineNamedOwn, 1),
            (K::Call, 1),
            (K::BinaryOp, 1),
            (K::CompareOp, 1),
            (K::JumpLoop, 1),
            (K::CreateClosure, 1),
        ],
        ics_only: &[
            (K::LoadNamed, 1),
            (K::LoadGlobal, 1),
            (K::Literal, 1),
            (K::DefineNamedOwn, 1),
        ],
    };
    let n = if ics_only() { 4 } else { 9 };
    assert_eq!(
        check_facts(&std::fs::read(&out).unwrap(), &hash, &eval_sites),
        n
    );
    let o = ep9(&["--strict", "-e", code, "--facts-out", s(&out)]);
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    let hash = unit_hash(code, SourceOpts::script(Strictness::Strict));
    assert_eq!(
        check_facts(&std::fs::read(&out).unwrap(), &hash, &eval_sites),
        n
    );

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
    // `ics_only` leaves out 7: CreateClosure, CompareOp, the 3 Call slots (2 `h(..)`,
    // `print(..)`), BinaryOp, JumpLoop.
    let module_sites = Sites {
        full: &[
            (K::LoadNamed, 1),
            (K::LoadGlobal, 1),
            (K::Literal, 2),
            (K::DefineNamedOwn, 2),
            (K::Call, 3),
            (K::BinaryOp, 1),
            (K::CompareOp, 1),
            (K::JumpLoop, 1),
            (K::CreateClosure, 1),
        ],
        ics_only: &[
            (K::LoadNamed, 1),
            (K::LoadGlobal, 1),
            (K::Literal, 2),
            (K::DefineNamedOwn, 2),
        ],
    };
    assert_eq!(
        check_facts(&std::fs::read(&out).unwrap(), &hash, &module_sites),
        if ics_only() { 6 } else { 13 }
    );
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
