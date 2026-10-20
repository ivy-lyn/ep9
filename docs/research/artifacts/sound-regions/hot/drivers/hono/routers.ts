import { RegExpRouter } from '$EP9_RESEARCH_REPOS/hono/src/router/reg-exp-router/index.ts'
import { TrieRouter } from '$EP9_RESEARCH_REPOS/hono/src/router/trie-router/index.ts'
import { PatternRouter } from '$EP9_RESEARCH_REPOS/hono/src/router/pattern-router/index.ts'
const handler = () => {}
const routes: { method: string; path: string }[] = [
  { method: 'GET', path: '/user' }, { method: 'GET', path: '/user/comments' }, { method: 'GET', path: '/user/avatar' },
  { method: 'GET', path: '/user/lookup/username/:username' }, { method: 'GET', path: '/user/lookup/email/:address' },
  { method: 'GET', path: '/event/:id' }, { method: 'GET', path: '/event/:id/comments' }, { method: 'POST', path: '/event/:id/comment' },
  { method: 'GET', path: '/map/:location/events' }, { method: 'GET', path: '/status' },
  { method: 'GET', path: '/very/deeply/nested/route/hello/there' }, { method: 'GET', path: '/static/*' },
]
// lookups from benchmarks/routers/src/bench.mts
const lookups = [
  { method: 'GET', path: '/user' }, { method: 'GET', path: '/user/comments' }, { method: 'GET', path: '/user/lookup/username/hey' },
  { method: 'GET', path: '/event/abcd1234/comments' }, { method: 'GET', path: '/very/deeply/nested/route/hello/there' },
  { method: 'GET', path: '/static/index.html' }, { method: 'POST', path: '/event/abcd1234/comment' },
]
const routers: [string, any][] = [['RegExpRouter', new RegExpRouter()], ['TrieRouter', new TrieRouter()], ['PatternRouter', new PatternRouter()]]
for (const [, r] of routers) for (const rt of routes) r.add(rt.method, rt.path, handler)
const ms = Number(process.argv[2] ?? 1500)
let sink: unknown
for (const [name, r] of routers) {
  const t0 = performance.now(); let n = 0
  while (performance.now() - t0 < ms / 3) { for (let k = 0; k < 1000; k++) for (const l of lookups) sink = r.match(l.method, l.path); n += 1000 * lookups.length }
  console.log(name, n, 'matches', (performance.now() - t0).toFixed(0), 'ms')
}
console.log(typeof sink)
