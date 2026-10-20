import { format } from '$EP9_RESEARCH_REPOS/date-fns/pkgs/core/src/format/index.ts'
import { parse } from '$EP9_RESEARCH_REPOS/date-fns/pkgs/core/src/parse/index.ts'
import { addDays } from '$EP9_RESEARCH_REPOS/date-fns/pkgs/core/src/addDays/index.ts'
import { differenceInCalendarDays } from '$EP9_RESEARCH_REPOS/date-fns/pkgs/core/src/differenceInCalendarDays/index.ts'
const ms = Number(process.argv[2] ?? 1500)
const base = new Date(2024, 0, 15, 10, 30, 45)
const fmts = ['yyyy-MM-dd', "yyyy-MM-dd'T'HH:mm:ss", 'MM/dd/yyyy hh:mm a', 'EEE, d MMM yyyy']
let sink: unknown, n = 0
const t0 = performance.now()
while (performance.now() - t0 < ms) {
  for (let i = 0; i < 50; i++) {
    const d = addDays(base, i)
    for (const f of fmts) {
      const s = format(d, f)
      const p = parse(s, f, base)
      sink = differenceInCalendarDays(p, base)
      n++
    }
  }
}
console.log('date-fns format+parse', n, (performance.now() - t0).toFixed(0), 'ms', typeof sink)
