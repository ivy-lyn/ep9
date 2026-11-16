//! The `collator-search` mode (feature `datagen`; INTL-V2-D17 Form A, INTL-W4-CLDR): the search
//! collations (`-u-co-search`, `-u-co-searchjl`) that the ICU4X compiled data leaves out. It
//! reads them from the ICU export data that `icuexport.pin` names, with the ICU4X source
//! provider of the same release, and writes `crates/ep9_intl/src/cldr/gen/collator_search/mod.rs`.
//!
//! ```text
//! icuexport/<file> (sync-icuexport.sh; tag, file and sha256 in icuexport.pin)
//!   collation/implicithan/<locale>_search*_meta.toml ──► CollationMetadataV1 ─┐
//!   collation/implicithan/<locale>_search*_data.toml ──► CollationTailoringV1 ─┴► SEARCH_COLLATIONS
//!   (icu_provider_source 2.4.0: the data conversion of ICU4X datagen)
//! ```
//!
//! The output is plain Rust data (numbers and strings): `ep9_intl` is `#![forbid(unsafe_code)]`
//! and its extref-lint refuses `include!`, so the ICU4X baked form (`unsafe` ZeroVec and trie
//! constructors, `include!` of the marker files) cannot go there. `ep9_intl` builds the data
//! structs at run time with the checked constructors (`CodePointTrie::try_new`).
//!
//! The ICU export data must be the data of the ICU4X compiled data that the engine links (the
//! root collation and every tailoring of a shared id must be equal; [`read`] checks it). The
//! data has no search entry for the diacritics and the reordering markers ([`read`] checks it
//! too), so the locale-specific search data is the metadata and the tailoring.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use icu_collator::provider::{
    Baked, CollationDiacriticsV1, CollationJamoV1, CollationMetadataV1, CollationReorderingV1,
    CollationRootV1, CollationSpecialPrimariesV1, CollationTailoringV1,
};
use icu_provider::prelude::*;
use icu_provider_source::SourceDataProvider;
use serde_json::Value;

/// The generated file, relative to the output directory (`crates/ep9_intl/src/cldr/gen`).
pub const OUTPUT_FILE: &str = "collator_search/mod.rs";

/// The ICU export data release of `icuexport.pin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The release tag (`release-79.1rc`).
    pub tag: String,
    /// The file name of the release asset.
    pub file: String,
    /// The SHA-256 of the file (lowercase hex).
    pub sha256: String,
}

/// Reads `icuexport.pin`: one line `<tag> <file> <sha256>`; `#` lines are comments.
pub fn read_pin(pin_file: &Path) -> Result<Pin, String> {
    let text =
        std::fs::read_to_string(pin_file).map_err(|e| format!("{}: {e}", pin_file.display()))?;
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| format!("{}: no pin line", pin_file.display()))?;
    match line.split_whitespace().collect::<Vec<_>>()[..] {
        [tag, file, sha256] if sha256.len() == 64 => Ok(Pin {
            tag: tag.to_owned(),
            file: file.to_owned(),
            sha256: sha256.to_owned(),
        }),
        _ => Err(format!(
            "{}: the pin line is not \"<tag> <file> <sha256>\"",
            pin_file.display()
        )),
    }
}

/// The default paths: the pin file next to this crate and the directory that
/// `sync-icuexport.sh` fills (the zip is `<dir>/<pin.file>`).
pub fn default_paths() -> (PathBuf, PathBuf) {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    (here.join("icuexport.pin"), here.join("icuexport"))
}

/// The parts of a `CodePointTrie<u32>` (the `CodePointTrieHeader` fields, the index, the data).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trie {
    /// `high_start`.
    pub high_start: u32,
    /// `shifted12_high_start`.
    pub shifted12_high_start: u16,
    /// `index3_null_offset`.
    pub index3_null_offset: u16,
    /// `data_null_offset`.
    pub data_null_offset: u32,
    /// `null_value`.
    pub null_value: u32,
    /// `trie_type` is `Small` (else `Fast`).
    pub small: bool,
    /// The index array.
    pub index: Vec<u16>,
    /// The data array.
    pub data: Vec<u32>,
}

/// One search collation: a data id (locale and collation type) with its metadata and tailoring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCollation {
    /// The data locale (`und`, `de`, `sr-Latn`).
    pub locale: String,
    /// The collation type, the marker attributes (`search`, `searchjl`).
    pub collation: String,
    /// `CollationMetadata::bits`.
    pub metadata: u32,
    /// `CollationData::trie`.
    pub trie: Trie,
    /// `CollationData::ce32s`.
    pub ce32s: Vec<u32>,
    /// `CollationData::ces`.
    pub ces: Vec<u64>,
    /// `CollationData::contexts`.
    pub contexts: Vec<u16>,
}

