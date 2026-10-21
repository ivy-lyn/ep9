// Runs one e2e program in node as a classic script, with the host functions of the ep9 CLI.
// Used by tests/e2e.rs to check that every expected output file equals node's output.
//
//   node tests/node_driver.cjs <file.js>
//
// - print(...args): ToString of each argument (a template literal applies ToString), joined by
//   a space, then a newline, written synchronously to stdout (as the ep9 CLI does). A symbol
//   prints as its descriptive string, String(sym) = "Symbol(desc)", as the ep9 CLI does.
// - gc(): no-op.
// - An uncaught exception prints "Uncaught <Name>: <message>" and the stack to stderr, exit 1.
'use strict';
const fs = require('fs');
const vm = require('vm');

const file = process.argv[2];
const source = fs.readFileSync(file, 'utf8');
globalThis.print = function print(...args) {
  const text = (a) => (typeof a === 'symbol' ? String(a) : `${a}`);
  fs.writeSync(1, args.map(text).join(' ') + '\n');
};
globalThis.gc = function gc() {};
try {
  vm.runInThisContext(source, { filename: file });
} catch (e) {
  const head = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
  fs.writeSync(2, `Uncaught ${head}\n`);
  if (e instanceof Error && typeof e.stack === 'string') {
    fs.writeSync(2, `${e.stack}\n`);
  }
  process.exitCode = 1;
}
