// Tests for the freshness line of ep9 (route P0-BUILDID; M6-ORCH-148, -150). `ep9 --version`
// prints "ep9 build: commit <sha> clean|dirty", or "ep9 build: commit unknown", on stderr
// (crates/ep9_cli/build.rs, src/main.rs); stdout keeps its two lines. cross/run.mjs,
// cross/mem.mjs and jetstream/run.mjs give a run the commit of the binary they measured, not of
// the checkout they run in: null when a binary prints no line, was built dirty, or two binaries
// name two commits, and `cargo xtask m2-exit` reads null as OPEN. The engines are shell-script
// fakes (Unix only, as run.test.mjs); no ep9 build, no shells and no bench checkout are needed.
// usage: node --test tools/bench/cross/buildid.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { cpuModel } from "../lib/host.mjs";
import { buildNote, cleanCommit, combineBuilds, parseBuild, platformId } from "./run.mjs";

const DIR = path.dirname(fileURLToPath(import.meta.url));
const FAKE = path.join(DIR, "testdata/fake-engine.sh");
const JS = path.join(DIR, "..", "jetstream");
const skip = process.platform === "win32" && "shell-script fakes";
const SHA = "0123456789abcdef0123456789abcdef01234567";
const SHA2 = "89abcdef0123456789abcdef0123456789abcdef";

test("parseBuild reads the three forms of the line and nothing else", () => {
  assert.deepEqual(parseBuild(`ep9 build: commit ${SHA} clean\n`), { commit: SHA, dirty: false });
  assert.deepEqual(parseBuild(`warning\nep9 build: commit ${SHA} dirty\n`), { commit: SHA, dirty: true });
  assert.deepEqual(parseBuild("ep9 build: commit unknown\n"), { commit: null, dirty: null });
  for (const bad of ["", "features jit; preset jit", `ep9 build: commit ${SHA.slice(1)} clean`, `ep9 build: commit ${SHA} maybe`, `ep9 build: commit ${SHA.toUpperCase()} clean`, null, undefined]) {
    assert.equal(parseBuild(bad), null, String(bad));
  }
});

test("only a clean build names a commit; several builds must agree", () => {
  assert.equal(cleanCommit({ commit: SHA, dirty: false }), SHA);
  assert.equal(cleanCommit({ commit: SHA, dirty: true }), null);
  assert.equal(cleanCommit({ commit: null, dirty: null }), null);
  assert.equal(cleanCommit(null), null);
  const c = { commit: SHA, dirty: false };
  assert.deepEqual(combineBuilds([c, c]), c);
  assert.deepEqual(combineBuilds([c, { commit: SHA, dirty: true }]), { commit: SHA, dirty: true });
  assert.deepEqual(combineBuilds([c, { commit: SHA2, dirty: false }]), { commit: null, dirty: false });
  assert.equal(combineBuilds([c, null]), null);
  assert.equal(combineBuilds([]), null);
  assert.equal(buildNote({ commit: SHA, dirty: false }), "");
  assert.match(buildNote(null), /prints no build line: OPEN in m2-exit/);
  assert.match(buildNote({ commit: SHA, dirty: true }), /built with uncommitted engine changes/);
});

/**
 * A temp dir with an ep9 fake whose `--version` prints the two stdout lines and the stderr line
 * of FAKE_BUILD ("none": no line), then runs fake-engine.sh; and fake shells.
 */