fn err(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

/// The ids of marker `M` in the source data whose collation type starts with "search".
fn search_ids<M: DataMarker>(p: &SourceDataProvider) -> Result<Vec<(String, String)>, String>
where
    SourceDataProvider: IterableDataProvider<M>,
{
    let mut ids: Vec<(String, String)> = IterableDataProvider::<M>::iter_ids(p)
        .map_err(|e| err(M::INFO.id.name(), e))?
        .into_iter()
        .filter(|id| id.marker_attributes.as_str().starts_with("search"))
        .map(|id| (id.locale.to_string(), id.marker_attributes.to_string()))
        .collect();
    ids.sort();
    Ok(ids)
}

/// The source payload of `M` for `id`.
fn load<M: DataMarker>(
    p: &impl DataProvider<M>,
    locale: &DataLocale,
    attrs: &DataMarkerAttributes,
) -> Result<DataResponse<M>, String> {
    p.load(DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attrs, locale),
        ..Default::default()
    })
    .map_err(|e| err(M::INFO.id.name(), e))
}

/// Checks that the source data of the singleton marker `M` equals the compiled data.
fn same_singleton<M: DataMarker>(p: &SourceDataProvider) -> Result<(), String>
where
    SourceDataProvider: DataProvider<M>,
    Baked: DataProvider<M>,
    for<'a> <M::DataStruct as yoke::Yokeable<'a>>::Output: PartialEq,
{
    let source = load::<M>(p, &DataLocale::default(), Default::default())?;
    let compiled = load::<M>(&Baked, &DataLocale::default(), Default::default())?;
    if source.payload.get() == compiled.payload.get() {
        Ok(())
    } else {
        Err(format!(
            "{}: the ICU export data differs from the ICU4X compiled data (pin another release)",
            M::INFO.id.name()
        ))
    }
}

/// Checks the non-search ids of `M` that the compiled data holds as they are (no fallback):
/// the source data must be equal. Returns the count of compared ids.
fn same_tailorings<M: DataMarker>(p: &SourceDataProvider) -> Result<usize, String>
where
    SourceDataProvider: IterableDataProvider<M>,
    Baked: DataProvider<M>,
    for<'a> <M::DataStruct as yoke::Yokeable<'a>>::Output: PartialEq,
{
    let mut n = 0;
    for id in IterableDataProvider::<M>::iter_ids(p).map_err(|e| err(M::INFO.id.name(), e))? {
        if id.marker_attributes.as_str().starts_with("search") {
            continue;
        }
        let Ok(compiled) = load::<M>(&Baked, &id.locale, &id.marker_attributes) else {
            continue;
        };
        if compiled.metadata.locale.is_some() {
            continue;
        }
        let source = load::<M>(p, &id.locale, &id.marker_attributes)?;
        if source.payload.get() != compiled.payload.get() {
            return Err(format!(
                "{} {}/{}: the ICU export data differs from the ICU4X compiled data",
                M::INFO.id.name(),
                id.locale,
                id.marker_attributes.as_str()
            ));
        }
        n += 1;
    }
    Ok(n)
}

fn numbers<T: TryFrom<u64>>(v: &Value, what: &str) -> Result<Vec<T>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what}: not an array"))?
        .iter()
        .map(|x| {
            x.as_u64()
                .and_then(|x| T::try_from(x).ok())
                .ok_or_else(|| format!("{what}: {x} is out of range"))
        })
        .collect()
}

fn number<T: TryFrom<u64>>(v: &Value, key: &str) -> Result<T, String> {
    v[key]
        .as_u64()
        .and_then(|x| T::try_from(x).ok())
        .ok_or_else(|| format!("trie header {key}: {}", v[key]))
}

/// The parts of a trie, from its serde form (`header`, `index`, `data`).
fn trie(t: &icu_collator::provider::CollationData<'_>) -> Result<Trie, String> {
    let v = serde_json::to_value(&t.trie).map_err(|e| err("trie", e))?;
    let h = &v["header"];
    let small = match h["trie_type"].as_str() {
        Some("Small") => true,
        Some("Fast") => false,
        _ => return Err(format!("trie_type: {}", h["trie_type"])),
    };
    Ok(Trie {
        high_start: number(h, "high_start")?,
        shifted12_high_start: number(h, "shifted12_high_start")?,
        index3_null_offset: number(h, "index3_null_offset")?,
        data_null_offset: number(h, "data_null_offset")?,
        null_value: number(h, "null_value")?,
        small,
        index: numbers(&v["index"], "trie index")?,
        data: numbers(&v["data"], "trie data")?,
    })
}

