// perf CI v0 workload (10-quality section 2): a fixed mix of T0 paths. The output is checked, so a
// wrong answer cannot pass as a fast run. Keep it small: CI counts it under cachegrind.
function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

function points(n) {
  const out = [];
  for (let i = 0; i < n; i++) out.push({ x: i, y: i * 2 });
  let sum = 0;
  for (const p of out) sum += p.x + p.y;
  return sum;
}

function strings(n) {
  let s = "";
  const abc = "abcdefghijklmnopqrstuvwxyz";
  for (let i = 0; i < n; i++) s += abc[i % 26];
  return s.length;
}

function loop(n) {
  let acc = 0;
  for (let i = 0; i < n; i++) acc = (acc + i * 7) % 1000003;
  return acc;
}

const got = [fib(20), points(5000), strings(2000), loop(100000)].join(" ");
const want = "6765 37492500 2000 545003";
if (got !== want) throw new Error("ci-v0: got " + got + ", want " + want);
print(got);
