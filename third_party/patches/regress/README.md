# regress fork patches

The RX0 backend of `ep9_regexp` is regress, built from a git checkout with ep9's patches on top
(V9-D15; decisions-log M5-PLAN-D41; docs/plan/08-repo.md section 1).

```text
third_party/regress.commit          upstream commit (tag v0.12.0)            tracked, orchestrator moves it
third_party/patches/regress/*.patch ep9 patches, applied in name order        tracked (this folder)
scripts/sync-regress.sh             sync_dep regress <url> (scripts/sync-lib.sh)
third_party/regress/                checkout + patches                       git-ignored
[patch.crates-io] regress = { path = ".../third_party/regress" }
                                    in Cargo.toml, spikes/spike-b/Cargo.toml, spikes/spike-c/Cargo.toml
```

## State

- Pin: `aae35dee1596cc9a9caaad2425b088081c09e5d5`, the commit of the annotated tag `v0.12.0` of
  https://github.com/ridiculousfish/regress.
- Patches: 7 (M5-P3 RX0b), each one upstreamable commit on the pin with its own test (the
  test file the patch adds under `tests/`, or its new `#[test]`s):

  | File | What | Test | ep9 use |
  |---|---|---|---|
  | `0001-poll-and-budget.patch` | `Limits`: an optional step budget and an interrupt poll every N steps (a start position, a backtrack, a loop iteration, also of a one-character loop); `Regex::find_limited` returns `Stopped` | `tests/limits.rs` | the poll of `Cx` interrupts (no budget: JS has none) |
  | `0002-single-position-exec.patch` | `Regex::match_at_limited`: one attempt at one position | `tests/match_at.rs` | a sticky exec tries one position, not every later one |
  | `0003-duplicate-name-backref.patch` | a backreference to a name that several groups share matches the group that took part (ES2025 duplicate named groups) | `tests/named_backrefs.rs` | `\k<n>` with `(?<n>a)\|(?<n>b)` |
  | `0004-legacy-icase-ascii-rule.patch` | `i` without `u`/`v`: Canonicalize keeps a character whose uppercase is not one code unit, and a non-ASCII one that would become ASCII (`ſ`, `ı`) | `tests/legacy_icase.rs` | the 11 KNOWN lines of the node golden |
  | `0005-latin1-input-and-start-predicates.patch` | `Text::Latin1` (one byte per unit) and code-unit start predicates (a first-unit set, a literal prefix, an anchor) under the `utf16` feature; the start-position search scans one chunk of at most 1,024 start positions per counted step (Latin-1, UCS-2, UTF-16: rejected prefix candidates and a set with no member spend the budget and poll), and the UTF-16 prefix search compares at most 16 units per candidate | `tests/latin1.rs`; `tests/limits.rs` (a long prefix polls; rejected candidates are metered) | a Latin-1 subject is read in place, not widened; a failed position costs a scan, not a match attempt |
  | `0006-caller-owned-scratch.patch` | `Regex::exec_into` with a caller-owned `Scratch` and capture buffer: no allocation per exec, no group-name clone | `tests/scratch.rs` | one `Scratch` per isolate |
  | `0007-flat-string-property-tables.patch` | the tables of the properties of strings (`\p{RGI_Emoji}` with `v`) as two integer arrays each (`StringSetTable`), not one pointer per string; `gen-unicode` writes the new form | `tests/string_properties.rs`, `unicode::tests::string_tables_are_well_formed` | 7,905 fewer load-time fixups: `ep9 --version` about 4.5% fewer instructions (W-REGEXP-1) |

  The source of the patches is the branch `rx0b` of a regress checkout; each file is its
  `git format-patch --zero-commit --no-signature` with the author `ep9` and no date.
- The pin equals the crates.io release 0.12.0 (the `Cargo.lock` checksum
  `32eef8b2...7c1` of the month-4 lock): its `.cargo_vcs_info.json` names the same commit, and
  `diff -r` of the two `src/` trees and of the two `tests/` trees prints nothing. The other
  differences are packaging only: the tarball has the normalized `Cargo.toml` (the git
  `Cargo.toml` is its `Cargo.toml.orig`, byte for byte), `Cargo.lock` and
  `.cargo_vcs_info.json`; the git tree has the `[workspace]` table and the `regress-tool` and
  `gen-unicode` members, which the ep9 graph never builds.

## Rules

- **Sync before cargo.** Every cargo command in the 3 workspaces needs `third_party/regress`,
  because a missing patch path fails the resolve step. Run `bash scripts/sync-regress.sh` in a
  new tree first. It does nothing when the pin and the patch set are unchanged (`.ep9-stamp`).
  CI runs it in `.github/actions/setup-rust` (cached by the pin and this folder).
- **Patch files.** `NNNN-<what>.patch`, made with `git format-patch` in the checkout, one
  upstreamable change per file, each with its own test. Applied with `git apply` in name order.
  A new or changed patch re-runs the sync (the stamp holds the hash of all patch files). A path
  dependency has no checksum in `Cargo.lock`, so a patch changes no Cargo table.
- **Live check.** `.github/scripts/dep-guards.sh` check 16: in all 3 workspaces `cargo metadata`
  shows regress with no registry source, from this checkout.
- **Licence.** regress is MIT OR Apache-2.0. A patch is code in this repository: the PR that
  adds the first one adds a regress entry to `THIRD-PARTY-LICENSE`.
