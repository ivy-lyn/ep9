# ep9-x86-sandbox

A real x86-64 Linux box on Cloudflare Containers, to run, test, bench and debug ep9
x86-64 builds. Use it for what the Docker (Rosetta) step of local CI cannot do: FMA, a
real `posix_spawn` of a missing program, `PR_SET_SECCOMP`, real perf counters, cachegrind.

It is a copy of the webcodecs harness
(`a local webcodecs-node-work/x86-sandbox checkout`, "v2"). Only the names,
the cap (8 instead of 4), the image tools and these docs differ. The measured evidence for
the design (one id = one container, the destroy/exec race fix, what `sbx` retries, limits,
token rotation timing) is in that README; it is not copied here.

```
 Mac                            Cloudflare (account 6db7cbc0423b5d56af93d13e4e2b469b)
 ───                            ─────────────────────────────────────────────────────
 cargo zigbuild --target x86_64-unknown-linux-gnu
   │
 ./sbx ──HTTPS + Bearer──▶ Worker ep9-x86-sandbox   (http -> 403, bad token -> 401, then route)
        (refuses http://)      │  env.Sandbox.getByName(<id>)
                               ▼
                           Durable Object "Sandbox" (one per <id>) ──▶ Durable Object "Registry"
                               │  ctx.container.start / exec / destroy      (cap: 8 running ids)
                               ▼
                           Container app ep9-x86-sandbox-sandbox (policy durable_object)
                           standard-4 = 4 vCPU, 12 GiB, 20 GB disk; linux/amd64, Debian 13
```

- URL: `.url` = `https://ep9-x86-sandbox.lynweklm.workers.dev` (must be `https://`)
- Token: `.token` (mode 600, git-ignored). Never print it. `sbx` reads it itself.
- Client needs: bash 3.2+, curl, jq, base64, split, shasum, zstd, tar, git (`push-git` needs
  `git archive --add-file`).

## Commands

All commands take a sandbox `<id>` (`^[a-z0-9-]{1,40}$`). Same id = same machine (same
files, same processes) until it is destroyed or sleeps (30 min after the last request).

```bash
SBX=<ep9 checkout>/infra/x86-sandbox/sbx

$SBX health                      # auth check
$SBX health <id>                 # is it running? (never starts it)
$SBX exec <id> 'nproc' [timeout_s]                 # bash -c in /work; exit code passes through
$SBX put  <id> ./file /work/in/file                # chunked upload, sha256 checked
$SBX get  <id> /work/out/file ./file               # chunked download, sha256 checked
$SBX push-git <id> <git-dir> <commit> /work/src    # committed files only (git archive); see below
$SBX push-git --clear <id> <git-dir> <commit> /work/src   # same, but empties a non-empty /work/src first
$SBX manifest <git-dir> <commit>                   # print the .sbx-manifest of <commit> (local, no token)
$SBX bg   <id> <job> '<long command>'              # anything over ~1-2 min
$SBX wait <id> <job> 20                            # poll, stream the log, exit with its code
$SBX destroy <id>                                  # stop it now (files are lost; billing stops)
```

Exit codes of `sbx`: 98 transport / HTTP error, 97 sha256 mismatch, 96 job not found,
95 job died without an exit file, 94 destroy answered but a container still runs,
93 push-git refused the remote dir (not empty, not a dir, or a symlink), 92 push-git or
manifest cannot describe the commit (nothing was sent), 99 bad usage.

## push-git

```
Mac                                                          box
git ls-tree -r <commit> ──> .sbx-manifest
git archive --add-file=.sbx-manifest <commit> ──> tar
  check: tar files == manifest files, each hashes to its blob id
zstd ──> put ──> /work/.upload/git-<commit>.tar.zst ──>      <remote-dir> missing or empty? (else exit 93,
                                                               or with --clear: delete its entries)
                                                             extract; write .sbx-commit
```

- `<remote-dir>` must be absolute, plain (no `.`, `..`, `//` or trailing `/`), not `/`, and not
  `/work/.upload` or a parent of it (so not `/work`). These checks run before any request.
