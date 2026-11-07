# icu4x patches (icu_calendar)

ep9 builds icu_calendar 2.3.0 from a git checkout of ICU4X with ep9's patches on top, in the
same form as the regress fork (third_party/patches/regress/README.md; vendor by hash). Only
icu_calendar comes from the checkout; every other ICU4X crate comes from crates.io.

```text
third_party/icu4x.commit            upstream commit (the vcs commit of the crates.io            tracked
                                    icu_calendar 2.3.0 package: "Bump component crates to 2.3.0")
third_party/patches/icu4x/*.patch   ep9 patches, applied in name order                         tracked (this folder)
scripts/sync-icu4x.sh               sync_dep icu4x <url> (scripts/sync-lib.sh)
third_party/icu4x/                  checkout + patches                                         git-ignored
[patch.crates-io] icu_calendar = { path = ".../third_party/icu4x/components/calendar" }
                                    in Cargo.toml and spikes/spike-c/Cargo.toml (spikes/spike-b
                                    has no icu_calendar); dep-guards check 16 checks both;
                                    tools/calprobe/run.sh patches its probe copy the same way
```

## State

- Pin: `a14f2dad852be26bad277ae704abf27a15cbfed1` of https://github.com/unicode-org/icu4x, the
  commit that `.cargo_vcs_info.json` of the crates.io icu_calendar 2.3.0 package names
  (`path_in_vcs` components/calendar). Its `components/calendar/src` equals the package's
  `src` (`diff -r`: no difference).
- Patches: 2.

  | File | What | Test | ep9 use |
  |---|---|---|---|
  | `0001-calendar-published-manifest.patch` | `components/calendar/Cargo.toml` becomes the manifest of the crates.io package (crates.io versions, no `workspace = true`), so the path package uses the crates.io ICU4X crates of the rest of the graph, not path copies from the checkout. Not for upstream | the build (one icu_calendar, one copy of each other ICU4X crate in Cargo.lock) | the `[patch.crates-io]` path entry |
  | `0002-calendar-until-month-search.patch` | `ArithmeticDate::until`: a binary search for the month count over the same NonISODateSurpasses check (monotone in the month count), between the lower bound and 13 months per year of the year difference plus one year; the old loop took one O(years) check per month past `min_months_from`, O(years^2) for chinese and dangi | `ep9_temporal` `tests/calendars.rs` (`chinese_and_dangi_until_in_months_over_20000_years`, `chinese_and_dangi_month_counts_equal_the_spec_loop`, `a_long_month_difference_ends_with_a_termination_pending`, the release-only `the_full_range_month_difference_ends_with_a_termination_pending`) | Temporal `until`/`since` of PlainDate, PlainYearMonth and ZonedDateTime, and Duration `round`/`total` with `relativeTo`, with largestUnit "months" or "years" (INTL W1 R2) |

- Bump: icu_calendar moves with the other ICU4X crates in one BUMP step (INTL-I1-D5). Then the
  pin moves to the vcs commit of the new package, 0001 is made again from the new package
  manifest, and 0002 goes away once upstream has a bounded `until` (the report draft of the
  INTL W1 fixer r3).
