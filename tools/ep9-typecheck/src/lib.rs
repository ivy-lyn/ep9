//! ep9-typecheck v0: the tsrs type oracle (H37 A1h items 1 and 7; `docs/plan/05-aot.md`
//! section 6, steps 1-4 and 6).
//!
//! ```text
//!  tsconfig.json (or its dir)
//!   1 config   tsoptions::get_parsed_command_line_of_config_file
//!   2 host     new_cached_fs_compiler_host + the bundled lib .d.ts files
//!   3 program  new_program (one checker)
//!   4 check    syntactic, program, bind, global and semantic diagnostics of every file
//!      ──► per file: path, SHA-256 of the decoded UTF-8 text, has_type_errors   (oracle)
//!   6 write    ep9_hints::write: header + file table, no records in v0          (this module)
//!  --verify    ep9_hints::read of the file, with the header's own TS flags      (this module)
//! ```
//!
//! Step 5 (the type queries that make the records) is not in v0: route A1H-TYPECHECK-QUERIES.
//!
//! Rules:
//! - The text hash is the SHA-256 of the UTF-8 bytes of the text that tsrs parsed: tsrs decodes
//!   the file (a UTF-8 BOM stripped, UTF-16 transcoded to UTF-8, bad UTF-8 as U+FFFD;
//!   `tsrs_vfs::internal::decode_bytes`), so this is the H37-Q3 hash "of the decoded UTF-8 text
//!   after the BOM strip". It is never a hash of the file bytes, a Latin-1 or a UTF-16 form.
//! - A path in the file table is relative to the directory of the tsconfig file, with `/`
//!   (for example `src/a.ts`). The bundled default lib files are not in the table.
//! - The same input gives the same bytes: the table is sorted by path and holds nothing that
//!   depends on the time, the host or the run.

pub mod oracle;

use ep9_hints::{
    Expect, FileEntry, Header, Hints, TextHash, TsFlags, TsrsCommit, offsets, parse_commit_hex,
};
use sha2::{Digest, Sha256};

/// The `ep9_facts::FACT_SCHEMA_VERSION` that the header names. ep9-typecheck cannot depend on
/// `ep9_facts` (H26:42 allows only `ep9_hints`), so a test reads the constant from its source.
pub const FACT_SCHEMA_VERSION: u32 = 2;

/// The tsrs pin that this binary is built from (`third_party/tsrs.commit`).
const TSRS_COMMIT_TEXT: &str = include_str!("../../../third_party/tsrs.commit");

/// The stamp that `sync_dep` wrote at the last tsrs sync: `<commit> <patch hash>`
/// (`scripts/sync-lib.sh`). rustc tracks the file, so a sync makes cargo build again.
const TSRS_STAMP: &str = include_str!("../../../third_party/tsrs/.ep9-stamp");

/// The first 40 bytes (the commit hex) of `a` and `b` are the same.
const fn same_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() < 40 || b.len() < 40 {
        return false;
    }
    let mut i = 0;
    while i < 40 {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

// The header names the commit of third_party/tsrs.commit, so the build must be of that commit:
// after a pin change with no sync, the path dependencies would still be the old checkout.
const _: () = assert!(
    same_commit(TSRS_STAMP, TSRS_COMMIT_TEXT),
    "third_party/tsrs is not synced to third_party/tsrs.commit: run bash scripts/sync-tsrs.sh"
);

/// The tsrs commit of the header, or `None` when `third_party/tsrs.commit` is not 40 hex digits.
pub fn tsrs_commit() -> Option<TsrsCommit> {
    parse_commit_hex(TSRS_COMMIT_TEXT)
}

/// The H37-Q3 text hash: the SHA-256 of the UTF-8 bytes of the decoded text.
pub fn text_hash(decoded: &str) -> TextHash {
    Sha256::digest(decoded.as_bytes()).into()
}

/// What the check gives for one source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFacts {
    /// The path relative to the tsconfig directory, with `/`.
    pub path: String,
    /// [`text_hash`] of the decoded text.
    pub text_hash: TextHash,
    /// tsrs reported an error in this file, or an error that names no file of the table.
    pub has_type_errors: bool,
}

