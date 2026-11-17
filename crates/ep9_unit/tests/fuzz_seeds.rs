//! The seeds of the `unit_loader` fuzz target (route MC-w41-8 part .1): this test writes
//! `fuzz/seeds/unit_loader/*` with `EP9_BLESS=1` and, without it, fails when a committed seed
//! would stop at the H35 S4 checks of this build (a contract version moved, a seed is missing
//! or one is left over). It replaces the hand-run seed writer of each contract move.
//!
//! ```text
//! 6 units (objects, fuses, stale, classes, loop, signed) x 3 target pairs (a64-macos,
//! x64-linux, a64-linux) = the 18 seeds <unit>-<pair>
//!   script --ep9_frontend--> bytecode section; Cx::unit_function_digest --functions::encode-->
//!   rodata (the function table); a 64-byte text section; the source section
//!   --TestUnit::encode--> container (engine build id: 16 zero bytes, so a rewrite of an
//!   unchanged seed gives the same bytes) --cut the zero bytes before the first section-->
//!   the fuzz input form (at most 4,096 bytes: the fuzz runs use -max_len 4096)
//! EP9_BLESS=1  write the 18 files
//! else         each committed file, as the fuzz target reads it: put the zero bytes back,
//!              write this build's engine build id and the content hash
//!                -> its versions are Versions::current()
//!                -> check(bytes, this host with the seed's target pair) == Load
//!                -> the bytecode section decodes and verifies; the function table decodes
//!                -> each table digest is this build's Cx::unit_function_digest of its
//!                   function (in `stale`, Point has the digest of leaf)
//! ```
//!
//! What each unit is for: `objects` three functions with jitless, jit and hybrid variants and
//! one that needs a CPU bit; `fuses` the same with fuse-id lists (one of them id 0xffff);
//! `stale` a function with the digest of another (its code set does not match: it runs its
//! bytecode);
//! `classes` class constructors; `loop` a run that the target's watchdog ends; `signed` a
//! signature section. Only the target pair differs among the 3 seeds of a unit.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::{TestUnit, blessing, header_for};
use ep9_bytecode::{CompiledUnit, UnitString, VerifiedUnit};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_unit::functions::{self, Entry, NO_META, Variant, VariantKind, lowering};
use ep9_unit::{
    ENGINE_BUILD_ID, HostInfo, Outcome, SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET, SectionKind,
    UnitHeader, Versions, check, entry_offsets, header_flags, offsets, target_abi, target_arch,
    unit_section,
};
use ep9_vm::{ExecOptions, Isolate, IsolateConfig};

const OBJECTS: &str = r#"function Point(x, y) { this.x = x; this.y = y; }
function leaf(i) { return { a: [0, 1, 2], tag: "leaf " + i }; }
function grow(n) { var a = [1, 2, 3]; if (n % 3 == 0) a[1] = 1.5; return a; }
var sum = 0;
for (var i = 0; i < 20; i++) {
  var p = new Point(i, i + 1);
  sum += p.x + p.y + leaf(i).a.length + grow(i).length;
}
print(sum);
"#;

const CLASSES: &str = r#"class B { constructor(x) { this.x = x; } }
class D extends B { constructor(x) { super(x); this.y = 2; } }
class E extends B { z = 3; }
class F extends B { constructor() { return 1; } }
var d = new D(1), e = new E(2), r = 0;
try { B(); } catch (err) { r = 1; }
try { new F(); } catch (err) { r += 2; }
print(d.x + d.y + e.x + e.z + r);
"#;

const LOOP: &str = "function spin() { var n = 0; for (;;) { n++; } }\nspin();\n";

/// The target pairs of the seeds: (file name suffix, target_arch, target_abi).
const PAIRS: [(&str, u8, u8); 3] = [
    ("a64-macos", target_arch::AARCH64, target_abi::APPLE),
    ("x64-linux", target_arch::X86_64, target_abi::LINUX),
    ("a64-linux", target_arch::AARCH64, target_abi::LINUX),
];

