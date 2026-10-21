//! Fact-schema version guard (H16), the same rule as the bytecode ABI history (H02).
//!
//! The schema fingerprint is SHA-256 over [`schema_text`] (header, types, the kind → payload
//! table, every enum value and bit, the rules) and the bytes of the sample snapshot (which
//! uses every kind, tag and enum value). `tests/golden/schema-history.txt` has one line per
//! schema version: `<FACT_SCHEMA_VERSION> <fingerprint>`.
//! - The current (version, fingerprint) must be the last line.
//! - A version never maps to two fingerprints. So a change to the slot-kind list (an
//!   `ep9_bytecode` ABI change) or to the fact format fails here until FACT_SCHEMA_VERSION is
//!   bumped, even after `schema.txt` and `facts-v0.hex` are re-blessed.
//!
//! `EP9_BLESS=1` rewrites `schema.txt` and appends a line for a new version; it never rewrites
//! an existing history line.

mod common;

use common::*;
use ep9_bytecode::{FeedbackSlotKind, sha256};
use ep9_facts::*;

fn fingerprint_of(schema: &str, sample: &[u8]) -> String {
    let mut h = sha256::Sha256::new();
    h.update(schema.as_bytes());
    h.update(b"\n");
    h.update(sample);
    sha256::to_hex(&h.finish())
}

fn fingerprint() -> String {
    fingerprint_of(&schema_text(), &sample().encode())
}

/// `Ok(true)` when the last history line is (`version`, `fingerprint`), `Ok(false)` when
/// `version` is new (a bless appends it), and `Err` when the schema changed under a version
/// the history already has, or the file is malformed.
fn history_state(history: &str, version: u32, fingerprint: &str) -> Result<bool, String> {
    let mut entries: Vec<(u32, String)> = Vec::new();
    for line in history
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut parts = line.split_whitespace();
        let v: u32 = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("bad history line {line:?}"))?;
        let fp = parts
            .next()
            .ok_or_else(|| format!("bad history line {line:?}"))?;
        if let Some(&(last, _)) = entries.last()
            && v <= last
        {
            return Err(format!(
                "schema version {v} follows {last} in schema-history.txt"
            ));
        }
        entries.push((v, fp.to_string()));
    }
    match entries.iter().position(|(v, _)| *v == version) {
        Some(i) if entries[i].1 != fingerprint => Err(format!(
            "the fact schema changed (fingerprint {fingerprint}) but FACT_SCHEMA_VERSION is \
             still {version}. Bump FACT_SCHEMA_VERSION in crates/ep9_facts/src/lib.rs, re-bless \
             the golden files, and record the change in docs/adr/H16-ics-and-feedback.md."
        )),
        Some(i) if i + 1 != entries.len() => Err(format!(
            "FACT_SCHEMA_VERSION {version} is not the newest version in schema-history.txt"
        )),
        Some(_) => Ok(true),
        None => match entries.last() {
            Some(&(last, _)) if version < last => Err(format!(
                "FACT_SCHEMA_VERSION {version} is older than {last} in schema-history.txt"
            )),
            _ => Ok(false),
        },
    }
}

#[test]
fn schema_text_matches_the_golden_file() {
    check_golden("schema.txt", &schema_text());
}

#[test]
fn schema_history_pins_the_schema() {
    let fp = fingerprint();
    let text = read_golden("schema-history.txt");
    match history_state(&text, FACT_SCHEMA_VERSION, &fp) {
        Ok(true) => {}
        Ok(false) if blessing() => {
            let mut out = if text.is_empty() {
                "# ep9 FactSnapshot schema history: <FACT_SCHEMA_VERSION> <SHA-256 of schema.txt \
                 + \\n + the sample snapshot bytes (facts-v0.hex)>\n"
                    .to_string()
            } else {
                text
            };
            out.push_str(&format!("{FACT_SCHEMA_VERSION} {fp}\n"));
            write_golden("schema-history.txt", &out);
        }
        Ok(false) => panic!(
            "FACT_SCHEMA_VERSION {FACT_SCHEMA_VERSION} is not in schema-history.txt; run with \
             EP9_BLESS=1"
        ),
        Err(e) => panic!("{e}"),
    }
}