/// Reads the search collations from the ICU export data `zip` of `pin`, after the checks of
/// the module doc (and the pin tag must be the source provider's tested tag). Sorted by
/// locale, then collation type.
pub fn read(pin: &Pin, zip: &Path) -> Result<Vec<SearchCollation>, String> {
    if pin.tag != SourceDataProvider::TESTED_ICUEXPORT_TAG {
        return Err(format!(
            "icuexport.pin names {}; icu_provider_source tests {}",
            pin.tag,
            SourceDataProvider::TESTED_ICUEXPORT_TAG
        ));
    }
    let p = SourceDataProvider::new_custom()
        .with_icuexport(zip)
        .map_err(|e| err(&zip.display().to_string(), e))?;

    same_singleton::<CollationRootV1>(&p)?;
    same_singleton::<CollationSpecialPrimariesV1>(&p)?;
    same_singleton::<CollationJamoV1>(&p)?;
    let n =
        same_tailorings::<CollationTailoringV1>(&p)? + same_tailorings::<CollationMetadataV1>(&p)?;
    if n < 100 {
        return Err(format!(
            "only {n} tailoring and metadata ids are in both the ICU export data and the compiled data"
        ));
    }

    let ids = search_ids::<CollationTailoringV1>(&p)?;
    if ids.is_empty() || ids != search_ids::<CollationMetadataV1>(&p)? {
        return Err("the search ids of the metadata and the tailoring differ".into());
    }
    for other in [
        search_ids::<CollationDiacriticsV1>(&p)?,
        search_ids::<CollationReorderingV1>(&p)?,
    ] {
        if !other.is_empty() {
            return Err(format!(
                "search data for the diacritics or the reordering marker: {other:?} (ep9_intl \
                 serves only the metadata and the tailoring)"
            ));
        }
    }

    let mut out = Vec::with_capacity(ids.len());
    for (locale, collation) in ids {
        let l: DataLocale = locale.parse().map_err(|e| err(&locale, e))?;
        let a = DataMarkerAttributes::try_from_str(&collation)
            .map_err(|e| format!("{collation}: {e:?}"))?;
        let meta = load::<CollationMetadataV1>(&p, &l, a)?;
        let data = load::<CollationTailoringV1>(&p, &l, a)?;
        let data = data.payload.get();
        out.push(SearchCollation {
            locale,
            collation,
            metadata: meta.payload.get().bits,
            trie: trie(data)?,
            ce32s: data.ce32s.iter().collect(),
            ces: data.ces.iter().collect(),
            contexts: data.contexts.iter().collect(),
        });
    }
    Ok(out)
}

/// `name: &[..],` with the numbers in lines of `per_line`, indented by `indent`.
fn array<T: std::fmt::Display>(
    out: &mut String,
    indent: &str,
    name: &str,
    v: &[T],
    per_line: usize,
) {
    let _ = writeln!(out, "{indent}{name}: &[");
    for chunk in v.chunks(per_line) {
        let line: Vec<String> = chunk.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "{indent}    {},", line.join(", "));
    }
    let _ = writeln!(out, "{indent}],");
}

/// The text of [`OUTPUT_FILE`].
pub fn render(pin: &Pin, list: &[SearchCollation]) -> String {
    let mut out = format!(
        "// @generated by tools/cldr-tables collator-search (INTL-V2-D17 Form A): do not edit. Make\n\
         // it again with\n\
         //   tools/cldr-tables/sync-icuexport.sh\n\
         //   cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml --features datagen -- collator-search\n\
         // and check it with `-- collator-search --check` (README.md there).\n\
         // Source: ICU export data {} ({}, sha256 {}),\n\
         // collation/implicithan/*_search*_{{meta,data}}.toml, through icu_provider_source 2.4.0.\n\
         // The root collation and the shared tailorings of that data equal the ICU4X compiled data.\n\
         // Data: Copyright (c) Unicode, Inc. SPDX-License-Identifier: Unicode-3.0\n\n\
         use crate::collator::search::{{SearchCollation, SearchTrie}};\n\n\
         /// The search collations, sorted by locale, then collation type.\n\
         pub(crate) static SEARCH_COLLATIONS: [SearchCollation; {}] = [\n",
        pin.tag,
        pin.file,
        pin.sha256,
        list.len()
    );
    for c in list {
        let t = &c.trie;
        let _ = writeln!(out, "    SearchCollation {{");
        let _ = writeln!(out, "        locale: {:?},", c.locale);
        let _ = writeln!(out, "        collation: {:?},", c.collation);
        let _ = writeln!(out, "        metadata: {:#x},", c.metadata);
        let _ = writeln!(out, "        trie: SearchTrie {{");
        let _ = writeln!(out, "            high_start: {:#x},", t.high_start);
        let _ = writeln!(
            out,
            "            shifted12_high_start: {:#x},",
            t.shifted12_high_start
        );
        let _ = writeln!(
            out,
            "            index3_null_offset: {:#x},",
            t.index3_null_offset
        );
        let _ = writeln!(
            out,
            "            data_null_offset: {:#x},",
            t.data_null_offset
        );
        let _ = writeln!(out, "            null_value: {:#x},", t.null_value);
        let _ = writeln!(out, "            small: {},", t.small);
        array(&mut out, "            ", "index", &t.index, 16);
        array(&mut out, "            ", "data", &t.data, 8);
        let _ = writeln!(out, "        }},");
        array(&mut out, "        ", "ce32s", &c.ce32s, 8);
        array(&mut out, "        ", "ces", &c.ces, 4);
        array(&mut out, "        ", "contexts", &c.contexts, 16);
        let _ = writeln!(out, "    }},");
    }
    out.push_str("];\n");
    out
}