function setup(t, preset = "jit") {
  const dir = mkdtempSync(path.join(os.tmpdir(), "ep9-buildid-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const n of ["qjs", "node", "d8"]) symlinkSync(FAKE, path.join(dir, n));
  const ep9 = (name, p) => {
    const f = path.join(dir, name);
    writeFileSync(
      f,
      `#!/bin/sh\nif [ "$1" = --version ]; then echo "fake ep9 0.0"; echo "features ${p === "jit" ? "jit" : "none"}; preset ${p}";\n` +
        `  if [ "$FAKE_BUILD" != none ]; then echo "ep9 build: \${FAKE_BUILD:-commit ${SHA} clean}" >&2; fi; exit 0; fi\nexec "${FAKE}" "$@"\n`,
    );
    chmodSync(f, 0o755);
    return f;
  };
  const host = path.join(dir, "host.toml");
  writeFileSync(host, `id = "test"\ncpu = "${cpuModel()}"\ncores = ${os.cpus().length}\nplatform = "${process.platform}-${process.arch}"\nos = "test"\ngate_host = false\nidle_baseline_load1 = 1000\n`);
  const shells = path.join(dir, "versions.txt");
  writeFileSync(shells, `# host: ${platformId()}\nquickjs fake 1 ${dir}/qjs\nnode v0 ${dir}/node\nd8 v0 ${dir}/d8\n`);
  const time = path.join(dir, "fake-time.sh");
  writeFileSync(
    time,
    `#!/bin/sh\nmode=$1; shift; if [ "$mode" = "-f" ]; then shift; fi; out=$2; shift 2; "$@"; st=$?\n` +
      `if [ "$mode" = "-l" ]; then printf '%20s  maximum resident set size\\n' 104857600 > "$out"; else echo 102400 > "$out"; fi; exit $st\n`,
  );
  chmodSync(time, 0o755);
  return { dir, host, shells, time, ep9: ep9("ep9", preset), ep9Jitless: ep9("ep9-jitless", "jitless") };
}

function cross(f, build, extra = []) {
  const out = path.join(f.dir, "cross.json");
  const r = spawnSync(
    process.execPath,
    [path.join(DIR, "run.mjs"), "--ep9", f.ep9, "--shells", f.shells, "--engines", "ep9,qjs", "--workloads", "t0-loop",
      "--settle-s", "0", "--after-s", "0", "--wait-min", "0", "--host", f.host, "--out", out, ...extra],
    { encoding: "utf8", env: { ...process.env, FAKE_BUILD: build } },
  );
  return { r, res: r.status === 0 ? JSON.parse(readFileSync(out, "utf8")) : null };
}

test("cross/run.mjs: commit.ep9 is the binary's clean commit; else null", { skip }, (t) => {
  const f = setup(t);
  let { r, res } = cross(f, `commit ${SHA} clean`);
  assert.equal(r.status, 0, r.stderr);
  assert.equal(res.commit.ep9, SHA);
  assert.deepEqual(res.commit.binary, { commit: SHA, dirty: false });
  assert.deepEqual(res.engines.find((e) => e.id === "ep9").build, { commit: SHA, dirty: false });
  // The runner's checkout commit is kept apart.
  assert.notEqual(res.commit.runner, SHA);
  for (const [build, binary] of [[`commit ${SHA} dirty`, { commit: SHA, dirty: true }], ["commit unknown", { commit: null, dirty: null }], ["none", null]]) {
    ({ r, res } = cross(f, build));
    assert.equal(r.status, 0, r.stderr);
    assert.equal(res.commit.ep9, null, build);
    assert.deepEqual(res.commit.binary, binary, build);
    assert.match(r.stdout, /ep9 commit `unknown` \(the ep9 binary .*OPEN in m2-exit\)/);
  }
  // --commit checks the binary: another commit is a usage error.
  ({ r } = cross(f, `commit ${SHA} clean`, ["--commit", SHA2]));
  assert.equal(r.status, 2, r.stderr);
  assert.match(r.stderr, /--commit 89abcdef.*names commit 01234567/);
  ({ r, res } = cross(f, `commit ${SHA} clean`, ["--commit", SHA]));
  assert.equal(res.commit.ep9, SHA);
});

function mem(f, env, jitless = true) {
  const out = path.join(f.dir, "mem.json");
  const r = spawnSync(
    process.execPath,
    [path.join(DIR, "mem.mjs"), "--ep9", f.ep9, ...(jitless ? ["--ep9-jitless", f.ep9Jitless] : []), "--shells", f.shells,
      "--workloads", "t0-loop", "--settle-s", "0", "--after-s", "0", "--wait-min", "0", "--out", out],
    { encoding: "utf8", env: { ...process.env, EP9_MEM_TIME: f.time, ...env } },
  );
  return { r, res: r.status === 0 ? JSON.parse(readFileSync(out, "utf8")) : null };
}

test("cross/mem.mjs: commit.ep9 is the one clean commit of both ep9 builds; else null", { skip }, (t) => {
  const f = setup(t);
  let { r, res } = mem(f, {});
  assert.equal(r.status, 0, r.stderr);
  assert.equal(res.commit.ep9, SHA);
  assert.deepEqual(res.commit.binary, { commit: SHA, dirty: false });
  assert.deepEqual(res.engines.filter((e) => e.build).map((e) => e.id), ["ep9", "ep9-jitless"]);
  ({ r, res } = mem(f, { FAKE_BUILD: "none" }));
  assert.equal(r.status, 0, r.stderr);
  assert.equal(res.commit.ep9, null);
  assert.equal(res.commit.binary, null);
  ({ r, res } = mem(f, { FAKE_BUILD: `commit ${SHA} dirty` }, false));
  assert.equal(res.commit.ep9, null);
  assert.deepEqual(res.commit.binary, { commit: SHA, dirty: true });
});

function jetstream(f, build) {
  const dir = mkdtempSync(path.join(f.dir, "js-"));
  const list = path.join(dir, "list.txt");
  const out = path.join(dir, "out.json");
  writeFileSync(list, "ok\n");
  const r = spawnSync(
    process.execPath,
    [path.join(JS, "run.mjs"), "--list", list, "--reps", "1", "--out", out, "--config", `a=ep9@${f.ep9}:${path.join(JS, "testdata/fake-ep9.mjs")}`],
    { encoding: "utf8", env: { ...process.env, FAKE_BUILD: build, EP9_JETSTREAM_DIR: path.join(JS, "testdata/fake-jetstream") } },
  );
  return { r, res: JSON.parse(readFileSync(out, "utf8")) };
}

test("jetstream/run.mjs: with ep9 configs, ep9.commit is their binary's clean commit; else null", { skip }, (t) => {
  const f = setup(t);
  // This fake ep9 runs node, so the JetStream realm runs (fake-ep9.mjs needs node).
  writeFileSync(
    f.ep9,
    `#!/bin/sh\nif [ "$1" = --version ]; then echo "fake ep9 0.0"; echo "features jit; preset jit";\n` +
      `  if [ "$FAKE_BUILD" != none ]; then echo "ep9 build: $FAKE_BUILD" >&2; fi; exit 0; fi\nexec "${process.execPath}" "$@"\n`,
  );
  let { res } = jetstream(f, `commit ${SHA} clean`);
  assert.equal(res.ep9.commit, SHA);
  assert.deepEqual(res.ep9.binary, [{ commit: SHA, dirty: false }]);
  assert.match(res.ep9.runner, /^[0-9a-f]{40}$/);
  assert.equal(res.complete, true);
  assert.deepEqual(res.configs[0].build, { commit: SHA, dirty: false });
  for (const build of [`commit ${SHA} dirty`, "commit unknown", "none"]) {
    ({ res } = jetstream(f, build));
    assert.equal(res.ep9.commit, null, build);
  }
});