/// The v0 hints: the header and the file table (sorted by path), no classes, fields or records.
/// Two files with one path give an error.
pub fn hints_v0(
    mut files: Vec<FileFacts>,
    ts_flags: TsFlags,
    tsrs_commit: TsrsCommit,
) -> Result<Hints, String> {
    files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    if let Some(w) = files.windows(2).find(|w| w[0].path == w[1].path) {
        return Err(format!("two source files have the path {}", w[0].path));
    }
    Ok(Hints {
        header: Header {
            tsrs_commit,
            fact_schema_version: FACT_SCHEMA_VERSION,
            ts_flags,
        },
        files: files
            .into_iter()
            .map(|f| FileEntry {
                path: f.path,
                text_hash: f.text_hash,
                has_type_errors: f.has_type_errors,
                first_record: 0,
                record_count: 0,
            })
            .collect(),
        classes: Vec::new(),
        fields: Vec::new(),
        records: Vec::new(),
    })
}

/// The bytes of the v0 `.ep9h` file.
pub fn write_v0(files: Vec<FileFacts>, ts_flags: TsFlags) -> Result<Vec<u8>, String> {
    let commit = tsrs_commit().ok_or("third_party/tsrs.commit is not 40 hex digits")?;
    let hints = hints_v0(files, ts_flags, commit)?;
    ep9_hints::write(&hints).map_err(|e| format!("ep9_hints::write: {e}"))
}

