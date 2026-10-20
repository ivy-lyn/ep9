// Write one tsconfig.json + roots.txt per repo for the tsrs-based region finder.
// Project roots and compiler options mirror the R12 harness (work/ts-type-trust/analyze.js) so the
// residual-diagnostic counts stay comparable; AWFY-TS and Preact mirror the R13 oracle corpus.
"use strict";
const path = require("path");
const fs = require("fs");
const S = "$EP9_RESEARCH";
const TT = path.join(S, "work/ts-type-trust");
const ts = require(path.join(TT, "node_modules/typescript"));
const R = path.join(S, "repos");
const OUT = path.join(S, "work/sound-regions/finder/projects");
const TYPEROOTS = [path.join(TT, "deps/node_modules/@types")];

function walkFiles(dir, pred, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (!/node_modules|__tests__|^tests?$|^dist$|^test-utils$/.test(e.name)) walkFiles(p, pred, out); }
    else if (pred(p)) out.push(p);
  }
  return out;
}
const isSrcTs = (p) => /\.m?ts$/.test(p) && !/\.d\.m?ts$/.test(p) && !/\.(test|spec|bench|tp)\.m?ts$/.test(p) && !/\/(test|tests|benchmark)\.ts$/.test(p);
function fromTsconfig(cfgPath, overrides, filter) {
  const host = { ...ts.sys, onUnRecoverableConfigFileDiagnostic: (d) => { throw new Error(ts.flattenDiagnosticMessageText(d.messageText, "\n")); } };
  const pc = ts.getParsedCommandLineOfConfigFile(cfgPath, overrides, host);
  const roots = pc.fileNames.filter(filter);
  return { roots, options: { ...pc.options, ...overrides } };
}
const PROJECTS = {
  typescript: () => fromTsconfig(path.join(R, "TypeScript/src/compiler/tsconfig.json"),
    { typeRoots: TYPEROOTS, composite: false, noEmit: true, declaration: false, emitDeclarationOnly: false, declarationMap: false, isolatedDeclarations: false },
    (f) => f.includes("/src/compiler/") && isSrcTs(f) && !f.endsWith(".generated.ts")),
  zod: () => ({
    roots: ["core", "classic", "mini"].flatMap((d) => walkFiles(path.join(R, "zod/packages/zod/src/v4", d), isSrcTs)),
    options: { target: ts.ScriptTarget.ES2020, lib: ["lib.es2020.d.ts", "lib.dom.d.ts"], module: ts.ModuleKind.NodeNext, moduleResolution: ts.ModuleResolutionKind.NodeNext,
      strict: true, exactOptionalPropertyTypes: true, types: [], skipLibCheck: true, noEmit: true, customConditions: ["@zod/source"], esModuleInterop: true, isolatedModules: true },
  }),
  datefns: () => ({
    roots: walkFiles(path.join(R, "date-fns/pkgs/core/src"), (p) => isSrcTs(p) && (!p.includes("/locale/") || p.includes("/locale/en-US/") || p.endsWith("/locale/index.ts") || p.endsWith("/locale/types.ts") || p.includes("/locale/_lib/"))),
    options: { target: ts.ScriptTarget.ES2020, lib: ["lib.esnext.d.ts", "lib.dom.d.ts"], module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler,
      strict: true, types: ["node"], typeRoots: TYPEROOTS, skipLibCheck: true, noEmit: true, allowImportingTsExtensions: true },
  }),
  hono: () => fromTsconfig(path.join(R, "hono/tsconfig.build.json"),
    { typeRoots: TYPEROOTS, composite: false, noEmit: true, declaration: false },
    (f) => isSrcTs(f) && !f.endsWith(".tsx")),
  pixijs: () => {
    const c = fromTsconfig(path.join(R, "pixijs/tsconfig.json"), { typeRoots: TYPEROOTS, noEmit: true }, () => false);
    return { roots: walkFiles(path.join(R, "pixijs/src"), isSrcTs), decls: [...walkFiles(path.join(R, "pixijs/src"), (p) => p.endsWith(".d.ts")), path.join(TT, "deps/node_modules/@webgpu/types/dist/index.d.ts")], options: c.options };
  },
  vscode: () => ({
    roots: ["src/vs/base/common", "src/vs/editor/common"].flatMap((d) => walkFiles(path.join(R, "vscode", d), isSrcTs)),
    decls: [...walkFiles(path.join(R, "vscode/src/typings"), (p) => p.endsWith(".d.ts")), ...walkFiles(path.join(R, "vscode/src/vs/base/common"), (p) => p.endsWith(".d.ts"))],
    options: { target: ts.ScriptTarget.ES2022, lib: ["lib.es2024.d.ts", "lib.esnext.disposable.d.ts", "lib.dom.d.ts", "lib.dom.iterable.d.ts"], module: ts.ModuleKind.NodeNext,
      moduleResolution: ts.ModuleResolutionKind.NodeNext, moduleDetection: ts.ModuleDetectionKind.Legacy, experimentalDecorators: true, strict: true,
      useUnknownInCatchVariables: false, useDefineForClassFields: false, types: [], skipLibCheck: true, noEmit: true },
  }),
};

