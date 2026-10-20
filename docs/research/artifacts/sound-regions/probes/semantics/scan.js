// Parse-only scan (typescript@6.0.3 parser, no checker) of real TS code for constructs that a
// sound static region finder must treat as breakers. Counts per KLOC of code lines.
// Usage: node scan.js   -> prints a table; writes scan.json
"use strict";
const path = require("path");
const fs = require("fs");
const W = path.resolve(__dirname, "../../ts-type-trust");
const ts = require(path.join(W, "node_modules/typescript"));
const R = path.resolve(__dirname, "../../../repos");

function walk(dir, pred, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (!/node_modules|__tests__|^tests?$|^dist$|^test-utils$|^benchmark/.test(e.name)) walk(p, pred, out); }
    else if (pred(p)) out.push(p);
  }
  return out;
}
const isSrc = (p) => /\.(m?ts|tsx|m?js|jsx)$/.test(p) && !/\.d\.m?ts$/.test(p) && !/\.(test|spec|bench|tp)\.\w+$/.test(p) && !/\/(test|tests|benchmark)\.ts$/.test(p) && !/\.generated\.ts$/.test(p);
const PROJECTS = {
  "ts-compiler": () => walk(path.join(R, "TypeScript/src/compiler"), isSrc),
  zod: () => ["core", "classic", "mini"].flatMap((d) => walk(path.join(R, "zod/packages/zod/src/v4", d), isSrc)),
  "date-fns": () => walk(path.join(R, "date-fns/pkgs/core/src"), (p) => isSrc(p) && (!p.includes("/locale/") || p.includes("/locale/en-US/") || p.includes("/locale/_lib/"))),
  hono: () => walk(path.join(R, "hono/src"), (p) => isSrc(p) && !p.endsWith(".tsx")),
  pixijs: () => walk(path.join(R, "pixijs/src"), isSrc),
  vscode: () => ["src/vs/base/common", "src/vs/editor/common"].flatMap((d) => walk(path.join(R, "vscode", d), isSrc)),
  preact: () => walk(path.join(R, "preact/src"), isSrc),
};

const REFLECT_MUT = new Set(["defineProperty", "defineProperties", "setPrototypeOf", "freeze", "seal", "preventExtensions", "assign", "deleteProperty", "set"]);
function codeLines(sf) {
  // token lines via the parser's own token view (getChildren), so templates and regexes scan correctly
  const lines = new Set();
  const rec = (n) => { const ch = n.getChildren(sf); if (ch.length === 0) { if (n.kind !== ts.SyntaxKind.EndOfFileToken) { lines.add(sf.getLineAndCharacterOfPosition(n.getStart(sf)).line); lines.add(sf.getLineAndCharacterOfPosition(n.end).line); } } else ch.forEach(rec); };
  rec(sf);
  return lines.size;
}
const isLitKey = (e) => e && (ts.isStringLiteralLike(e) || ts.isNumericLiteral(e) || (ts.isPrefixUnaryExpression(e) && ts.isNumericLiteral(e.operand)));
const calleeName = (e) => (ts.isPropertyAccessExpression(e) ? `${ts.isIdentifier(e.expression) ? e.expression.text : "?"}.${e.name.text}` : ts.isIdentifier(e) ? e.text : "");

