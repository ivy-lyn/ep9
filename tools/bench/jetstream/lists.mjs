// Writes expectations/jetstream-m7.txt and expectations/jetstream-m2.txt from the pinned JetStream.
//
// usage: node tools/bench/jetstream/lists.mjs [--check]
//   --check: exit 1 if the committed files differ from what this script makes.
//
// Rules (docs/plan/09-milestones.md M2 exit 7 and M7 exit 3; 10-quality section 2):
//   M7 list = JetStream "default" workloads tagged "js", without "WorkerTests" and "internationalization".
//   M2 list = M7 list without workloads whose own code uses a feature that the plan schedules after
//             M2 (M3 compliance, M4 runtime, M8 Wasm). The scan lexes each workload .js file and
//             each preloaded dist/ bundle.
//             A hit can be a feature test only; the scan then drops the workload anyway (safe side).
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const js = path.join(root, "third_party/bench-jetstream");
const pin = fs.readFileSync(path.join(root, "third_party/bench-jetstream.commit"), "utf8").trim();
const host = path.join(root, "tools/bench/jetstream/node-host.mjs");
const all = JSON.parse(
  execFileSync(process.execPath, [host, js, "--ep9-dump-benchmarks"], { encoding: "utf8", maxBuffer: 1 << 26 }),
);

// A small JS lexer: it drops comments, strings, template text and regex literals, so that
// words inside them (ROT13 data, a tokenizer's own regexes) do not count. It keeps template
// substitutions. Regex vs division: "/" starts a regex unless the previous token ends an operand.
function tokens(src) {
  const out = [];
  const stack = []; // brace depth of each open template substitution
  let depth = 0;
  let i = 0;
  const n = src.length;
  const operandEnd = () => {
    const t = out[out.length - 1];
    if (!t) return false;
    if (t.k === "num" || t.k === "str" || t.k === "re") return true;
    if (t.k === "id") return !/^(?:return|typeof|instanceof|in|of|new|delete|void|throw|case|do|else|yield|await)$/.test(t.v);
    return t.v === ")" || t.v === "]" || t.v === "}";
  };
  const skipQuoted = (q) => {
    for (i++; i < n && src[i] !== q; i++) if (src[i] === "\\") i++;
    i++;
  };
  // Template text from i (after ` or }) to the closing ` or to the next ${.
  const template = () => {
    for (; i < n; i++) {
      if (src[i] === "\\") { i++; continue; }
      if (src[i] === "`") { i++; out.push({ k: "str", v: "" }); return; }
      if (src[i] === "$" && src[i + 1] === "{") { i += 2; stack.push(depth); depth = 0; out.push({ k: "p", v: "(" }); return; }
    }
  };
  while (i < n) {
    const c = src[i];
    if (/\s/.test(c)) { i++; continue; }
    if (c === "/" && src[i + 1] === "/") { while (i < n && src[i] !== "\n") i++; continue; }
    if (c === "/" && src[i + 1] === "*") { const e = src.indexOf("*/", i + 2); i = e < 0 ? n : e + 2; continue; }
    if (c === "'" || c === '"') { skipQuoted(c); out.push({ k: "str", v: "" }); continue; }
    if (c === "`") { i++; template(); continue; }
    if (c === "/" && !operandEnd()) {
      let cls = false;
      for (i++; i < n; i++) {
        if (src[i] === "\\") { i++; continue; }
        if (src[i] === "[") cls = true;
        else if (src[i] === "]") cls = false;
        else if (src[i] === "/" && !cls) break;
        else if (src[i] === "\n") break;
      }
      i++;
      while (i < n && /[\w$]/.test(src[i])) i++;
      out.push({ k: "re", v: "" });
      continue;
    }
    if (/[0-9]/.test(c) || (c === "." && /[0-9]/.test(src[i + 1] ?? ""))) {
      const m = /^(?:0[xXbBoO][0-9a-fA-F_]+n?|[0-9][0-9_]*n|(?:[0-9][0-9_]*)?\.?[0-9_]*(?:[eE][+-]?[0-9_]+)?)/.exec(src.slice(i, i + 64));
      i += Math.max(1, m[0].length);
      out.push({ k: "num", v: m[0] });
      continue;
    }
    if (/[\p{ID_Start}$_\\]/u.test(c)) {
      const m = /^[\p{ID_Continue}$\u200c\u200d\\]+/u.exec(src.slice(i, i + 256));
      i += m[0].length;
      out.push({ k: "id", v: m[0] });
      continue;
    }
    if (c === "{") depth++;
    if (c === "}") {
      if (depth === 0 && stack.length) { depth = stack.pop(); i++; out.push({ k: "p", v: ")" }); template(); continue; }
      depth--;
    }
    out.push({ k: "p", v: c });
    i++;
  }
  return out;
}

