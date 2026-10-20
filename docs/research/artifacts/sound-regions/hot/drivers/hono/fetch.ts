import { Hono } from '$EP9_RESEARCH_REPOS/hono/src/index.ts'
import { RegExpRouter } from '$EP9_RESEARCH_REPOS/hono/src/router/reg-exp-router/index.ts'
const app = new Hono({ router: new RegExpRouter() })
app
  .get('/', (c: any) => c.text('Hi'))
  .get('/id/:id', (c: any) => {
    const id = c.req.param('id')
    const name = c.req.query('name')
    c.header('x-powered-by', 'benchmark')
    return c.text(`${id} ${name}`)
  })
  .get('/user', (c: any) => c.json({ id: 123, name: 'Alice', roles: ['admin', 'editor'] }))
  .use('/mw/*', async (_c: any, next: any) => { await next() })
  .get('/mw/hello', (c: any) => c.text('mw'))
const reqs = [new Request('http://localhost/'), new Request('http://localhost/id/1?name=bun'), new Request('http://localhost/user'), new Request('http://localhost/mw/hello')]
const ms = Number(process.argv[2] ?? 1500)
let sink: unknown
const t0 = performance.now()
let n = 0
while (performance.now() - t0 < ms) {
  for (let k = 0; k < 100; k++) { for (const r of reqs) { sink = await app.fetch(r) } }
  n += 400
}
console.log('hono fetch', n, 'requests', (performance.now() - t0).toFixed(0), 'ms', typeof sink)