/// The libFuzzer -max_len of the fuzz runs (`fuzz/scripts/run.sh`, FUZZ_MAX_LEN).
const MAX_LEN: usize = 4096;

/// (function name, its variants, the function whose digest it gets instead of its own)
type Spec = (&'static str, Vec<Variant>, Option<&'static str>);

fn seed_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/unit_loader")
}

fn compile(src: &str) -> VerifiedUnit {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the seed script compiles")
}

/// The index of the function named `name` in `cu`.
fn index(cu: &CompiledUnit, name: &str) -> u32 {
    cu.functions
        .iter()
        .position(|f| {
            f.name.is_some_and(|id| {
                matches!(&cu.strings[id.index()], UnitString::Latin1(b) if **b == *name.as_bytes())
            })
        })
        .unwrap_or_else(|| panic!("no function {name}")) as u32
}

fn var(kind: VariantKind, low: u8, entry: u32, cpu: u64) -> Variant {
    Variant {
        kind,
        lowering: low,
        tag: 0,
        entry,
        text_len: 8,
        meta: NO_META,
        meta_len: 0,
        fuse_refs: 0,
        fuse_count: 0,
        cpu_required: cpu,
    }
}

/// The unit of `src` with the function table of `specs` and the fuse-id lists `fuse_lists`
/// (entry, variant, ids), for this host's target pair (the caller sets the pair).
fn unit(
    iso: &mut Isolate,
    src: &str,
    specs: &[Spec],
    fuse_lists: &[(usize, usize, Vec<u32>)],
) -> TestUnit {
    let vu = compile(src);
    let cu = vu.unit();
    let mut entries: Vec<Entry> = specs
        .iter()
        .map(|(name, vars, from)| {
            let f = index(cu, name);
            let d = index(cu, from.unwrap_or(name));
            let digest = iso
                .with_cx(|cx| Ok(cx.unit_function_digest(&vu, d)))
                .expect("no throw")
                .expect("the function exists");
            Entry {
                function: f,
                digest,
                variants: vars.clone(),
            }
        })
        .collect();
    // The fuse-id lists go after the table.
    let table_len = functions::table_size(
        entries.len(),
        entries.iter().map(|e| e.variants.len()).sum(),
    );
    let mut tail = Vec::new();
    for (e, v, ids) in fuse_lists {
        let var = &mut entries[*e].variants[*v];
        var.fuse_refs = (table_len + tail.len()) as u32;
        var.fuse_count = ids.len() as u16;
        for id in ids {
            tail.extend_from_slice(&id.to_le_bytes());
        }
    }
    entries.sort_by_key(|e| e.function);
    let mut rodata = functions::encode(&entries);
    rodata.extend_from_slice(&tail);
    let mut header = header_for(&HostInfo::current(), 0);
    // The fuzz target writes this build's id; zero bytes keep a rewrite byte-equal.
    header.engine_build_id = [0; 16];
    header.source_flags = 0;
    TestUnit {
        header,
        sections: vec![
            (SectionKind::Text, vec![0xCC; 64]),
            (SectionKind::Rodata, rodata),
            (SectionKind::Bytecode, cu.to_bytes()),
            (SectionKind::Source, src.as_bytes().to_vec()),
        ],
        extensions: Vec::new(),
    }
}

