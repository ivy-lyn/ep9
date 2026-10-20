// For each sampled qualifying function, list the outer variables it reads or writes
// (closure captures and module variables) and the declared type of each.
// Usage: node captures.cjs <repo> <out.json> <res1.json> [res2.json ...]
"use strict";
const fs = require("fs");
const path = require("path");
const S = "$EP9_RESEARCH";
const ts = require(S + "/work/ts-type-trust/node_modules/typescript");
const FINDER = S + "/work/sound-regions/finder";

const [repo, outp, ...resFiles] = process.argv.slice(2);
const regions = JSON.parse(fs.readFileSync(FINDER + "/regions.json", "utf8")).repos[repo];
const qclass = new Set(regions.qualifying_classes.map((c) => c.file + "|" + c.name));

// functions to check: every qualifying function with samples in any of the result files
const want = new Map();
for (const rf of resFiles) {
  const r = JSON.parse(fs.readFileSync(rf, "utf8"));
  for (const f of r.per_function) if ((f.v0 || f.v3) && f.repo === repo) want.set(f.id, f);
}

const cfgPath = FINDER + "/projects/" + repo + "/tsconfig.json";
const cfg = ts.readConfigFile(cfgPath, ts.sys.readFile);
const parsed = ts.parseJsonConfigFileContent(cfg.config, ts.sys, path.dirname(cfgPath));
const program = ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options });
const checker = program.getTypeChecker();

function byteToChar(text) {
  // UTF-8 byte offset -> UTF-16 index
  const m = new Map();
  let b = 0;
  for (let i = 0; i < text.length; i++) {
    m.set(b, i);
    const c = text.codePointAt(i);
    if (c > 0xffff) {
      b += 4;
      i++;
    } else b += c < 0x80 ? 1 : c < 0x800 ? 2 : 3;
  }
  m.set(b, text.length);
  return m;
}

function isFnLike(n) {
  return ts.isFunctionDeclaration(n) || ts.isMethodDeclaration(n) || ts.isConstructorDeclaration(n) || ts.isGetAccessorDeclaration(n) ||
    ts.isSetAccessorDeclaration(n) || ts.isArrowFunction(n) || ts.isFunctionExpression(n);
}

function findFn(sf, cs, ce) {
  let best = null;
  function visit(n) {
    if (n.pos > ce || n.end < cs) return;
    if (isFnLike(n)) {
      const st = n.getStart(sf);
      if (st <= cs && n.end >= ce) {
        if (!best || n.end - st <= best.end - best.getStart(sf)) best = n;
      }
    }
    ts.forEachChild(n, visit);
  }
  visit(sf);
  return best;
}

const PRIM = ts.TypeFlags.NumberLike | ts.TypeFlags.StringLike | ts.TypeFlags.BooleanLike | ts.TypeFlags.BigIntLike | ts.TypeFlags.EnumLike;
const NULLISH = ts.TypeFlags.Null | ts.TypeFlags.Undefined | ts.TypeFlags.Void;

function tier(t) {
  if (t.flags & PRIM) return 0;
  if (t.isUnion()) {
    let m = 0;
    let nonNull = 0;
    for (const u of t.types) {
      if (u.flags & NULLISH) {
        m = Math.max(m, 1);
        continue;
      }
      nonNull++;
      const x = tier(u);
      if (x === null) return null;
      m = Math.max(m, x);
    }
    return nonNull ? m : 1;
  }
  if (checker.isArrayType(t)) {
    const el = checker.getTypeArguments(t)[0];
    const x = el ? tier(el) : null;
    return x === null ? null : 2;
  }
  const sym = t.getSymbol();
  if (sym && sym.valueDeclaration && ts.isClassLike(sym.valueDeclaration) && sym.valueDeclaration.name) {
    const file = sym.valueDeclaration.getSourceFile().fileName;
    if (qclass.has(file + "|" + sym.valueDeclaration.name.text)) return 0;
  }
  return null;
}

