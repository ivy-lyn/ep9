# ep9 plan: governance

- This file holds the project rules for licence, repository, CI cost, review, upstream contact and line endings.
- Source: the user's statement of 2026-10-20, verbatim: "Don't ask me questions. Deepresearch and descide yourself". The decision workflow made each decision below with the evidence in §7. The decisions log has the entries ([decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)).
- ADR process rules (P-1 to P-4) are in [adr/README §3-§4](../adr/README.md#3-amendments).
- Rule for every item: prefer the reversible option. Never make anything public.
- This file sits in `docs/plan/` because the decision run could edit only plan, ADR and log files. It replaces the planned `docs/governance.md`.

IDs used here: D2, D3, D6, D12; H28, H36; P1; RX1; M0-M5, M8; LICENSE, REMOTE-CI, ADR-P1-ADR-P4, CODEOWNERS, UPSTREAM, GITATTRIBUTES (decision ids, [decisions-log §5](../decisions-log.md#5-2026-10-20-decisions-made-by-the-agent-team-under-the-users-instruction)).

## 1. Licence

### 1.1 Decision

| Item | Value |
|---|---|
| ep9's own code | `MIT OR Apache-2.0` (SPDX expression). The user of ep9 picks either licence |
| Licence files | `LICENSE-MIT` ("Copyright (c) 2026-present the ep9 authors") and `LICENSE-APACHE` (the Apache text, byte-identical to apache.org, sha256 `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`) |
| NOTICE file | None, for ep9's own code and for every crate in the tree (`ep9_math` included). So Apache-2.0 §4(d) adds no duty for ep9's users |
| Third-party code copied into the tree | Listed in `THIRD-PARTY-LICENSE` with its licence text (the oxc and rolldown layout). The PR that copies or ports the code adds the entry |
| Cargo dependency notices | Release builds ship a notice file that `cargo about generate` makes from `Cargo.lock` (tested offline on the M0 graph, 2026-10-20). Not needed before the first binary release |
| Crate manifests | `[workspace.package] license = "MIT OR Apache-2.0"`; each member `license.workspace = true`. On 2026-10-21 all 10 members have it (7 in `crates/*`, 3 in `tools/*`; `cargo metadata --locked`). Each spike has its own `[workspace]` and states `license = "MIT OR Apache-2.0"` (`sp-dispatch`, `sp-gc`, `sp-jit-mem`). `tools/ep9-typecheck` (not created yet) states the same expression |
| Contributions | Inbound = outbound. README clause: a contribution is dual licensed as above, with no extra terms |
| Licence check | `cargo deny check licenses` with a root `deny.toml` on every PR. Allowed today: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSL-1.0, Unicode-3.0. Add an id only in the PR that adds a crate that needs it. Never: GPL, LGPL, AGPL, SSPL. MPL-2.0 only with a per-crate exception |

### 1.2 Why

| Fact | Effect |
|---|---|
| oxc, rolldown and vite are MIT ("VoidZero Inc. & Contributors"); rolldown keeps ported code in `THIRD-PARTY-LICENSE` | ep9 code can move into those projects: the MIT side of the dual licence fits |
| Rust API guidelines C-PERMISSIVE recommend `MIT OR Apache-2.0` for ecosystem compatibility. regress, temporal_rs, dashu and mmtk-core use it | ep9 can take code from them and give code to them with no licence change |
| Apache-2.0 §3 gives a patent licence from each contributor and ends it for a licensee who sues. LLVM moved to Apache-2.0 for this "patent commons"; wasmtime/Cranelift use Apache-2.0 WITH LLVM-exception | Embedders of a JIT/GC/AOT engine get an explicit patent grant from contributors. It does not cover third-party patents |
| MIT alone is GPL-2.0 compatible; Apache-2.0 alone is not | The MIT option lets GPL-2.0 projects reuse ep9 source. A full ep9 binary may contain Apache-2.0-only crates (for example `calendrical_calculations` for Temporal), so the binary as a whole is not GPL-2.0 compatible |
| Reversibility: a dual-licensed project can drop to MIT-only later with no one's consent (MIT is one of the choices). An MIT-only project needs every contributor's grant to add Apache-2.0 | Dual is the reversible choice |
| The root lock (95 packages, 2026-10-20): 31 "MIT OR Apache-2.0", 4 "Apache-2.0 OR MIT", 1 "Apache-2.0/MIT", 26 MIT, 18 Unicode-3.0 (ICU4X via textwrap), 2 "(MIT OR Apache-2.0) AND Unicode-3.0", 1 "Unlicense OR MIT" (memchr), 1 "Apache-2.0 WITH LLVM-exception OR BSL-1.0" (dragonbox_ecma), 1 "Apache-2.0 OR GPL-2.0-only" (self_cell), 10 workspace crates with no licence yet | All compatible with the decision. `cargo deny` 0.20.2 with the allow list above reports "licenses ok" (re-run by the critic, 2026-10-20) |

### 1.3 Duties for code that ep9 plans to copy or port

| Planned code | From | Licence | In the source | In shipped binaries |
|---|---|---|---|---|
| Any V8 port: RX1 irregexp (M2-M5), fixed-dtoa and bignum-dtoa (M1), DoubleToRadixCString, `src/bigint` (M8), Torque `MathHypot` (H36) | V8 | BSD-3-Clause (V8 `LICENSE`) | Keep "Copyright 20xx the V8 project authors" in each file. V8 licence text in `THIRD-PARTY-LICENSE` | Reproduce the V8 copyright, conditions and disclaimer in the release notices. Never use V8's or Google's name to promote ep9 (clause 3) |
| fdlibm-derived Math, if any file is ported | FreeBSD msun, V8 `ieee754`, musl | Sun notice: use is "freely granted, provided that this notice is preserved" | Keep the Sun notice in each ported file | Put the notice in the release notices (conservative reading) |
| CR Math seed (H36) | pxfm 0.1.30 | BSD-3-Clause OR Apache-2.0; ep9 takes the BSD-3-Clause side | Keep the headers; add "Ported for ep9 from ..."; entry in `THIRD-PARTY-LICENSE` | Reproduce the BSD-3 notice in the release notices |
| CR Math algorithms (H36) | CORE-MATH | MIT | Name CORE-MATH as the origin where a ported function matches its C source; entry in `THIRD-PARTY-LICENSE` | Include the MIT notice |
| CR Math functions from LLVM libc, if any (H36) | LLVM libc | Apache-2.0 WITH LLVM-exception | Keep the headers; entry in `THIRD-PARTY-LICENSE` | None: the LLVM exception covers object code |
| `fmod` (H36) | libm 0.2.16 / musl | MIT | Keep the copyright lines; entry in `THIRD-PARTY-LICENSE` | Include the MIT notice |
| dragonbox or ryu ports, if ep9 copies code instead of using the crates | dragonbox, ryu-js | Apache-2.0 WITH LLVM-exception OR BSL-1.0; Apache-2.0 OR BSL-1.0 | Keep the header. ep9 takes the BSL-1.0 side | None: BSL-1.0 exempts machine-executable object code |
| `ecmascript_atomics`, only if vendored | ecmascript_atomics | MPL-2.0 | Keep the files separate with their MPL header (per-crate exception in `deny.toml`) | Make the source of those files available with binary releases |
| tsrs scanner port (only if option D) | tsrs | Apache-2.0 + NOTICE (derived from TypeScript) | Keep headers; copy the NOTICE lines that apply into `THIRD-PARTY-LICENSE` | Ship those NOTICE lines |
| test262 patches | test262 | test262 BSD licence (Ecma); no patent licence | Keep the header | None: test data never ships |
| Typed AWFY kernels | are-we-fast-yet | MIT | Keep the copyright line | None: benchmark only |
| tsrs in `ep9-typecheck` | tsrs | Apache-2.0 + NOTICE | Never copied into the engine (dep guard) | The `ep9-typecheck` release ships tsrs `LICENSE` and `NOTICE` |

## 2. Repository

| Item | Decision |
|---|---|
| Host | The existing private GitHub repo `ep9` under the owner account. It exists and is empty (created 2026-10-20 14:28:21 UTC; no branch, no protection rules, 1 collaborator). Use it; do not create another |
| Visibility | Private. Only the user changes visibility. Agents never run `gh repo edit --visibility`, `gh repo create --public`, `gh gist create`, never enable GitHub Pages, never post, comment or react on any public repository (§5) |
| Default branch | `main`. The local HEAD is already `refs/heads/main`; the first push sets the GitHub default |
| Merge settings | Squash merge only; delete the branch on merge. One commit on `main` per PR, so per-commit perf data and the ADR change log line up with PRs |
| Branch protection | None now (§4.3) |
| Signing | The user's git config signs every commit. Never bypass it. If signing fails, stop and report |
| History | All changes reach `main` through a PR, after the first commit |

**First commit and push (critic change, 2026-10-20).** A commit or push happens only after an explicit instruction from the user (harness rule: commit or push only when the user asks). The orchestrator prepares it and reports "ready to commit and push", with the checklist output, as a statement, not a question. The checklist:
1. The in-flight fix workflow (tools/xtask, tools/test262, tools/bench/jetstream, spikes/sp-dispatch) has finished.
2. `.gitattributes`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD-PARTY-LICENSE` and `.github/CODEOWNERS` are in the tree before the first `git add` (done 2026-10-20), so line endings are normalized from commit 1.
3. The CI scope job and its script exist, so the first push does not start every job on every target.
4. These pass on the host: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`; `cargo run --release --locked -p ep9_test262 -- parse`; `bash .github/scripts/dep-guards.sh`; `actionlint .github/workflows/ci.yml`; `cargo deny check licenses`.
5. `git add -A --dry-run` lists no build output, no `third_party/<name>/` checkout, no file over 1 MB, and no secret (scan for key, token and password patterns).
6. The commit carries the attribution trailer. The push is `git push -u origin main` over ssh.

The user gave the instruction on 2026-10-20: "You are free to create repo, keep it private first" ([decisions-log §7](../decisions-log.md#7-2026-10-21-h15-dispatch-on-x86-64-sp-dispatch-reopen), row REMOTE-CI (note), corrected 2026-10-21).

After the push: set the merge settings, create the label `ci:full`, and start one full run with `gh workflow run ci.yml --ref main`. The full run gives the "CI green on 4 targets" evidence for the M0 week-1 exit. Expect cold caches.

**Why a personal repo and not an organization.** Members of an organization can see its private repos, depending on the base permission. A personal private repo is visible only to the owner and named collaborators. A later transfer to an organization keeps history, issues and redirects, so this choice is reversible.

## 3. CI triggers and cost

### 3.1 Prices (GitHub docs, read 2026-10-20)

| Runner (standard, 2-core unless named) | USD per minute | vs Linux x64 |
|---|---|---|
| Linux x64 | 0.006 | 1x |
| Linux arm64 | 0.005 | 0.83x |
| Windows x64 | 0.010 | 1.7x |
| macOS arm64 (M1, 3-core) | 0.062 | 10.3x |

- Included per month for private repos: 2,000 minutes (Free) or 3,000 (Pro). Usage counts against it at list price, so macOS uses it about 10x faster. Each job rounds up to a whole minute. Public repos are free; ep9 stays private.
- Prices fell by up to 39% on 2026-01-01. The planned $0.002/min fee for self-hosted runners is postponed.
- **Quota risk.** A nightly macOS run alone (about 12 min x $0.062, about $0.75 per night, about $22 per month) is worth more than the whole included quota at the Linux rate. Without a payment method on the account, usage is blocked once the quota is used, so CI is expected to block in month 1. A payment method and a budget are owner actions; agents never change billing. Until one exists, agents keep running the host gates, and macOS nightly runs only when `main` moved.

### 3.2 Triggers and tiers

| Event | Build + test, test262 parse | lint, dep guards | Spikes (4 targets) | sp-tsrs-win | perf CI v0 |
|---|---|---|---|---|---|
| Draft PR | Linux x64 | yes | no | no | no |
| Ready PR | Linux x64, Linux arm64, Windows x64; + macOS arm64 when the PR touches `crates/ep9_platform`, `spikes`, `rust-toolchain.toml`, `Cargo.lock`, `.cargo`, `scripts`, `.gitattributes` or CI files | yes | when spike inputs change | when tsrs inputs change | yes |
| PR with label `ci:full` (read at the next push) | all 4 | yes | yes | yes | yes |
| Push to `main` | Linux x64 | yes | no | no | yes (per-commit baseline) |
| Nightly 19:17 UTC | all 4 | yes | Mondays (UTC) | Mondays (UTC) | yes |
| `workflow_dispatch` | all 4 | yes | yes | yes | yes |
| Docs-only change | none: only the `scope` job runs | no | no | no | no |

- **State on 2026-10-20.** `.github/workflows/ci.yml` has the decided `pull_request` types and the concurrency rule. The `scope` job, `.github/scripts/ci-scope.sh` and the nightly `schedule` trigger land together in one follow-up PR, after the current tools/ work. Until then every event runs every job (the old behaviour), and no nightly run exists.
- `.github/scripts/ci-scope.sh` computes the tier. It reads the changed paths from git, so there is no `paths-ignore`: a path-skipped workflow leaves required checks "Pending", a skipped job does not.
- The nightly run is skipped when `main` has not moved since the last successful nightly run.
- Concurrency: a new push to a PR cancels that PR's running CI. Push, schedule and dispatch runs are never cancelled. Each push to `main` has its own group, so every main commit gets a run (per-commit perf baseline).
- `ci:full` is read when CI starts (open, push, ready for review). There is no `labeled` trigger: in the PR's group it would cancel the running CI on every label change. For a full run now: `gh workflow run ci.yml --ref <branch>`.
- Timeouts cap a runaway job: 40 min (build, test262), 45 (spikes), 90 (sp-tsrs-win).
- The agent host is macOS arm64. Before a PR is marked ready, the author runs the §2 item 4 commands on the host. So a PR without macOS CI still has a macOS run.

### 3.3 Budget

| Rule | Value |
|---|---|
| Ceiling | USD 50 per month of Actions spend above the included quota, for M0 (est., low confidence). Reviewed at M0 exit with measured data |
| Check | The orchestrator sums job minutes x price from `gh run list` / `gh run view --json jobs` every week |
| Over the ceiling (projected) | Set repo variable `CI_COST_MODE=lean`: ready PRs run Linux x64 only; nightly keeps all 4 targets. Unset it at the next month |
| Quota used up with no payment method | CI stops. Agents keep the host checks and report it. Agents never change billing, budgets or payment methods |

Estimate (job minutes are not measured yet; confidence low): ready PR USD 0.25-0.43; + macOS 0.43-0.81; draft PR 0.07-0.13; push to main 0.10-0.16; nightly 0.68-1.24 (+0.94-1.62 on Mondays). The current workflow (every job on every event) costs about USD 1.6-2.9 per run. The tiers cut about 80%.

## 4. Review and ownership

### 4.1 Roles

| Role | Who |
|---|---|
| Owner | The GitHub account that owns the repo: the only human |
| Orchestrator | The lead agent. It merges PRs and moves ADR status |
| Plan roles (lead/front end, GC, VM, JIT/platform, runtime, AOT) | Agents (D3) |

### 4.2 Review of a PR

| PR kind | Needed before merge |
|---|---|
| Docs-only, outside the decision files | Author self-check |
| Decision files: `docs/PLAN.md`, `docs/plan/`, `docs/decisions-log.md` (append only) | A reviewer agent that did not write the change |
| ADR (`docs/adr/`) | [adr/README §3](../adr/README.md#3-amendments) |
| Code, including build files, CI, pins and patches | Green CI for its tier. A Codex adversarial review (`/codex:adversarial-review --model gpt-6.1-sol`, in the background). An agent that did not write the code verifies each finding through the full control flow and sorts it: (a) serves the PR's goal, (b) a defect the PR introduced, (c) neither. Only (a) and (b) change the PR |
| Contract crates (the `crates/` lines of `.github/CODEOWNERS`) and every `unsafe` change | All of "Code", plus a reviewer agent that did not write it checks the change against the governing ADR's Rules table |

The PR body carries a "Review record": the reviewer's role, the Codex verdict per finding, the sort (a)/(b)/(c), and the data (gate output or benchmark) when the rule asks for it.

### 4.3 Why CODEOWNERS is not enforced

- Agents open every PR from the owner account. GitHub: "Pull request authors cannot approve their own pull requests." A required code-owner review would block every PR, or need an admin bypass on every merge.
- Protected branches and code owners in private repos need GitHub Pro or higher. The plan of the owner account is not visible to the agents' token.
- Code owners must have write access, and team owners need an organization. A personal repo can name only the owner.

So `.github/CODEOWNERS` lists the owner on the contract paths, the decision files and the build files. It routes the review rule in §4.2. It reopens if agents get their own identity (a GitHub App), so that the owner can approve their PRs; then a ruleset can require code-owner review (on Pro or higher). After the first push, check it with `gh api repos/{owner}/{repo}/codeowners/errors`.

## 5. Upstream contact (oxc and other public projects)

| Rule | Decision |
|---|---|
| Who posts | Only the user, by hand. Agents never create issues, PRs, comments, reactions or reviews on any public repository, and never drive the user's account for it |
| Drafts | In this private repo: `docs/upstream/<project>-<slug>.md`, one per request. The folder and its index exist since 2026-10-21 ([docs/upstream/README.md](../upstream/README.md)) |
| A draft is "ready" when | (1) a minimal repro runs on the pinned version (oxc `=0.155.0`) and on upstream main of the draft date, and an agent that did not write it reproduced it; (2) it names the ep9 need (milestone, gate) and the local workaround; (3) it stands alone: no ep9 internals, no private links, nothing that reveals unannounced plans beyond the need itself |
| When a draft goes to the user | Only if the need is concrete and verified and one of: the local workaround fails; the local patch count would pass the cap of 3 (gate P1); or the need is due within the next milestone. Ready drafts appear in the next status report as "ready to post", as a statement |
| Default meanwhile | Work with the local workaround or a local patch through `third_party/patches/oxc` (≤ 3, gate P1). No work waits on a post |
| tsrs | No request (D12) |
| Week-2 item "file or join #11793, #21953 and the Function() API" | Becomes: write the three drafts. #11793 (eval-string parse options; open since 2025-06-18, no PR) and #21953 (stack overflow on deep nesting; open since 2026-04-29, no PR) are both open, and oxc 0.155.0 `ParseOptions` has neither feature |

### 5.1 Planned drafts

Status runs: draft -> ready (all 3 checks pass) -> handed over (in a status report) -> posted (the user adds the link) or dropped.

| File | Project | Upstream ref | ep9 need (milestone, gate) | Local workaround | Status |
|---|---|---|---|---|---|
| [`oxc-eval-parse-options.md`](../upstream/oxc-eval-parse-options.md) | oxc | #11793 | Eval-context early errors; M1 exit; P1 | Nova-style wrapper parse + post-checks, then a local patch | draft (2026-10-21) |
| [`oxc-depth-limit.md`](../upstream/oxc-depth-limit.md) | oxc | #21953, #24375 | Depth error instead of abort; M1 exit; P1 | Bracket pre-scan, sized stack, AST-depth pass (M0) | draft (2026-10-21) |
| [`oxc-function-constructor.md`](../upstream/oxc-function-constructor.md) | oxc | none yet | Parse Function() parameters and body apart; M1 exit | Nova's single-script check | draft (2026-10-21) |

Draft template (fields): project and ref; status; ep9 need (milestone, gate, due date); workaround and local patch count after it (n/3); verified by (an agent role that did not write it, date); repro on the pinned version; repro on upstream main (date, commit); standalone text to post.

## 6. Line endings

- Root `.gitattributes`: `* text=auto eol=lf`, `*.patch -text`, CRLF for `*.bat`, `*.cmd`, `*.ps1`, and a binary list. All 187 files of the M0 tree stage as LF, also with `core.autocrlf=true` (simulated 2026-10-20). One file changes: `docs/research/artifacts/tsrs-conformance/results/summary.csv` has CRLF in the work tree (Python csv output) and is stored as LF.
- The attributes do not reach the nested checkouts under `third_party/<name>/`: git reads `.gitattributes` only up to the top of each work tree. So CI keeps `git config --global core.autocrlf false`, and `scripts/sync-lib.sh` sets `core.autocrlf=false` in each checkout on every run, before the stamp check (done 2026-10-21, PR #1).
- A test file that needs CR or CRLF bytes gets its own `-text` line in the PR that adds it.

## 7. Evidence

| Claim | Source |
|---|---|
| Runner prices | https://docs.github.com/en/billing/reference/actions-runner-pricing |
| Included minutes; public repos free; blocked without a payment method | https://docs.github.com/en/billing/concepts/product-billing/github-actions |
| Price cut 2026-01-01; self-hosted fee postponed; quota used at list price | https://github.blog/changelog/2025-12-16-coming-soon-simpler-pricing-and-a-better-experience-for-github-actions/ |
| Private-repo runner labels incl. `ubuntu-24.04-arm` | https://docs.github.com/en/actions/reference/runners/github-hosted-runners |
| Skipped-by-path workflow leaves required checks Pending | https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax |
| Concurrency: pending run replaced; expression in `cancel-in-progress` | https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency |
| Schedule: default branch only; delays at the top of the hour | https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows |
| Authors cannot approve their own PRs | https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/approving-a-pull-request-with-required-reviews |
| Code owners need write access; last match wins; no `!` | https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners |
| Protected branches in private repos need Pro or higher | https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches |
| C-PERMISSIVE | https://rust-lang.github.io/api-guidelines/necessities.html |
| LLVM: Apache-2.0 patent commons | https://llvm.org/docs/DeveloperPolicy.html |
| Apache-2.0 §3 patent grant and termination | https://www.apache.org/licenses/LICENSE-2.0 |
| V8 BSD-3 licence; fdlibm notice | https://raw.githubusercontent.com/v8/v8/main/LICENSE, https://raw.githubusercontent.com/v8/v8/main/LICENSE.fdlibm |
| rolldown, vite: MIT | https://raw.githubusercontent.com/rolldown/rolldown/main/LICENSE, https://raw.githubusercontent.com/vitejs/vite/main/LICENSE |
| oxc: MIT + THIRD-PARTY-LICENSE | reference checkout `oxc/LICENSE`, `oxc/THIRD-PARTY-LICENSE` |
| pxfm licence and missing provenance notes | `~/.cargo/registry/src/*/pxfm-0.1.30/Cargo.toml:38`; no "core-math" or "llvm" in its `src/` (grep, 2026-10-20) |
| oxc issues | https://github.com/oxc-project/oxc/issues/11793, https://github.com/oxc-project/oxc/issues/21953 |
| oxc 0.155.0 ParseOptions | `~/.cargo/registry/src/*/oxc_parser-0.155.0/src/lib.rs:222-270` (fields: parse_regular_expression, allow_return_outside_function, preserve_parens, allow_v8_intrinsics, enable_ident_hashes) |
