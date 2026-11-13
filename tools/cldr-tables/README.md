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

This crate has its own `[workspace]` and lock; the root `Cargo.toml` lists `tools/cldr-tables`
in `exclude`. It is a tool, never part of the engine graph. The CLDR data is Unicode-3.0
(`deny.toml` allows it).