function scanProject(name, files) {
  const c = { files: files.length, loc: 0, dynKeyWrite: 0, dynKeyWriteIdxNum: 0, litKeyWrite: 0, delete: 0, reflectMut: 0, protoPatch: 0, protoAssignWhole: 0,
    dunderProto: 0, evalLike: 0, withStmt: 0, proxy: 0, argumentsUse: 0, callApplyBind: 0, forIn: 0, objectAssignAny: 0,
    classes: 0, classesExported: 0, classesExtends: 0, classesDecorated: 0, classesThisEscapeInCtor: 0, classesProtoPatched: 0, filesWithReflectMut: 0, filesWithDynKeyWrite: 0 };
  const classNames = new Map(); // name -> count of declarations
  const patchedNames = new Set();
  for (const f of files) {
    const text = fs.readFileSync(f, "utf8");
    const kind = /\.tsx$/.test(f) ? ts.ScriptKind.TSX : /\.jsx?$/.test(f) ? ts.ScriptKind.JSX : ts.ScriptKind.TS;
    const sf = ts.createSourceFile(f, text, ts.ScriptTarget.Latest, true, kind);
    c.loc += codeLines(sf);
    let fileReflect = false, fileDyn = false;
    const exportedNames = new Set();
    const localClasses = [];
    const visit = (n, inCtorOf) => {
      if (ts.isBinaryExpression(n) && n.operatorToken.kind >= ts.SyntaxKind.FirstAssignment && n.operatorToken.kind <= ts.SyntaxKind.LastAssignment) {
        const l = n.left;
        if (ts.isElementAccessExpression(l)) {
          if (isLitKey(l.argumentExpression)) c.litKeyWrite++;
          else { c.dynKeyWrite++; fileDyn = true; }
        }
        // X.prototype.m = ... / X.prototype = ...
        if (ts.isPropertyAccessExpression(l) || ts.isElementAccessExpression(l)) {
          const tgt = l.expression;
          if (ts.isPropertyAccessExpression(tgt) && tgt.name.text === "prototype") { c.protoPatch++; if (ts.isIdentifier(tgt.expression)) patchedNames.add(tgt.expression.text); }
          if (ts.isPropertyAccessExpression(l) && l.name.text === "prototype") c.protoAssignWhole++;
          if (ts.isPropertyAccessExpression(l) && l.name.text === "__proto__") c.dunderProto++;
        }
      }
      if (ts.isDeleteExpression(n)) c.delete++;
      if (ts.isCallExpression(n) || ts.isNewExpression(n)) {
        const nm = n.expression ? calleeName(n.expression) : "";
        const [obj, meth] = nm.split(".");
        if ((obj === "Object" || obj === "Reflect") && REFLECT_MUT.has(meth) && !(obj === "Reflect" && meth === "assign") && !(obj === "Object" && (meth === "set" || meth === "deleteProperty"))) {
          c.reflectMut++; fileReflect = true;
          if (meth === "assign") c.objectAssignAny++;
          const a0 = n.arguments && n.arguments[0];
          if (a0 && ts.isPropertyAccessExpression(a0) && a0.name.text === "prototype" && ts.isIdentifier(a0.expression)) { c.protoPatch++; patchedNames.add(a0.expression.text); }
        }
        if (nm === "eval" || nm === "Function" || (ts.isNewExpression(n) && nm === "Function")) c.evalLike++;
        if (nm === "Proxy" && ts.isNewExpression(n)) c.proxy++;
        if (ts.isPropertyAccessExpression(n.expression) && ["call", "apply", "bind"].includes(n.expression.name.text)) c.callApplyBind++;
        if (inCtorOf && n.arguments && n.arguments.some((a) => a.kind === ts.SyntaxKind.ThisKeyword)) inCtorOf.thisEscape = true;
      }
      if (ts.isWithStatement(n)) c.withStmt++;
      if (ts.isForInStatement(n)) c.forIn++;
      if (ts.isIdentifier(n) && n.text === "arguments" && !(ts.isPropertyAccessExpression(n.parent) && n.parent.name === n)) c.argumentsUse++;
      if (inCtorOf && ts.isBinaryExpression(n) && n.operatorToken.kind === ts.SyntaxKind.EqualsToken && n.right.kind === ts.SyntaxKind.ThisKeyword) inCtorOf.thisEscape = true;
      if (ts.isClassDeclaration(n) || ts.isClassExpression(n)) {
        const rec = { name: n.name ? n.name.text : null, exported: false, extends: false, decorated: false, thisEscape: false };
        const mods = ts.canHaveModifiers(n) ? ts.getModifiers(n) || [] : [];
        if (mods.some((m) => m.kind === ts.SyntaxKind.ExportKeyword)) rec.exported = true;
        if ((ts.canHaveDecorators(n) && (ts.getDecorators(n) || []).length) || n.members.some((m) => ts.canHaveDecorators(m) && (ts.getDecorators(m) || []).length)) rec.decorated = true;
        if (n.heritageClauses && n.heritageClauses.some((h) => h.token === ts.SyntaxKind.ExtendsKeyword)) rec.extends = true;
        localClasses.push(rec);
        for (const m of n.members) {
          if (ts.isConstructorDeclaration(m)) { ts.forEachChild(m, (k) => visit(k, rec)); }
          else ts.forEachChild(m, (k) => visit(k, null));
        }
        if (n.heritageClauses) n.heritageClauses.forEach((h) => visit(h, null));
        return;
      }
      if (ts.isExportDeclaration(n) && n.exportClause && ts.isNamedExports(n.exportClause)) n.exportClause.elements.forEach((e) => exportedNames.add((e.propertyName || e.name).text));
      if (ts.isExportAssignment(n) && ts.isIdentifier(n.expression)) exportedNames.add(n.expression.text);
      // nested functions inside a ctor: `this` there may differ (arrow keeps it) -> keep the ctor record for arrows only
      if (inCtorOf && (ts.isFunctionDeclaration(n) || ts.isFunctionExpression(n))) { ts.forEachChild(n, (k) => visit(k, null)); return; }
      ts.forEachChild(n, (k) => visit(k, inCtorOf));
    };
    visit(sf, null);
    for (const r of localClasses) {
      if (r.name && exportedNames.has(r.name)) r.exported = true;
      c.classes++; if (r.exported) c.classesExported++; if (r.extends) c.classesExtends++; if (r.decorated) c.classesDecorated++; if (r.thisEscape) c.classesThisEscapeInCtor++;
      if (r.name) classNames.set(r.name, (classNames.get(r.name) || 0) + 1);
    }
    if (fileReflect) c.filesWithReflectMut++;
    if (fileDyn) c.filesWithDynKeyWrite++;
  }
  for (const [nm, k] of classNames) if (patchedNames.has(nm)) c.classesProtoPatched += k;
  return c;
}