- A non-empty `<remote-dir>` (a dotfile counts) is refused with exit 93; the upload is removed
  and the dir is not touched. `--clear` deletes its entries first. A file or a symlink at
  `<remote-dir>` is always refused. Without this rule, files of an earlier commit would stay
  next to the new ones and the tree would no longer be that commit (F5).
- The archive carries `.sbx-manifest` at the root of `<remote-dir>`; no extra request is made.
  The test262 runner compares the tree with it (CX6-2 (c)). Format:

  ```
  commit <full commit id>
  <blob id> <path>          one line per file of the archive, sorted by path byte by byte (LC_ALL=C)
  ```

  The lines come from `git ls-tree -r <commit>`. A gitlink is skipped (`git archive` writes no
  file for it); a symlink is listed with the blob id of its target text. `<path>` is the raw path
  from the root, not quoted: it is everything after the first space. Every line ends with `\n`.
  `.sbx-manifest` and `.sbx-commit` are not listed. A commit with a path that holds a newline, or
  with its own `.sbx-manifest` or `.sbx-commit`, is refused (exit 92).
- Before anything is sent, `sbx` extracts the archive locally and checks that it holds exactly
  the listed files and that each one hashes to its blob id (`git hash-object --no-filters`). A
  `.gitattributes` rule such as `eol=crlf`, `export-subst` or `export-ignore` would make the
  archive differ from the blobs; then `push-git` stops with exit 92.
- `.sbx-commit` (one line: the full commit id) is written on the box after the extract, as before.

Test the local parts with no sandbox, no network and no token (temp git repos; the two Worker
calls are replaced by local stand-ins):

```bash
bash infra/x86-sandbox/tests/push-git-local.sh
SBX_TEST_BASH=/bin/bash bash infra/x86-sandbox/tests/push-git-local.sh   # sbx under bash 3.2 (macOS)
```

## Cross-build on the Mac, run on the box

```
Mac (arm64)                                        box (x86-64, glibc 2.41)
cargo zigbuild --release --locked \                $SBX put  <id> <bin> /work/bin/<bin>
  --target x86_64-unknown-linux-gnu  ───────▶      $SBX exec <id> 'chmod +x /work/bin/<bin> && /work/bin/<bin>'
  (own CARGO_TARGET_DIR)
```

Tools on the box (checked 2026-10-21): valgrind 3.24.0 (cachegrind works), perf 6.12
(`perf stat` counts instructions and cycles), gdb 16.3, strace 6.13, node v24.22.0,
rustc/cargo 1.99.1 (rustup stable, minimal), gcc 14.2, git, zstd, jq, python 3.13.5.

## Smoke test

`./smoke.sh` (default id `ep9-smoke`; steps: health info tools rt bg auth destroy; `all` ends
with `destroy`). Raw logs go to `evidence/` (git-ignored). Result on 2026-10-21:

| check | result |
| --- | --- |
| box | `x86_64`, kernel 6.18.54-cloudflare-microvm, 4 vCPU, AMD EPYC, Debian 13, glibc 2.41 |
| CPU flags | fma yes, avx2 yes, avx512f yes |
| memory / disk | 12 GiB (`free -g` reads 4 right after a cold start, 12 seconds later), 18 GB free on `/` |
| put + get 20 MB random file | sha256 equal, `cmp` identical (put 4 s, get 2 s) |
| `bg` + `wait`, 200 s job | 10 ticks, exit 0, wall 214 s |
| no token / wrong token | 401 on `/exec`, `/write`, `/destroy`, `/health` |
| plain `http://` | Worker answers 403 (no token sent); `sbx` refuses the URL, exit 99 |
| cap | 3rd id while 2 run (cap 2 then; 8 since 2026-10-23): HTTP 429 with `"ran": false`, nothing started |
| destroy | `ok:true, wasRunning:true, runningAfter:false`; then `health <id>`: `running:false` |