/// The schema text lists every slot kind with its byte, name and payload, so a kind-list
/// change in `ep9_bytecode` changes the fingerprint.
#[test]
fn schema_text_lists_every_kind() {
    let text = schema_text();
    for &k in FeedbackSlotKind::ALL {
        let line = format!("\n  {} {} {}\n", k as u8, k.name(), Payload::of(k).name());
        assert!(text.contains(&line), "missing kind line {line:?}");
        assert_eq!(SlotFacts::empty(k).kind(), k);
    }
    assert_eq!(FeedbackSlotKind::ALL.len(), 21, "H16 v0 has 21 kinds");
}

#[test]
fn a_kind_list_change_without_a_version_bump_fails_the_history_check() {
    // An in-memory history like the golden file, so a bless run that rewrites the file
    // cannot race this test.
    let schema = schema_text();
    let sample = sample().encode();
    let fp = fingerprint_of(&schema, &sample);
    let history = format!("# history\n{FACT_SCHEMA_VERSION} {fp}\n");
    assert_eq!(history_state(&history, FACT_SCHEMA_VERSION, &fp), Ok(true));

    // The kind list as a later ABI might have it: one kind renamed, one kind added.
    let renamed = schema.replace("\n  13 TypeOf TypeSet\n", "\n  13 TypeOfValue TypeSet\n");
    let added = schema.replace(
        "\n  20 CreateClosure ClosureFacts\n",
        "\n  20 CreateClosure ClosureFacts\n  21 CallWithFeedback CallFacts\n",
    );
    for changed in [renamed, added] {
        assert_ne!(changed, schema);
        let changed_fp = fingerprint_of(&changed, &sample);
        let e = history_state(&history, FACT_SCHEMA_VERSION, &changed_fp).unwrap_err();
        assert!(e.contains("Bump FACT_SCHEMA_VERSION"), "{e}");
        // With a bump, the new version is accepted (a bless appends it).
        assert_eq!(
            history_state(&history, FACT_SCHEMA_VERSION + 1, &changed_fp),
            Ok(false)
        );
    }

    // An encoder change with the same text (one sample byte differs) fails the same way.
    let mut other_bytes = sample.clone();
    *other_bytes.last_mut().unwrap() ^= 1;
    let e = history_state(
        &history,
        FACT_SCHEMA_VERSION,
        &fingerprint_of(&schema, &other_bytes),
    )
    .unwrap_err();
    assert!(e.contains("Bump FACT_SCHEMA_VERSION"), "{e}");

    // Going back to an old version, or rewriting the order, is refused.
    let two = format!("{history}{} aa\n", FACT_SCHEMA_VERSION + 1);
    assert!(history_state(&two, FACT_SCHEMA_VERSION, &fp).is_err());
    assert!(history_state("2 aa\n1 bb\n", 2, "aa").is_err());
    assert!(history_state("1 aa\n1 aa\n", 1, "aa").is_err());
}

#[test]
fn tag_tables_match_the_tags() {
    // Each TAGS table has one name per tag byte, and the names are distinct.
    for tags in [
        Access::TAGS,
        ProtoRef::TAGS,
        KeyRef::TAGS,
        FunctionRef::TAGS,
    ] {
        let mut names = tags.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), tags.len());
    }
    assert_eq!(Access::Generic.tag() as usize, Access::TAGS.len() - 1);
    assert_eq!(ProtoRef::Unknown.tag() as usize, ProtoRef::TAGS.len() - 1);
    assert_eq!(KeyRef::OtherSymbol.tag() as usize, KeyRef::TAGS.len() - 1);
    assert_eq!(
        FunctionRef::Builtin(0).tag() as usize,
        FunctionRef::TAGS.len() - 1
    );
}