// Features that the plan schedules after M2 (09-milestones section 2.2: M3 compliance, M4
// runtime, M8 Wasm). Each test sees the token list t and an index j of an identifier.
const TYPED = /^(?:Int8|Uint8|Uint8Clamped|Int16|Uint16|Int32|Uint32|Float16|Float32|Float64|BigInt64|BigUint64)Array$|^ArrayBuffer$|^DataView$/;
const at = (t, j, v) => t[j]?.v === v;
const POST_M2 = [
  ["TypedArray/ArrayBuffer/DataView (M3)", (t, j) => t[j].k === "id" && TYPED.test(t[j].v)],
  ["BigInt (M3)", (t, j) => (t[j].k === "id" && t[j].v === "BigInt") || (t[j].k === "num" && t[j].v.endsWith("n"))],
  ["Proxy (M3)", (t, j) => at(t, j, "Proxy") && (at(t, j - 1, "new") || at(t, j + 1, "."))],
  ["WeakRef/FinalizationRegistry (M3)", (t, j) => t[j].k === "id" && /^(?:WeakRef|FinalizationRegistry)$/.test(t[j].v)],
  ["SharedArrayBuffer/Atomics (M3)", (t, j) => at(t, j, "SharedArrayBuffer") || (at(t, j, "Atomics") && at(t, j + 1, "."))],
  ["Intl (M3)", (t, j) => at(t, j, "Intl") && at(t, j + 1, ".")],
  ["Temporal (M3)", (t, j) => at(t, j, "Temporal") && at(t, j + 1, ".")],
  ["dynamic import (M3 modules)", (t, j) => at(t, j, "import") && at(t, j + 1, "(")],
  ["Worker (M4)", (t, j) => at(t, j, "Worker") && at(t, j - 1, "new")],
  ["WebAssembly (M8)", (t, j) => at(t, j, "WebAssembly")],
];

// .js.z files are zlib-deflated sources (utils/compress.mjs).
const source = (f) => {
  const bytes = fs.readFileSync(path.join(js, f));
  return (f.endsWith(".z") ? zlib.inflateSync(bytes) : bytes).toString("utf8");
};

const isM7 = (b) => {
  const t = new Set(b.tags);
  return t.has("default") && t.has("js") && !t.has("workertests") && !t.has("internationalization");
};
// Code that runs: the workload files, plus preloaded bundles under dist/ (startup workloads and
// the Web Tooling Benchmark eval them). Other preloads are data: parser inputs, JSON, CSS.
const filesOf = (b) =>
  b.group
    ? b.group.flatMap(filesOf)
    : [...b.files.filter((f) => /\.m?js(?:\.z)?$/.test(f)), ...b.preload.filter((f) => /\/dist\/.*\.m?js$/.test(f))];
const featuresOf = (b) => {
  const hits = new Set();
  for (const f of filesOf(b)) {
    const t = tokens(source(f));
    for (let j = 0; j < t.length; j++) for (const [name, test] of POST_M2) if (test(t, j)) hits.add(name);
  }
  return [...hits].sort();
};

const m7 = all.filter(isM7).map((b) => ({ name: b.name, feats: featuresOf(b) }));
m7.sort((a, b) => a.name.localeCompare(b.name));
const m2 = m7.filter((b) => b.feats.length === 0);

const head = (title, rule) =>
  [
    `# ${title}`,
    `# Generated by tools/bench/jetstream/lists.mjs from WebKit/JetStream ${pin} (branch JetStream3.0,`,
    `# third_party/bench-jetstream.commit). Do not edit by hand: change the script and re-run it.`,
    ...rule.map((l) => `# ${l}`),
    `# One JetStream workload name per line (cli.js --test=<name>).`,
  ].join("\n");

const m7Text = [
  head("JetStream 3 M7 list (G4; docs/plan/09-milestones.md M7 exit 3)", [
    "Rule: JetStream \"default\" workloads tagged \"js\", without \"WorkerTests\" and \"internationalization\".",
    "Gate: ep9 T2 score ratio >= 0.75 x (node --max-opt=2 / node full) on this list (est.).",
    `Count: ${m7.length}`,
  ]),
  ...m7.map((b) => b.name),
  "",
].join("\n");

const m2Text = [
  head("JetStream 3 M2 list (docs/plan/09-milestones.md M2 exit 7: it runs; ratio to node --jitless recorded)", [
    "Rule: the M7 list without workloads whose code (.js files and preloaded dist/ bundles) uses a feature that the plan schedules",
    "after M2 (M3: TypedArray, BigInt, Proxy, WeakRef, SAB/Atomics, Intl, Temporal, modules; M4: Worker;",
    "M8: WebAssembly). The scan lexes the code (no strings, comments or regexes) and is conservative:",
    "a feature test such as `typeof WeakRef` also drops a workload.",
    "Note: the harness itself puts `new Proxy({}, { get })` into every workload realm (JetStreamDriver.js,",
    "class Scripts). So these workloads need a minimal Proxy (get trap) at M2, or a local harness patch.",
    `Count: ${m2.length} of ${m7.length}`,
  ]),
  ...m2.map((b) => b.name),
  "",
  "# Dropped from the M7 list, with the features found:",
  ...m7.filter((b) => b.feats.length).map((b) => `#   ${b.name}: ${b.feats.join(", ")}`),
  "",
].join("\n");

const out = [
  [path.join(root, "expectations/jetstream-m7.txt"), m7Text],
  [path.join(root, "expectations/jetstream-m2.txt"), m2Text],
];
if (process.argv.includes("--check")) {
  let ok = true;
  for (const [f, text] of out) {
    const now = fs.existsSync(f) ? fs.readFileSync(f, "utf8") : "";
    if (now !== text) {
      console.error(`stale: ${path.relative(root, f)}`);
      ok = false;
    }
  }
  process.exit(ok ? 0 : 1);
}
fs.mkdirSync(path.join(root, "expectations"), { recursive: true });
for (const [f, text] of out) fs.writeFileSync(f, text);
console.log(`M7 list: ${m7.length} workloads; M2 list: ${m2.length} workloads`);