const out = [];
const byFile = new Map();
for (const f of want.values()) {
  if (!byFile.has(f.file)) byFile.set(f.file, []);
  byFile.get(f.file).push(f);
}
for (const [file, fns] of byFile) {
  const sf = program.getSourceFile(file);
  if (!sf) {
    for (const f of fns) out.push({ id: f.id, error: "no source file" });
    continue;
  }
  const b2c = byteToChar(sf.text);
  for (const f of fns) {
    const cs = b2c.get(f.s), ce = b2c.get(f.e);
    const fn = cs !== undefined && ce !== undefined ? findFn(sf, cs, ce) : null;
    if (!fn) {
      out.push({ id: f.id, name: f.name, error: "node not found" });
      continue;
    }
    const st = fn.getStart(sf), en = fn.end;
    const caps = new Map();
    function visit(n) {
      if (ts.isIdentifier(n)) {
        // skip property names in a.b and object literal keys
        const p = n.parent;
        if (p && ts.isPropertyAccessExpression(p) && p.name === n) return;
        if (p && (ts.isPropertyAssignment(p) || ts.isPropertyDeclaration(p) || ts.isMethodDeclaration(p)) && p.name === n) return;
        let sym = checker.getSymbolAtLocation(n);
        if (!sym) return;
        if (p && ts.isShorthandPropertyAssignment(p)) sym = checker.getShorthandAssignmentValueSymbol(p) || sym;
        if (sym.flags & ts.SymbolFlags.Alias) {
          try {
            sym = checker.getAliasedSymbol(sym);
          } catch (e) {
            return;
          }
        }
        if (!(sym.flags & (ts.SymbolFlags.Variable))) return; // only variables / params (functions, classes, enums are constant bindings)
        const d = sym.valueDeclaration || (sym.declarations && sym.declarations[0]);
        if (!d) return;
        const dsf = d.getSourceFile();
        if (dsf.isDeclarationFile) return; // lib globals such as Math, Number
        const ds = d.getStart(dsf);
        if (dsf === sf && ds >= st && ds < en) return; // declared inside the function
        const key = dsf.fileName + ":" + ds;
        if (caps.has(key)) return;
        let isConst = false, where = "closure";
        if (ts.isVariableDeclaration(d) && d.parent && ts.isVariableDeclarationList(d.parent)) {
          isConst = (d.parent.flags & ts.NodeFlags.Const) !== 0;
          // module-level (or namespace-level) variable?
          const stmt = d.parent.parent;
          if (stmt && stmt.parent && (ts.isSourceFile(stmt.parent) || ts.isModuleBlock(stmt.parent))) where = "module";
        } else if (ts.isParameter(d)) where = "outer-param";
        const t = checker.getTypeOfSymbol(sym);
        const ti = tier(t);
        caps.set(key, { name: sym.name, where, const: isConst, tier: ti, type: checker.typeToString(t).slice(0, 80) });
        return;
      }
      ts.forEachChild(n, visit);
    }
    if (fn.body) visit(fn.body);
    for (const prm of fn.parameters) if (prm.initializer) visit(prm.initializer);
    const list = [...caps.values()];
    const untyped = list.filter((c) => c.tier === null);
    out.push({
      id: f.id, name: f.name, file: f.file, line: f.line, kind: fn.kind, v0: f.v0,
      captures: list.length, untyped: untyped.length,
      untyped_mutable: untyped.filter((c) => !c.const).length,
      mutable_typed: list.filter((c) => c.tier !== null && !c.const).length,
      list,
    });
  }
}
fs.writeFileSync(outp, JSON.stringify(out, null, 1));
const n = out.length;
console.log(repo, "checked", n, "no captures", out.filter((x) => x.captures === 0).length, "typed-only", out.filter((x) => x.captures > 0 && x.untyped === 0).length,
  "untyped", out.filter((x) => x.untyped > 0).length, "errors", out.filter((x) => x.error).length);
