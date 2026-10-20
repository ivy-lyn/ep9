// Count how "typed" real TypeScript code is, from the codegen point of view.
// Usage: node analyze.js <project>   (projects defined below). Uses typescript@6.0.3 checker API.
"use strict";
const path = require("path");
const fs = require("fs");
const W = require("path").resolve(__dirname, "../../ts-type-trust");
const ts = require(path.join(W, "node_modules/typescript"));
const R = path.resolve(W, "../../repos");
const TYPEROOTS = [path.join(W, "deps/node_modules/@types")];

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
    return { roots: walkFiles(path.join(R, "pixijs/src"), isSrcTs), decls: [...walkFiles(path.join(R, "pixijs/src"), (p) => p.endsWith(".d.ts")), path.join(W, "deps/node_modules/@webgpu/types/dist/index.d.ts")], options: c.options };
  },
  vscode: () => ({
    roots: ["src/vs/base/common", "src/vs/editor/common"].flatMap((d) => walkFiles(path.join(R, "vscode", d), isSrcTs)),
    decls: [...walkFiles(path.join(R, "vscode/src/typings"), (p) => p.endsWith(".d.ts")), ...walkFiles(path.join(R, "vscode/src/vs/base/common"), (p) => p.endsWith(".d.ts"))],
    options: { target: ts.ScriptTarget.ES2022, lib: ["lib.es2024.d.ts", "lib.esnext.disposable.d.ts", "lib.dom.d.ts", "lib.dom.iterable.d.ts"], module: ts.ModuleKind.NodeNext,
      moduleResolution: ts.ModuleResolutionKind.NodeNext, moduleDetection: ts.ModuleDetectionKind.Legacy, experimentalDecorators: true, strict: true,
      useUnknownInCatchVariables: false, useDefineForClassFields: false, types: [], skipLibCheck: true, noEmit: true },
  }),
};

// ---- typed scan: classify the TARGET of computed-key writes `t[k] = v` and of reflective mutations
const name = process.argv[2];
const { roots, options, decls = [] } = PROJECTS[name]();
const program = ts.createProgram(roots.concat(decls), options);
const checker = program.getTypeChecker();
const rootSet = new Set(roots.map((f) => path.resolve(f)));
const files = program.getSourceFiles().filter((sf) => rootSet.has(path.resolve(sf.fileName)));
const F = ts.TypeFlags, OF = ts.ObjectFlags, SF = ts.SymbolFlags;

const inc = (o, k, n = 1) => { o[k] = (o[k] || 0) + n; };
const isLibDecl = (d) => { const sf = d.getSourceFile(); return program.isSourceFileDefaultLibrary(sf) || sf.isDeclarationFile; };
const symIsLib = (s) => !!(s && s.declarations && s.declarations.length && s.declarations.every(isLibDecl));
const INT32 = (v) => Number.isInteger(v) && v >= -2147483648 && v <= 2147483647 && !Object.is(v, -0);

function objKind(t) {
  if (checker.isTupleType(t)) return "array/tuple";
  if (checker.isArrayType(t)) return "array/tuple";
  const of = t.objectFlags || 0;
  const sym = t.getSymbol();
  const ctor = t.getConstructSignatures().length > 0;
  if (ctor && sym && sym.flags & SF.Class) return "class-ctor(static)";
  if (sym && sym.flags & (SF.ValueModule | SF.NamespaceModule)) return "namespace";
  if (sym && sym.flags & SF.Enum) return "enum-object";
  if (sym && sym.flags & SF.Class) return symIsLib(sym) ? "lib-object" : "class-instance";
  if (sym && sym.flags & SF.Interface) return symIsLib(sym) ? "lib-object" : "interface";
  if (t.getCallSignatures().length > 0 && t.getProperties().length === 0) return "function";
  if (sym && sym.flags & (SF.Function | SF.Method)) return "function";
  if (of & OF.Mapped) return "structural";
  if (sym && sym.flags & (SF.TypeLiteral | SF.ObjectLiteral)) return "structural";
  if (ctor) return "class-ctor(static)";
  return "structural";
}