/// The 6 seed units, by name.
fn units() -> Vec<(&'static str, TestUnit)> {
    use VariantKind::{AotBaseline as B, AotOpt as O};
    let j = lowering::JITLESS;
    let objects: Vec<Spec> = vec![
        ("Point", vec![var(B, j, 0, 0), var(B, j, 8, 1 << 62)], None),
        (
            "leaf",
            vec![var(B, lowering::JIT, 16, 0), var(B, j, 24, 0)],
            None,
        ),
        (
            "grow",
            vec![var(O, j, 32, 0), var(B, lowering::HYBRID, 40, 0)],
            None,
        ),
    ];
    let stale: Vec<Spec> = vec![
        ("Point", vec![var(B, j, 0, 0)], Some("leaf")),
        ("leaf", vec![var(B, j, 8, 0)], None),
    ];
    let classes: Vec<Spec> = vec![
        ("B", vec![var(B, j, 0, 0)], None),
        ("D", vec![var(B, j, 8, 0)], None),
        ("E", vec![var(B, j, 16, 0)], None),
        ("F", vec![var(B, j, 24, 0)], None),
    ];
    let spin: Vec<Spec> = vec![("spin", vec![var(B, j, 0, 0)], None)];
    let mut iso = Isolate::new_with(IsolateConfig::default(), ExecOptions::jitless())
        .expect("the jitless preset runs");
    let iso = &mut iso;
    vec![
        ("objects", unit(iso, OBJECTS, &objects, &[])),
        (
            "fuses",
            unit(
                iso,
                OBJECTS,
                &objects,
                &[(0, 0, vec![0]), (1, 1, vec![1, 2]), (2, 1, vec![0xffff])],
            ),
        ),
        ("stale", unit(iso, OBJECTS, &stale, &[])),
        ("classes", unit(iso, CLASSES, &classes, &[])),
        ("loop", unit(iso, LOOP, &spin, &[])),
        ("signed", {
            let mut u = unit(iso, OBJECTS, &objects, &[]);
            u.header.header_flags = header_flags::SIGNED;
            u.sections.push((SectionKind::Signature, vec![0x5a; 64]));
            u
        }),
    ]
}

/// (the end of the section table, the offset of the first section) of a container.
fn table_end_and_first(bytes: &[u8]) -> (usize, usize) {
    let count = u32::from_le_bytes(
        bytes[offsets::SECTION_COUNT..offsets::SECTION_COUNT + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    let at = SECTION_TABLE_OFFSET + entry_offsets::OFFSET;
    let first = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    (SECTION_TABLE_OFFSET + count * SECTION_ENTRY_SIZE, first)
}

/// The fuzz input form: the zero bytes between the section table and the first section cut.
fn compact(full: &[u8]) -> Vec<u8> {
    let (table_end, first) = table_end_and_first(full);
    assert!(full[table_end..first].iter().all(|&b| b == 0));
    let mut out = full[..table_end].to_vec();
    out.extend_from_slice(&full[first..]);
    out
}

/// The container of a seed: the zero bytes put back (the fuzz target's `padded`).
fn padded(data: &[u8]) -> Vec<u8> {
    let (table_end, first) = table_end_and_first(data);
    let mut out = data[..table_end].to_vec();
    out.resize(first, 0);
    out.extend_from_slice(&data[table_end..]);
    out
}

/// The 18 seeds, (file name, bytes in the fuzz input form).
fn seeds() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for (name, u) in &units() {
        for (pair, arch, abi) in PAIRS {
            let mut u = u.clone();
            u.header.target_arch = arch;
            u.header.target_abi = abi;
            let full = u.encode();
            let c = compact(&full);
            assert_eq!(padded(&c), full, "{name}-{pair}: the cut is not reversible");
            assert!(c.len() <= MAX_LEN, "{name}-{pair}: {} bytes", c.len());
            out.push((format!("{name}-{pair}"), c));
        }
    }
    out
}

/// What the fuzz target does before `check`: this build's engine build id and the content
/// hash (SHA-256 with the hash field and the signature section zero).
fn as_the_target_reads(seed: &[u8]) -> Vec<u8> {
    let mut bytes = padded(seed);
    let id = offsets::ENGINE_BUILD_ID;
    bytes[id..id + 16].copy_from_slice(&ENGINE_BUILD_ID);
    let mut copy = bytes.clone();
    copy[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].fill(0);
    if let Some(sig) = TestUnit::parse(&bytes)
        .expect("the seed parses")
        .sections
        .iter()
        .position(|(k, _)| *k == SectionKind::Signature)
    {
        let at = SECTION_TABLE_OFFSET + sig * SECTION_ENTRY_SIZE;
        let e = &bytes[at..at + SECTION_ENTRY_SIZE];
        let u64_at = |o: usize| u64::from_le_bytes(e[o..o + 8].try_into().unwrap()) as usize;
        let start = u64_at(entry_offsets::OFFSET);
        copy[start..start + u64_at(entry_offsets::SIZE)].fill(0);
    }
    let hash = ep9_bytecode::sha256::sha256(&copy);
    bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].copy_from_slice(&hash);
    bytes
}

