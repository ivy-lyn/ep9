import { DefaultLinesDiffComputer } from '$EP9_RESEARCH_REPOS/vscode/src/vs/editor/common/diff/defaultLinesDiffComputer/defaultLinesDiffComputer.ts'
import { readFileSync } from 'node:fs'
// original: a real source file; modified: deterministic edits (change, delete, insert, move a block)
const orig = readFileSync('$EP9_RESEARCH_REPOS/TypeScript/src/compiler/parser.ts', 'utf8').split('\n').slice(0, 4000)
const mod: string[] = []
let seed = 12345
const rnd = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed / 0x7fffffff }
for (let i = 0; i < orig.length; i++) {
  const r = rnd()
  if (r < 0.02) continue // delete
  if (r < 0.05) { mod.push(orig[i].replace(/[a-z]+/, (w) => w.toUpperCase())); continue } // change
  if (r < 0.06) mod.push('    // inserted line ' + i)
  mod.push(orig[i])
}
mod.splice(3000, 0, ...orig.slice(500, 560)) // copied block (move detection)
const c = new DefaultLinesDiffComputer()
const ms = Number(process.argv[2] ?? 1500)
const t0 = performance.now()
let n = 0, changes = 0
while (performance.now() - t0 < ms) {
  const d = c.computeDiff(orig, mod, { ignoreTrimWhitespace: false, maxComputationTimeMs: 0, computeMoves: true, extendToSubwords: false })
  changes = d.changes.length + d.moves.length
  n++
}
console.log('vscode diff', n, 'runs', changes, 'changes+moves', (performance.now() - t0).toFixed(0), 'ms')
