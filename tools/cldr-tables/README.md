# cldr-tables

The generator of the CLDR side tables of `ep9_intl` (decision INTL-V2-D5): CLDR data that the
ICU4X compiled data does not give. It reads the CLDR JSON release that `cldr-json.commit` pins
and writes `crates/ep9_intl/src/cldr/gen/*.rs`. Do not edit the generated files.

```text
sync-cldr-json.sh   cldr-json/ (git-ignored, sparse: only the files the generator reads)
                      pin: cldr-json.commit = the commit of the cldr-json tag of the CLDR release
cargo run           -> crates/ep9_intl/src/cldr/gen/
                      version.rs  CLDR_VERSION, CLDR_JSON_COMMIT
                      digits.rs   the 78 numbering systems with simple digit mappings
                      t1.rs       NaN, infinity, exponent symbol, approximately sign, range
                                  pattern per CLDR locale and numbering system
                      locales.rs  the availableLocales "full" ids (ep9_intl available.rs)
                      hour_cycles.rs  the timeData preferred hour cycles (locale_info.rs)
                      aliases.rs  the bcp47 -u- and -t- value aliases (tag.rs)
                      t2.rs       T2: the unit patterns (plural forms, per-unit pattern) of
                                  the 45 sanctioned units and the CLDR "X-per-Y" units of
                                  two of them, and the "per" pattern, per CLDR locale and
                                  width (number_format/unit.rs)
cargo run -- --check  writes nothing; exit 1 if a generated file differs (CI form)
                    (`cargo run -- tables` is the same mode; `collator-search` is below)
cargo test          the tables against the ICU4X compiled data of the same CLDR release,
                    the CLDR version against the ICU4X data README, the check, determinism
```

T2 replaces the ICU4X units data for Intl.NumberFormat style "unit": that data has only 5
unit categories (area, duration, length, mass, volume). The test `t2_equals_icu4x` compares
the "other" pattern of each unit that the ICU4X data has; the `ep9_intl` test
`cldr::tests::t2_equals_icu4x_in_every_plural_category` compares every plural form (it needs
`icu_plurals`, which this crate does not list).

Commands (from the repository root):

```sh
tools/cldr-tables/sync-cldr-json.sh
cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml             # write
cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml -- --check  # check
cargo test --locked --manifest-path tools/cldr-tables/Cargo.toml
```

The CLDR version must be the CLDR version of the ICU4X compiled data that the engine links
(INTL-V2 plan 2.5: ICU4X 2.4 data = CLDR 49.0.0). At every ICU4X BUMP: set `cldr-json.commit`
to the commit of the cldr-json tag of the new data's CLDR version (`git ls-remote
https://github.com/unicode-org/cldr-json.git refs/tags/<version>`), move the `icu_*`
dev-dependency pins in `Cargo.toml` to the new release, run the three commands, and commit the
new `gen/` files. The test `cldr_version_equals_icu4x_data` fails until the two versions agree.

## The search collations (`collator-search`)

The ICU4X compiled data leaves out the search collations (`-u-co-search`, `-u-co-searchjl`),
which `Intl.Collator` with `usage: "search"` needs (decision INTL-V2-D17 Form A, DL row
INTL-W4-CLDR). The mode `collator-search` (feature `datagen`) reads them from the ICU export
data release that `icuexport.pin` names (tag, file, sha256), with `icu_provider_source` of the
same ICU4X release, and writes `crates/ep9_intl/src/cldr/gen/collator_search/mod.rs`: the
metadata and the tailoring of the 22 `search*` ids as plain Rust data (`ep9_intl` has no
`unsafe` and no `include!`, so not the ICU4X baked form; `ep9_intl` builds the data structs with
the checked constructors). Before it writes, it checks that the pin tag is the source
provider's tested tag, that the root collation and every tailoring that the compiled data holds
under the same id are equal in the two data sets, and that no search id has diacritics or
reordering data.

```text
sync-icuexport.sh   icuexport/<file> (git-ignored; one fetch from the GitHub release, sha256
                      checked against icuexport.pin)
cargo run --features datagen -- collator-search            -> cldr/gen/collator_search/mod.rs
cargo run --features datagen -- collator-search --check    writes nothing; exit 1 if it differs
```

```sh
tools/cldr-tables/sync-icuexport.sh
cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml --features datagen -- collator-search
cargo run --locked --manifest-path tools/cldr-tables/Cargo.toml --features datagen -- collator-search --check
```

The feature is off in the CI form (the commands above without it), so CI does not build the
source provider. At every ICU4X BUMP: move the `icu_provider_source` and `icu_collator` pins in
`Cargo.toml` to the new release, set `icuexport.pin` to its `TESTED_ICUEXPORT_TAG` (the release
asset `icu4x-icuexportdata-<version>.zip` and its sha256), and run the commands.

This crate has its own `[workspace]` and lock; the root `Cargo.toml` lists `tools/cldr-tables`
in `exclude`. It is a tool, never part of the engine graph. The CLDR data is Unicode-3.0
(`deny.toml` allows it).
