// T0 kernel "gen" (P1-GEN-VM): generator suspend and resume. A counting generator under
// for-of, a generator driven by next(v) with sent values, and yield* delegation. Each yield
// is one resume from the native `next` and one SuspendGenerator. It checks its own output.
function* count(n) {
  for (let i = 0; i < n; i++) yield i;
}
function* accumulate() {
  let s = 0;
  for (;;) {
    const v = yield s;
    s = (s + v) & 0xffff;
  }
}
function* both(n) {
  yield* count(n);
  yield* count(n >> 1);
}

function run(n) {
  let s = 0;
  for (const v of count(n)) s = (s + v) % 1000003;
  const a = accumulate();
  a.next();
  for (let i = 0; i < n; i++) s = (s + a.next(i & 255).value) % 1000003;
  for (const v of both(n >> 2)) s = (s * 3 + v) % 1000003;
  return s;
}

let total = 0;
for (let r = 0; r < 4; r++) total = (total * 31 + run(100000 + r)) % 1000003;
const want = 351381;
if (total !== want) throw new Error("t0-gen: got " + total + ", want " + want);
print(total);
