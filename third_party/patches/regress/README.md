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
- Patches: none (M5-P0). The fork patches come in M5-P3 (RX0b).
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