const ENUMS = { target: ts.ScriptTarget, module: ts.ModuleKind, moduleResolution: ts.ModuleResolutionKind, moduleDetection: ts.ModuleDetectionKind, jsx: ts.JsxEmit, newLine: ts.NewLineKind };
const DROP = new Set(["newLine", "configFilePath", "pathsBasePath", "composite", "declaration", "declarationMap", "emitDeclarationOnly", "isolatedDeclarations", "outDir", "rootDir", "rootDirs", "tsBuildInfoFile", "incremental", "sourceMap", "inlineSources", "declarationDir", "outFile", "watch", "build", "plugins"]);
function toJson(options) {
  const o = {};
  for (const [k, v] of Object.entries(options)) {
    if (DROP.has(k) || v === undefined) continue;
    if (ENUMS[k]) { o[k] = Object.keys(ENUMS[k]).find((n) => ENUMS[k][n] === v && isNaN(+n)); continue; }
    if (k === "paths" && options.pathsBasePath) { o[k] = Object.fromEntries(Object.entries(v).map(([a, b]) => [a, b.map((x) => path.resolve(options.pathsBasePath, x))])); continue; }
    if (k === "lib") { o[k] = v.map((l) => path.basename(l).replace(/^lib\./, "").replace(/\.d\.ts$/, "")); continue; }
    o[k] = v;
  }
  o.noEmit = true;
  return o;
}

fs.mkdirSync(OUT, { recursive: true });
for (const [name, mk] of Object.entries(PROJECTS)) {
  const { roots, options, decls = [] } = mk();
  const dir = path.join(OUT, name);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "tsconfig.json"), JSON.stringify({ compilerOptions: toJson(options), files: roots.concat(decls) }, null, 1));
  fs.writeFileSync(path.join(dir, "roots.txt"), roots.map((r) => path.resolve(r)).join("\n") + "\n");
  console.log(name, roots.length, "roots", decls.length, "decls", JSON.stringify(toJson(options)).slice(0, 400));
}
// AWFY-TS (R13 typed port) and Preact (JSDoc JS, R13): copy the R13 corpus configs with absolute paths.
const OR = path.join(S, "work/tsrs-type-oracle/corpus");
{
  const src = path.join(OR, "awfy-ts/src");
  const roots = fs.readdirSync(src).filter((f) => f.endsWith(".ts")).map((f) => path.join(src, f));
  const cfg = JSON.parse(fs.readFileSync(path.join(OR, "awfy-ts/tsconfig.json"), "utf8"));
  delete cfg.include; cfg.files = roots;
  fs.mkdirSync(path.join(OUT, "awfy"), { recursive: true });
  fs.writeFileSync(path.join(OUT, "awfy/tsconfig.json"), JSON.stringify(cfg, null, 1));
  fs.writeFileSync(path.join(OUT, "awfy/roots.txt"), roots.join("\n") + "\n");
  console.log("awfy", roots.length);
}
{
  const roots = walkFiles(path.join(R, "preact/src"), (p) => p.endsWith(".js"));
  const decls = walkFiles(path.join(R, "preact/src"), (p) => p.endsWith(".d.ts"));
  const cfg = JSON.parse(fs.readFileSync(path.join(OR, "preact/tsconfig.json"), "utf8"));
  delete cfg.include; cfg.files = roots.concat(decls);
  fs.mkdirSync(path.join(OUT, "preact"), { recursive: true });
  fs.writeFileSync(path.join(OUT, "preact/tsconfig.json"), JSON.stringify(cfg, null, 1));
  fs.writeFileSync(path.join(OUT, "preact/roots.txt"), roots.join("\n") + "\n");
  console.log("preact", roots.length, decls.length);
}