// categorise one type into a codegen "representation bucket"
function kind(t, depth = 0) {
  const f = t.flags;
  if (f & F.Any) return t.intrinsicName === "error" ? "error" : "any";
  if (f & F.Unknown) return "unknown";
  if (f & F.Never) return "never";
  if (f & (F.Undefined | F.Null | F.Void)) return "nullish";
  if (f & F.NumberLiteral) return INT32(t.value) ? (f & F.EnumLiteral ? "int(enum-member)" : "int(const)") : "number";
  if (f & F.Enum) return "int(enum)";
  if (f & F.Number) return "number";
  if (f & F.BigIntLike) return "bigint";
  if (f & F.StringLike) return "string";
  if (f & F.BooleanLike) return "boolean";
  if (f & F.ESSymbolLike) return "symbol";
  if (f & F.TypeParameter) {
    if (t.isThisType) { const c = checker.getBaseConstraintOfType(t); if (c && c !== t) return kind(c, depth + 1); }
    return "generic";
  }
  if (f & (F.IndexedAccess | F.Conditional | F.Substitution | F.Index)) return "generic";
  if (f & F.NonPrimitive) return "structural";
  if (f & F.Intersection) return "structural";
  if (f & F.Union) {
    if (depth > 2) return "union-mixed";
    const ks = new Set();
    let nullish = false;
    for (const m of t.types) { const k = kind(m, depth + 1); if (k === "nullish") nullish = true; else ks.add(k.replace(/^nullable /, "")); }
    // collapse int literal + enum + number into numeric families
    if (ks.size >= 1 && [...ks].every((k) => k === "int(enum-member)" || k === "int(enum)") && t.types.filter((m) => !(m.flags & (F.Undefined | F.Null | F.Void))).length > 1) { ks.clear(); ks.add("int(enum)"); }
    else if (ks.size >= 1 && [...ks].every((k) => k.startsWith("int(")) && t.types.filter((m) => !(m.flags & (F.Undefined | F.Null | F.Void))).length > 1) { ks.clear(); ks.add("int(literal-union)"); }
    if (ks.size > 1 && [...ks].every((k) => k.startsWith("int(") || k === "number")) { ks.clear(); ks.add("number"); }
    if (ks.size === 0) return "nullish";
    if (ks.size === 1) { const k = [...ks][0]; return nullish ? "nullable " + k : k; }
    const objs = ["class-instance", "interface", "structural", "array/tuple", "lib-object", "function", "class-ctor(static)"];
    const all = [...ks];
    const tag = all.every((k) => objs.includes(k)) ? "union-objects" : all.every((k) => !objs.includes(k)) ? "union-primitives" : "union-obj+prim";
    return nullish ? "nullable " + tag : tag;
  }
  if (f & F.Object) return objKind(t);
  return "other";
}
const isLitKey = (e) => e && (ts.isStringLiteralLike(e) || ts.isNumericLiteral(e));
const isArrayLike = (t) => {
  const k = kind(t);
  if (k === "array/tuple" || k === "nullable array/tuple") return true;
  const s = t.getSymbol();
  return !!(s && /^(Int8|Uint8|Uint8Clamped|Int16|Uint16|Int32|Uint32|Float16|Float32|Float64|BigInt64|BigUint64)Array$/.test(s.name));
};
const out = { project: name, dynWrites: 0, byTarget: {}, reflect: 0, reflectByTarget: {}, deleteByTarget: {} };
const bucket = (t) => {
  if (isArrayLike(t)) return "array/typed-array";
  let k = kind(t).replace(/^nullable /, "");
  if (k === "class-instance") return "class-instance";
  if (k === "any" || k === "unknown" || k === "error" || k === "generic") return "any/unknown/generic";
  if (k === "lib-object") return "lib-object (Map/DOM/...)";
  return "interface/structural/record/other";
};
for (const sf of files) {
  const visit = (n) => {
    if (ts.isBinaryExpression(n) && n.operatorToken.kind >= ts.SyntaxKind.FirstAssignment && n.operatorToken.kind <= ts.SyntaxKind.LastAssignment
        && ts.isElementAccessExpression(n.left) && !isLitKey(n.left.argumentExpression)) {
      out.dynWrites++;
      const b = bucket(checker.getTypeAtLocation(n.left.expression));
      out.byTarget[b] = (out.byTarget[b] || 0) + 1;
    }
    if (ts.isDeleteExpression(n) && (ts.isElementAccessExpression(n.expression) || ts.isPropertyAccessExpression(n.expression))) {
      const b = bucket(checker.getTypeAtLocation(n.expression.expression));
      out.deleteByTarget[b] = (out.deleteByTarget[b] || 0) + 1;
    }
    if (ts.isCallExpression(n) && ts.isPropertyAccessExpression(n.expression) && ts.isIdentifier(n.expression.expression)
        && ["Object", "Reflect"].includes(n.expression.expression.text)
        && ["defineProperty", "defineProperties", "setPrototypeOf", "assign", "freeze", "seal", "preventExtensions", "deleteProperty"].includes(n.expression.name.text)
        && n.arguments.length) {
      out.reflect++;
      const b = bucket(checker.getTypeAtLocation(n.arguments[0]));
      out.reflectByTarget[b] = (out.reflectByTarget[b] || 0) + 1;
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
console.log(JSON.stringify(out));
