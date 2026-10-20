import * as z from '$EP9_RESEARCH_REPOS/zod/packages/zod/src/v4/classic/index.ts'
const User = z.object({
  id: z.number().int().nonnegative(),
  name: z.string().min(1).max(100),
  email: z.string().email(),
  tags: z.array(z.string()),
  address: z.object({ street: z.string(), city: z.string(), zip: z.string().regex(/^\d{5}$/) }),
  role: z.enum(['admin', 'user', 'guest']),
  age: z.number().optional(),
  scores: z.array(z.number()),
})
const good = Array.from({ length: 64 }, (_, i) => ({
  id: i, name: 'user' + i, email: 'u' + i + '@example.com', tags: ['a', 'b', 'c'],
  address: { street: i + ' Main St', city: 'Springfield', zip: '12345' }, role: i % 2 ? 'user' : 'admin',
  age: i % 3 ? 30 + i : undefined, scores: [1, 2, 3, 4, 5],
}))
const bad = good.map((g, i) => ({ ...g, email: i % 2 ? 'nope' : g.email, id: i % 3 ? -1 : g.id }))
const ms = Number(process.argv[2] ?? 1500)
let ok = 0, fail = 0
const t0 = performance.now()
while (performance.now() - t0 < ms) {
  for (const g of good) { User.parse(g); ok++ }
  for (const b of bad) { const r = User.safeParse(b); if (!r.success) fail++ }
}
console.log('zod parse', ok, 'ok', fail, 'fail', (performance.now() - t0).toFixed(0), 'ms', 'jit-eval', (z as any).config ? 'n/a' : '')