#[test]
fn unit_loader_seeds_match_this_build() {
    let dir = seed_dir();
    let seeds = seeds();
    if blessing() {
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in &seeds {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        return;
    }
    let bless = "run `EP9_BLESS=1 cargo test -p ep9_unit --test fuzz_seeds` and commit \
                 fuzz/seeds/unit_loader";
    let want: BTreeSet<String> = seeds.iter().map(|(n, _)| n.clone()).collect();
    let have: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}; {bless}", dir.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        have, want,
        "the seed files differ from the writer's list; {bless}"
    );
    let current = Versions::current();
    let mut iso = Isolate::new_with(IsolateConfig::default(), ExecOptions::jitless())
        .expect("the jitless preset runs");
    for (name, _) in &seeds {
        let seed = std::fs::read(dir.join(name)).unwrap();
        assert!(seed.len() <= MAX_LEN, "{name}: {} bytes", seed.len());
        let bytes = as_the_target_reads(&seed);
        let header = UnitHeader::decode(&bytes).expect("the header decodes");
        assert_eq!(
            header.versions, current,
            "{name}: the seed's versions are not Versions::current(); {bless}"
        );
        let (_, arch, abi) = PAIRS
            .iter()
            .find(|(p, _, _)| name.ends_with(p))
            .expect("a known pair");
        assert_eq!(
            (header.target_arch, header.target_abi),
            (*arch, *abi),
            "{name}: target pair"
        );
        let host = HostInfo {
            target_arch: *arch,
            target_abi: *abi,
            ..HostInfo::current()
        };
        assert_eq!(
            check(&bytes, &host),
            Outcome::Load,
            "{name}: the seed stops at the H35 S4 checks on its own target; {bless}"
        );
        let code = unit_section(&bytes, SectionKind::Bytecode).expect("a bytecode section");
        let unit = ep9_bytecode::decode_unit(code).expect("the bytecode decodes");
        let vu = ep9_bytecode::verify(unit).expect("the bytecode verifies");
        let text = unit_section(&bytes, SectionKind::Text).unwrap_or(&[]);
        let rodata = unit_section(&bytes, SectionKind::Rodata).unwrap_or(&[]);
        let (entries, _) =
            functions::decode(rodata, text.len()).expect("the function table decodes");
        // Each entry's digest is this build's digest of its function (the stale unit: `Point`
        // has the digest of `leaf`). A field added to `Function` with no version change moves
        // every digest: the loader then runs only bytecode, and the seeds lose the variant
        // paths.
        let mut digest = |f: u32| {
            iso.with_cx(|cx| Ok(cx.unit_function_digest(&vu, f)))
                .expect("no throw")
                .unwrap_or_else(|| panic!("{name}: no function {f}"))
        };
        // (Point, leaf) of a stale seed: the entry of Point has the digest of leaf.
        let swap = name.starts_with("stale-").then(|| {
            let (point, leaf) = (index(vu.unit(), "Point"), index(vu.unit(), "leaf"));
            assert_ne!(
                digest(point),
                digest(leaf),
                "{name}: Point and leaf have one digest, so the stale entry is not stale"
            );
            (point, leaf)
        });
        for e in &entries {
            let from = match swap {
                Some((point, leaf)) if e.function == point => leaf,
                _ => e.function,
            };
            assert_eq!(
                e.digest,
                digest(from),
                "{name}: the digest of function {} is not this build's digest; {bless}",
                e.function
            );
        }
    }
}
