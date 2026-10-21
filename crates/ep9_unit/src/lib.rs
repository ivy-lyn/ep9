//! The AOT unit container (H35, `docs/adr/H35-aot-unit-format.md`): header, versions and the
//! load check. The loader is in every build; the writer is behind the `aot` feature (H27).

/// The unit writer (A2). Empty until then.
#[cfg(feature = "aot")]
mod aot {}
