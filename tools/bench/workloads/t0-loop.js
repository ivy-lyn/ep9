// T0 dispatch kernel "loop" (H15 ADR): counted loops of small-integer arithmetic; every branch
// is predictable. It checks its own output, so a wrong answer cannot pass as a fast run.
function run(n, k) {
  let acc = k;
  for (let i = 0; i < n; i++) {
    acc = (acc + i * 7) % 1000003;
    acc = acc ^ (i & 255);
    acc = acc - (acc >> 3);
  }
  return acc;
}

let total = 0;
for (let r = 0; r < 8; r++) total = (total * 31 + run(3000000, r)) % 1000003;
const want = 604229;
if (total !== want) throw new Error("t0-loop: got " + total + ", want " + want);
print(total);