/// Reads a `.ep9h` file back (`--verify`): the reader of `ep9_hints` with
/// [`FACT_SCHEMA_VERSION`] and the TS flags of the file's own header, then the tsrs commit of
/// the header must be this binary's pin.
pub fn verify(bytes: &[u8]) -> Result<Hints, String> {
    let flags = bytes
        .get(offsets::TS_FLAGS..offsets::TS_FLAGS + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .and_then(TsFlags::from_bits)
        .ok_or("the header has no valid TS flags")?;
    let expect = Expect {
        fact_schema_version: FACT_SCHEMA_VERSION,
        ts_flags: flags,
    };
    let hints = ep9_hints::read(bytes, &expect).map_err(|r| format!("dropped: {r}"))?;
    if Some(hints.header.tsrs_commit) != tsrs_commit() {
        return Err(format!(
            "the header names tsrs {}, not the pin {}",
            ep9_hints::commit_hex(&hints.header.tsrs_commit),
            TSRS_COMMIT_TEXT.trim()
        ));
    }
    Ok(hints)
}

/// The lines that `--verify` prints: one for the header, one per file.
pub fn summary(hints: &Hints) -> String {
    let f = hints.header.ts_flags;
    let mut out = format!(
        "ok: schema {} fact-schema {} tsrs {} flags useDefineForClassFields={} \
         experimentalDecorators={} emitDecoratorMetadata={}; files {} (type errors {}), \
         classes {}, records {}\n",
        ep9_hints::HINT_SCHEMA_VERSION,
        hints.header.fact_schema_version,
        ep9_hints::commit_hex(&hints.header.tsrs_commit),
        f.use_define_for_class_fields,
        f.experimental_decorators,
        f.emit_decorator_metadata,
        hints.files.len(),
        hints.files.iter().filter(|f| f.has_type_errors).count(),
        hints.classes.len(),
        hints.records.len(),
    );
    for file in &hints.files {
        let hex: String = file.text_hash.iter().map(|b| format!("{b:02x}")).collect();
        out.push_str(&format!(
            "  {} has_type_errors={} sha256={hex}\n",
            file.path, file.has_type_errors as u8
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(path: &str, seed: u8, errors: bool) -> FileFacts {
        FileFacts {
            path: path.into(),
            text_hash: [seed; 32],
            has_type_errors: errors,
        }
    }

    #[test]
    fn fact_schema_version_is_the_ep9_facts_one() {
        let src = include_str!("../../../crates/ep9_facts/src/lib.rs");
        let v = src
            .lines()
            .find_map(|l| l.strip_prefix("pub const FACT_SCHEMA_VERSION: u32 = "))
            .and_then(|r| r.strip_suffix(';'))
            .expect("ep9_facts has FACT_SCHEMA_VERSION");
        assert_eq!(v.parse::<u32>().unwrap(), FACT_SCHEMA_VERSION);
    }

    #[test]
    fn tsrs_commit_is_the_pin() {
        let c = tsrs_commit().expect("third_party/tsrs.commit parses");
        assert_eq!(ep9_hints::commit_hex(&c), TSRS_COMMIT_TEXT.trim());
    }

    #[test]
    fn the_build_is_of_the_pin() {
        assert!(same_commit(TSRS_STAMP, TSRS_COMMIT_TEXT));
        let pin = TSRS_COMMIT_TEXT.trim();
        assert!(same_commit(&format!("{pin} e69de29b"), pin));
        // Another commit, a short stamp or an empty stamp (a sync that did not end).
        let mut other = pin.to_owned().into_bytes();
        other[39] = if other[39] == b'0' { b'1' } else { b'0' };
        assert!(!same_commit(std::str::from_utf8(&other).unwrap(), pin));
        assert!(!same_commit(&pin[..39], pin));
        assert!(!same_commit("", pin));
    }

    #[test]
    fn text_hash_is_sha256_of_the_utf8_bytes() {
        // FIPS 180-2 test vector "abc".
        let abc: String = text_hash("abc")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            abc,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // U+00E9 is 2 UTF-8 bytes (c3 a9), not the Latin-1 byte e9 or the UTF-16 unit.
        assert_eq!(
            text_hash("\u{e9}"),
            <[u8; 32]>::from(Sha256::digest([0xc3, 0xa9]))
        );
        assert_ne!(
            text_hash("\u{e9}"),
            <[u8; 32]>::from(Sha256::digest([0xe9]))
        );
        assert_ne!(
            text_hash("\u{e9}"),
            <[u8; 32]>::from(Sha256::digest([0xe9, 0x00]))
        );
    }

    #[test]
    fn hints_v0_sorts_by_path_and_has_no_records() {
        let flags = TsFlags {
            use_define_for_class_fields: true,
            experimental_decorators: false,
            emit_decorator_metadata: true,
        };
        let commit = [7u8; 20];
        let files = vec![
            facts("src/b.ts", 2, true),
            facts("a.ts", 1, false),
            facts("src/a.ts", 3, false),
        ];
        let h = hints_v0(files, flags, commit).unwrap();
        let paths: Vec<&str> = h.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.ts", "src/a.ts", "src/b.ts"]);
        assert!(h.records.is_empty() && h.classes.is_empty() && h.fields.is_empty());
        assert!(
            h.files
                .iter()
                .all(|f| f.record_count == 0 && f.first_record == 0)
        );
        assert_eq!(h.header.fact_schema_version, FACT_SCHEMA_VERSION);
        let bytes = ep9_hints::write(&h).unwrap();
        let expect = Expect {
            fact_schema_version: FACT_SCHEMA_VERSION,
            ts_flags: flags,
        };
        assert_eq!(ep9_hints::read(&bytes, &expect), Ok(h));
    }

    #[test]
    fn hints_v0_refuses_a_path_twice() {
        let files = vec![facts("a.ts", 1, false), facts("a.ts", 2, false)];
        assert!(hints_v0(files, TsFlags::default(), [0; 20]).is_err());
    }

    #[test]
    fn verify_reads_back_and_checks_the_pin() {
        let flags = TsFlags {
            use_define_for_class_fields: false,
            experimental_decorators: true,
            emit_decorator_metadata: false,
        };
        let bytes = write_v0(vec![facts("m.ts", 5, false)], flags).unwrap();
        let h = verify(&bytes).unwrap();
        assert_eq!(h.header.ts_flags, flags);
        assert!(summary(&h).starts_with("ok: schema 1 fact-schema 2 tsrs "));
        // Another tsrs commit in the header.
        let mut other = bytes.clone();
        other[offsets::TSRS_COMMIT] ^= 1;
        assert!(verify(&other).unwrap_err().contains("not the pin"));
        // A damaged file and a short file are dropped.
        let mut bad = bytes.clone();
        bad[offsets::HEADER_RESERVED] = 1;
        assert_eq!(verify(&bad).unwrap_err(), "dropped: bad-header");
        assert!(verify(&bytes[..10]).is_err());
    }
}