## Limits

| what | value |
| --- | --- |
| size | standard-4 (4 vCPU / 12 GiB / 20 GB) = the account limit per container: `GET /accounts/<id>/containers/me` on 2026-10-21 gave `vcpu_per_deployment 4`, `memory_mib_per_deployment 12288`, `disk_mb_per_deployment 20000`. To grow, ask Cloudflare for a limit raise, then set `INSTANCE_TYPE` in `src/index.ts` (a `{vcpu, memoryMib, diskMb}` object works too). |
| running at once | 8 (`MAX_RUNNING` in `src/index.ts`, kept by the `Registry` Durable Object) |
| idle sleep | 30 min after the last request; code running inside does not count (`sbx wait` keeps it awake) |
| exec output | stdout 8 MiB, stderr 1 MiB; write big output to a file and `sbx get` it |
| one exec | up to 6 h by timeout, but anything over ~1-2 min should be `bg` + `wait` |
| internet | on (apt, rustup, crates.io work) |

## Cost

Billed only while a container runs: one standard-4 is about $0.11/h idle and up to ~$0.40/h
with all 4 vCPU busy (2 at once: up to ~$0.80/h). An idle box bills until it sleeps
(30 min), so **`sbx destroy <id>` when done**, then `sbx health <id>` shows `running:false`.

## Deploy

```bash
cd <ep9 checkout>/infra/x86-sandbox
export CLOUDFLARE_ACCOUNT_ID=6db7cbc0423b5d56af93d13e4e2b469b
npm ci && npm run types && npx tsc --noEmit          # types file is generated, git-ignored
npx wrangler deploy                                  # OrbStack Docker builds the linux/amd64 image
(umask 077; openssl rand -hex 32 > .token)           # first deploy only
npx wrangler secret put SBX_TOKEN --name ep9-x86-sandbox < .token
printf '%s\n' https://ep9-x86-sandbox.lynweklm.workers.dev > .url
```

- Pins (exact): `@cloudflare/sandbox` 1.0.1, wrangler 4.151.0, typescript 6.0.3 (the set the
  webcodecs harness proved). The `cloudflare/sandbox:<tag>` in the Dockerfile must equal the
  npm `@cloudflare/sandbox` version, or `Files` (`/write`) throws `SandboxProtocolError`.
- The scheduling policy of a container app cannot be changed later; a new policy needs a new
  Worker + app name.
- Deploy only when no id runs. A running container keeps its image; a changed Dockerfile is
  used from the next start (after `sbx destroy` or the idle stop).
- First deploy: container calls can fail for a few minutes with `no container instance`;
  `sbx` retries that (36 x 10 s).

## Token rotation

When no id runs (the token only goes through files and stdin, never argv):

```bash
(umask 077; openssl rand -hex 32 > .token.new)
npx wrangler secret put SBX_TOKEN --name ep9-x86-sandbox < .token.new
mv .token.new .token
```

The old token kept working on some requests for ~30-40 s in the webcodecs harness, so wait
~1 min, then check `sbx health` works and nothing started.

## Is anything running?

`sbx health <id>` per id is the truth. The LIVE INSTANCES column of
`wrangler containers list` lags by ~30-60 s. `wrangler containers instances <app id>` lists
every id ever used with its state.

## Files

| file | what |
| --- | --- |
| `src/index.ts` | Worker + `Sandbox` Durable Object + `Registry` Durable Object (cap) |
| `src/secrets.d.ts` | `SBX_TOKEN` type (secrets are not in `wrangler.jsonc`) |
| `wrangler.jsonc` | Worker, container app, Durable Object bindings, `exports` (sqlite) |
| `Dockerfile` | node:24-trixie-slim (by digest) + sandbox-shim 1.0.1 + valgrind, perf, gdb, strace, build-essential, rustup stable minimal |
| `sbx` | client |
| `smoke.sh` | smoke test |
| `tests/push-git-local.sh` | local test of `push-git` and `manifest` (no sandbox, no token) |