const res = {};
for (const [name, list] of Object.entries(PROJECTS)) res[name] = scanProject(name, list());
fs.writeFileSync(path.join(__dirname, "scan.json"), JSON.stringify(res, null, 1));
const perK = (c, k) => (c[k] / (c.loc / 1000)).toFixed(2);
const cols = ["dynKeyWrite", "litKeyWrite", "delete", "reflectMut", "objectAssignAny", "protoPatch", "dunderProto", "evalLike", "withStmt", "proxy", "argumentsUse", "callApplyBind", "forIn"];
console.log(["project", "files", "codeLOC", ...cols.map((k) => k + "/KLOC")].join("\t"));
for (const [n, c] of Object.entries(res)) console.log([n, c.files, c.loc, ...cols.map((k) => perK(c, k))].join("\t"));
console.log("\nproject\tclasses\texported\textends\tdecorated\tthisEscInCtor\tprotoPatched\tfilesReflectMut%\tfilesDynKeyWrite%");
const pct = (a, b) => (b ? ((100 * a) / b).toFixed(1) + "%" : "-");
for (const [n, c] of Object.entries(res)) console.log([n, c.classes, pct(c.classesExported, c.classes), pct(c.classesExtends, c.classes), pct(c.classesDecorated, c.classes), pct(c.classesThisEscapeInCtor, c.classes), c.classesProtoPatched, pct(c.filesWithReflectMut, c.files), pct(c.filesWithDynKeyWrite, c.files)].join("\t"));
