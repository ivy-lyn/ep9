//! The binary on the fixture project (tests/fixture; its README lists the files). Each run is
//! its own process, because tsrs keeps process-wide state (src/oracle.rs).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ep9_hints::{Expect, Hints, TsFlags, read};
use sha2::{Digest, Sha256};

const BIN: &str = env!("CARGO_BIN_EXE_ep9-typecheck");

/// The UTF-16 file that the tests add to a copy of the fixture.
const UTF16_TEXT: &str = "export const u: string = \"\u{e9}\u{1f600}\";\n";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of the fixture in `dir/p`, with `src/utf16.ts` (UTF-16 LE with its BOM).
fn fixture_copy(dir: &Path) -> PathBuf {
    let p = dir.join("p");
    copy_dir(&fixture(), &p);
    let mut utf16 = vec![0xff, 0xfe];
    for unit in UTF16_TEXT.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(p.join("src/utf16.ts"), utf16).unwrap();
    p
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

/// Runs the check of `project` to `out` and returns the bytes.
fn check(cwd: &Path, project: &Path, out: &Path) -> Vec<u8> {
    let o = run(
        cwd,
        &[project.to_str().unwrap(), "--out", out.to_str().unwrap()],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::read(out).unwrap()
}

/// The fixture's flags: target es2022 gives useDefineForClassFields; experimentalDecorators set.
fn flags() -> TsFlags {
    TsFlags {
        use_define_for_class_fields: true,
        experimental_decorators: true,
        emit_decorator_metadata: false,
    }
}

fn hints(bytes: &[u8]) -> Hints {
    let expect = Expect {
        fact_schema_version: ep9_typecheck::FACT_SCHEMA_VERSION,
        ts_flags: flags(),
    };
    read(bytes, &expect).expect("ep9_hints::read accepts the output")
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn table(h: &Hints) -> Vec<(&str, bool)> {
    h.files
        .iter()
        .map(|f| (f.path.as_str(), f.has_type_errors))
        .collect()
}

#[test]
fn fixture_gives_the_header_and_the_file_table() {
    let dir = scratch("table");
    let p = fixture_copy(&dir);
    let h = hints(&check(&dir, &p, &dir.join("out.ep9h")));
    assert_eq!(Some(h.header.tsrs_commit), ep9_typecheck::tsrs_commit());
    assert_eq!(
        h.header.fact_schema_version,
        ep9_typecheck::FACT_SCHEMA_VERSION
    );
    assert_eq!(h.header.ts_flags, flags());
    assert_eq!(
        table(&h),
        [
            ("src/bad.ts", true),
            ("src/bom.ts", false),
            ("src/ok.ts", false),
            ("src/utf16.ts", false),
        ]
    );
    assert!(h.records.is_empty() && h.classes.is_empty() && h.fields.is_empty());

    // The hash is of the decoded UTF-8 text after the BOM strip (H37-Q3).
    let raw = |name: &str| std::fs::read(p.join("src").join(name)).unwrap();
    let hash = |i: usize| h.files[i].text_hash;
    assert_eq!(hash(0), sha(&raw("bad.ts")));
    let bom = raw("bom.ts");
    assert_eq!(&bom[..3], b"\xef\xbb\xbf");
    assert_eq!(hash(1), sha(&bom[3..]));
    assert_ne!(hash(1), sha(&bom));
    let ok = raw("ok.ts");
    assert!(std::str::from_utf8(&ok).unwrap().contains("größe € 😀"));
    assert_eq!(hash(2), sha(&ok));
    // UTF-16: the hash of the UTF-8 form, never of the UTF-16 bytes.
    let utf16 = raw("utf16.ts");
    assert_eq!(hash(3), sha(UTF16_TEXT.as_bytes()));
    assert_ne!(hash(3), sha(&utf16));
    assert_ne!(hash(3), sha(&utf16[2..]));
}

#[test]
fn same_input_twice_gives_the_same_bytes() {
    let dir = scratch("twice");
    let a = check(&fixture(), Path::new("tsconfig.json"), &dir.join("a.ep9h"));
    let b = check(&dir, &fixture(), &dir.join("b.ep9h"));
    let c = check(&dir, &fixture().join("tsconfig.json"), &dir.join("c.ep9h"));
    assert_eq!(a, b, "another current directory gives other bytes");
    assert_eq!(a, c, "the dir and its tsconfig.json give other bytes");
    assert_eq!(
        table(&hints(&a)),
        [
            ("src/bad.ts", true),
            ("src/bom.ts", false),
            ("src/ok.ts", false)
        ]
    );
}

#[test]
fn verify_reads_the_file_back() {
    let dir = scratch("verify");
    let out = dir.join("f.ep9h");
    let bytes = check(&dir, &fixture(), &out);
    let o = run(&dir, &["--verify", out.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = String::from_utf8(o.stdout).unwrap();
    assert!(
        text.starts_with("ok: schema 1 fact-schema 2 tsrs "),
        "{text}"
    );
    assert!(
        text.contains("files 3 (type errors 1), classes 0, records 0"),
        "{text}"
    );
    assert!(
        text.contains("  src/bad.ts has_type_errors=1 sha256="),
        "{text}"
    );
    // A trailing byte is dropped.
    let mut bad = bytes;
    bad.push(0);
    std::fs::write(&out, &bad).unwrap();
    let o = run(&dir, &["--verify", out.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("dropped: trailing-bytes"));
}

#[test]
fn a_syntax_error_does_not_hide_the_type_errors_of_other_files() {
    // get_diagnostics_of_any_program skips every semantic check after a syntax error; the
    // oracle does not (src/oracle.rs), so bad.ts still has its type error.
    let dir = scratch("syntax");
    let p = fixture_copy(&dir);
    std::fs::write(p.join("src/syntax.ts"), "export const = ;\n").unwrap();
    let h = hints(&check(&dir, &p, &dir.join("out.ep9h")));
    assert_eq!(
        table(&h),
        [
            ("src/bad.ts", true),
            ("src/bom.ts", false),
            ("src/ok.ts", false),
            ("src/syntax.ts", true),
            ("src/utf16.ts", false),
        ]
    );
}

#[test]
fn an_error_in_no_table_file_marks_every_file() {
    // A types entry that does not exist (TS2688) names no source file of the table.
    let dir = scratch("global");
    let p = fixture_copy(&dir);
    let config = std::fs::read_to_string(p.join("tsconfig.json")).unwrap();
    let config = config.replace("\"types\": []", "\"types\": [\"no-such-types\"]");
    std::fs::write(p.join("tsconfig.json"), config).unwrap();
    let o = run(&dir, &[p.to_str().unwrap(), "--out", "out.ep9h"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("TS2688"));
    let h = hints(&std::fs::read(dir.join("out.ep9h")).unwrap());
    assert_eq!(h.files.len(), 4);
    assert!(h.files.iter().all(|f| f.has_type_errors), "{:?}", table(&h));
}

#[test]
fn bad_arguments_and_a_missing_config() {
    let dir = scratch("args");
    for args in [
        &[][..],
        &["p"],
        &["--out", "x"],
        &["--verify"],
        &["a", "b", "c"],
    ] {
        assert_eq!(run(&dir, args).status.code(), Some(2), "{args:?}");
    }
    let o = run(&dir, &["no-such-dir", "--out", "x.ep9h"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!dir.join("x.ep9h").exists());
    let o = run(&dir, &["--verify", "no-such-file.ep9h"]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn out_never_replaces_an_input() {
    let dir = scratch("out-input");
    let p = fixture_copy(&dir);
    let ok = p.join("src/ok.ts");
    let config = p.join("tsconfig.json");
    let ok_bytes = std::fs::read(&ok).unwrap();
    let config_bytes = std::fs::read(&config).unwrap();
    let names = |d: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    let src_names = names(&p.join("src"));
    let refused_with = |cwd: &Path, project: &str, out: &str, message: &str| {
        let o = run(cwd, &[project, "--out", out]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "--out {out}: {err}");
        assert!(err.contains(message), "--out {out}: {err}");
    };
    // A known input (Checked.inputs).
    let refused = |cwd: &Path, project: &str, out: &str| {
        refused_with(cwd, project, out, "is an input of the project");
    };
    // Any other file that exists and is not a .ep9h file.
    let not_ep9h = |cwd: &Path, project: &str, out: &str| {
        refused_with(cwd, project, out, "exists and is not a .ep9h file");
    };
    let pp = p.to_str().unwrap();
    // A source file, the config, a source file by another path form, a relative path.
    refused(&dir, pp, ok.to_str().unwrap());
    refused(&dir, pp, config.to_str().unwrap());
    refused(&dir, pp, p.join("src/../src/ok.ts").to_str().unwrap());
    refused(&p, "tsconfig.json", "src/ok.ts");
    #[cfg(unix)]
    {
        // A link to a source file.
        let link = dir.join("link.ts");
        std::os::unix::fs::symlink(&ok, &link).unwrap();
        refused(&dir, pp, link.to_str().unwrap());
        let hard = dir.join("hard.ts");
        std::fs::hard_link(&ok, &hard).unwrap();
        refused(&dir, pp, hard.to_str().unwrap());
    }
    assert_eq!(std::fs::read(&ok).unwrap(), ok_bytes);
    assert_eq!(std::fs::read(&config).unwrap(), config_bytes);
    // No temporary file is left.
    assert_eq!(names(&p.join("src")), src_names);
    // A config that "extends" names: one step (tsconfig.json -> base.json), and a chain of 2
    // steps (tsconfig.json -> cfg/mid.json -> base.json). Each one is an input.
    let one = dir.join("one");
    copy_dir(&fixture(), &one);
    std::fs::write(one.join("base.json"), &config_bytes).unwrap();
    std::fs::write(one.join("tsconfig.json"), r#"{"extends":"./base.json"}"#).unwrap();
    let chain = dir.join("chain");
    copy_dir(&fixture(), &chain);
    std::fs::write(chain.join("base.json"), &config_bytes).unwrap();
    std::fs::create_dir_all(chain.join("cfg")).unwrap();
    std::fs::write(chain.join("cfg/mid.json"), r#"{"extends":"../base.json"}"#).unwrap();
    std::fs::write(
        chain.join("tsconfig.json"),
        r#"{"extends":"./cfg/mid.json"}"#,
    )
    .unwrap();
    let mid_bytes = std::fs::read(chain.join("cfg/mid.json")).unwrap();
    refused(
        &dir,
        one.to_str().unwrap(),
        one.join("base.json").to_str().unwrap(),
    );
    refused(&one, "tsconfig.json", "base.json");
    let cp = chain.to_str().unwrap();
    refused(&dir, cp, chain.join("base.json").to_str().unwrap());
    refused(&dir, cp, chain.join("cfg/mid.json").to_str().unwrap());
    refused(&chain, "tsconfig.json", "cfg/../base.json");
    assert_eq!(std::fs::read(one.join("base.json")).unwrap(), config_bytes);
    assert_eq!(
        std::fs::read(chain.join("base.json")).unwrap(),
        config_bytes
    );
    assert_eq!(
        std::fs::read(chain.join("cfg/mid.json")).unwrap(),
        mid_bytes
    );
    // The chain is read: the fixture's flags (hints checks them) and file table.
    let h = hints(&check(&dir, &chain, &dir.join("chain.ep9h")));
    assert_eq!(
        table(&h),
        [
            ("src/bad.ts", true),
            ("src/bom.ts", false),
            ("src/ok.ts", false)
        ]
    );
    // Files that the check reads but that are not in Checked.inputs: a config named by
    // "references" and a config that it extends; a package.json that module resolution reads.
    let refs = dir.join("refs");
    copy_dir(&fixture(), &refs);
    let with_refs = String::from_utf8(config_bytes.clone()).unwrap().replace(
        "\"include\": [\"src\"]",
        "\"include\": [\"src\"],\n  \"references\": [{ \"path\": \"./ref\" }]",
    );
    assert!(with_refs.contains("references"));
    std::fs::write(refs.join("tsconfig.json"), with_refs).unwrap();
    std::fs::create_dir_all(refs.join("ref")).unwrap();
    let ref_config =
        br#"{"extends":"./base.json","compilerOptions":{"composite":true},"files":[]}"#;
    std::fs::write(refs.join("ref/tsconfig.json"), ref_config).unwrap();
    std::fs::write(refs.join("ref/base.json"), b"{}").unwrap();
    let rp = refs.to_str().unwrap();
    not_ep9h(&dir, rp, refs.join("ref/tsconfig.json").to_str().unwrap());
    not_ep9h(&dir, rp, refs.join("ref/base.json").to_str().unwrap());
    not_ep9h(&refs, "tsconfig.json", "ref/tsconfig.json");
    assert_eq!(
        std::fs::read(refs.join("ref/tsconfig.json")).unwrap(),
        ref_config
    );
    assert_eq!(std::fs::read(refs.join("ref/base.json")).unwrap(), b"{}");
    let pkg = dir.join("pkg");
    copy_dir(&fixture(), &pkg);
    std::fs::write(
        pkg.join("src/usepkg.ts"),
        "import { v } from \"pkg\";\nexport const w: number = v;\n",
    )
    .unwrap();
    std::fs::create_dir_all(pkg.join("node_modules/pkg/lib")).unwrap();
    let pkg_json = br#"{"name":"pkg","version":"1.0.0","types":"lib/main.d.ts"}"#;
    let pkg_json_path = pkg.join("node_modules/pkg/package.json");
    std::fs::write(&pkg_json_path, pkg_json).unwrap();
    std::fs::write(
        pkg.join("node_modules/pkg/lib/main.d.ts"),
        "export declare const v: number;\n",
    )
    .unwrap();
    not_ep9h(&dir, pkg.to_str().unwrap(), pkg_json_path.to_str().unwrap());
    not_ep9h(&pkg, "tsconfig.json", "node_modules/pkg/package.json");
    assert_eq!(std::fs::read(&pkg_json_path).unwrap(), pkg_json);
    // The import resolves only through that package.json ("types" names lib/main.d.ts):
    // usepkg.ts has no type error.
    let h = hints(&check(&dir, &pkg, &dir.join("pkg.ep9h")));
    assert_eq!(
        table(&h),
        [
            ("node_modules/pkg/lib/main.d.ts", false),
            ("src/bad.ts", true),
            ("src/bom.ts", false),
            ("src/ok.ts", false),
            ("src/usepkg.ts", false),
        ]
    );
    // An existing file that is no input and not a .ep9h file.
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, b"old").unwrap();
    not_ep9h(&dir, pp, notes.to_str().unwrap());
    assert_eq!(std::fs::read(&notes).unwrap(), b"old");
    // An output of an earlier run is replaced (new bytes), and no temporary file is left.
    let out = dir.join("out.ep9h");
    let earlier = check(&dir, &fixture(), &out);
    #[cfg(unix)]
    {
        // A link to a .ep9h file is not a regular file.
        let link = dir.join("link.ep9h");
        std::os::unix::fs::symlink(&out, &link).unwrap();
        not_ep9h(&dir, pp, link.to_str().unwrap());
        assert_eq!(std::fs::read(&out).unwrap(), earlier);
        std::fs::remove_file(&link).unwrap();
    }
    let before = names(&dir);
    let bytes = check(&dir, &p, &out);
    assert_ne!(bytes, earlier);
    assert_eq!(table(&hints(&bytes)).len(), 4);
    assert_eq!(names(&dir), before);
}
