//! The AOT unit container (H35, `docs/adr/H35-aot-unit-format.md`): header v0, the section
//! table, and the load check. The check is in every build; the writer will be behind the
//! `aot` feature (H27), from A2.
//!
//! ```text
//!  unit bytes: header (256 B) | section table (32 B per section) | sections
//!              (text 64 KiB-aligned, the others 8-aligned)
//!                  |
//!  check(bytes, &HostInfo::current()) -> Load | Bytecode | Source | LoadError   (S4 steps 1-4)
//!                                         ^
//!  HostInfo: build id, ABI_VERSION, encoding::VERSION, BARRIER_VERSION, ep9_vm versions,
//!            FACT_SCHEMA_VERSION, PAGE_SIZE, target, CPU features
//! ```
//!
//! Golden files in `tests/golden/`: `header-v0.hex` (a sample header), `header-layout.txt`
//! ([`layout_text`]) and `container-history.txt` (one layout fingerprint per container
//! version). `EP9_BLESS=1 cargo test -p ep9_unit` rewrites the first two and appends a new
//! version line to the history; it never rewrites an existing history line.
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod check;
mod codes;
pub mod functions;
mod header;
mod host;
mod layout;
pub mod size;

pub use check::{Outcome, Verdict, check, verdict};
pub use codes::{Contract, SectionKind, TEXT_ALIGN_LOG2, Versions, cpu, target_abi, target_arch};
pub use header::{
    BadUnit, CONTAINER_VERSION, EXTENSION_ALIGN_LOG2, EXTENSION_KIND_MIN, ExtensionEntry,
    FROZEN_PREFIX_SIZE, HASH_SHA256, HEADER_SIZE, MAGIC, MAX_SECTIONS, MIN_SECTIONS,
    SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET, SOURCE_FLAGS_DEFINED, SectionEntry, TableEntry,
    UNIT_DATA_SLOTS, UnitHeader, compat_flags, entry_flags, entry_offsets, header_flags,
    incompat_flags, offsets, source_flags_valid,
};
pub use host::{ENGINE_BUILD_ID, HostInfo, RUNTIME_TABLE_LEN};
pub use layout::{
    CODE_TABLES_V4, ENTRY_FIELDS, Field, HEADER_FIELDS, OnChange, code_table_lines, layout_text,
};

/// The unit writer (A2). Empty until then: tests build units with their own encoder
/// (`tests/common/mod.rs`).
#[cfg(feature = "aot")]
mod aot {}
